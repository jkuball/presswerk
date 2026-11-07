---
title: Recovery
description: Recover from failed preparation, failed candidate checks, or partial publication without changing release identity.
---

Recovery starts by identifying the exact ordinary commit, release commit, tag, and workflow run.
Presswerk does not follow `main` to a replacement candidate and does not overwrite conflicting remote
state.

Use these invariants throughout:

- one version belongs to one release commit;
- an untagged failed candidate may be reverted before a fresh candidate is prepared;
- a tag reserves its version and must not be moved;
- matching publication state is retained, missing state is completed, and conflicting state stops;
- retries use the same commit, version, declaration, and frozen notes.

## Gather evidence first

Record the branch tip, candidate input, tags, and release state before changing history:

```console
$ git fetch origin main --tags
$ git log --oneline --decorate -8 origin/main
$ gh run list --workflow=ci.yml --limit=10
$ git ls-remote --tags origin 'v*'
$ gh release list --limit=10
```

For a dispatched run, inspect its event and candidate input in GitHub Actions. Save the ordinary push
SHA, release commit SHA, run URL, tag, and GitHub Release URL in the incident or change record.

## Failed preparation

Preparation can fail before creating or pushing a release commit because the declaration is invalid,
the lock file is inconsistent, the previous release is incomplete, GitHub is unavailable, or the
branch moved.

```mermaid A preparation failure adds no release commit; after the cause is fixed, a later ordinary commit can prepare normally.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "change"
  commit id: "preparation fix"
  commit id: "release 0.3.0" tag: "v0.3.0"
```

1. Confirm that `origin/main` has no new `🔖 Release …` commit from the failed run.
2. If the log says `main advanced`, do not revive the stale run. Inspect the newer push run instead.
3. Fix a deterministic problem in an ordinary commit and push it. The resulting push run owns the
   next preparation attempt.
4. If the failure was transient and `main` still points to the original SHA, rerun that push workflow.

Do not create a release commit by hand merely to bypass a preparation error. The workflow must still
build the ordinary revision and run the same stale-main guard.

## Failed candidate checks

A release commit may be pushed successfully and then fail its exact-candidate build or checks. With no
tag, it is a **pending release**. Later commits that retain its version are blocked and cannot become
replacement candidates.

```mermaid A failed untagged candidate is reverted, then the real fix is committed before a fresh candidate is prepared.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "feature"
  commit id: "failed 0.3.0"
  commit id: "revert candidate"
  commit id: "fix failure"
  commit id: "fresh 0.3.0" tag: "v0.3.0"
```

Before reverting, verify that publication never started:

```console
$ git ls-remote --tags origin refs/tags/v0.3.0
$ gh release view v0.3.0
```

Both should report no release state. If the tag or GitHub Release exists, treat the incident as
[partial publication](#partial-publication), not as a failed candidate.

Revert the exact untagged release commit, fix the cause, and only then push the repaired history:

```console
$ git switch main
$ git pull --ff-only
$ git revert --no-edit RELEASE_COMMIT_SHA
$ # make and test the real fix
$ git add PATHS_YOU_FIXED
$ git commit -m "🐛 Fix candidate checks"
$ git push origin main
```

Keeping the revert and fix in the same push avoids immediately preparing the same broken candidate
again. The push run checks the repaired ordinary tip, and `prepare` may create a fresh release commit.
Because the failed candidate had no tag, the new candidate may use the same version.

Do not add a fixup commit directly on top of the pending version and try to publish it. Only the
version-changing release commit can publish that version.

## Partial publication

Publication pushes the annotated tag before completing GitHub Releases. A network failure or API
error can therefore leave a matching tag, a draft GitHub Release, or another incomplete combination.
The version is reserved once the tag exists.

```mermaid A partial release is repaired by retrying the tagged candidate, even if main later contains another ordinary commit.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "feature"
  commit id: "candidate 0.3.0" tag: "v0.3.0"
  commit id: "later main"
```

Retry the dispatched workflow for the exact release commit. Either rerun its existing Actions run or
dispatch the workflow again with the same full SHA:

```console
$ gh workflow run ci.yml --ref main -f candidate=RELEASE_COMMIT_SHA
```

The run checks out `RELEASE_COMMIT_SHA`, rebuilds and checks it, then invokes `publish HEAD`.
Presswerk keeps a matching tag, continues a matching draft, creates missing GitHub Release state, and
verifies the final result.

Never:

- rerun `prepare` to invent a new candidate for the same tagged version;
- move or force-push the tag to another commit;
- delete a published tag or GitHub Release to make the version look unused;
- publish the current `main` tip when the retry belongs to an older SHA.

If the tag points to another commit, a release has incompatible metadata, or the remote state cannot
be established, Presswerk stops. Investigate manually and preserve the conflicting state. Do not
weaken the identity check.

## Verify recovery

After any successful recovery, record and compare:

```console
$ git fetch origin main --tags
$ git rev-list -n 1 v0.3.0
$ git show --no-patch --format=fuller RELEASE_COMMIT_SHA
$ gh run list --workflow=ci.yml --limit=10
$ gh release view v0.3.0 --json tagName,isDraft,isPrerelease,url
```

The tag must resolve to the release commit tested by the successful dispatched run, and the GitHub
Release must be published rather than left as a draft. The version section in `CHANGELOG.md` at that
commit is the canonical release body.
