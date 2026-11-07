---
title: Release model
description: The commits and remote state that make a Presswerk release exact, retryable, and conflict-safe.
---

Presswerk gives a few ordinary Git objects precise roles. The model is strict so that a version,
its notes, its tag, and its GitHub Release all identify the same source.

## Master record and pressings

A **master record** is the flake at one exact Git commit. It includes the committed version,
`flake.lock`, release declaration, and frozen changelog section. Uncommitted changes are never part
of it.

A **pressing** is published state made from that master record. Today Presswerk creates an annotated
Git tag and one or more GitHub Releases. See the
[GitHub Release publisher reference](/presswerk/reference/publishers/github-release/) for its exact
fields. Other pressing types described in design documents are not implemented.

```mermaid Every pressing for version 0.3.0 points back to its one release commit, never to a later commit on main.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "feature"
  commit id: "master 0.3.0" tag: "v0.3.0"
  commit id: "later work"
```

The `v0.3.0` tag and the GitHub Release named by it belong to `master 0.3.0`. Re-running publication
for that commit inspects and completes those same objects.

## Candidate and release commit

A **release commit** changes the declared version compared with its first parent and contains a
matching changelog section. `prepare` normally creates it with a `🔖` Gitmoji subject.

A **candidate** is the release commit currently being validated or published, identified by
repository, full commit SHA, and version. A branch name is not a candidate because it can move.

```mermaid Preparation inserts a version-changing release commit between ordinary development and publication.
gitGraph
  commit id: "v0.2.2 state" tag: "v0.2.2"
  commit id: "✨ feature"
  commit id: "🐛 fix"
  commit id: "🔖 0.3.0" tag: "v0.3.0"
```

For a new release, `publish` validates that:

- the version is a valid SemVer successor of its parent;
- the bump is at least what commit analysis requires;
- `CHANGELOG.md` contains the frozen section for that version;
- the previous release is complete;
- the candidate's tag is missing rather than conflicting.

A matching tag on the candidate changes the operation into a retry or republish. Policy is not
reinterpreted using newer analyzer behavior, but the candidate identity, predecessor, credentials,
and remote state are still checked.

## The initial baseline

A fresh repository starts from an implicit `0.0.0` baseline. It does not need a `v0.0.0` tag. If the
first analyzed range contains a release-worthy commit, preparation creates the first release
commit.

```mermaid A first Gitmoji feature moves the implicit 0.0.0 baseline to a committed 0.1.0 release.
gitGraph
  commit id: "0.0.0 baseline"
  commit id: "✨ first feature"
  commit id: "release 0.1.0" tag: "v0.1.0"
```

The baseline must be committed as `0.0.0`. Adoption of an already-versioned repository is not part
of the current supported workflow.

## Version selection below 1.0

Presswerk follows SemVer. Before `1.0.0`, commit analysis maps catalog levels this way:

| Highest catalog level | Example result from `0.4.2` |
| --- | --- |
| `patch` | `0.4.3` |
| `minor` | `0.5.0` |
| `major` | `0.5.0` |

Presswerk never selects `1.0.0` automatically. The current CLI also has no general version-override
interface, so the automated flow remains on `0.x`.

## Pending releases and the strict boundary

A **pending release** is an untagged release commit. It may still be running through CI or its checks
may have failed. Presswerk cannot infer which state applies, so it reports the pending commit and
stops later release work.

```mermaid A fixup after an untagged release commit inherits a pending version and is not a new candidate.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "feature"
  commit id: "pending 0.3.0"
  commit id: "fixup is blocked"
```

Only `pending 0.3.0` can publish `0.3.0`. The fixup commit cannot. Retry that exact commit when
candidate CI did not establish a content failure. If its committed contents fail required checks,
revert it, fix the problem, and let preparation create a fresh candidate. See
[Recovery](../recovery/#candidate-ci-did-not-complete).

```mermaid Reverting a failed untagged candidate restores the previous version, after which a fix can lead to a fresh release commit.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "feature"
  commit id: "failed 0.3.0"
  commit id: "revert candidate"
  commit id: "CI fix"
  commit id: "fresh 0.3.0" tag: "v0.3.0"
```

No tag points to the failed candidate, so the fresh candidate may reuse the version after the
revert restores `0.2.2`.

## Publication state

Publication is ordered and idempotent:

1. inspect the remote tag and every configured GitHub Release;
2. stop before writing if any existing state conflicts;
3. create and push the annotated tag;
4. create or continue each GitHub Release as a draft, then publish it;
5. inspect the final state again.

```mermaid An exact retry remains anchored to the tagged release commit even when main has advanced.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "feature"
  commit id: "candidate 0.3.0" tag: "v0.3.0"
  commit id: "main advanced"
```

A retry targets the full SHA of `candidate 0.3.0`, not `main advanced`. Matching remote objects are
kept, missing objects are completed, and a tag on another commit is a hard conflict. Never move a
release tag to make a retry pass.
