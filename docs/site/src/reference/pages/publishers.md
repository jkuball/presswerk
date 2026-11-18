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

A GitHub Release runs after every non-GitHub publisher by default, so it does not announce a release
whose images or charts are still missing. Defining its `after` option replaces that default
entirely. Set it to `[ ]` to remove the ordering, or declare the exact dependencies you need.

## Publishing

Before anything is written, `presswerk publish` checks the release tag and the state of every
publisher. A tag that is lightweight or points to another commit is conflicting, and so is any
conflicting state a publisher reports. `publish` then stops without writing anything. Otherwise it
pushes the tag and runs ready publishers serially in topological order.

If a publisher fails after the tag is pushed, its descendants are skipped and independent branches
continue. The command reports each failure and skip immediately, exits non-zero after the graph has
settled, and can be rerun to complete the release. `--fail-fast` instead starts no publisher after
the first failure.

`publish` never builds anything, and evaluating an output for another system does not build it
either. Before anything is written, `publish` fetches every artifact a publisher needs from the
local Nix store or a substituter, and stops if one is missing. Whatever builds the artifacts only
has to make them available to `publish`: in a binary cache it can read, or in the same Nix store,
such as when one machine builds and publishes.
