---
title: Release model
description: The commits and remote state that make a Presswerk release exact, retryable, and conflict-safe.
---

Presswerk gives a few ordinary Git objects precise roles. The rules for them are strict for one
reason: a version, its notes, its tag, and its published release must all come from the same commit.

## Terms

- An **ordinary commit** is any commit that does not change the declared version. Your changes are
  ordinary commits.
- A **release commit** changes the declared version compared with its first parent. Create it with
  `prepare`, which also writes every version file and the matching section in the changelog. A
  release commit written by hand must get all of that right itself.
- The **master record** of a release is the flake at its release commit: the committed version,
  `flake.lock`, release declaration, and frozen notes. Uncommitted changes are never part of it.
- A **pressing** is published state made from the master record. Today that is the annotated Git tag
  and one or more GitHub Releases.
- A **candidate** is a release commit that is being checked or published, identified by its full
  commit SHA and version. A branch name is never a candidate, because it can move.
- A **pending release** is a release commit without its tag.

## Lifecycle of a release commit

```mermaid A release commit starts pending; publishing makes it published, and publishing again keeps it published. A broken one is reverted instead.
stateDiagram-v2
  direction LR
  [*] --> Pending: prepare
  Pending --> Published: publish
  Pending --> Reverted: revert
  Published --> Published: publish again
  Reverted --> [*]
```

- **prepare** creates the release commit on top of the ordinary commits since the previous release.
  Until it is tagged, it is a pending release.
- **publish** tags the candidate and creates its pressings, ideally once your checks passed on it.
  From then on, the version is taken.
- **publish again** compares the pressings with the commit. Matching state is kept, missing state is
  created, and conflicting state fails the run. A retry never creates a second release.
- **revert** is the way out for a candidate whose contents fail its checks. It restores the previous
  version, so a fix can follow and `prepare` can create a fresh candidate, even with the same
  version, because no tag points to the broken one. See
  [Recovery](../recovery/#candidate-checks-fail).

Presswerk cannot tell whether the checks of a pending release are still running or have failed. It
reports the pending commit and refuses to prepare another release until the pending one is published
or reverted.

## The strict boundary

- One version belongs to exactly one release commit.
- Later commits never inherit a release, even though they contain the same version.
- A tag reserves its version and is never moved.

Together these rules mean a fix on top of a pending release cannot be published in its place:

```mermaid A fixup on top of an untagged release commit carries the pending version but is not a candidate; only the release commit can publish 0.3.0.
gitGraph
  commit id: "🔖 0.2.2" tag: "v0.2.2"
  commit id: "✨ feature"
  commit id: "🔖 0.3.0 (pending)"
  commit id: "🐛 fixup"
```

The release commit `🔖 0.3.0` has no tag yet. The fixup on top still contains version `0.3.0`, but
it did not change it, so it is not a release commit: `publish` on the fixup fails instead of
publishing `0.3.0`, and `prepare` refuses to start another release. Only `🔖 0.3.0` can publish
`0.3.0`. Retry it if its checks did not show a problem in its contents, or
[revert it](../recovery/#candidate-checks-fail).

## What publish checks

For a new release, `publish` validates that:

- the version is a valid SemVer successor of its parent;
- the bump is at least what commit analysis requires;
- the changelog contains the frozen section for that version;
- the previous release is complete;
- the candidate's tag is missing rather than conflicting.

On every run, new release or retry, `publish` also fetches every artifact it publishes from the
local store or a substituter before it writes anything. It never builds one: an artifact is
published exactly as CI built it for the candidate, and a missing one stops the run before the tag.

If the tag already exists and matches, `publish` treats the run as a retry. It does not apply newer
analysis rules to the old release, but it still checks the candidate, its predecessor, and the
remote state.

## Versions

Presswerk follows SemVer. A fresh repository starts from an implicit `0.0.0` baseline, committed as
`0.0.0` and without a tag. For projects that already have a version, see
[Adopt an existing project](../adoption/).

Presswerk never moves to `1.0.0` by itself; that step must be acknowledged explicitly, and a way to
choose the version for it is planned. Which level a commit selects, and how levels map to versions before `1.0.0`, is
described under [Version levels](/reference/analyzers/#version-levels).

How publication proceeds step by step, and what it does with existing releases, is described in the
[GitHub Release publisher](/reference/publishers/#behavior) reference.
