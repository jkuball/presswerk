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

Presswerk has two built-in publishers:

- [GitHub Release](/reference/publishers/github-release/)
- [OCI](/reference/publishers/oci/) for container images

Declaring publishers for other targets in the flake itself is planned.

## Publishing

Before anything is written, `presswerk publish` checks the release tag and every publisher. A tag
that is lightweight or points to another commit is conflicting, and so is any conflicting state a
publisher reports: `publish` then stops without writing anything. Otherwise it pushes the tag and
completes the publishers one after another, the OCI publishers first and the GitHub Releases last,
so a release is never announced while its images are still missing.

`publish` never builds anything, and evaluating an output for another system does not build it
either. Before anything is written, `publish` fetches every artifact a publisher needs from the
local store or a substituter, and stops if one is missing. The CI job that builds a system must
therefore push its outputs to a binary cache that the publishing job can read.
