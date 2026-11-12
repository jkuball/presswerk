use crate::analysis::{Analysis, Analyzer, CommitRange, render_release_notes};
use crate::declaration::{Publishers, ReleaseDeclaration, Tag, VersionFile};
use crate::evaluation::{EvaluatedReleaseDeclaration, evaluate_release_declaration};
use crate::publication::{VerifiedPredecessor, verify_predecessor_with_trust};
use crate::release_commit::{
    ReleaseFile, TrustedPredecessor, create_release_commit, release_subject,
};
use crate::version::VersionFileWriter;
use anyhow::{Context, Result, bail};
use git2::{ErrorCode, ObjectType, Oid, Repository, StatusOptions};
use semver::Version;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionFileChange {
    pub path: String,
    pub current: Version,
    pub next: Version,
}

#[derive(Debug, Clone)]
pub struct ReleasePlan {
    repository: PathBuf,
    pub commit: Oid,
    pub current_version: Version,
    pub previous_release: Option<VerifiedPredecessor>,
    pub analysis: Analysis,
    pub next_version: Option<Version>,
    /// The tag that `publish` creates for the next version.
    pub next_tag: Option<String>,
    /// The subject of the release commit for the next tag, in the analyzer's convention.
    pub release_subject: Option<String>,
    pub version_files: Vec<VersionFileChange>,
    pub notes: String,
    pub changelog_path: String,
    pub publishers: Publishers,
    release_files: Vec<ReleaseFile>,
    trusted_predecessor: Option<TrustedPredecessor>,
}

pub fn build_head_release_plan(
    repository_path: impl AsRef<Path>,
    analyzer: &dyn Analyzer,
) -> Result<ReleasePlan> {
    build_head_release_plan_with_trust(repository_path, analyzer, None)
}

pub fn build_head_release_plan_with_trust(
    repository_path: impl AsRef<Path>,
    analyzer: &dyn Analyzer,
    trusted_predecessor: Option<TrustedPredecessor>,
) -> Result<ReleasePlan> {
    let repository_path = repository_path.as_ref();
    let evaluated = evaluate_release_declaration(repository_path, "HEAD")?;
    let date = time::OffsetDateTime::now_utc().date().to_string();
    let mut plan = build_release_plan(
        repository_path,
        evaluated,
        analyzer,
        &date,
        |candidate, previous_version, declaration| {
            verify_predecessor_with_trust(
                repository_path,
                candidate,
                previous_version,
                declaration,
                "origin",
                trusted_predecessor.as_ref(),
            )
            .map_err(Into::into)
        },
    )?;
    plan.trusted_predecessor = trusted_predecessor;
    Ok(plan)
}

pub fn build_release_plan(
    repository_path: impl AsRef<Path>,
    evaluated: EvaluatedReleaseDeclaration,
    analyzer: &dyn Analyzer,
    date: &str,
    mut verify_previous_release: impl FnMut(
        Oid,
        &Version,
        &ReleaseDeclaration,
    ) -> Result<Option<VerifiedPredecessor>>,
) -> Result<ReleasePlan> {
    let repository_path = repository_path.as_ref();
    let repository = Repository::open(repository_path).with_context(|| {
        format!(
            "could not open Git repository {}",
            repository_path.display()
        )
    })?;
    let commit = repository
        .find_commit(evaluated.commit)
        .with_context(|| format!("could not read Git commit {}", evaluated.commit))?;
    let tree = commit.tree().context("could not read candidate tree")?;
    let declaration = evaluated.declaration;

    if declaration.version_files.is_empty() {
        bail!("release declaration must contain at least one version file");
    }
    ensure_version_files_match(
        &repository,
        &tree,
        &declaration.version_files,
        &declaration.version,
    )?;

    ensure_no_higher_reachable_release(&repository, evaluated.commit, &declaration)?;
    let previous_release =
        verify_previous_release(evaluated.commit, &declaration.version, &declaration)?;
    let range = previous_release.as_ref().map_or_else(
        || CommitRange::initial(repository_path, evaluated.commit.to_string()),
        |previous| {
            CommitRange::between(
                repository_path,
                previous.commit.to_string(),
                evaluated.commit.to_string(),
            )
        },
    )?;
    let analysis = analyzer.analyze(&range, &declaration.analyzer, &declaration.tag)?;
    let next_version = analysis.next_version(Some(&declaration.version))?;
    let notes = render_release_notes(&analysis)?;
    let mut version_files = Vec::new();
    let mut release_files = Vec::new();

    if let Some(next_version) = &next_version {
        for writer in &declaration.version_files {
            let (contents, current) = if writer.reads_version() {
                let contents = read_required_text(&repository, &tree, writer.path())?;
                let current = writer.read_version(&contents)?;
                (contents, current)
            } else {
                let contents = read_optional_text(&repository, &tree, writer.path())?;
                (contents.unwrap_or_default(), declaration.version.clone())
            };
            let written = writer.write_version(&contents, next_version)?;
            version_files.push(VersionFileChange {
                path: writer.path().into(),
                current,
                next: next_version.clone(),
            });
            release_files.push(ReleaseFile::new(writer.path(), written));
        }

        let changelog = read_optional_text(&repository, &tree, &declaration.changelog.path)?;
        let changelog = crate::changelog::insert_version_section(
            changelog.as_deref(),
            next_version,
            date,
            &notes,
        )?;
        release_files.push(ReleaseFile::new(&declaration.changelog.path, changelog));
    }

    let next_tag = next_version
        .as_ref()
        .map(|version| declaration.tag.render(version))
        .transpose()?;
    // Catch tags that the next version cannot fill before the release commit uses it up.
    if let Some(version) = &next_version {
        for (name, publisher) in &declaration.publishers.oci {
            crate::oci::render_tags(&publisher.tags, version)
                .map_err(|error| anyhow::anyhow!("oci.{name}: {error}"))?;
        }
    }
    let release_subject = next_tag
        .as_deref()
        .map(|tag| release_subject(&declaration.analyzer, tag));

    Ok(ReleasePlan {
        repository: repository_path.into(),
        commit: evaluated.commit,
        current_version: declaration.version,
        previous_release,
        analysis,
        next_version,
        next_tag,
        release_subject,
        version_files,
        notes,
        changelog_path: declaration.changelog.path,
        publishers: declaration.publishers,
        release_files,
        trusted_predecessor: None,
    })
}

pub fn prepare_release(plan: &ReleasePlan) -> Result<Option<Oid>> {
    let Some(subject) = &plan.release_subject else {
        return Ok(None);
    };

    let repository = Repository::open(&plan.repository).context("could not open Git repository")?;
    let changed = locally_changed_release_files(&repository, &plan.release_files)?;
    if !changed.is_empty() {
        bail!(
            "local changes in {} would be overwritten by the release commit; commit or stash them first",
            changed.join(", ")
        );
    }
    let head = repository.head().context("could not read HEAD")?;
    let actual = head
        .peel_to_commit()
        .context("HEAD does not resolve to a commit")?
        .id();
    if actual != plan.commit {
        bail!(
            "HEAD moved after planning: expected {}, found {actual}",
            plan.commit
        );
    }
    let reference = head.name().unwrap_or("HEAD").to_owned();
    drop(head);

    let commit = create_release_commit(
        &plan.repository,
        &reference,
        plan.commit,
        subject,
        &plan.release_files,
        plan.trusted_predecessor.as_ref(),
    )?;
    check_out_release_files(&repository, &plan.release_files)?;
    Ok(Some(commit))
}

fn locally_changed_release_files(
    repository: &Repository,
    files: &[ReleaseFile],
) -> Result<Vec<String>> {
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .include_ignored(false)
        .disable_pathspec_match(true);
    for file in files {
        options.pathspec(&file.path);
    }
    let statuses = repository
        .statuses(Some(&mut options))
        .context("could not read the working tree status")?;
    statuses
        .iter()
        .map(|entry| Ok(entry.path()?.to_owned()))
        .collect()
}

// The release commit is created from Git objects, so the files it changed are written to the index
// and working tree afterwards. Their local state was checked to be clean before.
fn check_out_release_files(repository: &Repository, files: &[ReleaseFile]) -> Result<()> {
    let Some(workdir) = repository.workdir() else {
        return Ok(());
    };
    let mut index = repository.index().context("could not read the Git index")?;
    for file in files {
        let target = workdir.join(&file.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        fs::write(&target, &file.contents)
            .with_context(|| format!("could not write {}", file.path.display()))?;
        index
            .add_path(&file.path)
            .with_context(|| format!("could not stage {}", file.path.display()))?;
    }
    index.write().context("could not write the Git index")
}

pub fn is_worktree_dirty(repository_path: impl AsRef<Path>) -> Result<bool> {
    let repository = Repository::open(repository_path).context("could not open Git repository")?;
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    Ok(!repository.statuses(Some(&mut options))?.is_empty())
}

#[must_use]
pub fn render_plan(plan: &ReleasePlan) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "commit             {}", plan.commit);
    match &plan.previous_release {
        Some(previous) => {
            let _ = writeln!(
                output,
                "previous release   {} ({})",
                previous.tag, previous.commit
            );
        }
        None => {
            let _ = writeln!(output, "previous release   none (implicit 0.0.0 baseline)");
        }
    }
    let _ = writeln!(output, "analyzed commits   {}", plan.analysis.commits.len());
    for commit in &plan.analysis.commits {
        let short = &commit.id[..commit.id.len().min(7)];
        let _ = writeln!(output, "  {short}  {}", commit.subject);
    }
    match &plan.next_version {
        Some(version) => {
            let _ = writeln!(output, "next version       {version}");
            output.push_str("\nversion files\n");
            for file in &plan.version_files {
                let _ = writeln!(output, "  {}  {} -> {}", file.path, file.current, file.next);
            }
        }
        None => output.push_str("next version       none (no release-worthy commits)\n"),
    }
    if let (Some(tag), Some(version)) = (&plan.next_tag, &plan.next_version) {
        output.push_str("\npublishers\n");
        if plan.publishers.oci.is_empty() && plan.publishers.github_release.is_empty() {
            output.push_str("  none\n");
        }
        for (name, publisher) in &plan.publishers.oci {
            let _ = writeln!(output, "  oci.{name}  {}", publisher.repository);
            // Store paths are left out: the release commit's images get their own paths.
            for platform in publisher.images.keys() {
                let _ = writeln!(output, "    image    {platform}");
            }
            for tag in crate::oci::render_tags(&publisher.tags, version).unwrap_or_default() {
                let kind = match tag {
                    crate::oci::OciTag::Version(_) => "tag",
                    crate::oci::OciTag::Alias(_) => "alias",
                };
                let _ = writeln!(output, "    {kind:<8} {}", tag.name());
            }
        }
        for (name, publisher) in &plan.publishers.github_release {
            let _ = writeln!(output, "  github-release.{name}  {}", publisher.repository);
            let _ = writeln!(output, "    release  {tag}");
            // Store paths are left out: the release commit's artifacts get their own paths.
            for asset in publisher.assets.keys() {
                let _ = writeln!(output, "    asset    {asset}");
            }
        }
    }
    output.push_str("\nnotes\n");
    for line in plan.notes.lines() {
        let _ = writeln!(output, "  {line}");
    }
    match &plan.next_version {
        Some(version) => {
            let _ = writeln!(
                output,
                "\nNothing written. Run `presswerk prepare` to commit {version}."
            );
        }
        None => output.push_str("\nNothing written.\n"),
    }
    output
}

fn ensure_no_higher_reachable_release(
    repository: &Repository,
    target: Oid,
    declaration: &ReleaseDeclaration,
) -> Result<()> {
    for (name, version, commit) in release_tags(repository, &declaration.tag)? {
        if version > declaration.version && is_ancestor(repository, commit, target)? {
            bail!(
                "reachable release tag {name} has version {version}, higher than committed version {}",
                declaration.version
            );
        }
    }
    Ok(())
}

fn release_tags(repository: &Repository, tag: &Tag) -> Result<Vec<(String, Version, Oid)>> {
    let mut releases = Vec::new();
    for name in repository.tag_names(None)?.iter() {
        let Some(name) = name? else {
            continue;
        };
        let Some(version) = tag.parse(name)? else {
            continue;
        };
        let reference = repository
            .find_reference(&format!("refs/tags/{name}"))
            .with_context(|| format!("could not read Git tag {name}"))?;
        let commit = reference
            .peel_to_commit()
            .with_context(|| format!("Git tag {name} does not resolve to a commit"))?
            .id();
        releases.push((name.into(), version, commit));
    }
    Ok(releases)
}

fn is_ancestor(repository: &Repository, ancestor: Oid, target: Oid) -> Result<bool> {
    Ok(ancestor == target || repository.graph_descendant_of(target, ancestor)?)
}

fn ensure_version_files_match(
    repository: &Repository,
    tree: &git2::Tree<'_>,
    version_files: &[VersionFile],
    expected: &Version,
) -> Result<()> {
    let actual = read_version(repository, tree, version_files)?;
    if actual != *expected {
        bail!(
            "release declaration contains version {expected}, but version files contain {actual}"
        );
    }
    Ok(())
}

fn read_version(
    repository: &Repository,
    tree: &git2::Tree<'_>,
    version_files: &[VersionFile],
) -> Result<Version> {
    let mut sources = version_files.iter().filter(|file| file.reads_version());
    let Some(first) = sources.next() else {
        bail!(
            "release declaration must contain at least one version file that carries the version"
        );
    };
    let first_contents = read_required_text(repository, tree, first.path())?;
    let expected = first.read_version(&first_contents)?;
    for writer in sources {
        let contents = read_required_text(repository, tree, writer.path())?;
        let actual = writer.read_version(&contents)?;
        if actual != expected {
            bail!(
                "version files disagree: {} contains {expected}, but {} contains {actual}",
                first.path(),
                writer.path()
            );
        }
    }
    Ok(expected)
}

fn read_required_text(
    repository: &Repository,
    tree: &git2::Tree<'_>,
    path: &str,
) -> Result<String> {
    read_optional_text(repository, tree, path)?
        .with_context(|| format!("commit does not contain required file {path}"))
}

fn read_optional_text(
    repository: &Repository,
    tree: &git2::Tree<'_>,
    path: &str,
) -> Result<Option<String>> {
    let entry = match tree.get_path(Path::new(path)) {
        Ok(entry) => entry,
        Err(error) if error.code() == ErrorCode::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if entry.kind() != Some(ObjectType::Blob) {
        bail!("{path} in commit is not a regular file");
    }
    let blob = repository.find_blob(entry.id())?;
    let contents = std::str::from_utf8(blob.content())
        .with_context(|| format!("{path} in commit is not valid UTF-8"))?;
    Ok(Some(contents.into()))
}
