A commit analyzer reads the commits since the previous release and decides two things:

- **The version bump.** Each commit selects a level, and the highest level selects the next version.
  If no commit selects one, there is nothing to release.
- **The release notes.** The analyzer groups the commits into the notes that `prepare` writes into
  the changelog and that the publishers publish. The changelog is `CHANGELOG.md` unless
  [`changelog.path`](/reference/options/#changelog.path) names another file.

`publish` runs the analyzer again for a new release and fails if the release commit's version is
smaller than the analysis requires.

## Choose an analyzer

Presswerk has two built-in analyzers: [Gitmoji](#gitmoji) and
[Conventional Commits](#conventional-commits). A release declares exactly one of them. Without a
declaration, Presswerk analyzes Gitmoji commits; declare
[`analyzer.conventional`](/reference/options/#analyzer.conventional) to analyze Conventional Commits
instead:

```nix presswerk-module
{
  analyzer.conventional = { };
}
```

Declaring both [`analyzer.gitmoji`](/reference/options/#analyzer.gitmoji) and
`analyzer.conventional` fails evaluation with a message that names both.

Both analyzers treat history the same way. A subject that their convention does not describe, or
that uses an intention missing from the catalog, contributes neither a bump nor a release note.
Merge commits are skipped, while the commits they merge are analyzed. A revert inside the range
cancels the commit it reverts. The release commit that `prepare` creates follows the analyzer's
convention.

## Gitmoji

The Gitmoji analyzer reads commit subjects written to the
[Gitmoji specification](https://gitmoji.dev/specification): a Gitmoji at the start of each subject
states the commit's intention. The release declaration contains the complete catalog used for
matching, grouping notes, and choosing a SemVer level. The CLI has no hidden Gitmoji table, so
analysis is pinned to the exact commit like the rest of the release declaration. The
[default Gitmoji catalog](#default-gitmoji-catalog) at the end of this page lists every entry.

### Commit subject shape

The [Gitmoji specification](https://gitmoji.dev/specification) defines the subject as:

```text
<intention> [scope?][:?] <message>
```

The intention is either the configured Unicode emoji or its shortcode:

```text
✨ Add search
:sparkles: Add search
✨ (cli): Add search
🐛 : Fix tag lookup
```

A scope in parentheses is optional. Whitespace must terminate the emoji or shortcode. A colon may
attach directly to the closing parenthesis of a scope, as in `✨ (cli): Add search`.

A subject that does not match an entry in the declared catalog does not contribute a bump or release
note. Keep one clear Gitmoji intention at the start of each commit subject.

For a release with these commits, newest first:

```text
✨ (cli): Add search
🐛 Fix tag lookup
📝 Document search
:bug: (cli): Keep the search order stable
```

`prepare` adds this section to the changelog:

```md
## [0.3.0] - 2026-11-10

### 🐛 Fix a bug

- Fix tag lookup
- (cli): Keep the search order stable

### 📝 Add or update documentation

- Document search

### ✨ Introduce new features

- (cli): Add search
```

Groups with more commits come first, and groups of equal size follow the order of their shortcodes.

### Version levels

Every catalog entry has `emoji`, `description`, and `semver`. `semver` is one of `"major"`,
`"minor"`, `"patch"`, or `null`.

Common defaults include:

| Gitmoji | Shortcode | Default level | Meaning in analysis |
| --- | --- | --- | --- |
| ✨ | `:sparkles:` | `minor` | release-worthy feature |
| 🐛 | `:bug:` | `patch` | release-worthy fix |
| 💥 | `:boom:` | `major` | breaking change |
| 📝 | `:memo:` | `null` | note only, no release by itself |
| 🔖 | `:bookmark:` | `null` | release/version commit |

Presswerk takes the highest level in the applicable commit range. Before `1.0.0`, both `minor` and
`major` advance the minor component; `patch` advances the patch component.

No commit moves a project to `1.0.0`. That step must always be acknowledged explicitly; a way to
choose the version for it is planned.

```mermaid A documentation commit is included in notes but does not trigger a release alone; the later feature selects the 0.3.0 release.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "📝 docs"
  commit id: "✨ feature"
  commit id: "release 0.3.0" tag: "v0.3.0"
```

Entries with `semver = null` still appear in the generated notes when another commit in the range
triggers a release. If every matched entry is `null`, `plan` reports that there is nothing to
prepare.

### Change a default level

Ordinary Nix module merging lets a project override one field without copying the catalog. For
example, make documentation changes release a patch and stop dependency upgrades from doing so:

```nix presswerk-module
{
  analyzer.gitmoji.catalog = {
    memo.semver = "patch";
    arrow_up.semver = null;
  };
}
```

```mermaid With memo changed to patch, a documentation commit can create the next patch release.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "📝 guide"
  commit id: "release 0.2.3" tag: "v0.2.3"
```

### Add a project-specific entry

Add an attribute with an emoji that is not already in the catalog. A new entry has no catalog values
to fall back on, so it must set [`emoji`](/reference/options/#analyzer.gitmoji.catalog.name.emoji),
[`description`](/reference/options/#analyzer.gitmoji.catalog.name.description), and
[`semver`](/reference/options/#analyzer.gitmoji.catalog.name.semver). Its attribute name becomes the
shortcode unless you set `shortcode` explicitly:

```nix presswerk-module
analyzer.gitmoji.catalog.flying_saucer = {
  emoji = "🛸";
  description = "Add an experimental feature behind a flag.";
  semver = "minor";
};
```

Both of these subjects now select the entry:

```text
🛸 Add the next parser behind a flag
:flying_saucer: Add the next parser behind a flag
```

The entry participates in bump selection and gets its own release-note group.

```mermaid A custom minor-level Gitmoji participates in the same range analysis as vendored entries.
gitGraph
  commit id: "0.4.1" tag: "v0.4.1"
  commit id: "🛸 experiment"
  commit id: "🐛 fix"
  commit id: "release 0.5.0" tag: "v0.5.0"
```

### Remove an entry

Set an entry to `lib.mkForce null` to remove it from the catalog. Commits with its Gitmoji then
match no entry, so they neither select a level nor appear in the release notes:

```nix presswerk-module
{ lib, ... }:
{
  analyzer.gitmoji.catalog.poop = lib.mkForce null;
}
```

### Replace the catalog

A project that wants a closed, minimal convention can replace the default attribute set with the Nix
module system's `lib.mkForce`. The replacement must define every entry the project still accepts:

```nix
flake.presswerk.default = presswerk.lib.mkRelease ({ lib, ... }: {
  sourceRoot = self.outPath;
  versionFiles.cargo.my-project = { };
  publishers.github-release.default.repository = "owner/my-project";

  analyzer.gitmoji.catalog = lib.mkForce {
    sparkles = {
      emoji = "✨";
      description = "Introduce a new feature.";
      semver = "minor";
    };
    bug = {
      emoji = "🐛";
      description = "Fix a bug.";
      semver = "patch";
    };
  };
});
```

Commits using removed entries are ignored by analysis. Use a full replacement only when that is the
intended project policy; field overrides and additions are smaller and preserve the vendored catalog.

### Release commits

`prepare` creates a `🔖 Release v…` commit. The candidate cannot include itself in its notes because
the notes are rendered before that commit exists. The exact candidate object is excluded during
publication analysis. Do not assign a release level to `bookmark` merely to make release commits
trigger more releases; keep the release boundary driven by the actual changes since the previous
release.

## Conventional Commits

The Conventional Commits analyzer reads commit messages written to the
[Conventional Commits specification](https://www.conventionalcommits.org/en/v1.0.0/). Like the
Gitmoji catalog, the declaration contains the complete type catalog, so analysis is pinned to the
exact commit.

### Commit message shape

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

### Types and version levels

Every catalog entry has a `title`, the heading of its release-note group, and a `semver` level:
`"major"`, `"minor"`, `"patch"`, or `null`. `feat` is a minor change, `fix` and `perf` are patches,
and the other default types do not cause a release on their own. The
[default type catalog](#default-type-catalog) lists every entry.

Presswerk takes the highest level in the applicable commit range. Before `1.0.0`, both `minor` and
`major` advance the minor component; `patch` advances the patch component, exactly as with
Gitmoji.

### Breaking changes

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

### Conventional release notes

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
`**BREAKING**`. The scope stays in front of the description. A `git revert` of a commit that was released before,
whose subject is `Revert "…"`, belongs to the `revert` type.

### Change the type catalog

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
lowercase. Replace the whole catalog with `lib.mkForce { … }`, as with Gitmoji.

### Conventional release commits

`prepare` creates a `chore(release): v…` commit, named by the tag. When this analyzer finds a commit
whose subject is exactly that, for a tag of the declared
[`tag.format`](/reference/options/#tag.format), it skips the commit, even if `chore` has a level.
Other `chore(release)` subjects are ordinary `chore` commits. Whether a commit is a release does not
depend on its subject: `publish` decides it from the version change.

### Default type catalog

The default for
[`analyzer.conventional.catalog`](/reference/options/#analyzer.conventional.catalog):

<!-- reference:conventional-catalog -->

## Options

<!-- reference:options -->

## Default Gitmoji catalog

The default for [`analyzer.gitmoji.catalog`](/reference/options/#analyzer.gitmoji.catalog) has
<!-- reference:catalog-size --> entries. It is vendored from the
[upstream Gitmoji project](https://github.com/carloscuesta/gitmoji), so analysis
is deterministic and works offline. The entries are grouped by the semantic-version bump a commit triggers; entries under
No release do not cause a release on their own.

<!-- reference:catalog -->

## License

The Gitmoji catalog entries come from the
[Gitmoji project](https://github.com/carloscuesta/gitmoji) and are published here under its MIT
license:

<!-- reference:license -->
