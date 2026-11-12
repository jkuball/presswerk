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
the next release before the previous release's tag and GitHub Releases are complete.

## Built-in publishers

Presswerk has two built-in publishers:

- [GitHub Release](/reference/publishers/github-release/), declared under
  [`publishers.github-release`](/reference/options/#publishers.github-release)
- [OCI](/reference/publishers/oci/) for container images, declared under
  [`publishers.oci`](/reference/options/#publishers.oci)

Every release currently needs a GitHub Release publisher. `publish` runs the OCI publishers before
the GitHub Releases, so a release is never announced while its images are still missing. Declaring
publishers for other targets in the flake itself is planned.
