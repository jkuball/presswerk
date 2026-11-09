---
title: CI integration
description: What a CI system has to do to run Presswerk, and the setups the guide shows.
sidebar:
  label: Overview
---

Presswerk does not ship a CI integration. A CI setup is a few commands in a job that run the two
steps from [Release modes](../release-modes/): `prepare` on the release branch, and `publish` on the
exact release commit once its checks passed.

## What a setup has to provide

- **Full history.** Both steps analyze the commits since the previous release and read its tag, so
  the checkout must not be shallow.
- **The exact commit.** `publish` receives the full SHA of the release commit, never a branch name,
  and the job checks out that commit before it builds and checks it.
- **Checks before publish.** The release commit gets the same build and checks as any other commit,
  and `publish` runs only after they passed.
- **No stale preparation.** Before `prepare`, the job makes sure the release branch still points to
  the commit it checked. If the branch moved on, the newer run takes over.
- **One release at a time.** Release jobs are serialized, so two runs never prepare or publish
  concurrently.
- **Credentials.** The job may push the release commit to the release branch and the tag to the
  remote, and it provides the publishers' tokens.

## Setups

| Setup | Status |
| --- | --- |
| [GitHub Actions](../github-actions/) | Presswerk releases itself this way, automatically or on demand. |
| [NixCI](../nixci/) | Written from its documentation, not yet tested. |

Other systems follow the same pattern. The [Recovery](../recovery/) guide works with Git and the
Presswerk CLI alone, so it applies to every setup.
