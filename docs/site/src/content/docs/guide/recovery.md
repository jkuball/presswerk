---
title: Recovery
description: Get the release chain moving again when a locked release cannot be published, by rewriting history or by retrying and reverting.
---

`prepare` locks a release into a release commit: its version, its notes, and the declaration it is
published with. `publish` then releases exactly that commit. Between the two, the locked release can
turn out to be unreleasable:

- `publish` was interrupted after pushing the tag, and a publisher is left incomplete;
- the checks for the release commit fail, so its locked contents cannot be released;
- the declaration locked into it is wrong, for example a publisher with a wrong repository.

Presswerk does not paper over this. Until the locked release is published or deliberately removed,
`prepare` refuses to lock another one on top of it. Recovery is how you get the release chain moving
again.

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
them were, the last line says that nothing changed. Once `publish` succeeds, the release is complete
and the rest of this page does not apply.

When retrying, do not:

- rerun `prepare` to invent a new candidate for the same tagged version;
- move or force-push the tag to another commit;
- publish the current `main` tip when the retry belongs to an older commit.

If the tag points to another commit, a release has incompatible metadata, or the remote state cannot
be established, Presswerk stops. Investigate manually and preserve the conflicting state. Do not
weaken the identity check.

## Rewrite history

You can also remove the release commit from `main` and force-push. If its tag already exists, that
means deleting the tag too, and cleaning up whatever the publishers have published, such as a GitHub
Release or an image in a registry, as far as that is possible. Presswerk does not do this for you.

The rest of this page recovers without force-pushing, so that `main` keeps a record of what
happened. Recovery starts by identifying the exact ordinary commit, release commit, and tag involved.
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
[`tag.format`](../../reference/options/#tag.format) is set, use the tags it produces instead.

Record the branch tip, recent release commits, and tags before changing history:

```console
$ git fetch origin main --tags
$ git log --oneline --decorate -8 origin/main
$ git ls-remote --tags origin 'v*'
```

Also look up the state of each configured publisher, for example the GitHub Release for the tag.
Note the ordinary commit, the release commit, and the tag before you act.

## Candidate checks fail

A release commit without a tag is a **pending release**. If its checks never ran or were canceled,
there is nothing to recover: run them on the same release commit and, when they pass, publish
exactly that commit with `presswerk publish RELEASE_COMMIT_SHA`. Do not revert a candidate merely
because its checks did not run.

Recover when the candidate's committed contents reproducibly fail a required build or check. Its
locked release cannot be published, and later commits that retain its version are blocked and cannot
become replacement candidates.

Before reverting, verify that publication never started:

```console
$ git ls-remote --tags origin refs/tags/v0.3.0
```

The tag should not exist, and no publisher should have state for the version. If either exists,
treat the incident as [partial publication](#partial-publication).

Revert that exact untagged release commit, fix the cause, and let preparation create a fresh
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
$ git revert --no-edit RELEASE_COMMIT_SHA
$ # make and test the real fix
$ git add PATHS_YOU_FIXED
$ git commit -m "🐛 Fix candidate checks"
$ git push origin main
```

Use `git revert` rather than undoing the changes by hand. The commit analyzer honors standard revert
commits, recognized by the `This reverts commit …` line that `git revert` writes: the reverted
commit and its revert cancel out and neither shows up in the release notes. An undo commit written
by hand is analyzed like any other commit.

The revert restores the previous version, so the chain is no longer pending. From there, `prepare`
works as usual: it analyzes every commit since the last tag again and computes the version from
them, including the fix. Because the failed candidate had no tag, the fresh candidate may get the
same version.

Do not add a fixup commit directly on top of the pending version and try to publish it. Only the
version-changing release commit can publish that version.

## The previous release uses an unsupported declaration

Before Presswerk 1.0, the declaration schema may change between Presswerk versions, and the CLI only
reads the schema of its own version. Run the CLI from the same Presswerk input as your release
module, for example with `nix run --inputs-from . presswerk`, so both always match.

That leaves one step where they cannot match: the first release after updating Presswerk. `prepare`
checks that the previous release is complete, but the new CLI cannot read the declaration it was
made with. `--trust-predecessor` exists for this step. Once you have confirmed that the previous
release is complete, for example by running `publish` for it with the old Presswerk version, pass
the command that `prepare` prints:

```console
$ presswerk prepare --trust-predecessor v0.3.0=RELEASE_COMMIT_SHA
```

Presswerk then skips the previous declaration but still checks that the tag points to that commit.
The release commit records the trust in a `Presswerk-Trusted-Predecessor` trailer, so `publish`
accepts it as well.
