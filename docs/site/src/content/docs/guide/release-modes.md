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
2. Once your checks passed on that exact commit, `publish` tags it and publishes it. It needs the
   same history, push access for the tag, and credentials that let the publishers create releases.

:::caution[GitHub is required for now]
Every release currently needs a [GitHub Release publisher](/presswerk/reference/publishers/github-release/),
so publication talks to GitHub. That is a limitation of the publishers available today, not of the
release model, and it will go away.
:::

## What every mode must ensure

Presswerk enforces its part, and whatever runs it provides the rest:

- **Presswerk** refuses to prepare while the previous release is incomplete, publishes only the
  exact commit it is given, and on a retry keeps matching state and fails the run on conflicts.
- **You or your CI** run the checks on the release commit before `publish`, run one release at a
  time, provide the publisher's token, and allow the release commit to be pushed to the release
  branch.

## Automatic releases

CI runs `prepare` on every push to the release branch, the branch you release from, usually
`main`. When the new commits contain a release-worthy change, a release commit appears, CI checks
it, and CI publishes it. Nobody decides when to release; the commit history does. Presswerk
releases itself this way.

[CI integration](../ci-integration/) lists what a CI setup has to provide and shows complete setups
for GitHub Actions and NixCI.

## Manual releases

A person decides when to release. The steps are the same; only the trigger changes.

### On demand from CI

Keep the workflow, but run preparation only when someone asks for it, for example from a
`workflow_dispatch` button. [GitHub Actions](../github-actions/#release-on-demand) shows the change.

### From your machine

Run the CLI from the Presswerk input locked in your flake, so it matches your declaration, with
the publisher's token in the environment:

```console
$ alias presswerk='nix run --inputs-from . presswerk --'
$ export GITHUB_TOKEN="$(gh auth token)"
$ git switch main
$ git pull --ff-only origin main
$ presswerk plan                 # check the version and notes
$ presswerk prepare              # create the release commit
$ git push origin main           # push the release commit for your checks
$ presswerk publish HEAD         # after the checks passed: tag and publish it
```

`publish HEAD` publishes the commit you prepared, as long as you have not moved `HEAD` since. You
can also pass its SHA. If the push is rejected because the branch moved, nothing has been
released yet: drop the release commit and prepare again on the new tip.
