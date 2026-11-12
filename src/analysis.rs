use crate::declaration::{
    self, ConventionalCatalog, GitmojiCatalog, GitmojiCatalogEntry, Tag, strip_variation_selectors,
};
use crate::release_commit::release_subject;
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
const CONVENTIONAL_REVERT_TYPE: &str = "revert";
const BREAKING_CHANGE_FOOTERS: [&str; 2] = ["BREAKING CHANGE:", "BREAKING-CHANGE:"];
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

/// A Conventional Commits type from the declared catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConventionalType {
    pub name: String,
    pub title: String,
    pub bump: Option<Bump>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParsedConventionalCommit {
    pub kind: ConventionalType,
    pub scope: Option<String>,
    pub message: String,
    /// Marked with `!` after the type or scope, or with a `BREAKING CHANGE:` footer.
    pub breaking: bool,
}

/// What a commit's subject says it does, in the declared convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Intention {
    Gitmoji(Gitmoji),
    Conventional(ConventionalType),
}

impl Intention {
    #[must_use]
    pub fn bump(&self) -> Option<Bump> {
        match self {
            Self::Gitmoji(gitmoji) => gitmoji.bump,
            Self::Conventional(kind) => kind.bump,
        }
    }

    #[must_use]
    pub fn gitmoji(&self) -> Option<&Gitmoji> {
        match self {
            Self::Gitmoji(gitmoji) => Some(gitmoji),
            Self::Conventional(_) => None,
        }
    }

    #[must_use]
    pub fn conventional(&self) -> Option<&ConventionalType> {
        match self {
            Self::Conventional(kind) => Some(kind),
            Self::Gitmoji(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnalyzedCommit {
    pub id: String,
    pub subject: String,
    pub intention: Intention,
    pub scope: Option<String>,
    pub message: String,
    /// A breaking change selects a major bump whatever its intention's level.
    pub breaking: bool,
}

impl AnalyzedCommit {
    #[must_use]
    pub fn bump(&self) -> Option<Bump> {
        if self.breaking {
            Some(Bump::Major)
        } else {
            self.intention.bump()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    pub bump: Option<Bump>,
    pub commits: Vec<AnalyzedCommit>,
}

impl Analysis {
    #[must_use]
    pub fn from_commits(commits: Vec<AnalyzedCommit>) -> Self {
        let bump = commits.iter().filter_map(AnalyzedCommit::bump).max();
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
    excluded: Option<Oid>,
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
            excluded: None,
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
            excluded: None,
        })
    }

    #[must_use]
    pub fn excluding(mut self, commit: Oid) -> Self {
        self.excluded = Some(commit);
        self
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
    /// Analyzes the commits in `range` with the declared analyzer; `tag` names the releases.
    fn analyze(
        &self,
        range: &CommitRange,
        analyzer: &declaration::Analyzer,
        tag: &Tag,
    ) -> Result<Analysis, AnalysisError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GitCliffAnalyzer;

impl Analyzer for GitCliffAnalyzer {
    fn analyze(
        &self,
        range: &CommitRange,
        analyzer: &declaration::Analyzer,
        tag: &Tag,
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
            if range.excluded == Some(commit.id())
                || suppressed.contains(&commit.id())
                || commit.parent_count() > 1
            {
                continue;
            }

            let message = commit.message().unwrap_or_default();
            if message.lines().next().is_none() {
                continue;
            }

            let Some(reverted) = reverted_commit(message) else {
                if let Some(entry) = analyzed_commit(commit.id(), message, analyzer, tag) {
                    analyzed.push(entry);
                }
                continue;
            };

            let target = resolve_commit_id(&git, commit.id(), reverted.commit)?;
            if !in_range.contains(&target.id()) {
                if let Some(entry) = reverted_outside_range(commit.id(), message, analyzer, tag) {
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

fn subject_line(message: &str) -> &str {
    message.lines().next().unwrap_or_default().trim_end()
}

/// The analyzed form of a commit, or `None` for a commit that its convention does not describe.
fn analyzed_commit(
    id: Oid,
    message: &str,
    analyzer: &declaration::Analyzer,
    tag: &Tag,
) -> Option<AnalyzedCommit> {
    let subject = subject_line(message);
    match analyzer {
        declaration::Analyzer::Gitmoji { gitmoji } => {
            let parsed = parse_gitmoji_subject(subject, &gitmoji.catalog)?;
            Some(AnalyzedCommit {
                id: id.to_string(),
                subject: subject.to_owned(),
                intention: Intention::Gitmoji(parsed.gitmoji),
                scope: parsed.scope,
                message: parsed.message,
                breaking: false,
            })
        }
        declaration::Analyzer::Conventional { conventional } => {
            if is_release_subject(subject, analyzer, tag) {
                return None;
            }
            let parsed = parse_conventional_commit(message, &conventional.catalog)?;
            Some(AnalyzedCommit {
                id: id.to_string(),
                subject: subject.to_owned(),
                intention: Intention::Conventional(parsed.kind),
                scope: parsed.scope,
                message: parsed.message,
                breaking: parsed.breaking,
            })
        }
    }
}

/// Whether a subject is the one `prepare` writes for a release commit of this declaration.
fn is_release_subject(subject: &str, analyzer: &declaration::Analyzer, tag: &Tag) -> bool {
    subject
        .rsplit(' ')
        .next()
        .and_then(|name| tag.parse(name).ok().flatten())
        .and_then(|version| tag.render(&version).ok())
        .is_some_and(|name| release_subject(analyzer, &name) == subject)
}

/// Records a revert whose target was released earlier under the catalog's revert intention,
/// keeping the revert's own subject as the message.
fn reverted_outside_range(
    id: Oid,
    message: &str,
    analyzer: &declaration::Analyzer,
    tag: &Tag,
) -> Option<AnalyzedCommit> {
    let subject = subject_line(message);
    match analyzer {
        declaration::Analyzer::Gitmoji { gitmoji } => {
            let Some(entry) = gitmoji.catalog.get("rewind") else {
                return analyzed_commit(id, message, analyzer, tag);
            };
            Some(AnalyzedCommit {
                id: id.to_string(),
                subject: format!("{} {subject}", entry.emoji),
                intention: Intention::Gitmoji(gitmoji_intention("rewind", entry)),
                scope: None,
                message: subject.to_owned(),
                breaking: false,
            })
        }
        // A subject that states its own type, such as `revert: …`, keeps it; the plain
        // `Revert "…"` subject that `git revert` writes falls under the revert type.
        declaration::Analyzer::Conventional { conventional } => {
            analyzed_commit(id, message, analyzer, tag).or_else(|| {
                let entry = conventional.catalog.get(CONVENTIONAL_REVERT_TYPE)?;
                Some(AnalyzedCommit {
                    id: id.to_string(),
                    subject: subject.to_owned(),
                    intention: Intention::Conventional(conventional_type(
                        CONVENTIONAL_REVERT_TYPE,
                        entry,
                    )),
                    scope: None,
                    message: subject.to_owned(),
                    breaking: false,
                })
            })
        }
    }
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
        gitmoji: gitmoji_intention(shortcode, entry),
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

fn gitmoji_intention(shortcode: &str, entry: &GitmojiCatalogEntry) -> Gitmoji {
    Gitmoji {
        emoji: entry.emoji.clone(),
        shortcode: format!(":{shortcode}:"),
        description: entry.description.clone(),
        bump: entry.semver,
    }
}

/// Parses a commit message written to the
/// [Conventional Commits specification](https://www.conventionalcommits.org/en/v1.0.0/):
/// `<type>[(scope)][!]: <description>`, with the type matched case-insensitively against the
/// declared catalog, and a `BREAKING CHANGE:` or `BREAKING-CHANGE:` line after the subject.
#[must_use]
pub fn parse_conventional_commit(
    message: &str,
    catalog: &ConventionalCatalog,
) -> Option<ParsedConventionalCommit> {
    let subject = subject_line(message);
    let prefix_end = subject.find(':')?;
    let (prefix, remainder) = subject.split_at(prefix_end);
    let description = remainder[1..]
        .strip_prefix(char::is_whitespace)?
        .trim_start();
    if description.is_empty() {
        return None;
    }

    let (prefix, bang) = prefix
        .strip_suffix('!')
        .map_or((prefix, false), |prefix| (prefix, true));
    let (name, scope) = match prefix.split_once('(') {
        Some((name, scope)) => {
            let scope = scope.strip_suffix(')')?;
            if scope.trim().is_empty() || scope.contains(['(', ')']) {
                return None;
            }
            (name, Some(scope.trim().to_owned()))
        }
        None => (prefix, None),
    };
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let entry = catalog.get(&name)?;

    let footer = message.lines().skip(1).any(|line| {
        BREAKING_CHANGE_FOOTERS.iter().any(|token| {
            line.strip_prefix(token)
                .and_then(|value| value.strip_prefix(char::is_whitespace))
                .is_some_and(|value| !value.trim().is_empty())
        })
    });

    Some(ParsedConventionalCommit {
        kind: conventional_type(&name, entry),
        scope,
        message: description.to_owned(),
        breaking: bang || footer,
    })
}

fn conventional_type(
    name: &str,
    entry: &declaration::ConventionalCatalogEntry,
) -> ConventionalType {
    ConventionalType {
        name: name.to_owned(),
        title: entry.title.clone(),
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

const BREAKING_CHANGE_MARKER: &str = "**BREAKING**";

#[derive(Serialize)]
struct NoteGroup<'a> {
    title: String,
    commits: Vec<NoteCommit>,
    /// Groups sort by rank, then by size, then by key.
    #[serde(skip)]
    rank: u8,
    #[serde(skip)]
    key: &'a str,
}

#[derive(Serialize)]
struct NoteCommit {
    /// The commit's line without the intention, which the group heading already shows.
    text: String,
}

impl NoteCommit {
    fn new(commit: &AnalyzedCommit) -> Self {
        let text = match &commit.intention {
            // The Gitmoji subject keeps its own scope spelling after the intention.
            Intention::Gitmoji(_) => commit
                .subject
                .split_once(char::is_whitespace)
                .map_or(commit.subject.as_str(), |(_, rest)| rest.trim_start())
                .to_owned(),
            Intention::Conventional(_) => {
                let marker = if commit.breaking {
                    format!("{BREAKING_CHANGE_MARKER} ")
                } else {
                    String::new()
                };
                match &commit.scope {
                    Some(scope) => format!("{marker}**{scope}:** {}", commit.message),
                    None => format!("{marker}{}", commit.message),
                }
            }
        };
        Self { text }
    }
}

/// The note group a commit belongs to: its key, heading, and rank.
fn note_group(commit: &AnalyzedCommit) -> (&str, String, u8) {
    match &commit.intention {
        Intention::Gitmoji(gitmoji) => (
            &gitmoji.shortcode,
            format!(
                "{} {}",
                gitmoji.emoji,
                gitmoji.description.trim_end_matches('.')
            ),
            0,
        ),
        // Types come in the order of the level they select; a breaking commit stays in its type.
        Intention::Conventional(kind) => {
            let rank = match kind.bump {
                Some(Bump::Major) => 0,
                Some(Bump::Minor) => 1,
                Some(Bump::Patch) => 2,
                None => 3,
            };
            (&kind.name, kind.title.clone(), rank)
        }
    }
}

pub fn render_release_notes(analysis: &Analysis) -> Result<String, RenderError> {
    let mut groups = BTreeMap::<&str, NoteGroup<'_>>::new();
    for commit in &analysis.commits {
        let (key, title, rank) = note_group(commit);
        groups
            .entry(key)
            .or_insert_with(|| NoteGroup {
                title,
                commits: Vec::new(),
                rank,
                key,
            })
            .commits
            .push(NoteCommit::new(commit));
    }

    let mut groups: Vec<_> = groups.into_values().collect();
    groups.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| right.commits.len().cmp(&left.commits.len()))
            .then_with(|| left.key.cmp(right.key))
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

    fn test_catalog() -> GitmojiCatalog {
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
        let parsed = parse_gitmoji_subject(subject, &test_catalog()).unwrap();
        AnalyzedCommit {
            id: id.into(),
            subject: subject.into(),
            intention: Intention::Gitmoji(parsed.gitmoji),
            scope: parsed.scope,
            message: parsed.message,
            breaking: false,
        }
    }

    #[test]
    fn parses_unicode_and_shortcode_subjects() {
        let catalog = test_catalog();
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
        let catalog = test_catalog();
        let scoped = parse_gitmoji_subject("♻️ (analysis) Simplify parsing", &catalog).unwrap();
        assert_eq!(scoped.scope.as_deref(), Some("analysis"));
        assert_eq!(scoped.message, "Simplify parsing");

        let colon = parse_gitmoji_subject("🐛 : Fix retries", &catalog).unwrap();
        assert_eq!(colon.scope, None);
        assert_eq!(colon.message, "Fix retries");
    }

    #[test]
    fn rejects_subjects_outside_the_gitmoji_specification() {
        let catalog = test_catalog();
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
        let mut catalog = test_catalog();
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
        let mut catalog = test_catalog();
        catalog.get_mut("sparkles").unwrap().semver = Some(Bump::Patch);

        let parsed = parse_gitmoji_subject("✨ Change behavior", &catalog).unwrap();

        assert_eq!(parsed.gitmoji.bump, Some(Bump::Patch));
    }

    #[test]
    fn a_declared_null_level_does_not_trigger_a_release() {
        let mut catalog = test_catalog();
        catalog.get_mut("sparkles").unwrap().semver = None;
        let parsed = parse_gitmoji_subject("✨ Change behavior", &catalog).unwrap();
        let analysis = Analysis::from_commits(vec![AnalyzedCommit {
            id: "1".into(),
            subject: "✨ Change behavior".into(),
            intention: Intention::Gitmoji(parsed.gitmoji),
            scope: parsed.scope,
            message: parsed.message,
            breaking: false,
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
            "### 📝 Add or update documentation\n\n- Document Unicode spelling\n- Document shortcode spelling\n\n### 🐛 Fix a bug\n\n- Fix bug\n\n### ✨ Introduce new features\n\n- Add feature\n"
        );
    }

    #[test]
    fn renders_a_message_when_no_gitmoji_commits_exist() {
        let notes = render_release_notes(&Analysis::from_commits(Vec::new())).unwrap();
        assert_eq!(notes, "No recorded changes.\n");
    }

    fn conventional_catalog() -> ConventionalCatalog {
        let entry = |title: &str, semver| declaration::ConventionalCatalogEntry {
            title: title.into(),
            semver,
        };
        BTreeMap::from([
            ("chore".into(), entry("Chores", None)),
            ("docs".into(), entry("Documentation", None)),
            ("feat".into(), entry("Features", Some(Bump::Minor))),
            ("fix".into(), entry("Bug fixes", Some(Bump::Patch))),
            ("perf".into(), entry("Performance", Some(Bump::Patch))),
        ])
    }

    fn conventional(id: &str, message: &str) -> AnalyzedCommit {
        let parsed = parse_conventional_commit(message, &conventional_catalog()).unwrap();
        AnalyzedCommit {
            id: id.into(),
            subject: subject_line(message).into(),
            intention: Intention::Conventional(parsed.kind),
            scope: parsed.scope,
            message: parsed.message,
            breaking: parsed.breaking,
        }
    }

    #[test]
    fn parses_conventional_types_scopes_and_descriptions() {
        let catalog = conventional_catalog();
        let plain = parse_conventional_commit("fix: Keep the tag order", &catalog).unwrap();
        assert_eq!(plain.kind.name, "fix");
        assert_eq!(plain.kind.title, "Bug fixes");
        assert_eq!(plain.kind.bump, Some(Bump::Patch));
        assert_eq!(plain.scope, None);
        assert_eq!(plain.message, "Keep the tag order");
        assert!(!plain.breaking);

        let scoped = parse_conventional_commit("feat(cli): add --json", &catalog).unwrap();
        assert_eq!(scoped.kind.bump, Some(Bump::Minor));
        assert_eq!(scoped.scope.as_deref(), Some("cli"));
        assert_eq!(scoped.message, "add --json");

        // Types are case-insensitive; the catalog spells them in lowercase.
        let upper = parse_conventional_commit("FEAT(Docs Site): Add search", &catalog).unwrap();
        assert_eq!(upper.kind.name, "feat");
        assert_eq!(upper.scope.as_deref(), Some("Docs Site"));

        // Only the first colon ends the prefix.
        let colons = parse_conventional_commit("docs: Explain a: b", &catalog).unwrap();
        assert_eq!(colons.message, "Explain a: b");
    }

    #[test]
    fn rejects_subjects_outside_the_conventional_commits_specification() {
        let catalog = conventional_catalog();
        for subject in [
            "Add a feature",
            "unknown: Use an undeclared type",
            "feat:missing space",
            "feat: ",
            "feat:",
            "feat (cli): Space before the scope",
            "feat(): Empty scope",
            "feat(cli: Unclosed scope",
            "feat(cli)(api): Two scopes",
            "feat!!: Two bangs",
            "!: No type",
            "(cli): No type",
            " feat: Leading whitespace",
            "✨ feat: Gitmoji first",
            "feat cli: Space in the type",
        ] {
            assert_eq!(
                parse_conventional_commit(subject, &catalog),
                None,
                "{subject}"
            );
        }
    }

    #[test]
    fn marks_bangs_and_breaking_change_footers_as_breaking() {
        let catalog = conventional_catalog();
        for message in [
            "feat!: Drop the legacy flag",
            "fix(cli)!: Rename the output field",
            "docs!: Remove the deprecated guide",
            "feat: Drop the legacy flag\n\nBREAKING CHANGE: the --legacy flag is gone",
            "fix: Rename the field\n\nBody text.\n\nRefs: #12\nBREAKING-CHANGE: field renamed",
        ] {
            let parsed = parse_conventional_commit(message, &catalog).unwrap();
            assert!(parsed.breaking, "{message}");
        }

        for message in [
            "feat: Mention it\n\nbreaking change: lowercase is not a footer",
            "feat: Mention it\n\nBREAKING CHANGES: plural is not a footer",
            "feat: Mention it\n\n  BREAKING CHANGE: indented is not a footer",
            "feat: Mention it\n\nBREAKING CHANGE:",
            "feat: Mention it\n\nThis is no BREAKING CHANGE: mid-line",
            "feat: BREAKING CHANGE: only in the subject",
        ] {
            let parsed = parse_conventional_commit(message, &catalog).unwrap();
            assert!(!parsed.breaking, "{message}");
        }
    }

    #[test]
    fn breaking_changes_select_major_and_minor_before_one_point_zero() {
        let analysis = Analysis::from_commits(vec![
            conventional("1", "fix: Fix behavior"),
            conventional("2", "docs!: Remove the old guide"),
        ]);

        assert_eq!(analysis.bump, Some(Bump::Major));
        assert_eq!(
            analysis.next_version(Some(&Version::new(1, 4, 2))).unwrap(),
            Some(Version::new(2, 0, 0))
        );
        assert_eq!(
            analysis.next_version(Some(&Version::new(0, 4, 2))).unwrap(),
            Some(Version::new(0, 5, 0))
        );
    }

    #[test]
    fn computes_the_highest_conventional_level() {
        let analysis = Analysis::from_commits(vec![
            conventional("1", "docs: Document behavior"),
            conventional("2", "perf: Speed up behavior"),
        ]);
        assert_eq!(analysis.bump, Some(Bump::Patch));

        let analysis = Analysis::from_commits(vec![
            conventional("1", "fix: Fix behavior"),
            conventional("2", "feat: Add behavior"),
        ]);
        assert_eq!(analysis.bump, Some(Bump::Minor));

        let analysis = Analysis::from_commits(vec![conventional("1", "chore: Tidy up")]);
        assert_eq!(analysis.bump, None);
        assert_eq!(analysis.next_version(None).unwrap(), None);
    }

    #[test]
    fn uses_the_declared_conventional_levels() {
        let mut catalog = conventional_catalog();
        catalog.get_mut("docs").unwrap().semver = Some(Bump::Patch);
        catalog.get_mut("feat").unwrap().semver = None;

        let docs = parse_conventional_commit("docs: Write the guide", &catalog).unwrap();
        assert_eq!(docs.kind.bump, Some(Bump::Patch));
        let feat = parse_conventional_commit("feat: Add behavior", &catalog).unwrap();
        assert_eq!(feat.kind.bump, None);

        catalog.remove("docs");
        assert_eq!(
            parse_conventional_commit("docs: Write the guide", &catalog),
            None
        );
    }

    #[test]
    fn renders_conventional_notes_by_type_level_size_and_name() {
        let analysis = Analysis::from_commits(vec![
            conventional("1", "docs: Document search"),
            conventional("2", "fix(cli): Keep the search order stable"),
            conventional("3", "feat(cli): Add --json"),
            conventional("4", "fix: Fix tag lookup"),
            conventional("5", "perf: Cache the catalog"),
            conventional("6", "feat(api)!: Drop the v1 endpoint"),
            conventional("7", "chore: Update dependencies"),
            conventional(
                "8",
                "fix: Rename the output field\n\nBREAKING CHANGE: `version` is now `next`.",
            ),
        ]);

        let notes = render_release_notes(&analysis).unwrap();
        assert_eq!(
            notes,
            "### Features\n\n- **cli:** Add --json\n- **BREAKING** **api:** Drop the v1 endpoint\n\n\
             ### Bug fixes\n\n- **cli:** Keep the search order stable\n- Fix tag lookup\n- **BREAKING** Rename the output field\n\n\
             ### Performance\n\n- Cache the catalog\n\n\
             ### Chores\n\n- Update dependencies\n\n\
             ### Documentation\n\n- Document search\n"
        );
    }
}
