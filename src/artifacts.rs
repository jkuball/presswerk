use crate::evaluation::NIX_EVALUATION;
use anyhow::Context as _;
use nix_bindings_store::store::Store;
use std::error::Error;
use std::fmt;

/// Fetch every store path that a file in `paths` lies in, from the local store or a substituter.
///
/// The paths are outputs, not derivations, so Nix can only substitute them: `publish` never builds
/// an artifact. Each entry is reported by its label, such as `github-release.default` and an asset
/// name, when it cannot be obtained.
pub fn fetch<'a>(paths: impl IntoIterator<Item = (String, &'a str)>) -> Result<(), ArtifactError> {
    let paths: Vec<_> = paths.into_iter().collect();
    if paths.is_empty() {
        return Ok(());
    }
    let _nix = NIX_EVALUATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    nix_bindings_expr::eval_state::init()
        .context("initialize Nix")
        .map_err(ArtifactError::Nix)?;
    let mut store = Store::open(None, [])
        .context("open the Nix store")
        .map_err(ArtifactError::Nix)?;
    let store_dir = store
        .get_storedir()
        .context("read the Nix store directory")
        .map_err(ArtifactError::Nix)?;

    for (label, path) in paths {
        let root = store_path_root(&store_dir, path).ok_or_else(|| ArtifactError::NotInStore {
            label: label.clone(),
            path: path.into(),
        })?;
        let store_path = store
            .parse_store_path(&root)
            .context("parse the store path")
            .map_err(|source| ArtifactError::Unavailable {
                label: label.clone(),
                path: path.into(),
                source,
            })?;
        store
            .realise(&store_path)
            .map_err(|source| ArtifactError::Unavailable {
                label,
                path: path.into(),
                source,
            })?;
    }
    Ok(())
}

/// The top-level store path that contains `path`, such as `/nix/store/…-tool` for
/// `/nix/store/…-tool/bin/tool`.
fn store_path_root(store_dir: &str, path: &str) -> Option<String> {
    let name = path
        .strip_prefix(store_dir)?
        .strip_prefix('/')?
        .split('/')
        .next()
        .filter(|name| !name.is_empty())?;
    Some(format!("{store_dir}/{name}"))
}

#[derive(Debug)]
pub enum ArtifactError {
    Nix(anyhow::Error),
    NotInStore {
        label: String,
        path: String,
    },
    Unavailable {
        label: String,
        path: String,
        source: anyhow::Error,
    },
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nix(error) => write!(formatter, "{error:#}"),
            Self::NotInStore { label, path } => {
                write!(formatter, "{label} ({path}) is not in the Nix store")
            }
            Self::Unavailable {
                label,
                path,
                source,
            } => write!(
                formatter,
                "{label} ({path}) is not in the local store or any substituter; did CI build and push it? {source:#}"
            ),
        }
    }
}

impl Error for ArtifactError {}

#[cfg(test)]
mod tests {
    use super::store_path_root;

    #[test]
    fn finds_the_store_path_that_contains_a_file() {
        assert_eq!(
            store_path_root("/nix/store", "/nix/store/abc-tool-1.0/bin/tool").as_deref(),
            Some("/nix/store/abc-tool-1.0")
        );
        assert_eq!(
            store_path_root("/nix/store", "/nix/store/abc-tool.tar.gz").as_deref(),
            Some("/nix/store/abc-tool.tar.gz")
        );
        assert_eq!(store_path_root("/nix/store", "/tmp/tool"), None);
        assert_eq!(store_path_root("/nix/store", "/nix/storefoo/x"), None);
    }
}
