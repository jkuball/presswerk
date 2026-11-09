---
title: Release modes
description: Release automatically from CI or manually from any machine; Presswerk needs only a flake and a Git repository.
---

Presswerk is not tied to a CI system. It needs a flake that declares the release and a Git
repository whose remote is named `origin`. Who runs the commands is up to you.

Every release takes the same two steps, whatever runs them:

1. `prepare` creates a release commit with the new version and frozen notes on top of the current
   branch tip. You push that commit. It needs the history since the previous release, and read
   access to the remote's tags and, once a previous release exists, to the publishers, because it
   checks that the previous release is complete.
2. Once the release commit is ready, for example when your CI has run all its checks on it,
   `publish` tags it and publishes it. It needs the same history, push access for the tag, and
   credentials that let the publishers create releases.

:::caution[GitHub is required for now]
Every release currently needs a [GitHub Release publisher](/presswerk/reference/publishers/github-release/),
so publication talks to GitHub.
:::

## What every mode must ensure

Presswerk takes care of the release itself; whatever runs it takes care of the surroundings.

- **Presswerk** prepares only once the previous release is complete, publishes exactly the commit
  it is given, and can safely run again: matching state is kept, and conflicting state fails the
  run.
- **You or your CI** run one release at a time, provide the publishers' credentials, may push the
  release commit to the release branch, and decide what has to pass before `publish`.

## Automatic releases

With Presswerk, releasing can be a side effect of merging: no decision, no ceremony, no release
day. CI runs `prepare` on every push to the release branch, the branch you release from, usually
`main`. When the new commits contain a release-worthy change, a release commit appears, and once it
is green, CI publishes it. Nobody decides when to release; the commit history does. Presswerk
releases itself this way.

[CI integration](../ci-integration/) describes what a CI setup has to provide and how to build one.

## Manual releases

A person decides when to release. The steps are the same; only the trigger changes.

### On demand from CI

Keep the workflow, but run preparation only when someone asks for it, for example from a
`workflow_dispatch` button. [GitHub Actions](../github-actions/#release-on-demand) shows how.

### From your machine

Run the commands yourself:

```console
$ presswerk plan            # check the version and notes
$ presswerk prepare         # create the release commit
$ git push                  # push it to the release branch
$ presswerk publish HEAD    # tag and publish it
```

`publish HEAD` publishes the commit you prepared, as long as you have not moved `HEAD` since. You
can also pass its SHA. If the push is rejected because the branch moved, nothing has been
released yet: drop the release commit and prepare again on the new tip.
