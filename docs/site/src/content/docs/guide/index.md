---
title: Why Presswerk
description: The problems of releasing reproducible software, the idea behind Presswerk, and what it deliberately is not.
---

## The problem

A Nix flake pins source, dependencies, and build instructions to one Git commit. The release around
that build usually does not. Someone chooses a version, updates files, writes notes, creates a tag,
and calls a forge API, often in a script that follows whatever the branch points to at the moment.
That goes wrong in familiar ways:

- **The wrong commit ships.** The tests ran on one commit, but the tag lands on a newer branch tip,
  or the release is built again from a branch that moved in the meantime.
- **Notes drift.** Release notes are generated again later, from a different range of commits or with
  different rules, and no longer match what was released.
- **Releases end halfway.** A run stops after the tag but before the release exists, or something in
  the outside world is deleted afterwards. A rerun then creates a second release, refuses because the
  tag exists, or needs someone to clean up by hand.

## The idea

A flake is locked, so the release can be locked too. Presswerk records each release in one commit,
the **release commit**, and treats it as the **master record**: everything it publishes is pressed
from that commit.

- `prepare` creates this master record on top of your changes: a release commit with the new
  version and frozen notes. Your checks should run on that commit: then they check exactly what
  will be published.
- The version is committed before anything is built, so it is an ordinary input of your flake. Your
  build can use it, for example in `--version` output or package metadata, and a check of the
  release commit builds exactly what you release. The price: everything that reads the version is
  rebuilt for each release.
- The notes are committed too. `CHANGELOG.md` is part of the release commit, locked like the code,
  and the published release text is taken from it.
- The release configuration is part of the release. The declaration lives in the flake, so its rules
  are locked with everything else: which files carry the version, how commits are analyzed, where
  the release is published, and even the Presswerk version, through `flake.lock`.
- `publish` tags and publishes that exact commit. Run it again, and it compares the outside world
  with the commit: matching state is kept, missing state is created again, and conflicting state
  stops the run.

## What Presswerk is not

- **Not a service.** There is no database and no hosted control plane. The state lives in Git and at
  the publishers, so anyone with the repository can inspect it, and anyone with credentials for the
  target systems can continue it.
- **Not a CI system.** Presswerk needs a flake and a Git repository, and runs wherever you start it,
  so it fits into any common CI as a few commands in a job. See [Release modes](./release-modes/).
- **Not a build system.** Nix builds your software. Presswerk only decides, records, and publishes the
  release.

## Next

The [release model](./model/) defines the terms used throughout these docs. The
[walkthrough](./walkthrough/) sets up a project from scratch.
