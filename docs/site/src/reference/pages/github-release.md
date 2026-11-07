The GitHub Release publisher creates one GitHub Release for the release tag, with the frozen
release notes from the changelog as its body. Declare it under
[`publishers.github-release`](/reference/options/#publishers.github-release), keyed by a name of
your choice:

```nix
presswerk.lib.mkRelease {
  # ...
  publishers.github-release.default.repository = "owner/project";
}
```

## Options

<!-- reference:options -->

## Token

`token.secret` names the environment variable that holds the GitHub API token, `GITHUB_TOKEN`
by default. The token never enters the declaration or the Nix store; `presswerk publish` reads
it at run time and stops if the variable is unset or empty. In GitHub Actions, the token needs
the `contents: write` permission.

## Behavior

`presswerk publish` pushes the release tag first and then completes every GitHub Release
publisher. For each publisher it looks up the release for the tag and acts on what it finds:

- **Missing:** it creates the release as a draft that targets the release commit, then
  publishes the draft, so the release never appears without its notes.
- **Draft:** it publishes the existing draft.
- **Published:** the release is complete, and it is left as it is.

Before anything is written, Presswerk checks the state of the tag and of every publisher. A tag
that is lightweight or points to another commit, or a release whose prerelease flag does not match the version, is
*conflicting*: Presswerk stops and never overwrites it. An existing release is otherwise never edited, only a draft is published.

So a retry of the same candidate completes missing state and keeps everything that already
matches. After publishing, Presswerk reads every release back and fails unless it is published.
