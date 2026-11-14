The GitHub Release publisher creates one GitHub Release for the release tag, with the frozen
release notes from the changelog as its body. Declare it under
[`publishers.github-release`](/reference/options/#publishers.github-release), keyed by a name of
your choice:

```nix presswerk-module
{
  publishers.github-release.default.repository = "owner/project";
}
```

## Behavior

Presswerk looks up the release for the tag. GitHub finds only published releases by tag, so when
there is none, Presswerk also searches every page of the repository's release listing for a draft
with exactly that tag. Then it acts on what it finds:

- **Missing:** it creates the release as a draft that targets the release commit, then
  publishes the draft, so the release never appears without its notes. It reports `published`.
- **Draft:** a previous run stopped between the two steps, so it publishes the existing draft
  instead of creating another one. It reports `draft completed`.
- **Published:** the release is complete, and it is left as it is. It reports `already published`.
- **Conflicting:** its prerelease flag does not match the version, or there are several releases
  for the tag. Presswerk stops before anything is written and never changes them.

An existing release is never edited; at most a draft is published. So a retry of the same candidate
completes missing state and keeps everything that already matches. After publishing, Presswerk
reads every release back and fails unless it is published.

## Assets

`assets` attaches files to the release, keyed by asset name. Each value is a file in a flake
output: a string built from a derivation, such as `"${pkg}/bin/tool"`, or a derivation whose
output is a single file, such as a tarball. A directory is rejected.

```nix presswerk-module
{
  publishers.github-release.default = {
    repository = "owner/project";
    assets = {
      "tool-x86_64-linux" = "${self.packages.x86_64-linux.default}/bin/tool";
      "tool-aarch64-darwin" = "${self.packages.aarch64-darwin.default}/bin/tool";
    };
  };
}
```

Presswerk attaches the file as it is. A binary that links libraries from `/nix/store` runs only
where those libraries exist; build a static or bundled binary if the asset must run elsewhere.

Each asset has its own state, compared by the SHA-256 digest that GitHub reports:

- **Missing:** no asset with that name. Presswerk uploads it, into the draft before the release is
  published, or to the published release on a retry.
- **Matching:** the same name and digest. It is left as it is.
- **Conflicting:** the same name with another digest, or without a digest. Presswerk stops and
  never replaces it.

Assets on the release that the declaration does not name are ignored. Because a missing asset is
uploaded on any retry, an asset that was deleted can be restored later by publishing the release
again.

## Token

`token.secret` names the environment variable that holds the GitHub API token, `GITHUB_TOKEN`
by default. The token never enters the declaration or the Nix store; Presswerk reads it at run
time and stops if the variable is unset or empty.

`presswerk publish` needs the token whenever it publishes or retries a release commit. On an
ordinary commit it reports that there is nothing to publish without reading the token. Once a
previous release declared a GitHub Release publisher, `presswerk plan` and `presswerk prepare` need
it too: they check that the previous release's GitHub Release is complete before planning the next
one. The very first release, and a release whose predecessor declared no GitHub Release publisher,
can be planned and prepared without it. In GitHub Actions, the token needs the
`contents: write` permission to publish.

## Options

<!-- reference:options -->
