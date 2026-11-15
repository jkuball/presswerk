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
            // Trailing whitespace is not part of a heading; found lines are trimmed as well.
            format: format.trim_end().into(),
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

    /// The version of a heading line in this format, if the line is one. `{date}` matches the
    /// ISO date that `prepare` writes, which keeps formats such as `{version}-{date}` unambiguous.
    fn version(&self, line: &str) -> Option<Version> {
        let line = line.trim_end();
        let (before, after) = self.format.split_once(VERSION_PLACEHOLDER)?;
        let version = if let Some((prefix, middle)) = before.split_once(DATE_PLACEHOLDER) {
            let rest = line.strip_prefix(prefix)?.strip_suffix(after)?;
            let (date, rest) = (rest.get(..ISO_DATE_LEN)?, rest.get(ISO_DATE_LEN..)?);
            is_iso_date(date).then_some(())?;
            rest.strip_prefix(middle)?
        } else if let Some((middle, suffix)) = after.split_once(DATE_PLACEHOLDER) {
            let rest = line.strip_prefix(before)?.strip_suffix(suffix)?;
            let split = rest.len().checked_sub(ISO_DATE_LEN)?;
            let (rest, date) = (rest.get(..split)?, rest.get(split..)?);
            is_iso_date(date).then_some(())?;
            rest.strip_suffix(middle)?
        } else {
            line.strip_prefix(before)?.strip_suffix(after)?
        };
        version.parse().ok()
    }

    /// Whether a line is a heading of the declared level or a higher one, which ends a section.
    fn ends_section(&self, line: &str) -> bool {
        let rest = line.trim_start_matches('#');
        let level = line.len() - rest.len();
        ((1..=self.level).contains(&level) && (rest.is_empty() || rest.starts_with(' ')))
            || self.version(line).is_some()
    }
}

impl Default for ChangelogHeading {
    fn default() -> Self {
        Self::parse(DEFAULT_HEADING).expect("the default changelog heading is valid")
    }
}

const ISO_DATE_LEN: usize = "YYYY-MM-DD".len();

/// Whether `date` has the shape of an ISO calendar date, `YYYY-MM-DD`.
fn is_iso_date(date: &str) -> bool {
    date.len() == ISO_DATE_LEN
        && date.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

pub fn insert_version_section(
    contents: Option<&str>,
    heading: &ChangelogHeading,
    version: &Version,
    date: &str,
    notes: &str,
) -> Result<String, ChangelogError> {
    if !is_iso_date(date) {
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
        let changelog = format!("# Changelog{line_ending}{line_ending}{section}");
        ensure_readable(&changelog, heading, version, notes)?;
        return Ok(changelog);
    }

    // Without a recognized version heading, take the first level-2 heading of any format,
    // so changelogs written by other tools get the new section above their history.
    let insertion = version_headings(contents, heading)
        .next()
        .map(|(start, _, _)| start)
        .or_else(|| {
            heading_lines(contents)
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

    let changelog = format!("{before}{separator}{section}{after}");
    ensure_readable(&changelog, heading, version, notes)?;
    Ok(changelog)
}

/// Publishers read the section back with the same heading, so a section that reads back as
/// something else, for example because the notes contain headings of the heading's level, must
/// fail before the release commit exists.
fn ensure_readable(
    changelog: &str,
    heading: &ChangelogHeading,
    version: &Version,
    notes: &str,
) -> Result<(), ChangelogError> {
    if read_version_section(changelog, heading, version)? != notes {
        return Err(ChangelogError::UnreadableSection(version.clone()));
    }
    Ok(())
}

pub fn read_version_section<'a>(
    contents: &'a str,
    heading: &ChangelogHeading,
    version: &Version,
) -> Result<&'a str, ChangelogError> {
    let (_, heading_end) = find_version_heading(contents, heading, version)?
        .ok_or_else(|| ChangelogError::MissingVersion(version.clone()))?;
    let body_start = heading_end + line_ending_prefix_len(&contents[heading_end..]);
    let body_end = heading_lines(contents)
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
    heading_lines(contents)
        .filter_map(|(start, end, line)| Some((start, end, heading.version(line)?)))
}

/// The lines that can be headings, with their byte offsets: every line outside fenced code
/// blocks, so `## ` or `# comment` lines inside a fence never count. An unclosed fence runs to
/// the end of the changelog, as in CommonMark.
fn heading_lines(contents: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut offset = 0;
    let mut open_fence: Option<(u8, usize)> = None;
    contents.split_inclusive('\n').filter_map(move |line| {
        let start = offset;
        offset += line.len();
        let line = line.trim_end_matches(['\r', '\n']);
        match (open_fence, code_fence(line)) {
            (Some((marker, length)), Some((closing, closing_length, info)))
                if closing == marker && closing_length >= length && info.trim().is_empty() =>
            {
                open_fence = None;
                None
            }
            (Some(_), _) => None,
            (None, Some((marker, length, _))) => {
                open_fence = Some((marker, length));
                None
            }
            (None, None) => Some((start, offset, line)),
        }
    })
}

/// A code fence line: up to three spaces of indentation, then at least three backticks or
/// tildes, where a backtick fence has no backtick in the rest of the line, so inline code such
/// as ```` ```presswerk plan``` ```` does not open one. Returns the marker, the run length, and
/// the rest of the line.
fn code_fence(line: &str) -> Option<(u8, usize, &str)> {
    let unindented = line.trim_start_matches(' ');
    if line.len() - unindented.len() > 3 {
        return None;
    }
    let marker = *unindented
        .as_bytes()
        .first()
        .filter(|byte| matches!(byte, b'`' | b'~'))?;
    let length = unindented
        .bytes()
        .take_while(|byte| *byte == marker)
        .count();
    let rest = &unindented[length..];
    (length >= 3 && !(marker == b'`' && rest.contains('`'))).then_some((marker, length, rest))
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChangelogError {
    DuplicateVersion(Version),
    MissingVersion(Version),
    InvalidDate(String),
    InvalidHeading(String),
    UnreadableSection(Version),
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
            Self::UnreadableSection(version) => write!(
                formatter,
                "the changelog section of {version} does not read back as the written notes; changelog.template must not use headings of changelog.heading's level or higher"
            ),
            Self::InvalidDate(date) => write!(formatter, "invalid changelog date {date:?}"),
            Self::InvalidHeading(format) => write!(
                formatter,
                "changelog.heading {format:?} must be one line that starts with #, with {{version}} exactly once and {{date}} at most once"
            ),
        }
    }
}

impl Error for ChangelogError {}
