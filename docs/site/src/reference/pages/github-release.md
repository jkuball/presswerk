A release publisher is Presswerk's interface to the outside world. It is the bridge between the
immutable release commit with its frozen notes and the places where a release is published, which
Presswerk does not control and which change on their own. A project usually has more than one
publisher, for example a GitHub Release next to packages on PyPI, crates.io, or npm, and container
images on GHCR. `publish` first pushes the tag, then runs every declared publisher.

Every publisher follows the same rules on a retry:

- matching state is kept;
- missing state is created;
- conflicting state fails the run.

A release is complete once its tag and every publisher's release exist. `prepare` refuses to start
the next release before that.

GitHub Release is the only built-in publisher today, and every release currently needs one.
Declaring publishers for other targets in the flake itself is planned.

## GitHub Release

The GitHub Release publisher creates one GitHub Release for the release tag, with the frozen
release notes from the changelog as its body. Declare it under
[`publishers.github-release`](/reference/options/#publishers.github-release), keyed by a name of
your choice:

```nix presswerk-module
{
  publishers.github-release.default.repository = "owner/project";
}
```

### Behavior

`presswerk publish` pushes the release tag first and then completes every GitHub Release
publisher.

Before anything is written, Presswerk checks the state of the tag and of every publisher. A tag
that is lightweight or points to another commit, a release whose prerelease flag does not match
the version, or several releases for the same tag are *conflicting*: Presswerk stops and never
overwrites them. An existing release is otherwise never edited, only a draft is published.

For each publisher, Presswerk looks up the release for the tag. GitHub finds only published
releases by tag, so when there is none, Presswerk also searches every page of the repository's
release listing for a draft with exactly that tag. Then it acts on what it finds:

- **Missing:** it creates the release as a draft that targets the release commit, then
  publishes the draft, so the release never appears without its notes. It reports `published`.
- **Draft:** a previous run stopped between the two steps, so it publishes the existing draft
  instead of creating another one. It reports `draft completed`.
- **Published:** the release is complete, and it is left as it is. It reports `already published`.

So a retry of the same candidate completes missing state and keeps everything that already
matches. After publishing, Presswerk reads every release back and fails unless it is published.

### Assets

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

Evaluating an output for another system does not build it. `publish` fetches each asset from the
local store or a substituter and stops before writing anything if one is missing, so the CI job
that builds a system must push its outputs to a binary cache that the publishing job can read.

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

### Token

`token.secret` names the environment variable that holds the GitHub API token, `GITHUB_TOKEN`
by default. The token never enters the declaration or the Nix store; Presswerk reads it at run
time and stops if the variable is unset or empty.

`presswerk publish` needs the token whenever it publishes or retries a release commit. On an
ordinary commit it reports that there is nothing to publish without reading the token. Once a
previous release exists, `presswerk plan` and `presswerk prepare` need it too: they check that the
previous release's GitHub Release is complete before planning the next one. Only the very first
release can be planned and prepared without it. In GitHub Actions, the token needs the
`contents: write` permission to publish.

### Options

<!-- reference:options -->
