Presswerk analyzes commits by their [Gitmoji](https://gitmoji.dev/). This catalog of
<!-- reference:catalog-size --> entries is the default for
[`analyzer.gitmoji.catalog`](/reference/options/#analyzer.gitmoji.catalog). It is vendored from the
upstream Gitmoji project, so analysis is deterministic and works offline. The [Gitmoji guide](/guide/gitmoji/)
explains how commits select versions and release-note groups.

A commit's Gitmoji may be written as the emoji or as its `:shortcode:`. The release level is the
semantic-version bump the commit triggers; `none` means the commit does not cause a release on
its own.

Override fields of an existing entry in the release module; fields you leave out keep their
catalog values:

```nix
presswerk.lib.mkRelease {
  # ...
  analyzer.gitmoji.catalog.construction_worker.semver = "patch";
}
```

A new entry has no catalog values to fall back on: it must set
[`emoji`](/reference/options/#analyzer.gitmoji.catalog.name.emoji),
[`description`](/reference/options/#analyzer.gitmoji.catalog.name.description), and
[`semver`](/reference/options/#analyzer.gitmoji.catalog.name.semver) (`null` for no release),
and only its shortcode defaults to its attribute name. See
[Add a project-specific entry](/guide/gitmoji/#add-a-project-specific-entry) in the guide.

<!-- reference:catalog -->

## License

The catalog entries come from the
[Gitmoji project](https://github.com/carloscuesta/gitmoji) and are published here under its MIT
license:

<!-- reference:license -->
