use semver::Version;
use std::error::Error;
use std::fmt;

pub const DEFAULT_HEADING: &str = "## [{version}] - {date}";
const VERSION_PLACEHOLDER: &str = "{version}";
const DATE_PLACEHOLDER: &str = "{date}";

/// The declared version heading, a plain replacement template that both renders new headings
/// and finds existing ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangelogHeading {
    format: String,
    level: usize,
}

impl ChangelogHeading {
    pub fn parse(format: &str) -> Result<Self, ChangelogError> {
        let invalid = || ChangelogError::InvalidHeading(format.into());
        if format.contains(['\r', '\n'])
            || !format.starts_with('#')
            || format.matches(VERSION_PLACEHOLDER).count() != 1
            || format.matches(DATE_PLACEHOLDER).count() > 1
        {
            return Err(invalid());
        }
        let level = format.len() - format.trim_start_matches('#').len();
        Ok(Self {
            format: format.into(),
            level,
        })
    }

    fn render(&self, version: &Version, date: &str) -> String {
        self.format
            .split(VERSION_PLACEHOLDER)
            .map(|part| part.replacen(DATE_PLACEHOLDER, date, 1))
            .collect::<Vec<_>>()
            .join(&version.to_string())
    }

    /// The version of a heading line in this format, if the line is one.
    fn version(&self, line: &str) -> Option<Version> {
        let line = line.trim_end();
        let (before, after) = self.format.split_once(VERSION_PLACEHOLDER)?;
        if let Some((prefix, middle)) = before.split_once(DATE_PLACEHOLDER) {
            let rest = line.strip_prefix(prefix)?.strip_suffix(after)?;
            split_candidates(rest, middle)
                .filter(|(date, _)| !date.is_empty())
                .find_map(|(_, version)| version.parse().ok())
        } else if let Some((middle, suffix)) = after.split_once(DATE_PLACEHOLDER) {
            let rest = line.strip_prefix(before)?.strip_suffix(suffix)?;
            split_candidates(rest, middle)
                .filter(|(_, date)| !date.is_empty())
                .find_map(|(version, _)| version.parse().ok())
        } else {
            line.strip_prefix(before)?.strip_suffix(after)?.parse().ok()
        }
    }

    /// Whether a line is a heading of the declared level, which ends a section.
    fn ends_section(&self, line: &str) -> bool {
        let rest = line.trim_start_matches('#');
        (line.len() - rest.len() == self.level && (rest.is_empty() || rest.starts_with(' ')))
            || self.version(line).is_some()
    }
}

impl Default for ChangelogHeading {
    fn default() -> Self {
        Self::parse(DEFAULT_HEADING).expect("the default changelog heading is valid")
    }
}

/// Every way to split `text` around one occurrence of a non-empty `separator`.
fn split_candidates<'a>(
    text: &'a str,
    separator: &'a str,
) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
    text.match_indices(separator)
        .filter(move |_| !separator.is_empty())
        .map(move |(index, _)| (&text[..index], &text[index + separator.len()..]))
}

pub fn insert_version_section(
    contents: Option<&str>,
    heading: &ChangelogHeading,
    version: &Version,
    date: &str,
    notes: &str,
) -> Result<String, ChangelogError> {
    if date.is_empty() || date.contains(['\r', '\n']) {
        return Err(ChangelogError::InvalidDate(date.into()));
    }

    let contents = contents.unwrap_or("");
    if find_version_heading(contents, heading, version)?.is_some() {
        return Err(ChangelogError::DuplicateVersion(version.clone()));
    }

    let line_ending = line_ending(contents, notes);
    let heading_line = heading.render(version, date);
    let section = format!("{heading_line}{line_ending}{line_ending}{notes}{line_ending}");
    if contents.is_empty() {
        return Ok(format!("# Changelog{line_ending}{line_ending}{section}"));
    }

    // Without a recognized version heading, take the first level-2 heading of any format,
    // so changelogs written by other tools get the new section above their history.
    let insertion = version_headings(contents, heading)
        .next()
        .map(|(start, _, _)| start)
        .or_else(|| {
            lines_with_offsets(contents)
                .find(|(_, _, line)| is_level_two_heading(line))
                .map(|(start, _, _)| start)
        })
        .unwrap_or(contents.len());
    let before = &contents[..insertion];
    let after = &contents[insertion..];
    let blank_line = format!("{line_ending}{line_ending}");
    let separator = if before.ends_with(&blank_line) {
        ""
    } else if before.ends_with(line_ending) {
        line_ending
    } else {
        &blank_line
    };

    Ok(format!("{before}{separator}{section}{after}"))
}

pub fn read_version_section<'a>(
    contents: &'a str,
    heading: &ChangelogHeading,
    version: &Version,
) -> Result<&'a str, ChangelogError> {
    let (_, heading_end) = find_version_heading(contents, heading, version)?
        .ok_or_else(|| ChangelogError::MissingVersion(version.clone()))?;
    let body_start = heading_end + line_ending_prefix_len(&contents[heading_end..]);
    let body_end = lines_with_offsets(contents)
        .find(|(start, _, line)| *start >= body_start && heading.ends_section(line))
        .map_or(contents.len(), |(start, _, _)| start);

    let body = &contents[body_start..body_end];
    Ok(body
        .strip_suffix("\r\n")
        .or_else(|| body.strip_suffix('\n'))
        .unwrap_or(body))
}

fn find_version_heading(
    contents: &str,
    heading: &ChangelogHeading,
    version: &Version,
) -> Result<Option<(usize, usize)>, ChangelogError> {
    let mut matches = version_headings(contents, heading)
        .filter(|(_, _, heading_version)| heading_version == version)
        .map(|(start, end, _)| (start, end));
    let heading = matches.next();
    if matches.next().is_some() {
        return Err(ChangelogError::DuplicateVersion(version.clone()));
    }
    Ok(heading)
}

fn line_ending(contents: &str, notes: &str) -> &'static str {
    for source in [contents, notes] {
        if let Some(newline) = source.find('\n') {
            return if source[..newline].ends_with('\r') {
                "\r\n"
            } else {
                "\n"
            };
        }
    }
    "\n"
}

fn line_ending_prefix_len(contents: &str) -> usize {
    if contents.starts_with("\r\n") {
        2
    } else if contents.starts_with('\n') {
        1
    } else {
        0
    }
}

fn is_level_two_heading(line: &str) -> bool {
    line.starts_with("## ")
}

fn version_headings<'a>(
    contents: &'a str,
    heading: &'a ChangelogHeading,
) -> impl Iterator<Item = (usize, usize, Version)> + 'a {
    lines_with_offsets(contents)
        .filter_map(|(start, end, line)| Some((start, end, heading.version(line)?)))
}

fn lines_with_offsets(contents: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut offset = 0;
    contents.split_inclusive('\n').map(move |line| {
        let start = offset;
        offset += line.len();
        (start, offset, line.trim_end_matches(['\r', '\n']))
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChangelogError {
    DuplicateVersion(Version),
    MissingVersion(Version),
    InvalidDate(String),
    InvalidHeading(String),
}

impl fmt::Display for ChangelogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateVersion(version) => {
                write!(formatter, "changelog already contains version {version}")
            }
            Self::MissingVersion(version) => {
                write!(formatter, "changelog does not contain version {version}")
            }
            Self::InvalidDate(date) => write!(formatter, "invalid changelog date {date:?}"),
            Self::InvalidHeading(format) => write!(
                formatter,
                "changelog.heading {format:?} must be one line that starts with #, with {{version}} exactly once and {{date}} at most once"
            ),
        }
    }
}

impl Error for ChangelogError {}
