<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/site/src/assets/brand/lockup-dark.svg">
    <img alt="Presswerk" src="docs/site/src/assets/brand/lockup-light.svg" width="420">
  </picture>
</h1>

<p align="center">
  Bringing controlled outside world mutation into your immutable software builds.
</p>

<p align="center">
  <a href="https://github.com/jkuball/presswerk/releases">Releases</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="docs/conception/README.md">Conception</a>
</p>

> [!WARNING]
> Presswerk is early, work-in-progress software. It already releases itself, but it has no
> stable interface yet: commands, the release declaration, and output may change in any
> version. Do not rely on it for your own releases yet.

Presswerk is a FOSS release tool for projects whose release artifacts are built with Nix flakes.
Flakes already take care of dependencies and packaging; Presswerk takes care of the release
cycle: versions, tags, forge releases, release notes, and eventually publication to registries.
It is meant to work from CI and from a local machine alike, for public FOSS and private,
proprietary repositories alike.

The name means *pressing plant*: one immutable master record, the flake at an exact commit, is
pressed into every published release. See the [glossary](docs/conception/glossary.md).

## Ideas

- **The flake is the configuration.** A standalone Rust CLI reads the release declaration
  from your flake. It evaluates the flake in-process at an exact commit, so uncommitted changes
  never leak into a release, and there is no second configuration language.
- **The version is committed first.** A release commit contains the new version and frozen
  release notes, so CI builds and checks the exact candidate before anything is published.
- **No state of its own.** Git commits, Git tags, and the publication destinations hold
  everything. There is no database, state file, or hosted service.
- **Idempotent publication.** Rerunning a candidate completes missing state and never silently
  moves or overwrites existing state.
- **Private repositories are first-class.** Self-hosted forges, private registries, and
  organization credentials are ordinary configuration, not an afterthought.

## Status

The current milestone is [self-release](docs/conception/roadmap.md#self-release-milestone):
Presswerk releases itself from `main` after CI passes. What works today:

- reading a release declaration from the flake at an exact commit;
- [Gitmoji](https://gitmoji.dev/) commit analysis since the latest release tag, choosing the
  next semantic version;
- a release commit that updates `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md`;
- publishing an annotated Git tag and a GitHub Release with the release notes.

Other forges, registries, release assets, Conventional Commits, and local release workflows are
planned but not implemented. See the [roadmap](docs/conception/roadmap.md).

## A quick look

A release is declared with `lib.mkRelease`, which evaluates a Nix module and produces plain
data at the `presswerk.default` flake output. A Rust project releasing to GitHub looks like this:

```nix
{
  inputs.presswerk.url = "github:jkuball/presswerk";

  outputs = { self, presswerk, ... }: {
    presswerk.default = presswerk.lib.mkRelease {
      sourceRoot = self.outPath;
      versionFiles.cargo.my-crate = { };
      publishers.github-release.default.repository = "owner/project";
    };
  };
}
```

The CLI then works on the committed state of the repository:

```console
$ nix run github:jkuball/presswerk -- plan      # show the next release, write nothing
$ nix run github:jkuball/presswerk -- prepare   # create the release commit
$ nix run github:jkuball/presswerk -- publish   # tag and publish an exact candidate
```

`publish` reads the GitHub token from `GITHUB_TOKEN`. Presswerk's own
[declaration](modules/release/self.nix) and [CI workflow](.github/workflows/ci.yml) show the
complete automatic flow, and [release recovery](docs/release-recovery.md) describes what to do
when a release fails halfway.

## Development

Everything is built and tested through the flake:

```console
$ nix develop          # development shell
$ nix flake check      # Rust tests, Nix module tests, formatting
```

Commit subjects follow [Gitmoji](https://gitmoji.dev/), because Presswerk analyzes its own
history to decide its releases. A commit-msg hook in the development shell checks this.

## Documentation

The [conception documents](docs/conception/README.md) describe the intended design: motivation,
product shape, release model, publication, and the decisions that are still open. They describe
intent, not an implementation contract.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.

The vendored [Gitmoji catalog](modules/release/gitmoji/README.md) is licensed under the MIT license by its
upstream authors.
