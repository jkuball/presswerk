A release publisher is Presswerk's interface to the outside world. It is the bridge between the
immutable release commit with its frozen notes and the places where a release is published, which
Presswerk does not control and which change on their own. A project usually has more than one
publisher, for example a GitHub Release next to packages on PyPI, crates.io, or npm, and container
images on GHCR.

Every publisher follows the same rules on a retry:

- matching state is kept;
- missing state is created;
- conflicting state fails the run.

No publisher is required: a release without publishers consists of its tag alone. A release is
complete once its tag points to the release commit on the remote and every publisher it declared
reports its own part as complete. Before `plan` and `prepare` start the next release, they check the
previous release's tag and every publisher declared in the previous release's own declaration, and
refuse to continue if any of them is incomplete. What counts as complete is described on each
publisher's page.

## Built-in publishers

Presswerk has three built-in publishers:

- [GitHub Release](/reference/publishers/github-release/)
- [OCI: Helm Chart](/reference/publishers/oci-helm/) for packaged charts in OCI registries
- [OCI: Image](/reference/publishers/oci-image/) for container images

Declaring publishers for other targets in the flake itself is planned.

## Dependencies

Publishers form a dependency graph. Name a publisher as `<type>.<name>` in [`after`](/reference/options/#publishers.github-release.name.after) when it must complete first:

```nix
{
  publishers.oci-image.api = {
    # …
  };
  publishers.oci-helm.chart = {
    # …
    after = [ "oci-image.api" ];
  };
}
```

[`before`](/reference/options/#publishers.github-release.name.before) expresses the same edge from
the other side. Unknown publisher names and cycles fail module evaluation.

## Claims

Two publishers must never write the same remote object, such as the same tag in one OCI repository
or two GitHub Releases for one tag in one repository: the second would only conflict once the tag is
pushed and leave a partial release. Every publisher therefore claims what it writes. A claim is a
destination, the remote, plus a key, the object within it. Only the key is a version template; the
destination is fixed by the publisher's options:

| Type | Destination | Keys |
| --- | --- | --- |
| `oci-image` | the OCI repository | every template in `tags` and `aliases` |
| `oci-helm` | the OCI repository | the chart tag, `{version}` |
| `github-release` | the GitHub repository, case-insensitive | the release tag, from `tag.format` |

So two publishers that both use `{version}` collide only if they write to the same destination.
An image and a Helm chart pushed to one repository do, since image and chart tags share the
repository's tags:

```nix
{
  publishers.oci-image.api = {
    repository = "ghcr.io/foo/api";
    tags = [ "{version}" ];
    # …
  };
  publishers.oci-helm.chart = {
    repository = "ghcr.io/foo/api"; # use ghcr.io/foo/charts/api instead
    # …
  };
}
```

Module evaluation fails with:

```text
publishers.oci-helm.chart and publishers.oci-image.api both push to oci://ghcr.io/foo/api and both render 1.2.3 for version 1.2.3
```

With the chart in `ghcr.io/foo/charts/api`, both still render `{version}`, but the destinations
differ, so nothing collides.

Nix renders the keys with the sample version `1.2.3`. The claims are part of the release
declaration, and `plan`, `prepare`, and `publish` render them again with the version they release,
so a collision that only that version renders stops before anything is written. An image alias
`0.1.0` next to a Helm chart in the same repository passes with `1.2.3`, but collides with the
chart tag when `0.1.0` is released.

## Credentials

Every credential of every publisher has the same shape: exactly one of a secret source read at run
time, such as `{ env = "NAME"; }` for the environment variable `NAME`, or, for what is no secret,
such as a registry username, the value itself, `{ value = "…"; }`. Tokens and passwords are always
secrets.

```nix
{
  publishers.oci-image.api = {
    # …
    username.value = "oauth2accesstoken";
    password.env = "REGISTRY_TOKEN";
  };
}
```

Secrets never enter the declaration or the Nix store. `publish` reads every secret the declaration
names before it writes anything and stops if one is unset or empty, even if this run would not need
it, so a release never stops halfway for a missing secret. `plan` and `prepare` only warn about
missing secrets, unless they need one to check the previous release. Presswerk replaces every
secret value it read with `[redacted]` in what it prints.

## Publishing

Before anything is written, `presswerk publish` checks the release tag and the state of every
publisher. A tag that is lightweight or points to another commit is conflicting, and so is any
conflicting state a publisher reports. `publish` then stops without writing anything. Otherwise it
pushes the tag and runs every publisher whose dependencies are complete, with up to eight publisher
nodes running at once by default. `--jobs` changes that limit and also controls the concurrent
publisher checks.

If a publisher fails after the tag is pushed, its descendants are skipped and independent branches
continue. The command reports each failure and skip immediately, exits non-zero after the graph has
settled, and can be rerun to complete the release. `--fail-fast` instead starts no new publisher
after the first failure; publishers already running finish.

`publish` never builds anything, and evaluating an output for another system does not build it
either. Before anything is written, `publish` fetches every artifact a publisher needs from the
local Nix store or a substituter, and stops if one is missing. Whatever builds the artifacts only
has to make them available to `publish`: in a binary cache it can read, or in the same Nix store,
such as when one machine builds and publishes.
