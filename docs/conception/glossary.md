# Glossary

Names and analogies used across the Presswerk documentation. Add terms here when they are
introduced, and use them consistently elsewhere.

## The pressing-plant analogy

A *Presswerk* is a pressing plant. In record manufacturing, one master record is cut once, and
every copy that leaves the plant is pressed from it. Copies never change the master, and two
pressings from the same master are identical.

Presswerk applies this to releases. The master record is the flake at one exact commit. It is
immutable: locked inputs, pure evaluation, outputs addressed by revision. Every published
artifact, tag, forge release, and registry entry is a pressing made from that master. Pressing
again from the same master must give the same result, which is why a retry is always safe.

## Terms

### Master record

The immutable source of a release: the flake at one exact Git commit, including the committed
version. Everything Presswerk publishes is derived from it.

### Pressing

One published result derived from a master record, such as a Git tag, a forge release, a crate
on crates.io, or an OCI image. A release normally consists of several pressings.

### Candidate

A master record that Presswerk is about to release, identified by repository, commit, and
version. The candidate is *what* is released. Only a release commit can be a candidate.

### Release commit

A commit that changes the version compared to its parent, usually created by `prepare`. This
is a structural property, independent of whether the version has been tagged or published
yet. It is the only commit from which that version can ever be published; later commits never
inherit it. Before its first publication, a release commit must also be valid: it has a
changelog section for its version, the version is a valid successor, and it is at least what
commit analysis requires. Who made it does not matter. A fresh repository starts from an
implicit `0.0.0` baseline, without requiring a tag or a manual first release; how it is
distinguished from an existing untagged version is an open adoption question.

### Pending release

A release commit whose version has no tag yet: it is still on its way through CI, or it failed.
Presswerk cannot tell which, so a later release attempt reports the pending release and fails.
A tagged but incompletely published release also blocks the release chain. See
[release model](release-model.md#pending-releases).

### Release context

The immutable data Presswerk derives about one candidate during a release run: identity,
previous release, analyzed commits, frozen notes, and artifact identities. It never contains
credentials. The release context is *what Presswerk knows* about the candidate. See
[release model](release-model.md#candidate-and-release-context).

### Release declaration

The release configuration in the consumer's flake: artifacts, publishers, version source, and
related settings. The CLI reads it by evaluating the flake. See
[product shape](product-shape.md#release-declaration-in-the-flake).

### Frozen notes

Release notes rendered once during preparation, committed as a version's section in a
`CHANGELOG.md` in the release commit, and read back unchanged by every publication attempt for
that version.

### Analyzer

The adapter that turns the commits since the previous release into a version recommendation and
release-note data. git-cliff is the first implementation. See
[release model](release-model.md#commit-analysis).

### Convention

The rules a project's commit messages follow, such as Conventional Commits or Gitmoji. It
determines categories and version bumps, and is independent of the analyzer backend that
applies it.

### Publisher

A component that turns a candidate into one kind of pressing at one destination, such as a
GitHub release or crates.io. Every publisher follows the same
[safety requirements](publication.md#publisher-safety-requirements).

### Remote state

What a publisher finds at its destination before writing: *missing*, *matching*,
*conflicting*, or *unknown*.

### Release line

A branch that produces releases for one version range, such as `release/v1.4.x` for maintained
`1.4` patch releases.

### Runtime

Wherever the CLI runs: a local shell, GitHub Actions, Forgejo Actions, or another CI system.
See [runtime integration](runtime-integration.md).

### Forge

A Git hosting platform with releases, pull requests, and an API, such as GitHub or Forgejo.

### Consumer

A repository that uses Presswerk to release itself.
