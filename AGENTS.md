# Agent instructions

Read [CONTRIBUTING.md](CONTRIBUTING.md) for the development commands, the commit convention, and
pull requests. This file only adds what agents tend to miss.

## Changelog

Every user-visible change adds one hand-written line to `CHANGELOG.md` under `## [Unreleased]`.
If that section is missing, create it directly below `# Changelog`. Write the line from the
user's point of view, in the style of the existing highlights, and wrap it at 100 columns:

```markdown
## [Unreleased]

- ✨ A `cargo` publisher uploads a crate built by the flake to crates.io or any sparse-index
  registry.
```

The line starts with the Gitmoji of the change and is one sentence. On release, `prepare`
replaces the `## [Unreleased]` section with the new version heading and puts the hand-written
lines above the folded list of commits that it generates from the commit subjects. The lines
alone do not cause a release.

Changes that are not user-visible, such as CI, refactors, and internal tooling, need no line.
