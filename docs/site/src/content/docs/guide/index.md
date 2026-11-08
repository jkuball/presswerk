---
title: Why Presswerk
description: Why immutable builds still need controlled release mutation, and what Presswerk does today.
---

A Nix flake can pin source, dependencies, and build instructions to one Git commit. Publishing a
release is still an impure operation: something must choose a version, update files, create a tag,
and call a forge API. If those steps use different revisions, regenerate notes later, or rebuild
from a moving branch, the release no longer has one clear source.

Presswerk keeps that boundary explicit. It treats the flake at one exact commit as the **master
record**, then derives every published result from that commit. The release declaration lives in
the flake, while credentials and mutation stay in the runtime that invokes the CLI.

```mermaid Development reaches a release commit only after Presswerk commits the version and notes; the tag stays on that exact commit.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "add search"
  commit id: "fix parsing"
  commit id: "release 0.3.0" tag: "v0.3.0"
  commit id: "next change"
```

In this history, `release 0.3.0` is the only commit that may publish version `0.3.0`. The later
`next change` commit cannot inherit that release merely because it still contains the same version.

## What Presswerk does

The current release flow has three commands (see the [CLI reference](/presswerk/reference/cli/)):

1. `plan` evaluates committed `HEAD`, analyzes commits, and shows the version and changelog that
   preparation would create. It writes nothing.
2. `prepare` updates configured Cargo version files, freezes generated notes in `CHANGELOG.md`, and
   creates a `🔖 Release v…` commit.
3. `publish [REVISION]` validates one exact candidate, pushes its annotated Git tag, and creates or
   completes its GitHub Release.

```mermaid Plan reads an ordinary commit, prepare creates a new release commit, and publish adds remote state without creating another commit.
gitGraph
  commit id: "previous release" tag: "v0.2.2"
  commit id: "ordinary HEAD"
  commit id: "prepared 0.3.0" tag: "v0.3.0"
```

The tag in this diagram represents publication. `plan` does not change history, and `publish` does
not advance to a newer branch tip.

Presswerk keeps no database or hosted control plane. Git history, the committed version and notes,
the annotated tag, and the GitHub Release are the durable state. An exact retry inspects that state,
keeps matching pieces, fills in missing pieces, and stops on conflicts.

## What the flake contributes

A consumer exposes a plain release declaration at `presswerk.default`. `presswerk.lib.mkRelease`
provides the supported authoring interface and validates the declaration with the Nix module system.
The declaration currently covers:

- the immutable source root;
- Cargo version files and the effective version;
- the changelog path and tag format;
- the Gitmoji catalog used for analysis;
- one or more GitHub Release destinations.

Nix evaluates the declaration at an exact commit. The CLI performs the impure Git and GitHub work.
Secrets never belong in the declaration; the GitHub token arrives through the runtime environment.
Use the [authoring options](/presswerk/reference/options/) and
[normalized declaration schema](/presswerk/reference/declaration-schema/) as the field-level
references.

## Current scope

:::caution
Presswerk is early, work-in-progress software. It releases itself, but its commands,
configuration, and output are not stable yet. Do not rely on it for production releases.
:::

The implemented path updates Cargo versions, generates Gitmoji-based notes, pushes an annotated Git
tag, and publishes GitHub Releases. The supported automated runtime is the documented two-phase
GitHub Actions workflow.

Design notes discuss more destinations, assets, artifact generators, `check`, `revert`, a dedicated
flake-parts adapter, and local push workflows. Those are not available features. A project may use
flake-parts normally and place `presswerk.default` in its flake outputs, but Presswerk does not yet
provide a flake-parts-specific release adapter.

## Read the guide

- [Release model](./model/) defines master records, pressings, candidates, release commits, and
  pending releases.
- [Quickstart](./quickstart/) releases a small Rust flake project through GitHub Actions.
- [GitHub Actions](./github-actions/) explains preparation, exact-candidate dispatch, serialization,
  and stale-run protection.
- [Gitmoji](../reference/analyzers/gitmoji/) covers commit subjects, version levels, overrides, and custom
  entries.
- [Recovery](./recovery/) handles failed preparation, failed candidate checks, and partial
  publication without moving or deleting release state.
