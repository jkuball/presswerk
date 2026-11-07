use crate::declaration::{GitmojiCatalog, GitmojiCatalogEntry, strip_variation_selectors};
use git_cliff_core::repo::Repository as GitCliffRepository;
use git2::{Oid, Repository as GitRepository};
use minijinja::{Environment, context, value::Serde};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

const DEFAULT_NOTES_TEMPLATE: &str = include_str!("release-notes.md.j2");
const REVERT_PREFIX: &str = "This reverts commit ";
const MAINLINE_PREFIX: &str = "changes made to ";
// Git's shortest abbreviation up to a full SHA-256 object ID.
const COMMIT_ID_LENGTHS: RangeInclusive<usize> = 4..=64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gitmoji {
    pub emoji: String,
    pub shortcode: String,
    pub description: String,
    pub bump: Option<Bump>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParsedGitmojiSubject {
    pub gitmoji: Gitmoji,
    pub scope: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnalyzedCommit {
    pub id: String,
    pub subject: String,
    pub gitmoji: Gitmoji,
    pub scope: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    pub bump: Option<Bump>,
    pub commits: Vec<AnalyzedCommit>,
}

impl Analysis {
    #[must_use]
    pub fn from_commits(commits: Vec<AnalyzedCommit>) -> Self {
        let bump = commits
            .iter()
            .filter_map(|commit| commit.gitmoji.bump)
            .max();
        Self { bump, commits }
    }

    pub fn next_version(&self, current: Option<&Version>) -> Result<Option<Version>, VersionError> {
        self.bump
            .map(|bump| next_version(current, bump))
            .transpose()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRange {
    repository: PathBuf,
    base: Option<String>,
    target: String,
}

impl CommitRange {
    pub fn initial(
        repository: impl Into<PathBuf>,
        target: impl Into<String>,
    ) -> Result<Self, AnalysisError> {
        let repository = repository.into();
        let target = resolve_commit(&repository, &target.into())?;
        Ok(Self {
            repository,
            base: None,
            target,
        })
    }

    pub fn between(
        repository: impl Into<PathBuf>,
        base: impl Into<String>,
        target: impl Into<String>,
    ) -> Result<Self, AnalysisError> {
        let repository = repository.into();
        let base = resolve_commit(&repository, &base.into())?;
        let target = resolve_commit(&repository, &target.into())?;
        Ok(Self {
            repository,
            base: Some(base),
            target,
        })
    }

    #[must_use]
    pub fn repository(&self) -> &Path {
        &self.repository
    }

    #[must_use]
    pub fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }

    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    fn revision(&self) -> String {
        self.base.as_ref().map_or_else(
            || self.target.clone(),
            |base| format!("{base}..{}", self.target),
        )
    }
}

fn resolve_commit(repository: &Path, revision: &str) -> Result<String, AnalysisError> {
    let repository = GitRepository::open(repository).map_err(|error| {
        AnalysisError::new(format!(
            "failed to open Git repository {}: {error}",
            repository.display()
        ))
    })?;
    let object = repository.revparse_single(revision).map_err(|error| {
        AnalysisError::new(format!(
            "failed to resolve Git revision {revision:?}: {error}"
        ))
    })?;
    let commit = object.peel_to_commit().map_err(|error| {
        AnalysisError::new(format!(
            "Git revision {revision:?} does not resolve to a commit: {error}"
        ))
    })?;
    Ok(commit.id().to_string())
}

pub trait Analyzer {
    fn analyze(
        &self,
        range: &CommitRange,
        catalog: &GitmojiCatalog,
    ) -> Result<Analysis, AnalysisError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GitCliffAnalyzer;

impl Analyzer for GitCliffAnalyzer {
    fn analyze(
        &self,
        range: &CommitRange,
        catalog: &GitmojiCatalog,
    ) -> Result<Analysis, AnalysisError> {
        let repository = GitCliffRepository::init(range.repository.clone()).map_err(|error| {
            AnalysisError::new(format!(
                "failed to open Git repository {}: {error}",
                range.repository.display()
            ))
        })?;
        let revision = range.revision();
        let commits = repository
            .commits(Some(&revision), None, None, true)
            .map_err(|error| {
                AnalysisError::new(format!("failed to read commit range {revision:?}: {error}"))
            })?;
        let git = GitRepository::open(&range.repository).map_err(|error| {
            AnalysisError::new(format!(
                "failed to open Git repository {}: {error}",
                range.repository.display()
            ))
        })?;

        let in_range: HashSet<Oid> = commits.iter().map(git2::Commit::id).collect();
        let mut visited = HashSet::new();
        let mut suppressed = HashSet::<Oid>::new();
        let mut analyzed = Vec::new();

        // Topological order visits every commit before its parents, so a revert is always
        // decided before the commits it reverts.
        for commit in &commits {
            visited.insert(commit.id());
            if suppressed.contains(&commit.id()) || commit.parent_count() > 1 {
                continue;
            }

            let message = commit.message().unwrap_or_default();
            let Some(subject) = message.lines().next().map(str::trim_end) else {
                continue;
            };

            let Some(reverted) = reverted_commit(message) else {
                if let Some(entry) = analyzed_commit(commit.id(), subject, catalog) {
                    analyzed.push(entry);
                }
                continue;
            };

            let target = resolve_commit_id(&git, commit.id(), reverted.commit)?;
            if !in_range.contains(&target.id()) {
                if let Some(entry) = reverted_outside_range(commit.id(), subject, catalog) {
                    analyzed.push(entry);
                }
                continue;
            }
            if visited.contains(&target.id()) {
                return Err(AnalysisError::new(format!(
                    "revert {} targets commit {} which is not one of its ancestors",
                    commit.id(),
                    target.id()
                )));
            }
            if target.parent_count() > 1 || reverted.mainline.is_some() {
                ensure_first_parent_mainline(&git, commit.id(), &target, reverted.mainline)?;
            }
            suppressed.extend(introduced_commits(&git, &target)?.intersection(&in_range));
        }

        Ok(Analysis::from_commits(analyzed))
    }
}

fn analyzed_commit(id: Oid, subject: &str, catalog: &GitmojiCatalog) -> Option<AnalyzedCommit> {
    let parsed = parse_gitmoji_subject(subject, catalog)?;
    Some(AnalyzedCommit {
        id: id.to_string(),
        subject: subject.to_owned(),
        gitmoji: parsed.gitmoji,
        scope: parsed.scope,
        message: parsed.message,
    })
}

/// Records a revert whose target was released earlier under the catalog's revert intention,
/// keeping the revert's own subject as the message.
fn reverted_outside_range(
    id: Oid,
    subject: &str,
    catalog: &GitmojiCatalog,
) -> Option<AnalyzedCommit> {
    let Some(entry) = catalog.get("rewind") else {
        return analyzed_commit(id, subject, catalog);
    };
    Some(AnalyzedCommit {
        id: id.to_string(),
        subject: format!("{} {subject}", entry.emoji),
        gitmoji: gitmoji("rewind", entry),
        scope: None,
        message: subject.to_owned(),
    })
}

/// The commit a standard `git revert` message body names, as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RevertedCommit<'a> {
    commit: &'a str,
    mainline: Option<&'a str>,
}

fn reverted_commit(message: &str) -> Option<RevertedCommit<'_>> {
    // Only the body's first line identifies a revert; later lines may quote other messages.
    let mut lines = message
        .lines()
        .skip(1)
        .skip_while(|line| line.trim().is_empty());
    let rest = lines.next()?.strip_prefix(REVERT_PREFIX)?;
    let commit = leading_token(rest);
    let mainline = rest[commit.len()..]
        .starts_with(", reversing")
        .then(|| {
            lines
                .next()?
                .strip_prefix(MAINLINE_PREFIX)
                .map(leading_token)
        })
        .flatten();
    Some(RevertedCommit { commit, mainline })
}

fn leading_token(text: &str) -> &str {
    let end = text
        .find(|character: char| character.is_whitespace() || matches!(character, '.' | ','))
        .unwrap_or(text.len());
    &text[..end]
}

/// The first-parent formula for merged commits only holds for reverts that record the first
/// parent as their mainline.
fn ensure_first_parent_mainline(
    repository: &GitRepository,
    revert: Oid,
    target: &git2::Commit<'_>,
    mainline: Option<&str>,
) -> Result<(), AnalysisError> {
    let Some(mainline) = mainline else {
        return Err(AnalysisError::new(format!(
            "revert {revert} reverts merge {} without recording its mainline parent",
            target.id()
        )));
    };
    let mainline = resolve_commit_id(repository, revert, mainline)?;
    if target.parent_count() < 2 || target.parent_id(0).ok() != Some(mainline.id()) {
        return Err(AnalysisError::new(format!(
            "revert {revert} reverts merge {} relative to {}, which is not its first parent",
            target.id(),
            mainline.id()
        )));
    }
    Ok(())
}

fn resolve_commit_id<'repo>(
    repository: &'repo GitRepository,
    revert: Oid,
    id: &str,
) -> Result<git2::Commit<'repo>, AnalysisError> {
    if !COMMIT_ID_LENGTHS.contains(&id.len())
        || !id.chars().all(|character| character.is_ascii_hexdigit())
    {
        return Err(AnalysisError::new(format!(
            "revert {revert} names malformed commit ID {id:?}"
        )));
    }
    repository.find_commit_by_prefix(id).map_err(|error| {
        AnalysisError::new(format!(
            "revert {revert} names commit {id:?}, which does not resolve to exactly one commit: {error}"
        ))
    })
}

/// The commits a reverted commit contributed: the commit itself, or for a merge every commit
/// that its secondary parents introduced over its first parent.
fn introduced_commits(
    repository: &GitRepository,
    commit: &git2::Commit<'_>,
) -> Result<HashSet<Oid>, AnalysisError> {
    if commit.parent_count() < 2 {
        return Ok(HashSet::from([commit.id()]));
    }

    let walk = || -> Result<HashSet<Oid>, git2::Error> {
        let mut revwalk = repository.revwalk()?;
        for parent in commit.parent_ids().skip(1) {
            revwalk.push(parent)?;
        }
        revwalk.hide(commit.parent_id(0)?)?;
        revwalk.collect()
    };
    walk().map_err(|error| {
        AnalysisError::new(format!(
            "failed to read the commits merged by {}: {error}",
            commit.id()
        ))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisError {
    message: String,
}

impl AnalysisError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AnalysisError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    ComponentOverflow(&'static str),
}

impl fmt::Display for VersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ComponentOverflow(component) => {
                write!(
                    formatter,
                    "semantic version {component} component overflowed"
                )
            }
        }
    }
}

impl Error for VersionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError {
    message: String,
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for RenderError {}

#[must_use]
pub fn parse_gitmoji_subject(
    subject: &str,
    catalog: &GitmojiCatalog,
) -> Option<ParsedGitmojiSubject> {
    let subject = subject.lines().next()?.trim_end();
    let separator = subject.find(char::is_whitespace)?;
    let intention = &subject[..separator];
    let remainder = subject[separator..].trim_start();
    if remainder.is_empty() {
        return None;
    }

    let (shortcode, entry) = catalog.iter().find(|(shortcode, entry)| {
        if let Some(shortcode_subject) = intention
            .strip_prefix(':')
            .and_then(|value| value.strip_suffix(':'))
        {
            shortcode_subject == shortcode.as_str()
        } else {
            strip_variation_selectors(intention) == strip_variation_selectors(&entry.emoji)
        }
    })?;
    let (scope, message) = parse_scope_and_message(remainder)?;

    Some(ParsedGitmojiSubject {
        gitmoji: gitmoji(shortcode, entry),
        scope,
        message: message.to_owned(),
    })
}

fn parse_scope_and_message(remainder: &str) -> Option<(Option<String>, &str)> {
    if let Some(scoped) = remainder.strip_prefix('(') {
        let scope_end = scoped.find(')')?;
        let scope = scoped[..scope_end].trim();
        if scope.is_empty() {
            return None;
        }

        let suffix = &scoped[scope_end + 1..];
        let suffix = suffix.strip_prefix(':').unwrap_or(suffix);
        if !suffix.starts_with(char::is_whitespace) {
            return None;
        }
        let message = suffix.trim_start();
        if message.is_empty() {
            return None;
        }
        return Some((Some(scope.to_owned()), message));
    }

    let message = if let Some(message) = remainder.strip_prefix(':') {
        if !message.starts_with(char::is_whitespace) {
            return None;
        }
        message.trim_start()
    } else {
        remainder
    };

    (!message.is_empty()).then_some((None, message))
}

fn gitmoji(shortcode: &str, entry: &GitmojiCatalogEntry) -> Gitmoji {
    Gitmoji {
        emoji: entry.emoji.clone(),
        shortcode: format!(":{shortcode}:"),
        description: entry.description.clone(),
        bump: entry.semver,
    }
}

pub fn next_version(current: Option<&Version>, bump: Bump) -> Result<Version, VersionError> {
    let baseline = Version::new(0, 0, 0);
    let current = current.unwrap_or(&baseline);

    if current.major == 0 {
        return match bump {
            Bump::Patch => Ok(Version::new(
                0,
                current.minor,
                increment(current.patch, "patch")?,
            )),
            Bump::Minor | Bump::Major => Ok(Version::new(0, increment(current.minor, "minor")?, 0)),
        };
    }

    match bump {
        Bump::Patch => Ok(Version::new(
            current.major,
            current.minor,
            increment(current.patch, "patch")?,
        )),
        Bump::Minor => Ok(Version::new(
            current.major,
            increment(current.minor, "minor")?,
            0,
        )),
        Bump::Major => Ok(Version::new(increment(current.major, "major")?, 0, 0)),
    }
}

fn increment(value: u64, component: &'static str) -> Result<u64, VersionError> {
    value
        .checked_add(1)
        .ok_or(VersionError::ComponentOverflow(component))
}

#[derive(Serialize)]
struct NoteGroup<'a> {
    emoji: &'a str,
    shortcode: &'a str,
    description: &'a str,
    commits: Vec<&'a AnalyzedCommit>,
}

pub fn render_release_notes(analysis: &Analysis) -> Result<String, RenderError> {
    let mut groups = BTreeMap::<&str, NoteGroup<'_>>::new();
    for commit in &analysis.commits {
        groups
            .entry(&commit.gitmoji.shortcode)
            .or_insert_with(|| NoteGroup {
                emoji: &commit.gitmoji.emoji,
                shortcode: &commit.gitmoji.shortcode,
                description: commit.gitmoji.description.trim_end_matches('.'),
                commits: Vec::new(),
            })
            .commits
            .push(commit);
    }

    let mut groups: Vec<_> = groups.into_values().collect();
    groups.sort_by(|left, right| {
        right
            .commits
            .len()
            .cmp(&left.commits.len())
            .then_with(|| left.shortcode.cmp(right.shortcode))
    });

    Environment::new()
        .render_str(DEFAULT_NOTES_TEMPLATE, context!(groups => Serde(&groups)))
        .map_err(|error| RenderError {
            message: format!("failed to render release notes: {error}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(emoji: &str, description: &str, semver: Option<Bump>) -> GitmojiCatalogEntry {
        GitmojiCatalogEntry {
            emoji: emoji.into(),
            description: description.into(),
            semver,
        }
    }

    fn catalog() -> GitmojiCatalog {
        BTreeMap::from([
            (
                "boom".into(),
                entry("💥", "Introduce breaking changes.", Some(Bump::Major)),
            ),
            ("bug".into(), entry("🐛", "Fix a bug.", Some(Bump::Patch))),
            (
                "memo".into(),
                entry("📝", "Add or update documentation.", None),
            ),
            ("recycle".into(), entry("♻️", "Refactor code.", None)),
            (
                "sparkles".into(),
                entry("✨", "Introduce new features.", Some(Bump::Minor)),
            ),
            (
                "zap".into(),
                entry("⚡️", "Improve performance.", Some(Bump::Patch)),
            ),
        ])
    }

    fn commit(id: &str, subject: &str) -> AnalyzedCommit {
        let parsed = parse_gitmoji_subject(subject, &catalog()).unwrap();
        AnalyzedCommit {
            id: id.into(),
            subject: subject.into(),
            gitmoji: parsed.gitmoji,
            scope: parsed.scope,
            message: parsed.message,
        }
    }

    #[test]
    fn parses_unicode_and_shortcode_subjects() {
        let catalog = catalog();
        let unicode = parse_gitmoji_subject("⚡ Improve startup time", &catalog).unwrap();
        assert_eq!(unicode.gitmoji.shortcode, ":zap:");
        assert_eq!(unicode.gitmoji.bump, Some(Bump::Patch));
        assert_eq!(unicode.scope, None);
        assert_eq!(unicode.message, "Improve startup time");

        let shortcode =
            parse_gitmoji_subject(":sparkles: (cli): Add release planning", &catalog).unwrap();
        assert_eq!(shortcode.gitmoji.emoji, "✨");
        assert_eq!(shortcode.gitmoji.bump, Some(Bump::Minor));
        assert_eq!(shortcode.scope.as_deref(), Some("cli"));
        assert_eq!(shortcode.message, "Add release planning");
    }

    #[test]
    fn parses_optional_scope_and_colon() {
        let catalog = catalog();
        let scoped = parse_gitmoji_subject("♻️ (analysis) Simplify parsing", &catalog).unwrap();
        assert_eq!(scoped.scope.as_deref(), Some("analysis"));
        assert_eq!(scoped.message, "Simplify parsing");

        let colon = parse_gitmoji_subject("🐛 : Fix retries", &catalog).unwrap();
        assert_eq!(colon.scope, None);
        assert_eq!(colon.message, "Fix retries");
    }

    #[test]
    fn rejects_subjects_outside_the_gitmoji_specification() {
        let catalog = catalog();
        for subject in [
            "Add a feature",
            "🦄 Unknown intention",
            ":sparkles:Missing whitespace",
            "✨",
            "✨ (cli):",
            "✨ () Empty scope",
        ] {
            assert_eq!(parse_gitmoji_subject(subject, &catalog), None, "{subject}");
        }
    }

    #[test]
    fn parses_a_custom_catalog_entry_in_unicode_and_shortcode_forms() {
        let mut catalog = catalog();
        catalog.insert(
            "flying_saucer".into(),
            entry(
                "🛸",
                "Experimental feature behind a flag.",
                Some(Bump::Minor),
            ),
        );

        for subject in ["🛸 Add experiment", ":flying_saucer: Add experiment"] {
            let parsed = parse_gitmoji_subject(subject, &catalog).unwrap();
            assert_eq!(parsed.gitmoji.shortcode, ":flying_saucer:");
            assert_eq!(parsed.gitmoji.bump, Some(Bump::Minor));
        }
    }

    #[test]
    fn uses_the_declared_level_instead_of_an_upstream_default() {
        let mut catalog = catalog();
        catalog.get_mut("sparkles").unwrap().semver = Some(Bump::Patch);

        let parsed = parse_gitmoji_subject("✨ Change behavior", &catalog).unwrap();

        assert_eq!(parsed.gitmoji.bump, Some(Bump::Patch));
    }

    #[test]
    fn a_declared_null_level_does_not_trigger_a_release() {
        let mut catalog = catalog();
        catalog.get_mut("sparkles").unwrap().semver = None;
        let parsed = parse_gitmoji_subject("✨ Change behavior", &catalog).unwrap();
        let analysis = Analysis::from_commits(vec![AnalyzedCommit {
            id: "1".into(),
            subject: "✨ Change behavior".into(),
            gitmoji: parsed.gitmoji,
            scope: parsed.scope,
            message: parsed.message,
        }]);

        assert_eq!(analysis.bump, None);
        assert_eq!(analysis.next_version(None).unwrap(), None);
    }

    #[test]
    fn computes_the_highest_catalog_bump() {
        let analysis = Analysis::from_commits(vec![
            commit("1", "📝 Document behavior"),
            commit("2", "🐛 Fix behavior"),
            commit("3", "✨ Add behavior"),
            commit("4", "💥 Break behavior"),
        ]);

        assert_eq!(analysis.bump, Some(Bump::Major));
    }

    #[test]
    fn computes_semver_successors_without_automatic_one_point_zero() {
        let cases = [
            (
                Some(Version::new(1, 2, 3)),
                Bump::Patch,
                Version::new(1, 2, 4),
            ),
            (
                Some(Version::new(1, 2, 3)),
                Bump::Minor,
                Version::new(1, 3, 0),
            ),
            (
                Some(Version::new(1, 2, 3)),
                Bump::Major,
                Version::new(2, 0, 0),
            ),
            (
                Some(Version::new(0, 4, 2)),
                Bump::Patch,
                Version::new(0, 4, 3),
            ),
            (
                Some(Version::new(0, 4, 2)),
                Bump::Minor,
                Version::new(0, 5, 0),
            ),
            (
                Some(Version::new(0, 4, 2)),
                Bump::Major,
                Version::new(0, 5, 0),
            ),
            (None, Bump::Patch, Version::new(0, 0, 1)),
            (None, Bump::Minor, Version::new(0, 1, 0)),
            (None, Bump::Major, Version::new(0, 1, 0)),
        ];

        for (current, bump, expected) in cases {
            assert_eq!(next_version(current.as_ref(), bump).unwrap(), expected);
        }
    }

    #[test]
    fn returns_no_successor_without_a_bumping_commit() {
        let analysis = Analysis::from_commits(vec![commit("1", "📝 Document behavior")]);
        assert_eq!(analysis.next_version(None).unwrap(), None);
    }

    #[test]
    fn renders_grouped_notes_by_size_then_shortcode() {
        let analysis = Analysis::from_commits(vec![
            commit("1", "📝 Document Unicode spelling"),
            commit("2", ":memo: Document shortcode spelling"),
            commit("3", "✨ Add feature"),
            commit("4", ":bug: Fix bug"),
        ]);

        let notes = render_release_notes(&analysis).unwrap();
        assert_eq!(
            notes,
            "### 📝 Add or update documentation\n\n- 📝 Document Unicode spelling\n- :memo: Document shortcode spelling\n\n### 🐛 Fix a bug\n\n- :bug: Fix bug\n\n### ✨ Introduce new features\n\n- ✨ Add feature\n"
        );
    }

    #[test]
    fn renders_a_message_when_no_gitmoji_commits_exist() {
        let notes = render_release_notes(&Analysis::from_commits(Vec::new())).unwrap();
        assert_eq!(notes, "No recorded changes.\n");
    }
}
