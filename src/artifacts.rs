use anyhow::Context as _;
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Temporary GC roots for the artifacts that [`fetch`] made valid.
///
/// The roots protect the artifacts from garbage collection only while this guard lives, so keep it
/// until every reader of the artifacts is done. Dropping it removes the roots on a best-effort
/// basis; if the process is killed, they stay until the temporary directory is cleaned up.
#[must_use = "the artifacts lose their GC roots when the guard is dropped"]
pub struct ArtifactRoots {
    _directory: Option<TempDir>,
}

/// Fetch every store path that a file in `paths` lies in, from the local store or a substituter.
///
/// The paths are outputs, not derivations, so `nix-store --realise` can only substitute them:
/// `publish` never builds an artifact. Each entry is reported by its label, such as
/// `github-release.default` and an asset name, when it cannot be obtained. Every fetched path gets a
/// temporary GC root that lasts as long as the returned [`ArtifactRoots`].
pub fn fetch<'a>(
    paths: impl IntoIterator<Item = (String, &'a str)>,
) -> Result<ArtifactRoots, ArtifactError> {
    fetch_from(None, paths)
}

/// [`fetch`] from the Nix store at `store`, or from the default store when it is `None`.
fn fetch_from<'a>(
    store: Option<&str>,
    paths: impl IntoIterator<Item = (String, &'a str)>,
) -> Result<ArtifactRoots, ArtifactError> {
    let paths: Vec<_> = paths.into_iter().collect();
    if paths.is_empty() {
        return Ok(ArtifactRoots { _directory: None });
    }
    let store_dir = store_dir(store).map_err(ArtifactError::Nix)?;
    let directory = tempfile::Builder::new()
        .prefix("presswerk-gc-roots-")
        .tempdir()
        .context("create a directory for temporary GC roots")
        .map_err(ArtifactError::Nix)?;

    for (index, (label, path)) in paths.into_iter().enumerate() {
        let root = store_path_root(&store_dir, path).ok_or_else(|| ArtifactError::NotInStore {
            label: label.clone(),
            path: path.into(),
        })?;
        substitute(store, &root, &directory.path().join(index.to_string())).map_err(|source| {
            ArtifactError::Unavailable {
                label,
                path: path.into(),
                source,
            }
        })?;
    }
    Ok(ArtifactRoots {
        _directory: Some(directory),
    })
}

/// A `command` that talks to the Nix store at `store`, or to the default store when it is `None`.
fn nix_command(program: &str, store: Option<&str>) -> Command {
    let mut command = Command::new(program);
    if let Some(store) = store {
        command.env("NIX_REMOTE", store);
    }
    command
}

fn store_dir(store: Option<&str>) -> anyhow::Result<String> {
    let output = nix_command("nix", store)
        .args([
            "eval",
            "--extra-experimental-features",
            "nix-command flakes",
            "--raw",
            "--expr",
            "builtins.storeDir",
        ])
        .output()
        .context("run nix eval to read the store directory")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim_end());
    }
    String::from_utf8(output.stdout).context("Nix store directory is not UTF-8")
}

/// Make an output path valid by substituting it; Nix cannot build a path without its derivation.
///
/// The path is kept alive by an indirect GC root at `root`.
fn substitute(store: Option<&str>, store_path: &str, root: &Path) -> anyhow::Result<()> {
    let output = nix_command("nix-store", store)
        .args(["--realise", "--quiet", "--add-root"])
        .arg(root)
        .arg(store_path)
        .output()
        .context("run nix-store --realise")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!(
        "{}",
        stderr
            .lines()
            .rfind(|line| !line.trim().is_empty())
            .unwrap_or("nix-store --realise failed")
    )
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
    use super::{fetch_from, nix_command, store_path_root};

    fn nix_store(store: &str, args: &[&str]) -> String {
        let output = nix_command("nix-store", Some(store))
            .args(args)
            .output()
            .expect("run nix-store");
        assert!(
            output.status.success(),
            "nix-store {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn is_valid(store: &str, path: &str) -> bool {
        nix_command("nix-store", Some(store))
            .args(["--check-validity", path])
            .output()
            .expect("run nix-store")
            .status
            .success()
    }

    #[test]
    fn keeps_fetched_artifacts_from_garbage_collection_while_the_roots_live() {
        // A temporary local store, so the test never touches the host's store.
        let root = tempfile::tempdir().unwrap();
        let store = format!(
            "local?root={}",
            root.path().canonicalize().unwrap().display()
        );
        let source = root.path().join("artifact.txt");
        std::fs::write(&source, "artifact\n").unwrap();
        let path = nix_store(&store, &["--add", source.to_str().unwrap()]);

        let roots = fetch_from(Some(&store), [("test artifact".to_owned(), path.as_str())])
            .expect("fetch the artifact");
        nix_store(&store, &["--gc"]);
        assert!(is_valid(&store, &path), "{path} was collected while rooted");

        drop(roots);
        nix_store(&store, &["--gc"]);
        assert!(!is_valid(&store, &path), "{path} survived without a root");
    }

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
