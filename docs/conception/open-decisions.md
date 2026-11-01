# Open decisions

Everything listed here is deliberately undecided. When a decision is made, record it in the
relevant document and remove it from this list.

## Product

- the license, which must allow unrestricted use in proprietary projects (see
  [overview](overview.md#target-users));
- whether Presswerk ships a reusable GitHub Action;
- branding: logo, color scheme, tagline, and whatever else the documentation and website need.
  Must be decided before `1.0.0`.

## Release flow

- the preparation-branch flow, deliberately after the first release (see
  [release model](release-model.md#preparation-branches-later)): recording the base the notes
  were rendered from, and whether human commits on the preparation branch are incorporated
  (`prepare --reuse`) or invalidate it.

## Release declaration

- shape: plain data output, validating library function, or runnable package (see
  [product shape](product-shape.md#shape-of-the-declaration));
- per-system handling (the output is `presswerk.default`; other names are not supported yet);
- exact analyzer options for conventions and backends;
- required Nix version for nix-bindings-rust, and whether the CLI is distributed outside Nix.

## Developer experience

- final commands and whether preparation is one command or a sequence;
- how the agent skill is delivered by the CLI: `--skill` or a `skill` subcommand;
- interactive confirmation versus fully non-interactive operation;
- machine-readable output.

## Versions and branches

- shape of the configuration for version files and replacements;
- build metadata;
- pre-release identifiers other than `rc` (`alpha`, `beta`) and switching between them;
- default branch policy;
- stable candidate integration.

## Commit analysis and changelog

- analyzer interface, independent of git-cliff;
- default handling of unknown commits;
- whether changelog-merge is merely documented or directly integrated;
- layout of the place for developer-written notes (such as `Unreleased`) and how it reaches
  templates;
- how changelogs are declared in the flake;
- for several changelogs: which changelog each publisher uses (see
  [release model](release-model.md#changelogs));
- the template context for rendering notes (see
  [release model](release-model.md#rendering-notes)).

## Secrets

- depth of the secretspec integration (see
  [runtime integration](runtime-integration.md#secrets));
- how publishers declare the secrets they need.

## Extensions and publication

- final adapter protocol for publishers and analyzers;
- how consumers ship private, organization-internal publishers;
- ordering of publishers after the tag;
- initial GitHub, Forgejo, crates.io, OCI, PyPI, and npm adapter order;
- OCI image input formats: `dockerTools` produces compressed archives that must be unpacked or
  converted before pushing to some registries (such as Google Artifact Registry), while
  nix2container describes images as JSON. Two separate requirements apply: the digest must
  describe the final published manifest or index, and a retry must reproduce exactly that
  representation. Preferred: the Nix build produces the registry-ready representation and its
  digest together, and the publisher only transports it. Does not block the self-release
  milestone, which publishes to GitHub only;
- tag type and signing;
- whether `publish` ever gets an explicit `--allow-build` escape hatch;
- what Presswerk itself must back up: nothing, if the release context is always derived again
  (see [overview](overview.md#no-state-of-its-own)); to be confirmed by the implementation.

## Observability

- see [observability](observability.md#open-questions).
