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

Every field is required unless noted otherwise, and unknown fields are rejected.

- `schemaVersion`: the integer `0`.
- `version`: the effective release version, such as `"1.4.0"` or `"2.0.0-rc.1"`.
- `tag.format`: the release tag format, containing `{version}` exactly once.
- `changelog.path`: the repository-relative path of the changelog. `changelog.template` is the
  MiniJinja template that renders a version section from the analyzed commit groups; it may be
  missing in older declarations and then means the built-in template. `changelog.heading` is the
  version heading format with `{version}` and an optional `{date}` or `{date:<strftime format>}`; it may be missing in older
  declarations and then means `## [{version}] - {date}`. `changelog.unreleasedHeading` is the
  literal heading line of the hand-written section above the first version heading; it may be
  missing in older declarations and then means `## [Unreleased]`.
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
- `publishers`: keyed by publisher type, `cargo`, `github-release`, `npm`, `oci-helm`, or `oci-image`, and then by
  name. All may be empty. Every publisher has `after`, a list of publisher addresses in
  `<type>.<name>` form that must complete first. It may be missing in older declarations and then
  means an empty list. The module-only `before` option never enters the declaration. A credential
  is exactly one of `{ "value": "…" }` or a secret source, `{ "env": "NAME" }`; a `token` or
  `password` is always a secret. Every publisher also has `claims`, the remote objects it writes, each with a
  `destination`, the remote, such as `oci://ghcr.io/owner/project`, and a `key`, the object within
  it, as a version template such as the tag `{version}` or `latest`. The CLI renders the keys with the version it releases and refuses two
  publishers that claim the same object. `claims` may be missing in older declarations and then
  means none. Each GitHub Release publisher has an `apiUrl`, a `repository` in `owner/name` form,
  a `token` credential, a MiniJinja `body` template, and `assets`, which maps asset names to
  absolute store paths of files and may be missing in older declarations. `body` may be missing in
  older declarations and then means `{{ notes }}`. `oci-helm` and `oci-image` may be missing in
  older declarations. Both registry publishers have a `repository`, a `username` and a `password`
  credential, and `public`, which may be missing and then means `false`. An OCI Helm publisher has
  the absolute store path of its packaged `chart`. An OCI image publisher has `platforms`, which
  maps platforms such as `linux/amd64` to absolute store paths of image archives or executables
  that write an uncompressed one to stdout. Its `tags` and `aliases` are lists of templates that
  may use `{major}`, `{minor}`, and `{patch}`; every tag contains `{version}`, no alias does, and
  `aliases` may be missing in older declarations. `compression` has either `gzip` or `zstd`, each
  with its `level`; it may be missing in older declarations and then means gzip at level 6.
  `npm` may be missing in older declarations. An npm publisher has the `name` of its package, the
  absolute store path of its `package` tarball or of a directory with exactly one, its `registry`
  URL, its `access`, `public` or `restricted`, the dist-tag `tag` sent with the upload, and
  `aliases`, a list of templates without `{version}`. It has either a `token` credential, or a
  `username` and a `password` credential.
  `cargo` may be missing in older declarations. A Cargo publisher has the `name` of its crate, the
  absolute store path of its `crate` archive, its sparse `index` URL, and a `token` credential.
- `trustedPredecessor`: optional, and present only when declared. The previous release the CLI
  trusts without reading its declaration, with its `tag` and the full hexadecimal `commit` SHA
  that tag points to.
