---
title: CI integration
description: What a CI system has to do to run Presswerk, and the setups the guide shows.
sidebar:
  label: Overview
---

Presswerk needs no CI integration. It runs wherever there is a flake and a Git checkout, so it fits
into the usual CI systems as a few commands: `prepare` on the release branch, and `publish` on the
exact release commit once its checks passed. [Release modes](../release-modes/) describes both steps.

## What a setup has to provide

- **Full history.** Both steps analyze the commits since the previous release and read its tag, so
  your CI should fetch the full history instead of a shallow clone.
- **The exact commit.** `publish` receives the full SHA of the release commit, never a branch name,
  and your CI should check out that commit before it builds and checks it.
- **Checks before publish.** Your CI should build and check the release commit like any other
  commit, and run `publish` only after those checks passed.
- **No stale release commit.** Your CI should push the release commit without force, so the push
  is rejected if the release branch moved on in the meantime. CI for the newer commit takes over.
- **One release at a time.** Your CI should never prepare or publish two releases at once.
- **Credentials.** Your CI should be allowed to push the release commit to the release branch and
  the tag to the remote, and it should provide the publishers' tokens.

## Setups

| Setup | Status |
| --- | --- |
| [GitHub Actions](../github-actions/) | Presswerk releases itself this way, automatically or on demand. |
| [NixCI](../nixci/) | Written from its documentation, not yet tested. |

Other systems follow the same pattern. The [Recovery](../recovery/) guide works with Git and the
Presswerk CLI alone, so it applies to every setup.
