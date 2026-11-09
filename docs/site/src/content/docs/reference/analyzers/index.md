---
title: Commit analyzers
description: How Presswerk turns the commits since the previous release into a version bump and release notes.
sidebar:
  label: Overview
---

A commit analyzer reads the commits since the previous release and decides two things:

- **The version bump.** Each commit selects a level, and the highest level selects the next version.
  If no commit selects one, there is nothing to release.
- **The release notes.** The analyzer groups the commits into the notes that `prepare` writes into
  `CHANGELOG.md` and that the publishers publish.

`publish` runs the analyzer again for a new release and fails if the release commit's version is
smaller than the analysis requires.

## Available analyzers

| Analyzer | Option | Convention |
| --- | --- | --- |
| [Gitmoji](./gitmoji/) | [`analyzer.convention`](/presswerk/reference/options/#analyzer.convention) `= "gitmoji"` | A Gitmoji at the start of each commit subject, with a catalog you can extend. |

## Your own

Analyzers for other commit conventions are meant to be declared in the flake itself. See
[Build your own](./build-your-own/).
