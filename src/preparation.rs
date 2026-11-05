use crate::analysis::{Analysis, Analyzer, CommitRange, render_release_notes};
use crate::declaration::{ReleaseDeclaration, Tag, VersionFile};
use crate::evaluation::{EvaluatedReleaseDeclaration, evaluate_release_declaration};
use crate::publication::{VerifiedPredecessor, verify_predecessor};
use crate::release_commit::{ReleaseFile, create_release_commit};
use crate::version::VersionFileWriter;
use anyhow::{Context, Result, bail};
use git2::{ErrorCode, ObjectType, Oid, Repository, StatusOptions};
use semver::Version;
use std::fmt::Write as _;
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
    pub version_files: Vec<VersionFileChange>,
    pub notes: String,
    release_files: Vec<ReleaseFile>,
}

pub fn build_head_release_plan(
    repository_path: impl AsRef<Path>,
    analyzer: &dyn Analyzer,
) -> Result<ReleasePlan> {
    let repository_path = repository_path.as_ref();
    let evaluated = evaluate_release_declaration(repository_path, "HEAD")?;
    let date = time::OffsetDateTime::now_utc().date().to_string();
    build_release_plan(
        repository_path,
        evaluated,
        analyzer,
        &date,
        |candidate, previous_version, declaration| {
            verify_predecessor(
                repository_path,
                candidate,
                previous_version,
                declaration,
                "origin",
            )
            .map_err(Into::into)
        },
    )
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
    let analysis = analyzer.analyze(&range)?;
    let next_version = analysis.next_version(Some(&declaration.version))?;
    let notes = render_release_notes(&analysis)?;
    let mut version_files = Vec::new();
    let mut release_files = Vec::new();

    if let Some(next_version) = &next_version {
        for writer in &declaration.version_files {
            let contents = read_required_text(&repository, &tree, writer.path())?;
            let current = writer.read_version(&contents)?;
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

    Ok(ReleasePlan {
        repository: repository_path.into(),
        commit: evaluated.commit,
        current_version: declaration.version,
        previous_release,
        analysis,
        next_version,
        version_files,
        notes,
        release_files,
    })
}

pub fn prepare_release(plan: &ReleasePlan) -> Result<Option<Oid>> {
    if is_worktree_dirty(&plan.repository)? {
        bail!("working tree is dirty; commit or stash your changes first");
    }
    let Some(version) = &plan.next_version else {
        return Ok(None);
    };

    let repository = Repository::open(&plan.repository).context("could not open Git repository")?;
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

    create_release_commit(
        &plan.repository,
        &reference,
        plan.commit,
        version,
        &plan.release_files,
    )
    .map(Some)
    .map_err(Into::into)
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
    let Some(first) = version_files.first() else {
        bail!("release declaration must contain at least one version file");
    };
    let first_contents = read_required_text(repository, tree, first.path())?;
    let expected = first.read_version(&first_contents)?;
    for writer in &version_files[1..] {
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
