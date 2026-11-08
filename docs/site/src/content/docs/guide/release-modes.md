---
title: Release modes
description: Release automatically from CI or manually from any machine; Presswerk needs only a flake and a Git repository.
---

Presswerk is not tied to a CI system. It needs a flake that declares the release and a Git
repository whose remote is named `origin`. Who runs the commands, and where, is up to you.

Every release takes the same two steps, whatever runs them:

1. `prepare` creates a release commit with the new version and frozen notes on top of the current
   branch tip. You push that commit.
2. Once that exact commit has passed your checks, `publish` tags it and publishes it.

```mermaid Prepare adds the release commit; publish tags that same commit after its checks, whether CI or a person runs the commands.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "✨ feature"
  commit id: "🔖 release 0.3.0" tag: "v0.3.0"
```

:::note
Every release currently needs a [GitHub Release publisher](/presswerk/reference/publishers/github-release/),
so publication talks to GitHub. That is a limitation of the publishers available today, not of the
release model.
:::

## Automatic releases

CI runs `prepare` on every push to the release branch. When the new commits contain a
release-worthy change, a release commit appears, CI checks it, and CI publishes it. Nobody decides
when to release; the commit history does. Presswerk releases itself this way.

[GitHub Actions](../github-actions/) shows a complete workflow for this mode.

## Manual releases

A person decides when to release. The steps are the same; only the trigger changes.

### On demand from CI

Keep the workflow, but run preparation only when someone asks for it, for example from a
`workflow_dispatch` button. [GitHub Actions](../github-actions/#release-on-demand) shows the change.

### From your machine

Run the CLI from the Presswerk input locked in your flake, so it matches your declaration. The
publisher's token must be in the environment once a previous release exists, because `plan` and
`prepare` check that the previous release is complete:

```console
$ alias presswerk='nix run --inputs-from . presswerk --'
$ export GITHUB_TOKEN="$(gh auth token)"
$ git switch main
$ git pull --ff-only origin main
$ presswerk plan                 # check the version and notes
$ presswerk prepare              # create the release commit
$ git push origin main           # publish the release commit for your checks
$ presswerk publish HEAD         # after the checks passed: tag and publish it
```

`publish HEAD` publishes the commit you prepared, as long as you have not moved `HEAD` since. You
can also pass its SHA. If the push is rejected because the branch moved, the release commit was
never published: drop it and prepare again on the new tip.

## What every mode must ensure

Presswerk enforces its part, and whatever runs it provides the rest:

- **Presswerk** refuses to prepare while the previous release is incomplete, publishes only the
  exact commit it is given, and on a retry keeps matching state and stops on conflicts.
- **You or your CI** run the checks on the release commit before `publish`, run one release at a
  time, provide the publisher's token, and allow the release commit to be pushed to the release
  branch.

## CI integration

Automating Presswerk means running these two steps from a CI system. The guide includes one
complete example:

- [GitHub Actions](../github-actions/)

Other systems follow the same pattern: prepare on the release branch, push the release commit,
check it, and publish that exact commit.
