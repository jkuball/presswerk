# Presswerk

Controlled releases for software built with Nix flakes.

> [!WARNING]
> Presswerk is early, work-in-progress software. It releases itself, but its commands,
> configuration, and output are not stable yet. Do not rely on it for production releases.

Presswerk evaluates a release declaration from an exact Git commit, prepares committed versions
and release notes, then publishes the matching Git tag and GitHub Release. It keeps no database
or hosted state of its own.

## Documentation

The canonical documentation is at <https://jkuball.github.io/presswerk/>.

The repository also contains the current design notes under [`docs/conception`](docs/conception/)
and the self-release recovery guide at [`docs/release-recovery.md`](docs/release-recovery.md).

## Quick look

```console
$ nix run github:jkuball/presswerk -- plan
$ nix run github:jkuball/presswerk -- prepare
$ nix run github:jkuball/presswerk -- publish HEAD
```

`plan` is read-only. `prepare` creates a release commit, and `publish` releases an exact
candidate after CI has checked it.

## Development

```console
$ nix develop
$ nix flake check
```

Commit subjects follow [Gitmoji](https://gitmoji.dev/), which Presswerk also uses to determine
its own release version and notes.

## Security

Report vulnerabilities privately as described in [`SECURITY.md`](SECURITY.md).

## License

Presswerk is licensed under either the [MIT license](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option. Unless stated otherwise, contributions are
licensed on the same terms.

The vendored [Gitmoji catalog](src/gitmoji/README.md) is licensed under the MIT license by its
upstream authors.
