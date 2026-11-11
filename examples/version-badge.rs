//! Print the version badge for a version and colour scheme, as the documentation site shows it.

use presswerk::declaration::{BadgeOptions, BadgeVersionFile, ColorScheme};
use presswerk::version::VersionFileWriter;

fn main() -> anyhow::Result<()> {
    let version = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: version-badge VERSION [auto|light|dark]"))?;
    let color_scheme: ColorScheme = match std::env::args().nth(2) {
        Some(scheme) => serde_json::from_value(serde_json::Value::String(scheme))?,
        None => ColorScheme::Auto,
    };
    let badge = BadgeVersionFile {
        path: "badge.svg".into(),
        badge: BadgeOptions { color_scheme },
    };
    print!("{}", badge.write_version("", &version.parse()?)?);
    Ok(())
}
