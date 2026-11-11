//! Print the version badge for a version, as the documentation site shows it.

use presswerk::declaration::{BadgeOptions, BadgeVersionFile};
use presswerk::version::VersionFileWriter;

fn main() -> anyhow::Result<()> {
    let version = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: version-badge VERSION"))?;
    let badge = BadgeVersionFile {
        path: "badge.svg".into(),
        badge: BadgeOptions {},
    };
    print!("{}", badge.write_version("", &version.parse()?)?);
    Ok(())
}
