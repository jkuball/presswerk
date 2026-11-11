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
  <a href="https://presswerk.dev/">Documentation</a> ·
  <a href="https://github.com/jkuball/presswerk/releases">Releases</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

<p align="center">
  <a href="https://presswerk.dev/"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/site/public/badge-light.svg"><img alt="Released with Presswerk" src="docs/site/public/badge.svg" width="164" height="20"></picture></a>
  <img alt="Version" src=".github/presswerk-badge.svg" height="20">
</p>

> [!WARNING]
> Presswerk is early, work-in-progress software. It releases itself, but its commands,
> configuration, and output are not stable yet. Do not rely on it for production releases.

Presswerk evaluates a release declaration from an exact Git commit, prepares committed versions
and release notes, then publishes the matching Git tag and GitHub Release. It keeps no database
or hosted state of its own.

## Documentation

Start with the [guide](https://presswerk.dev/guide/) and its
[walkthrough](https://presswerk.dev/guide/walkthrough/). Design notes live in
[`docs/conception`](docs/conception/).

## Quick look

Declare a release in your flake:

```nix
{
  inputs.presswerk.url = "github:jkuball/presswerk";

  outputs = { self, presswerk, ... }: {
    presswerk.default = presswerk.lib.mkRelease {
      sourceRoot = self.outPath;
      # Read and bump the version of a Cargo package.
      versionFiles.cargo.my-crate = { };
      # Keep a version badge in the repository.
      versionFiles.badge.default = { };
      # Publish a GitHub Release.
      publishers.github-release.default.repository = "me/my-crate";
    };
  };
}
```

Then let Presswerk take it from there:

```console
$ presswerk plan          # show the next version and notes, change nothing
$ presswerk prepare       # commit the version bump and changelog
$ presswerk publish HEAD  # tag and release the checked commit
```

The [walkthrough](https://presswerk.dev/guide/walkthrough/) sets this up end to end, including
CI and how to run the CLI from your locked input.

## Development

```console
$ nix develop
$ nix flake check
```

Commit subjects follow [Gitmoji](https://gitmoji.dev/), which Presswerk also uses to determine
its own release version and notes.

## License

Presswerk is licensed under either the [MIT license](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option. Unless stated otherwise, contributions are
licensed on the same terms.

The vendored [Gitmoji catalog](modules/release/gitmoji/README.md) is licensed under the MIT
license by its upstream authors.
