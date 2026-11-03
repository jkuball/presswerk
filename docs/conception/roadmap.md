# Roadmap

GitHub milestones and issues will track implementation. Issues should link to the relevant
conception documents. Those documents remain local product-design references; the roadmap and
future feature requests may move to GitHub later. This document records the current self-release
milestone and future directions without committing to a complete task breakdown for later
milestones.

## Self-release milestone

Presswerk releases itself automatically from `main`. The intended experience is that developers
work normally and release-worthy changes that pass CI get released without a separate manual
release step.

The milestone is complete when:

1. Presswerk's Rust CLI is built and tested through its flake on `x86_64-linux`. Its initial
   Cargo version is `0.0.0`. The flake uses [rust-flake](https://github.com/juspay/rust-flake),
   based on its [single-crate example](https://github.com/juspay/rust-flake/blob/main/examples/single-crate/flake.nix).
1. Presswerk reads its own plain release declaration from the flake at an exact commit.
1. Gitmoji commit analysis determines whether to release and selects the version from the full
   range since the latest release tag, not just the latest commit. A repository without a
   previous release starts from an implicit `0.0.0`, including its root commit in the analysis,
   without a manual bootstrap command or tag.
1. A release commit contains the selected version and frozen commit-derived notes. Minijinja
   renders notes containing at least each Gitmoji commit's subject, including emojis that do
   not trigger a bump. Unicode and shortcode spellings share a group; larger groups come first
   and ties sort by shortcode. Whether the default includes bodies can be decided during
   implementation. CI builds and checks that exact commit before publication.
1. A Git tag points to that commit and a GitHub Release contains its notes. Retrying the same
   candidate completes missing state without silently moving or overwriting existing state. A
   missing tag or incomplete GitHub Release breaks the chain and blocks the next release until
   the previous candidate is handled.
1. An initial release and a subsequent Gitmoji-triggered release demonstrate the complete
   automated path in Presswerk's private GitHub repository.
1. Pushes with no release-worthy changes do not create releases. A run stays pinned to the
   commit it started for, and never silently includes later pushes. Each later push has its own
   CI run; a stale run may stop instead of releasing. The runtime serializes release work, with
   no separate Presswerk locking feature.

Only `x86_64-linux` runs in CI for this milestone, and the GitHub Release does not need binary
assets. Multi-architecture builds and artifact distribution are later work. Presswerk itself
does not require CI to build unrelated flake outputs; CI must provide the artifacts being
released, and consumers choose additional checks. Presswerk's own CI will build and check its
relevant buildable flake outputs, including packages, checks, dev shells, and the formatter.
Following the reference workflow, CI runs for pull requests and pushes to `main`; publishing is
restricted to trusted `main` release candidates.

An early Nix integration spike confirmed that the CLI can read an exact-commit flake
declaration in-process with [nix-bindings-rust](https://github.com/nixops4/nix-bindings-rust),
so the CLI embeds Nix instead of invoking Nix tools. Artifact context extraction,
substitution-only artifact realization, private inputs, and import-from-derivation edge cases are
not gates for this asset-free milestone.

The intended work breakdown is currently:

1. Bootstrap the Rust CLI and Nix flake.
1. Test exact-commit flake evaluation from Rust.
1. Analyze Gitmoji commits and prepare release commits, with an internal version-file generator
   trait.
1. Publish tags and GitHub Releases safely, behind an internal publisher trait. These boundaries
   live in the main crate, not separate crates or a public plugin SDK.
1. Wire the automatic GitHub Actions flow, after the underlying behavior is tested.
1. Verify the first self-release and a subsequent automatic release.

The GitHub workflow is exercised in a separate private test repository first. The production
CI wiring is the last integration step, not a prerequisite for testing the core. Manual local
preparation is a future goal, not a prerequisite for this milestone. Failed releases require
manual recovery, including rerunning an appropriate CI job when needed; M1 has no autoheal.

## Testing

Rust unit and integration tests run as Nix checks. Temporary Git repositories and a mock
publisher or HTTP server cover analysis, version and notes preparation, exact candidate
identity, retries, conflicts, and overlapping work. Tests distinguish a release commit ready to
publish from a later commit inheriting its untagged version; they also cover a tag with an
incomplete GitHub Release and a revert that restores the previous tagged version. They also
verify that a non-triggering latest commit does not hide an earlier release-worthy commit since
the last tag. Tests cover failures relevant to the slice without requiring every future recovery
scenario to be implemented first.

A real private GitHub self-release is the final end-to-end test. A Forgejo VM is not needed for
the first milestone: it would exercise a different forge. Additional VM tests can be introduced
when a real requirement justifies them.

## Future directions

Potential later issues include:

- multi-architecture CI, release binaries, and artifact transfer or publication from multiple
  sources;
- local release preparation, richer recovery, and manual republishing from tags;
- private Nix inputs, artifact string contexts, substitution-only publication, and
  import-from-derivation behavior;
- a flake-parts consumer module, a public extension mechanism, and additional publishers;
- preparation branches, protected-branch workflows, maintenance lines, and pre-releases;
- configurable release conditions such as weekends only or at most once a day, combined with a
  scheduled CI workflow instead of an on-push workflow;
- observability, adoption documentation, a flake template, and an agent skill;
- option documentation and a [NuschtOS search](https://github.com/NuschtOS/search) page for the
  future flake-parts module.

The first working workflow should also be documented, including its recovery steps.

The design should also be driven by real consumers after Presswerk itself:

- `nitpick.nvim` uses flake-parts, keeps its version in `flake.nix`, and mirrors it in a plugin
  manifest. It would exercise synchronized version sources without necessarily shipping assets.
- `ferret-ingest-agents` is a Rust project with its version in `Cargo.toml`. It would exercise
  Cargo metadata, crates.io, and release binaries.
- Private and proprietary consumers must remain supported, including private forges and
  registries. They are not named here.

Future consumers and their requirements should determine which abstractions are worth making
public. See [open decisions](open-decisions.md) and the other conception documents for ideas
that have not yet been scheduled.
