# Roadmap

## First consumers

The design should be driven by real repositories. The public ones are listed here. Private and
proprietary repositories are equally important design drivers (see
[target users](overview.md#target-users)), but they are not named in this repository.

### Presswerk itself

Presswerk should release itself as soon as a minimal path works. This proves the Rust CLI, Nix
interface, a GitHub Actions setup, Git tag, forge release, notes, and retry behavior.

### nitpick.nvim

`nitpick.nvim` currently:

- builds on Linux and macOS with GitHub Actions;
- uses flake-parts;
- keeps its effective version in `flake.nix`;
- mirrors that version in a Claude Code plugin manifest;
- checks that both versions agree.

It tests synchronized version files and a release that may only need a tag and forge release.

### ferret-ingest-agents

`ferret-ingest-agents` currently:

- builds on Linux and macOS with GitHub Actions;
- uses flake-parts and Rust tooling;
- keeps its version in `Cargo.toml`;
- can be installed through Nix and Cargo.

It tests Cargo metadata, crates.io publication, release binaries, and forge releases.

### Later

Later consumers should be able to add OCI images, Helm charts, PyPI, npm, and other registries,
public or private, without changing the core release model.

## Self-release milestone

Before the release model is implemented, an early go/no-go checkpoint proves that embedding
Nix works for this product:

- exact-commit evaluation and extraction of artifact references through nix-bindings-rust;
- substitution-only realisation, where a missing output fails instead of building;
- evaluation that cannot build: import from derivation satisfied by substitution or failing
  with a clear error;
- private inputs fetched with Nix-level authentication;
- working Linux and macOS packages of the CLI under the intended installation contract,
  outside the development shell.

The smallest useful bootstrap is:

1. build and test the Rust CLI with Nix;
1. declare the release in Presswerk's own flake and load it with nix-bindings-rust;
1. select a version first, using commit analysis through the git-cliff analyzer;
1. prepare and commit that version together with its section in `CHANGELOG.md`;
1. run a GitHub Actions release job after all checks;
1. create an exact Git tag and GitHub Release with frozen notes;
1. attach CLI binaries for at least `x86_64-linux` and `aarch64-darwin` to that release,
   packaged under the installation contract proven by the checkpoint. A fully static Linux
   binary is possible; macOS does not support fully static executables, so its binary bundles
   all non-system dependencies and links the system libraries dynamically;
1. retry that exact release without duplicating or moving external state;
1. republish it from its tag.

Afterward, migrate either `nitpick.nvim` or `ferret-ingest-agents`. Its different version
source or publisher should determine the next abstraction.

Presswerk's own repository is currently private. Running the self-release from a private
repository is a useful early check of the proprietary path.

## Testing

Presswerk is implemented test-driven. Its whole point is handling the outside world correctly
when things go wrong, so the edge cases are the product:

- every case in [scenarios](scenarios.md) becomes a test, including partial releases,
  conflicts, reverts, freed versions, and invalid release commits;
- version generators: stray outputs, rejected paths and symlinks, merge conflicts between
  writers, files staged by another process while `prepare` runs, and an interruption between
  the branch move and the checkout synchronization;
- mutable aliases: republishing an older version, a slow older run, maintenance releases, and
  a retry after a failed alias update;
- crash cases between steps, such as a crash after uploading one asset to a draft release,
  where a rerun must finish that same draft instead of creating another release;
- most tests run as local Nix checks against bare Git repositories, with no network;
- forge behavior is covered later by NixOS VM tests against a real instance, such as Gitea or
  Forgejo.

## Documentation

User documentation is written alongside the first working workflow: installation, adoption,
and especially recovery, which is part of this product's correctness. Branding (logo, color
scheme, tagline, see [open decisions](open-decisions.md#product)) only affects how the
documentation looks, not when it is written. Already planned for it:

- a [NuschtOS search](https://github.com/NuschtOS/search) page for the options of the
  flake-parts module;
- an adoption guide: which state an existing repository has to be in before its first release
  with Presswerk. Most users add Presswerk to an existing flake, so this guide, the
  flake-parts module, and its options search are the main onboarding path.

## Agent skill

The repository ships a skill that teaches coding agents how to use the CLI. The CLI may also
print it, through `presswerk --skill` or a `presswerk skill` subcommand, so an agent always
gets the version that matches the installed CLI.

## Flake template

New projects start from a flake template (`nix flake init -t github:jkuball/presswerk`)
instead of a `presswerk init` command. A project created from it releases `0.1.0` from its
very first commit, without any manual tagging.
