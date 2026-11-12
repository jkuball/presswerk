With [`analyzer.conventional.enable`](/reference/options/#analyzer.conventional.enable), Presswerk
reads commit messages written to the
[Conventional Commits specification](https://www.conventionalcommits.org/en/v1.0.0/).

## Commit message shape

```text
<type>[(scope)][!]: <description>

[body]

[footers]
```

For example:

```text
feat: Add search
feat(cli): Add --json
fix!: Reject unknown flags
FIX(cli): Keep the search order stable
```

The type is matched against the catalog without regard to case. A scope in parentheses is
optional. A colon and whitespace must follow the type, scope, or `!`, and a description must follow
them. A subject that does not have this shape, such as `Update the readme`, or that uses a type
missing from the catalog does not contribute a bump or release note.

## Types and version levels

Every catalog entry has a `title`, the heading of its release-note group, and a `semver` level:
`"major"`, `"minor"`, `"patch"`, or `null`. `feat` is a minor change, `fix`, `perf`, and `revert` are
patches, and the other default types do not cause a release on their own. Presswerk combines the
levels as described under [Version levels](/reference/analyzers/#version-levels).

## Breaking changes

A commit is a breaking change when `!` follows its type or scope, or when a line after the subject
starts with `BREAKING CHANGE:` or `BREAKING-CHANGE:` followed by a description. Both of these
commits are breaking changes:

```text
feat(api)!: Drop the v1 endpoint
```

```text
fix: Rename the output field

BREAKING CHANGE: `version` is now called `next`.
```

A breaking change selects `major` whatever its type's level, so `0.4.2` becomes `0.5.0` and `1.4.2`
becomes `2.0.0`. The footer must be written in uppercase, as the specification requires.

## Release notes

For a release with these commits, newest first:

```text
feat(api)!: Drop the v1 endpoint
fix: Fix tag lookup
docs: Document search
feat(cli): Add --json
fix(cli): Keep the search order stable
Update the readme
```

`prepare` adds this section to the changelog:

```md
## [0.4.0] - 2026-11-12

### Features

- **BREAKING** **api:** Drop the v1 endpoint
- **cli:** Add --json

### Bug fixes

- Fix tag lookup
- **cli:** Keep the search order stable

### Documentation

- Document search
```

Groups follow the level of their type, from `major` to `null`; groups of the same level are ordered
by size and then by type. A breaking change stays in the group of its type and starts with
`**BREAKING**`. The scope stays in front of the description. A `git revert` of a commit that was
released before, whose subject is `Revert "…"`, belongs to the `revert` type, so by default it
releases a patch.

## Change the type catalog

Override a field, add a type, or remove one with ordinary module merging:

```nix presswerk-module
{ lib, ... }:
{
  analyzer.conventional.catalog = {
    # Release documentation changes as patches.
    docs.semver = "patch";
    # Accept a project-specific type.
    deps = {
      title = "Dependencies";
      semver = "patch";
    };
    # Stop accepting style commits.
    style = lib.mkForce null;
  };
}
```

A new type must set [`title`](/reference/options/#analyzer.conventional.catalog.name.title) and
[`semver`](/reference/options/#analyzer.conventional.catalog.name.semver), and its name must be
lowercase. Replace the whole catalog with `lib.mkForce { … }`; the replacement must define every
type the project still accepts.

## Release commits

`prepare` creates a `chore(release): v…` commit, named by the tag. When this analyzer finds a commit
whose subject is exactly that, for a tag of the declared
[`tag.format`](/reference/options/#tag.format), it skips the commit, even if `chore` has a level.
Other `chore(release)` subjects are ordinary `chore` commits. Whether a commit is a release does not
depend on its subject: `publish` decides it from the version change.

## Options

<!-- reference:options -->

## Default type catalog

The default for
[`analyzer.conventional.catalog`](/reference/options/#analyzer.conventional.catalog):

<!-- reference:catalog -->
