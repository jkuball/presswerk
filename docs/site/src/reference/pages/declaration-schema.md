:::note
You do not need this page to use Presswerk. Configure releases through the
[release options](/reference/options/); this page only describes the data they produce, for
debugging or for tools that read it.
:::

`lib.mkRelease` evaluates the release options and returns a plain declaration: JSON-compatible data
with no functions, paths, or module internals. Your flake exposes it as `presswerk.default`, and the
CLI evaluates that output at an exact commit.

## Example

Presswerk's own release settings, evaluated with this version of `lib.mkRelease`. The Gitmoji
catalog is shortened to two of its <!-- reference:catalog-size --> entries:

<!-- reference:example -->

## Fields

:::caution
Before Presswerk 1.0, the shape may change with any Presswerk version, and the CLI refuses a
declaration it cannot decode instead of guessing; see
[recovery](/guide/recovery/#the-previous-release-uses-an-unsupported-declaration). Presswerk 1.0
introduces `schemaVersion = 1`, and from then on newer CLIs also read older declarations.
:::

Every field is required, and unknown fields are rejected.

- `schemaVersion`: the integer `0`.
- `version`: the effective release version, such as `"1.4.0"` or `"2.0.0-rc.1"`.
- `tag.format`: the release tag format, containing `{version}` exactly once.
- `changelog.path`: the repository-relative path of the changelog.
- `versionFiles`: the files whose version `prepare` updates. Each entry has a repository-relative
  `path` and either `toml` or `json`, a dotted key selector such as `"package.version"`, `plain`,
  an empty object for a file that contains only the version, `regex`, a pattern with a `version`
  capture group, `cargoLock`, the name of the package
  whose entry in `Cargo.lock` is updated, or `badge` for a version badge
  that `prepare` generates and never reads, with the `colorScheme` it is made for: `"auto"`,
  `"light"`, or `"dark"`.
- `analyzer`: the commit `convention`, currently always `"gitmoji"`, and the Gitmoji `catalog`
  keyed by shortcode. Each entry has an `emoji`, a `description`, and a `semver` level of
  `"major"`, `"minor"`, `"patch"`, or `null`.
- `publishers`: keyed by publisher type, currently only `github-release`, and then by name. At
  least one publisher is required. Each has an `apiUrl`, a `repository` in `owner/name` form, and a
  `token.secret` naming the environment variable that holds the token.
