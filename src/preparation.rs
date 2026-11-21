use crate::analysis::{
    Analysis, Analyzer, CommitRange, render_release_notes, template_uses_unreleased,
};
use crate::declaration::{Publishers, ReleaseDeclaration, Tag};
use crate::evaluation::{EvaluatedReleaseDeclaration, evaluate_release_declaration};
use crate::publication::{Release, VerifiedPredecessor, short_oid, verify_head_predecessor};
use crate::publishers::{Context as PublisherContext, Description, Session, Warning, WarningLevel};
use crate::release_commit::{
    ReleaseFile, TrustedPredecessor, create_release_commit, release_subject,
};
use crate::version::{SourceTree, VersionFileError, VersionFileWriter};
use anyhow::{Context, Result, bail};
use git2::{ErrorCode, ObjectType, Oid, Repository, StatusOptions, TreeWalkMode, TreeWalkResult};
use semver::Version;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    /// What each publisher writes for the next version, by address.
    pub publisher_descriptions: Vec<(String, Description)>,
    release_files: Vec<ReleaseFile>,
    trusted_predecessor: Option<TrustedPredecessor>,
    /// Trust given only by `--trust-predecessor`, which `prepare` needs again.
    trust_argument: Option<TrustedPredecessor>,
    /// What the publishers found on their remotes for the next release before anything is
    /// written, such as an alias another tool pushed.
    pub warnings: Vec<Warning>,
}

impl ReleasePlan {
    /// The previous release if it is a release commit whose tag is not published yet.
    #[must_use]
    pub fn pending_release(&self) -> Option<&VerifiedPredecessor> {
        self.previous_release
            .as_ref()
            .filter(|previous| previous.pending)
    }
}

/// Plan the release of `HEAD` for `plan`. A pending release below `HEAD` is the base of the plan.
/// `trusted_predecessor` comes from `--trust-predecessor`.
pub fn build_head_release_plan(
    repository_path: impl AsRef<Path>,
    analyzer: &dyn Analyzer,
    trusted_predecessor: Option<TrustedPredecessor>,
) -> Result<ReleasePlan> {
    build_head_plan(
        repository_path.as_ref(),
        analyzer,
        trusted_predecessor,
        false,
    )
}

/// Plan the release of `HEAD` for `prepare`. A pending release below `HEAD` is refused before the
/// next release is computed, so no error about that hypothetical release hides the refusal.
/// `trusted_predecessor` comes from `--trust-predecessor`.
pub fn build_head_preparation_plan(
    repository_path: impl AsRef<Path>,
    analyzer: &dyn Analyzer,
    trusted_predecessor: Option<TrustedPredecessor>,
) -> Result<ReleasePlan> {
    build_head_plan(
        repository_path.as_ref(),
        analyzer,
        trusted_predecessor,
        true,
    )
}

fn build_head_plan(
    repository_path: &Path,
    analyzer: &dyn Analyzer,
    trusted_predecessor: Option<TrustedPredecessor>,
    refuse_pending_base: bool,
) -> Result<ReleasePlan> {
    let evaluated = evaluate_release_declaration(repository_path, "HEAD")?;
    // Read the secrets first, so that nothing printed from here on can show one.
    let secrets = crate::publishers::shared::secrets::Secrets::resolve(
        evaluated.declaration.publishers.secrets(),
    );
    let declares_trust = evaluated.declaration.trusted_predecessor.is_some();
    let (trusted_predecessor, other_entry) =
        resolve_trusted_predecessor(&evaluated, trusted_predecessor)?;
    let mut trust_warning = None;
    let date = time::OffsetDateTime::now_utc().date().to_string();
    let mut plan = build_release_plan(
        repository_path,
        evaluated,
        analyzer,
        &date,
        |candidate, previous_version, declaration| {
            let verified = verify_head_predecessor(
                repository_path,
                candidate,
                previous_version,
                declaration,
                "origin",
                trusted_predecessor.as_ref(),
            );
            if let Some(entry) = &other_entry {
                let previous_tag = declaration.tag.render(previous_version)?;
                trust_warning = Some(older_entry_warning(
                    repository_path,
                    entry,
                    &previous_tag,
                    &verified,
                )?);
            }
            let verified = verified?;
            if refuse_pending_base
                && let Some(pending) = verified.as_ref().filter(|previous| previous.pending)
                && pending.commit != candidate
            {
                return Err(pending_refusal(pending));
            }
            Ok(verified)
        },
    )?;
    plan.trust_argument = trusted_predecessor.clone().filter(|_| !declares_trust);
    plan.trusted_predecessor = trusted_predecessor;
    let session = plan_session(&plan, secrets);
    preview_publishers(&mut plan, &session)?;
    plan.warnings.splice(0..0, trust_warning);
    Ok(plan)
}

/// The trust in the previous release from `--trust-predecessor` and the `trustedPredecessor`
/// option, which must name the same release if both are given; and, apart from it, an option entry
/// whose tag is not the previous release's.
///
/// The option applies only to the direct predecessor. One that names another tag must name an
/// older release, such as the one the previous release commit trusted, and then has no effect.
fn resolve_trusted_predecessor(
    evaluated: &EvaluatedReleaseDeclaration,
    flag: Option<TrustedPredecessor>,
) -> Result<(Option<TrustedPredecessor>, Option<TrustedPredecessor>)> {
    let declaration = &evaluated.declaration;
    let Some(declared) = declaration
        .trusted_predecessor
        .as_ref()
        .map(TrustedPredecessor::from_declaration)
        .transpose()
        .context("invalid trustedPredecessor in the release declaration")?
    else {
        return Ok((flag, None));
    };
    if let Some(flag) = flag {
        if flag != declared {
            bail!(
                "--trust-predecessor {flag} differs from trustedPredecessor {declared} in the release declaration; give one of them, or the same release in both"
            );
        }
        return Ok((Some(flag), None));
    }
    let previous_tag = declaration.tag.render(&declaration.version)?;
    if declared.tag == previous_tag {
        return Ok((Some(declared), None));
    }
    Ok((None, Some(declared)))
}

/// The warning for an option entry that names an older release than the predecessor Presswerk
/// found, published, pending, or unreadable; an error if it names no such release.
fn older_entry_warning(
    repository_path: &Path,
    entry: &TrustedPredecessor,
    previous_tag: &str,
    verified: &Result<Option<VerifiedPredecessor>, crate::publication::PublicationError>,
) -> Result<Warning> {
    let predecessor = match verified {
        Ok(verified) => verified.as_ref().map(|previous| previous.commit),
        Err(error) => match error.as_unreadable_predecessor() {
            Some(unreadable) => Some(unreadable.commit),
            // The predecessor could not be established; that error is the one to report.
            None => return Err(error.clone().into()),
        },
    };
    let repository = Repository::open(repository_path).context("could not open Git repository")?;
    let older = match predecessor {
        Some(predecessor) => {
            entry.commit != predecessor
                && repository.find_commit(entry.commit).is_ok()
                && repository
                    .graph_descendant_of(predecessor, entry.commit)
                    .context("could not check the trusted predecessor's ancestry")?
        }
        None => false,
    };
    if !older {
        bail!(
            "trustedPredecessor {entry} in the release declaration names neither the previous release {previous_tag} nor an older one"
        );
    }
    Ok(Warning {
        level: WarningLevel::Warning,
        message: format!(
            "trustedPredecessor {entry} names an older release than the previous release {previous_tag}; it has no effect and can be removed"
        ),
    })
}

fn plan_session(
    plan: &ReleasePlan,
    secrets: crate::publishers::shared::secrets::Secrets,
) -> Session {
    Session {
        changelog: plan.changelog_path.clone(),
        // A trusted predecessor released by another tool leaves aliases the release takes over.
        trusted_predecessor: plan
            .trusted_predecessor
            .as_ref()
            .and(plan.previous_release.as_ref())
            .map(|previous| previous.version.clone()),
        secrets,
    }
}

/// Let every publisher read its remote for the next release before anything is written, so
/// something another tool left there does not stay behind unnoticed. Reading is best effort and
/// never stops the plan.
fn preview_publishers(plan: &mut ReleasePlan, session: &Session) -> Result<()> {
    // `publish` stops without them; `plan` and `prepare` only tell.
    let warnings = Mutex::new(
        session
            .secrets
            .missing()
            .into_iter()
            .map(|(path, source)| Warning {
                level: WarningLevel::Warning,
                message: format!(
                    "secret {source} for publishers.{path} is not set; publish stops before writing anything without it"
                ),
            })
            .collect::<Vec<_>>(),
    );
    if let (Some(version), Some(tag)) = (&plan.next_version, &plan.next_tag) {
        let release = Release {
            commit: plan.commit.to_string(),
            version: version.clone(),
            tag: tag.clone(),
            notes: plan.notes.clone(),
        };
        let warn = |warning: Warning| {
            warnings
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(warning);
        };
        for node in plan.publishers.nodes()? {
            let ctx = PublisherContext::new(session, &node.name).with_warnings(&warn);
            node.publisher.preview(&release, &ctx);
        }
    }
    plan.warnings = warnings
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(())
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
    let source_tree = GitSourceTree {
        repository: &repository,
        tree: &tree,
    };
    let writers = declaration
        .version_files
        .iter()
        .map(|file| file.writer(&source_tree))
        .collect::<Result<Vec<_>, _>>()?;
    ensure_version_files_match(&repository, &tree, &writers, &declaration.version)?;

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
    let next_version = analysis.next_version(Some(&declaration.version), declaration.stable)?;
    let heading = declaration.changelog.heading()?;
    let changelog = read_optional_text(&repository, &tree, &declaration.changelog.path)?;
    // A template without `unreleased` leaves the hand-written section untouched.
    let unreleased_heading = template_uses_unreleased(&declaration.changelog.template)
        .then_some(declaration.changelog.unreleased_heading.as_str());
    let unreleased = changelog.as_deref().zip(unreleased_heading).map_or(
        "",
        |(changelog, unreleased_heading)| {
            crate::changelog::read_unreleased(changelog, &heading, unreleased_heading)
        },
    );
    // `HEAD` is a pending release itself: nothing follows it, so there are no notes to render.
    let at_pending_release = previous_release
        .as_ref()
        .is_some_and(|previous| previous.pending && previous.commit == evaluated.commit);
    let notes = if at_pending_release {
        String::new()
    } else {
        render_release_notes(
            &analysis,
            next_version.as_ref().unwrap_or(&declaration.version),
            unreleased,
            &declaration.changelog.template,
        )?
    };
    let mut version_files = Vec::new();
    let mut release_files = Vec::new();

    if let Some(next_version) = &next_version {
        let mut written_files: Vec<(&str, String)> = Vec::new();
        for writer in &writers {
            let (contents, current) = if writer.reads_version() {
                let contents = read_required_text(&repository, &tree, writer.path())?;
                let current = writer.read_version(&contents)?;
                (contents, current)
            } else {
                let contents = read_optional_text(&repository, &tree, writer.path())?;
                (contents.unwrap_or_default(), declaration.version.clone())
            };
            // Several selectors may write one file, such as both versions in package-lock.json;
            // each later writer edits what the earlier ones wrote.
            let earlier = written_files
                .iter_mut()
                .find(|(path, _): &&mut (&str, String)| *path == writer.path());
            let written = writer.write_version(
                earlier.as_ref().map_or(&contents, |(_, written)| written),
                next_version,
            )?;
            version_files.push(VersionFileChange {
                path: writer.path().into(),
                current,
                next: next_version.clone(),
            });
            match earlier {
                Some((_, contents)) => *contents = written,
                None => written_files.push((writer.path(), written)),
            }
        }

        release_files.extend(
            written_files
                .into_iter()
                .map(|(path, contents)| ReleaseFile::new(path, contents)),
        );

        let changelog = crate::changelog::insert_version_section(
            changelog.as_deref(),
            &heading,
            unreleased_heading,
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
    // Every publisher must be able to express the next version, such as a tag template that
    // still renders a valid tag.
    let mut publisher_descriptions = Vec::new();
    if let (Some(version), Some(tag)) = (&next_version, &next_tag) {
        for node in declaration.publishers.nodes()? {
            let description = node
                .publisher
                .describe(version, tag)
                .map_err(|error| anyhow::anyhow!("{}: {error}", node.name))?;
            publisher_descriptions.push((node.name, description));
        }
        crate::publishers::check_claims(&declaration.publishers, version)
            .map_err(anyhow::Error::msg)?;
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
        publisher_descriptions,
        release_files,
        trusted_predecessor: None,
        trust_argument: None,
        warnings: Vec::new(),
    })
}

/// Create the release commit of the plan, or return `None` if there is nothing to prepare, such as
/// when `HEAD` is itself a pending release.
pub fn prepare_release(plan: &ReleasePlan) -> Result<Option<Oid>> {
    if let Some(pending) = plan.pending_release() {
        if pending.commit == plan.commit {
            return Ok(None);
        }
        return Err(pending_refusal(pending));
    }
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

fn pending_refusal(pending: &VerifiedPredecessor) -> anyhow::Error {
    anyhow::anyhow!(
        "pending release {} at {} is not published yet; publish it before preparing the next one",
        pending.tag,
        short_oid(pending.commit)
    )
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
        Some(previous) if previous.pending => {
            let _ = writeln!(
                output,
                "previous release   {} ({}, pending: not published yet)",
                previous.tag, previous.commit
            );
        }
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
    if plan.next_tag.is_some() && plan.next_version.is_some() {
        output.push_str("\npublishers\n");
        if plan.publisher_descriptions.is_empty() {
            output.push_str("  none\n");
        }
        for (address, description) in &plan.publisher_descriptions {
            let _ = writeln!(output, "  {address}  {}", description.target);
            for (kind, item) in &description.items {
                let _ = writeln!(output, "    {kind:<8} {item}");
            }
        }
    }
    output.push_str("\nnotes\n");
    for line in plan.notes.lines() {
        let _ = writeln!(output, "  {line}");
    }
    // `prepare` refuses to build on a pending release, so it is not suggested then.
    match &plan.next_version {
        Some(version) if plan.pending_release().is_none() => {
            let trust = plan
                .trust_argument
                .as_ref()
                .map(|trust| format!(" {}", trust.flag()))
                .unwrap_or_default();
            let _ = writeln!(
                output,
                "\nNothing written. Run `presswerk prepare{trust}` to commit {version}."
            );
        }
        _ => output.push_str("\nNothing written.\n"),
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
    version_files: &[Box<dyn VersionFileWriter + '_>],
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
    version_files: &[Box<dyn VersionFileWriter + '_>],
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

/// The files of the candidate commit, for version files that read beyond their own file.
struct GitSourceTree<'a> {
    repository: &'a Repository,
    tree: &'a git2::Tree<'a>,
}

impl SourceTree for GitSourceTree<'_> {
    fn read(&self, path: &str) -> Result<Option<String>, VersionFileError> {
        read_optional_text(self.repository, self.tree, path)
            .map_err(|error| VersionFileError::Invalid(format!("{error:#}")))
    }

    fn directories(&self) -> Result<Vec<String>, VersionFileError> {
        let mut directories = Vec::new();
        self.tree
            .walk(TreeWalkMode::PreOrder, |root, entry| {
                if entry.kind() == Some(ObjectType::Tree)
                    && let Ok(name) = entry.name()
                {
                    directories.push(format!("{root}{name}"));
                }
                TreeWalkResult::Ok
            })
            .map_err(|error| {
                VersionFileError::Invalid(format!(
                    "could not list the commit's directories: {error}"
                ))
            })?;
        Ok(directories)
    }
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
