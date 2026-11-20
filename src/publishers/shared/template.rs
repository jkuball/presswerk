//! Version templates, such as OCI tags and claim keys: `{version}`, `{major}`, `{minor}`, and
//! `{patch}` stand for the version and its parts. Nix renders the same templates with a sample
//! version.

use semver::Version;

/// Replaces the version placeholders of a template.
#[must_use]
pub fn render(template: &str, version: &Version) -> String {
    template
        .replace("{version}", &version.to_string())
        .replace("{major}", &version.major.to_string())
        .replace("{minor}", &version.minor.to_string())
        .replace("{patch}", &version.patch.to_string())
}
