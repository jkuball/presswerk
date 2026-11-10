A commit analyzer reads the commits since the previous release and decides two things:

- **The version bump.** Each commit selects a level, and the highest level selects the next version.
  If no commit selects one, there is nothing to release.
- **The release notes.** The analyzer groups the commits into the notes that `prepare` writes into
  the changelog and that the publishers publish. The changelog is `CHANGELOG.md` unless
  [`changelog.path`](/reference/options/#changelog.path) names another file.

`publish` runs the analyzer again for a new release and fails if the release commit's version is
smaller than the analysis requires.

Gitmoji, selected with [`analyzer.convention`](/reference/options/#analyzer.convention), is the only
built-in analyzer today. Declaring analyzers for other commit conventions in the flake itself is
planned.

The Gitmoji analyzer reads commit subjects written to the
[Gitmoji specification](https://gitmoji.dev/specification): a Gitmoji at the start of each subject
states the commit's intention. The release declaration contains the complete catalog used for
matching, grouping notes, and choosing a SemVer level. The CLI has no hidden Gitmoji table, so
analysis is pinned to the exact commit like the rest of the release declaration. The
[default catalog](#default-catalog) at the end of this page lists every entry.

## Commit subject shape

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

## Version levels

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

## Change a default level

Ordinary Nix module merging lets a project override one field without copying the catalog. For
example, make documentation changes release a patch and stop dependency upgrades from doing so. The
argument of `mkRelease` is a module, so this is a complete one; add it to your `mkRelease` call or
import it there:

```nix
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

The resolved catalog is serialized into `presswerk.default` and read from the candidate commit.
Changing the catalog affects future analysis only; an exact tagged retry keeps the release commit's
own declaration.

## Add a project-specific entry

Add an attribute with an emoji that is not already in the catalog. A new entry has no catalog values
to fall back on, so it must set [`emoji`](/reference/options/#analyzer.gitmoji.catalog.name.emoji),
[`description`](/reference/options/#analyzer.gitmoji.catalog.name.description), and
[`semver`](/reference/options/#analyzer.gitmoji.catalog.name.semver). Its attribute name becomes the
shortcode unless you set `shortcode` explicitly:

```nix
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

## Catalog validation

`mkRelease` rejects catalogs with:

- an empty shortcode or a shortcode containing colons;
- duplicate resolved shortcodes;
- duplicate emoji after removing variation selector-16;
- an empty emoji or description;
- a SemVer value outside the supported levels.

Unicode spellings with and without variation selector-16 match the same entry. Shortcodes in commit
subjects include surrounding colons, while declaration keys and explicit `shortcode` values do not.

## Replace the catalog

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

## Release commits

`prepare` creates a `🔖 Release v…` commit. The candidate cannot include itself in its notes because
the notes are rendered before that commit exists. The exact candidate object is excluded during
publication analysis. Do not assign a release level to `bookmark` merely to make release commits
trigger more releases; keep the release boundary driven by the actual changes since the previous
release.

## Default catalog

The default for [`analyzer.gitmoji.catalog`](/reference/options/#analyzer.gitmoji.catalog) has
<!-- reference:catalog-size --> entries. It is vendored from the
[upstream Gitmoji project](https://github.com/carloscuesta/gitmoji), so analysis
is deterministic and works offline. A commit's Gitmoji may be written as the emoji or as its
`:shortcode:`. The entries are grouped by the semantic-version bump a commit triggers; entries under
No release do not cause a release on their own.

<!-- reference:catalog -->

## License

The catalog entries come from the
[Gitmoji project](https://github.com/carloscuesta/gitmoji) and are published here under its MIT
license:

<!-- reference:license -->
