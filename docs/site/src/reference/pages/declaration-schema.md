`lib.mkRelease` evaluates the [release options](/reference/options/) and returns a plain declaration:
JSON-compatible data with no functions, paths, or module internals. Your flake exposes it as
`presswerk.default`, and the CLI evaluates that output at an exact commit.

:::caution
The schema is not frozen before Presswerk 1.0. Until then, `schemaVersion` is `0`, and its shape
may change with any Presswerk version. The CLI refuses a declaration it cannot decode instead of
guessing. With Presswerk 1.0, the schema becomes `schemaVersion = 1`, and newer CLIs will also read
declarations with older schema versions. Until then,
[recovery](/guide/recovery/#the-previous-release-uses-an-unsupported-declaration) explains how to
continue after a previous release with an unsupported schema.
:::

This page documents `schemaVersion = 0` as of this Presswerk version. Produce the declaration with `lib.mkRelease` rather
than writing it by hand: the module checks values that the plain data cannot express, and it
keeps the declaration in step with the CLI from the same Presswerk input.

## Example

Presswerk's own declaration, evaluated with this version of `lib.mkRelease`. The Gitmoji catalog
is shortened to two of its <!-- reference:catalog-size --> entries:

<!-- reference:example -->

## Fields

Every field is required, and unknown fields are rejected.

### `schemaVersion`

The integer `0`. Presswerk reads it before anything else and stops with an error for any other
value, or when a declaration with `0` does not decode, because it was then made by another Presswerk
version.

### `version`

The effective release version as a semantic version, such as `"1.4.0"` or `"2.0.0-rc.1"`. With
[`versionFiles.cargo`](/reference/options/#versionFiles.cargo) it is read from the declared
`Cargo.toml`.

### `versionFiles`

A list of files whose version Presswerk updates when it prepares a release. Each entry has a
repository-relative `path` and one of:

- `toml`: a dotted key selector whose string value is the version, such as `"package.version"`;
- `cargoLock`: the name of the package whose entry in `Cargo.lock` is updated.

Every [Cargo entry](/reference/options/#versionFiles.cargo.name.crate) of the module produces one entry
of each kind.

### `tag.format`

The release tag format, containing `{version}` exactly once, such as `"v{version}"`.

### `analyzer`

- `convention`: the commit convention, currently always `"gitmoji"`.
- `gitmoji.catalog`: the Gitmoji catalog keyed by shortcode, without colons. Each entry has an
  `emoji`, a `description` used as the release-note group, and a `semver` level of `"major"`,
  `"minor"`, `"patch"`, or `null` for no release. See the
  [default catalog](/reference/analyzers/#default-catalog).

### `changelog.path`

The repository-relative path of the changelog that holds the release notes.

### `publishers`

The publishers of a release, keyed by publisher type and then by a name of your choice. Schema 2
has one type, `github-release`, and requires at least one publisher of it. Each publisher has
an `apiUrl`, a `repository` in `owner/name` form, and a `token.secret` naming the environment
variable that holds the token, never the token itself; see the
[GitHub Release publisher](/reference/publishers/).
