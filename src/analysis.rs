use git_cliff_core::repo::Repository;
use minijinja::{Environment, context, value::Serde};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

const DEFAULT_NOTES_TEMPLATE: &str = "{% if groups %}{% for group in groups %}### {{ group.emoji }} {{ group.shortcode }}\n\n{% for commit in group.commits %}- {{ commit.subject }}\n{% endfor %}{% if not loop.last %}\n{% endif %}{% endfor %}{% else %}No recorded changes.\n{% endif %}";
const BOOKMARK_SHORTCODE: &str = ":bookmark:";

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
    pub repository: PathBuf,
    pub base: Option<String>,
    pub target: String,
}

impl CommitRange {
    #[must_use]
    pub fn initial(repository: impl Into<PathBuf>, target: impl Into<String>) -> Self {
        Self {
            repository: repository.into(),
            base: None,
            target: target.into(),
        }
    }

    #[must_use]
    pub fn between(
        repository: impl Into<PathBuf>,
        base: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            repository: repository.into(),
            base: Some(base.into()),
            target: target.into(),
        }
    }

    fn revision(&self) -> String {
        self.base.as_ref().map_or_else(
            || self.target.clone(),
            |base| format!("{base}..{}", self.target),
        )
    }
}

pub trait Analyzer {
    fn analyze(&self, range: &CommitRange) -> Result<Analysis, AnalysisError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GitCliffAnalyzer;

impl Analyzer for GitCliffAnalyzer {
    fn analyze(&self, range: &CommitRange) -> Result<Analysis, AnalysisError> {
        let repository = Repository::init(range.repository.clone()).map_err(|error| {
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

        let commits = commits
            .iter()
            .filter_map(|commit| {
                let commit = git_cliff_core::commit::Commit::from(commit);
                let subject = commit.message.lines().next()?.trim();
                let parsed = parse_gitmoji_subject(subject)?;
                if parsed.gitmoji.shortcode == BOOKMARK_SHORTCODE {
                    return None;
                }
                Some(AnalyzedCommit {
                    id: commit.id,
                    subject: subject.to_owned(),
                    gitmoji: parsed.gitmoji,
                    scope: parsed.scope,
                    message: parsed.message,
                })
            })
            .collect();

        Ok(Analysis::from_commits(commits))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisError {
    message: String,
}

impl AnalysisError {
    fn new(message: String) -> Self {
        Self { message }
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

#[derive(Debug)]
struct CatalogGitmoji {
    emoji: &'static str,
    code: &'static str,
    description: &'static str,
    semver: Option<Bump>,
}

include!("gitmoji/catalog.rs");

#[must_use]
pub fn parse_gitmoji_subject(subject: &str) -> Option<ParsedGitmojiSubject> {
    let subject = subject.lines().next()?.trim_end();
    let separator = subject.find(char::is_whitespace)?;
    let intention = &subject[..separator];
    let remainder = subject[separator..].trim_start();
    if remainder.is_empty() {
        return None;
    }

    let entry = GITMOJIS.iter().find(|entry| {
        if intention.starts_with(':') {
            intention == entry.code
        } else {
            strip_variation_selectors(intention) == strip_variation_selectors(entry.emoji)
        }
    })?;
    let (scope, message) = parse_scope_and_message(remainder)?;

    Some(ParsedGitmojiSubject {
        gitmoji: Gitmoji {
            emoji: entry.emoji.to_owned(),
            shortcode: entry.code.to_owned(),
            description: entry.description.to_owned(),
            bump: entry.semver,
        },
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

fn strip_variation_selectors(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '\u{fe0f}')
        .collect()
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
                description: &commit.gitmoji.description,
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

    fn commit(id: &str, subject: &str) -> AnalyzedCommit {
        let parsed = parse_gitmoji_subject(subject).unwrap();
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
        let unicode = parse_gitmoji_subject("⚡ Improve startup time").unwrap();
        assert_eq!(unicode.gitmoji.shortcode, ":zap:");
        assert_eq!(unicode.gitmoji.bump, Some(Bump::Patch));
        assert_eq!(unicode.scope, None);
        assert_eq!(unicode.message, "Improve startup time");

        let shortcode = parse_gitmoji_subject(":sparkles: (cli): Add release planning").unwrap();
        assert_eq!(shortcode.gitmoji.emoji, "✨");
        assert_eq!(shortcode.gitmoji.bump, Some(Bump::Minor));
        assert_eq!(shortcode.scope.as_deref(), Some("cli"));
        assert_eq!(shortcode.message, "Add release planning");
    }

    #[test]
    fn parses_optional_scope_and_colon() {
        let scoped = parse_gitmoji_subject("♻️ (analysis) Simplify parsing").unwrap();
        assert_eq!(scoped.scope.as_deref(), Some("analysis"));
        assert_eq!(scoped.message, "Simplify parsing");

        let colon = parse_gitmoji_subject("🐛 : Fix retries").unwrap();
        assert_eq!(colon.scope, None);
        assert_eq!(colon.message, "Fix retries");
    }

    #[test]
    fn rejects_subjects_outside_the_gitmoji_specification() {
        for subject in [
            "Add a feature",
            "🦄 Unknown intention",
            ":sparkles:Missing whitespace",
            "✨",
            "✨ (cli):",
            "✨ () Empty scope",
        ] {
            assert_eq!(parse_gitmoji_subject(subject), None, "{subject}");
        }
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
            "### 📝 :memo:\n\n- 📝 Document Unicode spelling\n- :memo: Document shortcode spelling\n\n### 🐛 :bug:\n\n- :bug: Fix bug\n\n### ✨ :sparkles:\n\n- ✨ Add feature\n"
        );
    }

    #[test]
    fn renders_a_message_when_no_gitmoji_commits_exist() {
        let notes = render_release_notes(&Analysis::from_commits(Vec::new())).unwrap();
        assert_eq!(notes, "No recorded changes.\n");
    }
}
