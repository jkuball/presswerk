With [`analyzer.gitmoji.enable`](/reference/options/#analyzer.gitmoji.enable), Presswerk reads
commit subjects written to the [Gitmoji specification](https://gitmoji.dev/specification): a
Gitmoji at the start of each subject states the commit's intention.

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

## Release notes

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
Change the Markdown without changing analysis through
[`changelog.template`](/reference/changelog/#template), whose context exposes each group's title
and each commit's display text, parsed message and scope, breaking flag, and object ID.

## Version levels

Every catalog entry has `emoji`, `description`, and `semver`. `semver` is one of `"major"`,
`"minor"`, `"patch"`, or `null`, and Presswerk combines them as described under
[Version levels](/reference/analyzers/#version-levels). Common defaults include:

| Gitmoji | Shortcode | Default level | Meaning in analysis |
| --- | --- | --- | --- |
| ✨ | `:sparkles:` | `minor` | release-worthy feature |
| 🐛 | `:bug:` | `patch` | release-worthy fix |
| 💥 | `:boom:` | `major` | breaking change |
| 📝 | `:memo:` | `null` | note only, no release by itself |
| 🔖 | `:bookmark:` | `null` | release/version commit |

```mermaid A documentation commit is included in notes but does not trigger a release alone; the later feature selects the 0.3.0 release.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "📝 docs"
  commit id: "✨ feature"
  commit id: "release 0.3.0" tag: "v0.3.0"
```

## Change a default level

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

## Add a project-specific entry

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

## Remove an entry

Set an entry to `lib.mkForce null` to remove it from the catalog. Commits with its Gitmoji then
match no entry, so they neither select a level nor appear in the release notes:

```nix presswerk-module
{ lib, ... }:
{
  analyzer.gitmoji.catalog.poop = lib.mkForce null;
}
```

## Replace the catalog

A project that wants a closed, minimal convention can replace the default attribute set with the Nix
module system's `lib.mkForce`. The replacement must define every entry the project still accepts:

```nix
flake.presswerk.default = presswerk.lib.mkRelease ({ lib, ... }: {
  sourceRoot = self.outPath;
  versionFiles.cargo.my-project = { };
  publishers.github-release.default.repository = "owner/my-project";

  analyzer.gitmoji.enable = true;
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
