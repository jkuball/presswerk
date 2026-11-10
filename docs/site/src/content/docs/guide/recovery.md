---
title: Recovery
description: Recover from failed preparation, failed candidate checks, or partial publication without changing release identity.
---

Recovery starts by identifying the exact ordinary commit, release commit, and tag involved.
Presswerk does not follow `main` to a replacement candidate and does not overwrite conflicting remote
state.

This page uses Git and the Presswerk CLI only, so it applies whether you
[release automatically or manually](../release-modes/). Where it says to check or publish a commit,
run your checks and `publish` for that exact commit yourself, or rerun the job that does it in your
CI. [GitHub Actions](../github-actions/#recovery) shows the commands for its workflow.

Use these invariants throughout:

- one version belongs to one release commit;
- an untagged failed candidate may be reverted before a fresh candidate is prepared;
- a tag reserves its version and must not be moved;
- matching publication state is retained, missing state is completed, and conflicting state stops;
- retries use the same commit, version, declaration, and frozen notes.

## Gather evidence first

The examples on this page use the default tag format `v{version}`. If
[`tag.format`](/reference/options/#tag.format) is set, use the tags it produces instead.

Record the branch tip, recent release commits, and tags before changing history:

```console
$ git fetch origin main --tags
$ git log --oneline --decorate -8 origin/main
$ git ls-remote --tags origin 'v*'
```

Also look up the state of each configured publisher, for example the GitHub Release for the tag.
Note the ordinary commit, the release commit, and the tag before you act.

## Failed preparation

Preparation can fail before creating a release commit because the declaration is invalid,
`flake.lock` is missing or out of date, an earlier release is pending or incomplete, the publisher
state of the previous release cannot be read, or the branch moved.

```mermaid A preparation failure adds no release commit; after the cause is fixed, a later ordinary commit can prepare normally.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "change"
  commit id: "preparation fix"
  commit id: "release 0.3.0" tag: "v0.3.0"
```

1. Confirm that `origin/main` has no new `🔖 Release …` commit from the failed attempt.
2. If `main` moved on in the meantime, leave the old attempt alone and prepare from the new tip.
3. If preparation reports a pending release, an earlier release commit has no tag yet. Follow
   [candidate checks did not complete](#candidate-checks-did-not-complete) for that commit.
4. If preparation reports an incomplete previous release, identify that tag and release commit, then
   follow [partial publication](#partial-publication) to retry the exact earlier candidate. Preparation
   cannot continue until that release chain is complete.
5. Fix any deterministic declaration, lock-file, or code problem in an ordinary commit and push it.
   Preparation from that commit is the next attempt.
6. If the failure was transient and `main` has not moved, run `prepare` again.

Do not write a release commit yourself to bypass a preparation error; fix the cause and run
`prepare` again.

### The previous release uses an unsupported declaration

Before 1.0, a newer CLI may not read the declaration of the previous release. `prepare` then stops
and names the previous tag and release commit together with the command to trust it. First check that
release with the Presswerk version it was
made with, which is the one locked in its `flake.lock`:

```console
$ git worktree add --detach ../previous-release v0.3.0
$ cd ../previous-release
$ nix run --inputs-from . presswerk -- publish HEAD
v0.3.0 from 7b3e5f1 was already released; nothing changed.
$ cd - && git worktree remove ../previous-release
```

If the release is complete, prepare again and trust exactly that predecessor by its tag and full
commit ID:

```console
$ presswerk prepare --trust-predecessor v0.3.0=RELEASE_COMMIT_SHA
```

Presswerk still checks that the tag points to that commit, but skips reading the previous
declaration. The release commit records the trusted predecessor in a `Presswerk-Trusted-Predecessor`
trailer, so `publish` accepts it as well. The flag is refused when the previous declaration is
readable.

## Candidate checks did not complete

A release commit may be pushed without its checks completing. The checks may never have started, may
have been canceled or hit transient infrastructure trouble, or the candidate's contents may genuinely
fail a build or check. With no tag, the commit is a **pending release** in every case. Later commits
that retain its version are blocked and cannot become replacement candidates.

Before choosing a recovery, verify that publication never started:

```console
$ git ls-remote --tags origin refs/tags/v0.3.0
```

The tag should not exist, and no publisher should have state for the version. If either exists,
treat the incident as [partial publication](#partial-publication).

### Retry a candidate that has not failed its checks

If the checks never ran or failed for reasons outside the candidate, run them again on the same
release commit. When they pass, publish exactly that commit:

```console
$ presswerk publish RELEASE_COMMIT_SHA
```

Do not revert a candidate merely because its checks did not run. A successful retry completes the
pending release without creating another commit.

### Revert a confirmed broken candidate

Reserve replacement for a candidate whose committed contents reproducibly fail a required build or
check. Revert that exact untagged release commit, fix the cause, and let preparation create a fresh
candidate.

```mermaid A confirmed broken untagged candidate is reverted, then the real fix is committed before a fresh candidate is prepared.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "feature"
  commit id: "failed 0.3.0"
  commit id: "revert candidate"
  commit id: "fix failure"
  commit id: "fresh 0.3.0" tag: "v0.3.0"
```

```console
$ git switch main
$ git pull --ff-only
$ git revert --no-edit RELEASE_COMMIT_SHA
$ # make and test the real fix
$ git add PATHS_YOU_FIXED
$ git commit -m "🐛 Fix candidate checks"
$ git push origin main
```

The revert restores the previous version, so the chain is no longer pending. Push the revert and the
fix together: preparing between them would create the same broken candidate again. After the push,
`prepare` may create a fresh release commit. Because the failed candidate had no tag, the new
candidate may use the same version.

Do not add a fixup commit directly on top of the pending version and try to publish it. Only the
version-changing release commit can publish that version.

## Partial publication

Publication pushes the annotated tag before the publishers run. A network failure or API error can
therefore leave a matching tag with incomplete publisher state, such as a draft GitHub Release. The
version is reserved once the tag exists.

```mermaid A partial release is repaired by retrying the tagged candidate, even if main later contains another ordinary commit.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "feature"
  commit id: "candidate 0.3.0" tag: "v0.3.0"
  commit id: "later main"
```

Publish the exact release commit again:

```console
$ presswerk publish RELEASE_COMMIT_SHA
Publishing v0.3.0 from 7b3e5f1
  tag      v0.3.0                  already pushed
  release  github-release.default  draft completed
Released v0.3.0 from 7b3e5f1.
```

Presswerk keeps a matching tag, continues matching publisher state, creates what is missing, and
verifies the final result. Each line reports whether that step was already complete. When all of
them were, the last line says that nothing changed.

Never:

- rerun `prepare` to invent a new candidate for the same tagged version;
- move or force-push the tag to another commit;
- delete a published tag or release to make the version look unused;
- publish the current `main` tip when the retry belongs to an older commit.

If the tag points to another commit, a release has incompatible metadata, or the remote state cannot
be established, Presswerk stops. Investigate manually and preserve the conflicting state. Do not
weaken the identity check.

## Verify recovery

After any successful recovery, compare the tag with the release commit:

```console
$ git fetch origin main --tags
$ git rev-list -n 1 v0.3.0
$ git show --no-patch --format=fuller RELEASE_COMMIT_SHA
```

The tag must resolve to the release commit whose checks passed, and every publisher must show a
complete release, for example a published rather than draft GitHub Release. The version section in
the changelog at that commit is the canonical release body.
