---
title: Release publishers
description: How Presswerk creates releases outside the repository once the release commit is tagged.
sidebar:
  label: Overview
---

A release publisher creates a release somewhere outside the Git repository, from the release
commit and its frozen notes. `publish` first pushes the tag, then runs every declared publisher.

Every publisher follows the same rules on a retry:

- matching state is kept;
- missing state is created;
- conflicting state fails the run.

A release is complete once its tag and every publisher's release exist. `prepare` refuses to start
the next release before that.

## Available publishers

| Publisher | Option | Creates |
| --- | --- | --- |
| [GitHub Release](./github-release/) | [`publishers.github-release`](/presswerk/reference/options/#publishers.github-release) | A GitHub Release for the tag, with the notes as its text. |

Every release currently needs at least one GitHub Release publisher.

## Your own

Publishers for other targets are meant to be declared in the flake itself. See
[Build your own](./build-your-own/).
