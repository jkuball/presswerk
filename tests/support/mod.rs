#![allow(dead_code)]

use presswerk::analysis::Bump;
use presswerk::declaration::{
    ConventionalCatalog, ConventionalCatalogEntry, GitmojiCatalog, GitmojiCatalogEntry,
};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

pub fn gitmoji_catalog() -> GitmojiCatalog {
    let entry = |emoji: &str, description: &str, semver| GitmojiCatalogEntry {
        emoji: emoji.into(),
        description: description.into(),
        semver,
    };
    BTreeMap::from([
        (
            "bookmark".into(),
            entry("🔖", "Release / Version tags.", None),
        ),
        (
            "boom".into(),
            entry("💥", "Introduce breaking changes.", Some(Bump::Major)),
        ),
        ("bug".into(), entry("🐛", "Fix a bug.", Some(Bump::Patch))),
        (
            "memo".into(),
            entry("📝", "Add or update documentation.", None),
        ),
        (
            "rewind".into(),
            entry("⏪️", "Revert changes.", Some(Bump::Patch)),
        ),
        (
            "sparkles".into(),
            entry("✨", "Introduce new features.", Some(Bump::Minor)),
        ),
        ("tada".into(), entry("🎉", "Begin a project.", None)),
        (
            "twisted_rightwards_arrows".into(),
            entry("🔀", "Merge branches.", None),
        ),
    ])
}

/// The default Conventional Commits catalog of the release module.
pub fn conventional_catalog() -> ConventionalCatalog {
    let entry = |title: &str, semver| ConventionalCatalogEntry {
        title: title.into(),
        semver,
    };
    BTreeMap::from([
        ("build".into(), entry("Build system", None)),
        ("chore".into(), entry("Chores", None)),
        ("ci".into(), entry("Continuous integration", None)),
        ("docs".into(), entry("Documentation", None)),
        ("feat".into(), entry("Features", Some(Bump::Minor))),
        ("fix".into(), entry("Bug fixes", Some(Bump::Patch))),
        ("perf".into(), entry("Performance", Some(Bump::Patch))),
        ("refactor".into(), entry("Refactoring", None)),
        ("revert".into(), entry("Reverts", Some(Bump::Patch))),
        ("style".into(), entry("Code style", None)),
        ("test".into(), entry("Tests", None)),
    ])
}

pub struct GitRepo {
    directory: TempDir,
}

impl GitRepo {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().expect("create temporary Git repository");
        let repository = Self { directory };

        repository.git(["init", "--quiet"]);
        repository.git(["config", "user.name", "Presswerk Tests"]);
        repository.git(["config", "user.email", "presswerk@example.invalid"]);
        repository.git(["config", "commit.gpgSign", "false"]);
        repository.git(["config", "tag.gpgSign", "false"]);
        fs::create_dir_all(repository.path().join(".git/disabled-hooks"))
            .expect("create empty Git hooks directory");
        repository.git(["config", "core.hooksPath", ".git/disabled-hooks"]);
        repository
    }

    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    pub fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
        let path = self.path().join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create test repository directory");
        }
        fs::write(path, contents).expect("write test repository file");
    }

    pub fn commit(&self, message: &str) -> String {
        self.git(["add", "--all"]);
        self.git(["commit", "--quiet", "--message", message]);
        self.head()
    }

    pub fn commit_version(&self, version: &str, message: &str) -> String {
        self.write("VERSION", format!("{version}\n"));
        self.commit(message)
    }

    pub fn tag(&self, name: &str) {
        self.git(["tag", name]);
    }

    pub fn annotated_tag(&self, name: &str) {
        self.git(["tag", "--annotate", "--message", name, name]);
    }

    pub fn head(&self) -> String {
        self.git(["rev-parse", "HEAD"])
    }

    pub fn version_at(&self, commit: &str) -> String {
        self.git(["show", &format!("{commit}:VERSION")])
    }

    pub fn git<I, S>(&self, arguments: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.git_output(arguments);
        if !output.status.success() {
            panic!(
                "git failed with {}:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8(output.stdout)
            .expect("Git output is UTF-8")
            .trim()
            .to_owned()
    }

    pub fn git_output<I, S>(&self, arguments: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Command::new("git")
            .args(arguments)
            .current_dir(self.path())
            .output()
            .expect("run Git")
    }
}
