# Overview

## Motivation

Most CI work is reinventing the same wheels: installing dependencies, packaging, and releasing.
Flakes removed the first two. A project gets a flake, CI evaluates and builds the whole flake,
and the results land in a binary cache. That part is now almost no effort.

Releases did not get that treatment. Every project still grows its own script that pushes
things, loads secrets, creates Git tags and GitHub releases, and deals with artifact registries.
Each one is written again from scratch, slightly differently, and rarely handles a failure
halfway through.

Presswerk should be that missing piece: add it to a project's flake and releases just work.
They should work the same whether triggered from CI or from a local machine.

### Why flakes are the core

A release should be defined from something completely immutable: the source at one Git commit.
Flakes are exactly that contract: locked inputs, a pure evaluation, and outputs addressed by
revision. Presswerk calls this the [master record](glossary.md#the-pressing-plant-analogy), and
every published result is pressed from it.

Channel-based or otherwise impure Nix setups break the contract. Reproducible non-flake Nix is
possible, but rebuilding what flakes already provide is not a goal. Presswerk therefore assumes
flakes.

### No state of its own

Presswerk keeps no storage of its own. Everything it needs already lives somewhere:

- **Git commits** hold the release declaration, the version, and the notes;
- **Git tags** pin each version to its release commit and act as reservation and lock;
- **the destinations** (registries, forges) hold the published pressings.

Every run derives what it needs from these three again. There is no database, no state file,
and no hosted service, so there is nothing to lose, migrate, or keep in sync.

The artifacts themselves are not Presswerk's state either: they live in the consumer's binary
caches. Keeping them available, for example to restore an old release later, is the consumer's
responsibility, like running CI.

## Purpose

Nix can build exact artifacts from an exact revision, but release publication is usually a
repository-specific mix of scripts and CI configuration. Presswerk should coordinate:

- committing the version before version-dependent derivations are built;
- understanding changes since the relevant previous release;
- preparing releases and maintenance lines;
- running only after the required CI suite passed;
- publishing to several external systems safely;
- creating Git tags and forge releases;
- generating and freezing release notes;
- retrying an exact candidate after partial publication.

Presswerk is a kindred spirit of [semantic-release](https://github.com/semantic-release/semantic-release)
and [Python Semantic Release](https://python-semantic-release.readthedocs.io/en/latest/). It
borrows ideas it likes from them, but it is not a rebuild of either. The most important
difference: the version must be known and committed before Nix builds the candidate.

## Target users

Presswerk is FOSS and developed in its author's free time. It is not a commercial product.

It must serve two kinds of consumers equally:

- **Public FOSS repositories**, with public forges, public registries, and public release notes.
- **Private and proprietary repositories** inside organizations, with private forges or
  organizations, private registries and binary caches, organization-managed credentials, and
  release notes that are not public.

Proprietary use is a primary requirement, not an afterthought. If Presswerk cannot release a
private repository inside an organization, it has failed. Concretely, this means:

- no assumption that source, history, release notes, or artifacts are publicly reachable;
- private registries, self-hosted forges, and custom API endpoints are ordinary configuration;
- organization credential models (GitHub Apps, organization secrets, OIDC, deploy keys) work
  alongside personal tokens;
- no calls to services the consumer did not configure; telemetry is opt-in and exported only
  to endpoints the consumer chose (see [observability](observability.md));
- the license must allow unrestricted use in proprietary projects. Presswerk is therefore
  dual-licensed under MIT or Apache-2.0, at the user's option.

## Goals

1. A standalone Rust CLI owns release planning, preparation, validation, publication, and
   recovery behavior.
1. The release configuration is declared in the consumer's flake and read by the CLI. Its exact
   shape is open (see [product shape](product-shape.md)).
1. A flake-parts module is an optional convenience layer that produces the same declaration.
1. Releases can eventually be run from a local machine as well as from any CI system, with the
   same safety checks. Automatic CI-driven releases come first.
1. The core is runtime-agnostic and contains no code specific to one CI system. Runtime wiring
   is the consumer's job, helped by documentation and possibly thin wrappers such as a GitHub
   Action.
1. Git forms the common source-control layer, with GitHub and Forgejo adapters.
1. A candidate is identified by repository, commit, and version.
1. The version is the first thing a release commits. It is committed before final candidate
   artifacts are built. This is a hard requirement.
1. Artifacts for several systems are a goal, but Presswerk's first self-release does not need
   binary assets. Its initial CI targets `x86_64-linux` only.
1. Commit analysis and release-note rendering are mandatory features behind a replaceable
   adapter. git-cliff is the first implementation.
1. Publication is idempotent: rerunning the same candidate completes missing state and never
   duplicates or silently overwrites existing state.
1. Any existing release can be published again from its tag, for example to restore a lost
   artifact. This is a core feature, not a recovery hack.
1. Credentials enter only at runtime and remain outside Nix evaluation results and the Nix store.
1. Private and proprietary repositories are supported as fully as public ones. Adaptability to
   self-hosted forges, private registries, and organization-internal publishers is key.
1. The common path remains small enough for a personal FOSS repository.

## Non-goals for the first release

- replacing Nix as the artifact builder;
- replacing a CI scheduler;
- CI-specific code in the core;
- a hosted release service or persistent controller;
- preview environments and preview builds. Teams often push artifacts for pull requests, such
  as an OCI image tagged `pr123`, to try them out. That is not a release, and Presswerk neither
  does it nor interferes with it: publishers only touch the names of real releases. Such flows
  can still use `presswerk plan --json` to learn the next version;
- per-component monorepo versions;
- support for every registry from day one;
- copying semantic-release's complete plugin API;
- pretending unrelated external services can form one atomic transaction.
