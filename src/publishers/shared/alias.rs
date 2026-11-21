//! Aliases: pointers that follow a line of versions, such as the OCI tag `latest` or the npm
//! dist-tag `release-{major}.{minor}`. An alias template may use `{major}`, `{minor}`, and
//! `{patch}`, but not `{version}`, which names a single version. An alias moves only to a higher
//! final version than the one it points to, so `{major}` stays within its major version and a
//! maintenance release leaves `latest` alone.

use crate::publishers::shared::template;
use semver::Version;

/// The placeholder that names a single version, which an alias must not contain.
pub const VERSION_PLACEHOLDER: &str = "{version}";

/// The first alias template that names a single version, which a publisher rejects.
#[must_use]
pub fn single_version_template(aliases: &[String]) -> Option<&String> {
    aliases
        .iter()
        .find(|alias| alias.contains(VERSION_PLACEHOLDER))
}

/// Renders alias templates for a version, in order.
#[must_use]
pub fn render(aliases: &[String], version: &Version) -> Vec<String> {
    aliases
        .iter()
        .map(|alias| template::render(alias, version))
        .collect()
}

/// Where an alias stands for a release, by the version it points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Position {
    /// The alias does not exist; it is set to the release.
    Missing,
    /// The alias points to a lower version; it moves to the release.
    Behind,
    /// The alias points to the release's version already.
    Same,
    /// The alias is left alone, with the reason: the release is a pre-release, or the alias
    /// points to a higher version.
    Kept(String),
}

impl Position {
    /// Whether the alias is set or moved to the release.
    #[must_use]
    pub fn moves(&self) -> bool {
        matches!(self, Self::Missing | Self::Behind)
    }
}

/// Where an alias that points to `current`, or to nothing, stands for the release of `version`.
/// A pre-release moves no alias.
#[must_use]
pub fn position(current: Option<&Version>, version: &Version) -> Position {
    if !version.pre.is_empty() {
        return Position::Kept("pre-release".into());
    }
    match current {
        None => Position::Missing,
        Some(current) if current > version => Position::Kept(format!("{current} is higher")),
        Some(current) if current == version => Position::Same,
        Some(_) => Position::Behind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn an_alias_moves_only_to_a_higher_final_version() {
        let release = version("1.4.1");
        assert_eq!(position(None, &release), Position::Missing);
        assert_eq!(
            position(Some(&version("1.4.0")), &release),
            Position::Behind
        );
        // A final version is higher than its pre-releases.
        assert_eq!(
            position(Some(&version("1.4.1-rc.1")), &release),
            Position::Behind
        );
        assert_eq!(position(Some(&release), &release), Position::Same);
        assert_eq!(
            position(Some(&version("2.0.0")), &release),
            Position::Kept("2.0.0 is higher".into())
        );
        for current in [None, Some(version("1.0.0"))] {
            assert_eq!(
                position(current.as_ref(), &version("2.0.0-rc.1")),
                Position::Kept("pre-release".into())
            );
        }
        assert!(Position::Missing.moves() && Position::Behind.moves());
        assert!(!Position::Same.moves() && !Position::Kept(String::new()).moves());
    }

    #[test]
    fn renders_aliases_and_finds_single_version_templates() {
        let aliases = ["latest".to_owned(), "release-{major}.{minor}".to_owned()];
        assert_eq!(
            render(&aliases, &version("1.2.3")),
            ["latest", "release-1.2"]
        );
        assert_eq!(single_version_template(&aliases), None);
        let aliases = ["latest".to_owned(), "{version}-slim".to_owned()];
        assert_eq!(
            single_version_template(&aliases).map(String::as_str),
            Some("{version}-slim")
        );
    }
}
