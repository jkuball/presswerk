---
title: CI integration
description: What a CI system has to do to run Presswerk, and the setups the guide shows.
sidebar:
  label: Overview
---

Presswerk needs no CI integration. It runs wherever there is a flake and a Git checkout, so it fits
into the usual CI systems as a few commands in a job: `prepare` on the release branch, and `publish`
on the exact release commit once its checks passed. [Release modes](../release-modes/) describes both
steps.

## What a setup has to provide

- **Full history.** Both steps analyze the commits since the previous release and read its tag, so
  the checkout must not be shallow.
- **The exact commit.** `publish` receives the full SHA of the release commit, never a branch name,
  and the job checks out that commit before it builds and checks it.
- **Checks before publish.** The release commit gets the same build and checks as any other commit,
  and `publish` runs only after they passed.
- **No stale release commit.** The job pushes the release commit without force, so the push is
  rejected if the release branch moved on in the meantime. The newer run takes over.
- **One release at a time.** Release jobs are serialized, so two runs never prepare or publish
  concurrently.
- **Credentials.** The job can push the release commit to the release branch and the tag to the
  remote, and it provides the publishers' tokens.

## Setups

| Setup | Status |
| --- | --- |
| [GitHub Actions](../github-actions/) | Presswerk releases itself this way, automatically or on demand. |
| [NixCI](../nixci/) | Written from its documentation, not yet tested. |

Other systems follow the same pattern. The [Recovery](../recovery/) guide works with Git and the
Presswerk CLI alone, so it applies to every setup.
