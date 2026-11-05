use semver::Version;
use std::error::Error;
use std::fmt;

pub fn insert_version_section(
    contents: Option<&str>,
    version: &Version,
    date: &str,
    notes: &str,
) -> Result<String, ChangelogError> {
    if date.is_empty() || date.contains(['\r', '\n']) {
        return Err(ChangelogError::InvalidDate(date.into()));
    }

    let contents = contents.unwrap_or("");
    if find_version_heading(contents, version)?.is_some() {
        return Err(ChangelogError::DuplicateVersion(version.clone()));
    }

    let line_ending = line_ending(contents, notes);
    let section = format!("## [{version}] - {date}{line_ending}{line_ending}{notes}{line_ending}");
    if contents.is_empty() {
        return Ok(format!("# Changelog{line_ending}{line_ending}{section}"));
    }

    let insertion = version_headings(contents)
        .next()
        .map_or(contents.len(), |(start, _, _)| start);
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
    version: &Version,
) -> Result<&'a str, ChangelogError> {
    let (_, heading_end) = find_version_heading(contents, version)?
        .ok_or_else(|| ChangelogError::MissingVersion(version.clone()))?;
    let body_start = heading_end + line_ending_prefix_len(&contents[heading_end..]);
    let body_end = lines_with_offsets(contents)
        .find(|(start, _, line)| *start >= body_start && line.starts_with("## "))
        .map_or(contents.len(), |(start, _, _)| start);

    let body = &contents[body_start..body_end];
    Ok(body
        .strip_suffix("\r\n")
        .or_else(|| body.strip_suffix('\n'))
        .unwrap_or(body))
}

fn find_version_heading(
    contents: &str,
    version: &Version,
) -> Result<Option<(usize, usize)>, ChangelogError> {
    let mut matches = version_headings(contents)
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

fn version_headings(contents: &str) -> impl Iterator<Item = (usize, usize, Version)> + '_ {
    lines_with_offsets(contents).filter_map(|(start, end, line)| {
        let version = line.strip_prefix("## [")?.split_once(']')?.0.parse().ok()?;
        Some((start, end, version))
    })
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
        }
    }
}

impl Error for ChangelogError {}
