# Presswerk

Bringing controlled outside world mutation into your immutable software builds.

Presswerk is intended to become a FOSS release tool for projects whose release artifacts are
built with Nix flakes. Flakes already take care of dependencies and packaging; Presswerk should
take care of the release cycle: versions, tags, forge releases, release notes, and publication
to registries. It should be a standalone Rust CLI that reads its release configuration directly
from the project's flake, with an optional flake-parts module for convenience. Releases should
work from CI and from a local machine alike, for public FOSS and private, proprietary
repositories alike.

The name means *pressing plant*: one immutable master record, the flake at an exact commit, is
pressed into every published release. See the [glossary](docs/conception/glossary.md).

The project is currently under active development and has no stable interface yet. Start with
the [documentation index](docs/conception/README.md). Presswerk releases itself from `main` after
CI passes; see [release recovery](docs/release-recovery.md) for the manual failure paths.
