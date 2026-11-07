Presswerk analyzes commits by their [Gitmoji](https://gitmoji.dev/). This catalog of
<!-- reference:catalog-size --> entries is the default for
[`analyzer.gitmoji.catalog`](/reference/options/#analyzer.gitmoji.catalog). It is vendored from the
upstream Gitmoji project, so analysis is deterministic and works offline. The [Gitmoji guide](/guide/gitmoji/)
explains how commits select versions and release-note groups.

A commit's Gitmoji may be written as the emoji or as its `:shortcode:`. The release level is the
semantic-version bump the commit triggers; `none` means the commit does not cause a release on
its own.

Change entries or add your own in the release module; every field defaults to the catalog value:

```nix
presswerk.lib.mkRelease {
  # ...
  analyzer.gitmoji.catalog.construction_worker.semver = "patch";
}
```

<!-- reference:catalog -->
