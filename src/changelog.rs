use semver::Version;
use std::error::Error;
use std::fmt;
use time::format_description::{self, OwnedFormatItem};

pub const DEFAULT_HEADING: &str = "## [{version}] - {date}";
const VERSION_PLACEHOLDER: &str = "{version}";
const DEFAULT_DATE_FORMAT: &str = "%Y-%m-%d";

/// The declared version heading, a template with `{version}` and an optional `{date}` or
/// `{date:<strftime format>}` that both renders new headings and finds existing ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangelogHeading {
    level: usize,
    prefix: String,
    middle: String,
    suffix: String,
    date: Option<(DateFormat, DatePosition)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatePosition {
    BeforeVersion,
    AfterVersion,
}

impl ChangelogHeading {
    pub fn parse(format: &str) -> Result<Self, ChangelogError> {
        let invalid = || ChangelogError::InvalidHeading(format.into());
        // Trailing whitespace is not part of a heading; found lines are trimmed as well.
        let trimmed = format.trim_end();
        if format.contains(['\r', '\n'])
            || !format.starts_with('#')
            || trimmed.matches(VERSION_PLACEHOLDER).count() != 1
        {
            return Err(invalid());
        }

        let mut date = None;
        for (start, _) in trimmed.match_indices("{date") {
            let rest = &trimmed[start + "{date".len()..];
            let (end, date_format) = if rest.starts_with('}') {
                (start + "{date}".len(), DEFAULT_DATE_FORMAT)
            } else if let Some(spec) = rest.strip_prefix(':') {
                let close = spec.find('}').ok_or_else(invalid)?;
                (start + "{date:".len() + close + 1, &spec[..close])
            } else {
                continue;
            };
            if date.is_some() {
                return Err(invalid());
            }
            let date_format = DateFormat::parse(date_format).ok_or_else(invalid)?;
            date = Some((start, end, date_format));
        }

        let version_start = trimmed.find(VERSION_PLACEHOLDER).ok_or_else(invalid)?;
        let version_end = version_start + VERSION_PLACEHOLDER.len();
        let (prefix, middle, suffix, date) = match date {
            None => (&trimmed[..version_start], "", &trimmed[version_end..], None),
            Some((start, end, date_format)) if start >= version_end => (
                &trimmed[..version_start],
                &trimmed[version_end..start],
                &trimmed[end..],
                Some((date_format, DatePosition::AfterVersion)),
            ),
            Some((start, end, date_format)) if end <= version_start => (
                &trimmed[..start],
                &trimmed[end..version_start],
                &trimmed[version_end..],
                Some((date_format, DatePosition::BeforeVersion)),
            ),
            Some(_) => return Err(invalid()),
        };

        Ok(Self {
            level: format.len() - format.trim_start_matches('#').len(),
            prefix: prefix.into(),
            middle: middle.into(),
            suffix: suffix.into(),
            date,
        })
    }

    fn render(&self, version: &Version, date: time::Date) -> Option<String> {
        let Self {
            prefix,
            middle,
            suffix,
            ..
        } = self;
        Some(match &self.date {
            None => format!("{prefix}{version}{suffix}"),
            Some((date_format, DatePosition::AfterVersion)) => {
                format!(
                    "{prefix}{version}{middle}{}{suffix}",
                    date_format.render(date)?
                )
            }
            Some((date_format, DatePosition::BeforeVersion)) => {
                format!(
                    "{prefix}{}{middle}{version}{suffix}",
                    date_format.render(date)?
                )
            }
        })
    }

    /// The version of a heading line in this format, if the line is one. The date part must parse
    /// in the declared date format. A line that splits into more than one valid version and date
    /// is an error, never a silent pick.
    fn classify(&self, line: &str) -> Result<Option<Version>, ChangelogError> {
        let Some(rest) = line
            .trim_end()
            .strip_prefix(self.prefix.as_str())
            .and_then(|rest| rest.strip_suffix(self.suffix.as_str()))
        else {
            return Ok(None);
        };
        let Some((date_format, position)) = &self.date else {
            return Ok(rest.parse().ok());
        };
        let mut candidates = rest
            .char_indices()
            .map(|(index, _)| index)
            .chain([rest.len()])
            .filter_map(|index| {
                let (version, date) = match position {
                    DatePosition::BeforeVersion => (
                        rest[index..].strip_prefix(self.middle.as_str())?,
                        &rest[..index],
                    ),
                    DatePosition::AfterVersion => (
                        rest[..index].strip_suffix(self.middle.as_str())?,
                        &rest[index..],
                    ),
                };
                date_format.parse_date(date)?;
                version.parse::<Version>().ok()
            });
        let version = candidates.next();
        if candidates.next().is_some() {
            return Err(ChangelogError::AmbiguousHeading(line.trim_end().into()));
        }
        Ok(version)
    }

    /// The version of a heading line; ambiguous lines were rejected by [`Self::check`] first.
    fn version(&self, line: &str) -> Option<Version> {
        self.classify(line).ok().flatten()
    }

    /// Fails on the first heading line that splits into more than one version and date.
    fn check(&self, contents: &str) -> Result<(), ChangelogError> {
        heading_lines(contents).try_for_each(|(_, _, line)| self.classify(line).map(|_| ()))
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

/// A strftime date format, formatted and parsed by the `time` crate.
#[derive(Clone)]
struct DateFormat {
    format: String,
    items: OwnedFormatItem,
}

impl DateFormat {
    /// The format, if `time` accepts it and reads back the full date it writes.
    fn parse(format: &str) -> Option<Self> {
        if format.is_empty() || format.contains(['{', '}']) {
            return None;
        }
        let date_format = Self {
            format: format.into(),
            items: format_description::parse_strftime_owned(format).ok()?,
        };
        let sample = time::Date::from_calendar_date(2026, time::Month::November, 5).ok()?;
        let rendered = date_format.render(sample)?;
        (date_format.parse_date(&rendered)? == sample).then_some(date_format)
    }

    fn render(&self, date: time::Date) -> Option<String> {
        date.format(&self.items).ok()
    }

    /// The date if all of `text` is one in this format.
    fn parse_date(&self, text: &str) -> Option<time::Date> {
        time::Date::parse(text, &self.items).ok()
    }
}

impl PartialEq for DateFormat {
    fn eq(&self, other: &Self) -> bool {
        self.format == other.format
    }
}

impl Eq for DateFormat {}

impl fmt::Debug for DateFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DateFormat")
            .field(&self.format)
            .finish()
    }
}

/// The release date, given as `YYYY-MM-DD` as `prepare` passes it.
fn parse_release_date(date: &str) -> Option<time::Date> {
    DateFormat::parse(DEFAULT_DATE_FORMAT)?.parse_date(date)
}

pub fn insert_version_section(
    contents: Option<&str>,
    heading: &ChangelogHeading,
    version: &Version,
    date: &str,
    notes: &str,
) -> Result<String, ChangelogError> {
    let date = parse_release_date(date).ok_or_else(|| ChangelogError::InvalidDate(date.into()))?;

    let contents = contents.unwrap_or("");
    heading.check(contents)?;
    if find_version_heading(contents, heading, version)?.is_some() {
        return Err(ChangelogError::DuplicateVersion(version.clone()));
    }

    let line_ending = line_ending(contents, notes);
    let heading_line = heading
        .render(version, date)
        .ok_or_else(|| ChangelogError::InvalidDate(date.to_string()))?;
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
    heading.check(contents)?;
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
    AmbiguousHeading(String),
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
            Self::AmbiguousHeading(line) => write!(
                formatter,
                "changelog heading {line:?} matches changelog.heading with more than one version and date; put literal text between {{version}} and {{date}}"
            ),
            Self::InvalidDate(date) => write!(formatter, "invalid changelog date {date:?}"),
            Self::InvalidHeading(format) => write!(
                formatter,
                "changelog.heading {format:?} must be one line that starts with #, with {{version}} exactly once and {{date}} or {{date:<format>}} at most once, where the format is a strftime format that writes and reads back a full date"
            ),
        }
    }
}

impl Error for ChangelogError {}
