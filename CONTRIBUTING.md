# Contributing to Presswerk

Thanks for helping. Bug reports, questions, and pull requests are all welcome. For anything
larger than a fix, open an issue first so we can agree on the shape before you build it.

## Development

Everything runs through the flake:

```console
$ nix develop        # tools, plus Git hooks for formatting and commit subjects
$ nix flake check    # builds, tests, lints, and the documentation
$ nix fmt            # formats Rust, Nix, and the rest
```

The documentation site lives in [`docs/site`](docs/site/); `pnpm dev` there serves it locally.
Design notes live in [`docs/conception`](docs/conception/).

## Commits

Commit subjects follow [Gitmoji](https://gitmoji.dev/): `<emoji> [(scope):] <message>`, for
example `🐛 (docs): Put the badge link on one line`. Presswerk releases itself from these
subjects, so the emoji decides the next version and the release-note group. The commit hook
from `nix develop` checks them.

## Pull requests

Every change goes through a pull request against `main`, and `nix flake check` must pass in CI.
Describe what the change does in its final state; the commits tell the history.

## License

Unless stated otherwise, contributions are licensed under the same terms as Presswerk: the
[MIT license](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your option.
