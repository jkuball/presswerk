# Vendored Gitmoji catalog

`catalog.json` contains the `emoji`, `description`, and `semver` fields from
[`packages/gitmojis/src/gitmojis.json`](https://github.com/carloscuesta/gitmoji/blob/c7e067f43a33c5d9d42450314977efa409101308/packages/gitmojis/src/gitmojis.json)
at upstream commit `c7e067f43a33c5d9d42450314977efa409101308`. Its object keys are the upstream
shortcodes without surrounding colons.

The catalog is the default for the release declaration, so commit analysis is deterministic and
works offline while projects can override or extend it through the Nix module system. Upstream
licenses this material under the MIT license in `LICENSE`.
