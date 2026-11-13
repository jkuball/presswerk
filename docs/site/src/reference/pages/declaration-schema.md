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
- `analyzer`: the commit `convention`, `"gitmoji"` or `"conventional"`, and the settings of that
  one analyzer under the same name. `gitmoji.catalog` is keyed by shortcode, and each entry has an
  `emoji`, a `description`, and a `semver` level of `"major"`, `"minor"`, `"patch"`, or `null`.
  `conventional.catalog` is keyed by lowercase type, and each entry has a `title` and a `semver`
  level.
- `publishers`: keyed by publisher type, `github-release` or `oci`, and then by name. Both may be
  empty. Each GitHub Release publisher has an `apiUrl`, a `repository` in `owner/name` form, a
  `token.secret` naming the environment variable that holds the token, and `assets`, which maps
  asset names to absolute store paths of files and may be missing in older declarations.
  `oci` may be missing in older declarations. Each OCI publisher has a `repository` starting with
  the registry host, `platforms`, which maps platforms such as `linux/amd64` to absolute store
  paths of image archives or executables that write an uncompressed one to stdout, `tags`, a list
  of tag templates where `{version}` stands for the version, `public`, whether the images may be
  pulled anonymously, which may be missing in older declarations and then means `false`, and
  `username.secret` and `password.secret` naming the environment variables that hold the registry
  credentials.
