---
title: "Quickstart: release a Rust flake"
sidebar:
  badge:
    text: Start here
    variant: tip
description: Configure a small Cargo project, run a release plan, and publish its first GitHub Release through exact-candidate CI.
---

This quickstart starts a new Cargo package at Presswerk's implicit `0.0.0` baseline. A Gitmoji feature
commit makes the first planned version `0.1.0`. GitHub Actions then prepares the release commit,
checks that exact commit, and publishes its tag and GitHub Release.

:::caution
Use a throwaway repository. Presswerk is early software, and this flow writes a commit, an annotated
tag, and a GitHub Release to the configured repository.
:::

## Prerequisites

You need:

- Git, Cargo, and a [flake-enabled Nix installation](https://nixos.org/download/);
- an authenticated [GitHub CLI](https://cli.github.com/) for local verification;
- an empty GitHub repository whose default branch is `main`;
- permission for GitHub Actions to write repository contents and dispatch workflows;
- a branch policy that permits the workflow's release commit push.

The implemented publisher creates a Git tag and GitHub Release only. This example has no release
assets and needs no binary cache.

## 1. Create the project

Create a Cargo binary and set its committed version to `0.0.0`:

```console
$ mkdir pktest && cd pktest
$ git init -b main
$ cargo init --name pktest .
```

Edit `Cargo.toml`:

```toml
[package]
name = "pktest"
version = "0.0.0"
edition = "2024"
```

Regenerate `Cargo.lock` after changing the package version:

```console
$ cargo generate-lockfile
```

Your generated `src/main.rs` is enough for the exercise.

## 2. Declare the package and release

Create `flake.nix`. Replace `YOUR-OWNER/pktest` with the repository you created. The Presswerk input
is pinned by `flake.lock`; the tag below is the v0.3.0 release toolchain this guide documents.

```nix title="flake.nix"
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    presswerk.url = "github:jkuball/presswerk/v0.3.0";
    presswerk.inputs.presswerk-release.url = "github:jkuball/presswerk/v0.1.0";
  };

  outputs = inputs@{ self, flake-parts, presswerk, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
      ];

      flake.presswerk.default = presswerk.lib.mkRelease {
        sourceRoot = self.outPath;
        versionFiles.cargo.pktest = { };
        publishers.github-release.default.repository = "YOUR-OWNER/pktest";
      };

      perSystem = { pkgs, ... }:
        let
          cargoPackage = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package;
        in
        {
          packages.default = pkgs.rustPlatform.buildRustPackage {
            inherit (cargoPackage) version;
            pname = cargoPackage.name;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
          };
        };
    };
}
```

The nested `presswerk-release` override keeps Presswerk's current self-release pin on the public
GitHub transport, so a consumer does not need an SSH key merely to lock the input.

`mkRelease` reads the version from `Cargo.toml`, updates that file and the matching `Cargo.lock`
package during preparation, uses the default Gitmoji catalog, writes `CHANGELOG.md`, and configures
the default `GITHUB_TOKEN` secret for publication. The [options reference](/presswerk/reference/options/)
documents every supported authoring field.

Lock and check the flake:

```console
$ nix flake lock
$ nix build .# --no-link
$ nix flake check
```

`--no-link` prevents `nix build` from leaving an untracked `result` symlink. `prepare` requires a
clean worktree.

A project may use flake-parts as shown here, but this is ordinary flake output composition. A
Presswerk-specific flake-parts adapter is not implemented.

## 3. Add the two-phase workflow

Create `.github/workflows/ci.yml`:

```yaml title=".github/workflows/ci.yml"
name: CI

on:
  pull_request:
  push:
    branches: [main]
  workflow_dispatch:
    inputs:
      candidate:
        description: Exact release commit to test and publish
        required: false
        type: string

permissions:
  actions: write
  contents: write

concurrency:
  group: ${{ github.event_name == 'pull_request' && github.ref || 'presswerk-release' }}
  cancel-in-progress: false

jobs:
  ci:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0
          ref: ${{ inputs.candidate || github.sha }}

      - uses: DeterminateSystems/determinate-nix-action@v3

      - name: Build and check
        run: |
          nix build .# --no-link --print-build-logs
          nix flake check --print-build-logs

      - name: Plan release
        if: github.event_name == 'pull_request'
        env:
          GITHUB_TOKEN: ${{ github.token }}
        run: nix run --inputs-from . presswerk -- plan

      - name: Prepare release
        if: github.event_name == 'push'
        env:
          GH_TOKEN: ${{ github.token }}
          GITHUB_TOKEN: ${{ github.token }}
        run: |
          git fetch origin main --tags
          if test "$(git rev-parse origin/main)" != "$GITHUB_SHA"; then
            echo "main advanced; leaving release work to the newer run"
            exit 0
          fi

          git config user.name github-actions[bot]
          git config user.email 41898282+github-actions[bot]@users.noreply.github.com

          before=$(git rev-parse HEAD)
          nix run --inputs-from . presswerk -- prepare
          candidate=$(git rev-parse HEAD)
          if test "$candidate" = "$before"; then
            exit 0
          fi

          git push origin HEAD:main
          gh workflow run ci.yml --ref main -f candidate="$candidate"

      - name: Publish release
        if: github.event_name == 'workflow_dispatch' && inputs.candidate != ''
        env:
          GITHUB_TOKEN: ${{ github.token }}
        run: |
          test "$(git rev-parse HEAD)" = "${{ inputs.candidate }}"
          git config user.name github-actions[bot]
          git config user.email 41898282+github-actions[bot]@users.noreply.github.com
          nix run --inputs-from . presswerk -- publish HEAD
```

[GitHub Actions](../github-actions/) explains why the stale-main guard, explicit dispatch, and
shared concurrency group are required.

## 4. Plan the first release

Add your repository remote, then commit the complete project with a release-worthy Gitmoji subject:

```console
$ git remote add origin git@github.com:YOUR-OWNER/pktest.git
$ git add .gitignore Cargo.toml Cargo.lock src flake.nix flake.lock .github/workflows/ci.yml
$ git commit -m "✨ Add the first command"
$ nix run --inputs-from . presswerk -- plan
```

The plan should report:

- no previous release, using the implicit `0.0.0` baseline;
- the `✨` commit in the analyzed range;
- `0.1.0` as the next version;
- pending updates to `Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md`.

`plan` reads committed `HEAD`. If the working tree is dirty, it warns and still excludes those local
changes. See the [CLI reference](/presswerk/reference/cli/) for command arguments and defaults.

## 5. Push and let both phases run

Push the ordinary commit:

```console
$ git push -u origin main
```

The push run builds the ordinary commit, invokes `prepare`, and pushes a new `🔖 Release v0.1.0`
commit. Because pushes made with `GITHUB_TOKEN` do not start another push workflow, the run explicitly
dispatches CI with the release commit's full SHA. The dispatched run checks out that exact candidate,
builds it, and calls `publish HEAD` only after its checks pass.

```mermaid The first push creates a release commit, then an explicit dispatch validates and publishes that exact commit.
gitGraph
  commit id: "✨ first command"
  commit id: "🔖 release 0.1.0" tag: "v0.1.0"
```

The tag appears only after candidate CI succeeds. If preparation finds no release-worthy commit, it
creates no release commit and dispatches nothing.

## 6. Verify the evidence

Fetch the result and compare the release commit across Git, Actions, and GitHub Releases:

```console
$ git fetch origin main --tags
$ git pull --ff-only origin main
$ git log --oneline --decorate -3 main
$ git rev-list -n 1 v0.1.0
$ gh run list --workflow=ci.yml --limit=5
$ gh release view v0.1.0 --json tagName,isDraft,isPrerelease,url
```

You should find:

1. the original push run with a successful **Prepare release** step;
2. a `workflow_dispatch` run whose candidate input equals the `🔖` commit SHA;
3. `v0.1.0` resolving to that same commit;
4. a published, non-draft GitHub Release for `v0.1.0`.

Open `CHANGELOG.md` at the release commit as a final check. Its `0.1.0` section is the frozen release
body that publication used.

## Next change

The workflow pushed the release commit, so start the next change only after fast-forwarding your
clean local `main` to it. Then use the [Gitmoji convention](../gitmoji/) and pass your GitHub token
when planning. From the second release onward, `plan` verifies that the previous GitHub Release
completed:

```console
$ git switch main
$ git pull --ff-only origin main
$ # edit, stage, and commit the next change
$ GITHUB_TOKEN="$(gh auth token)" nix run --inputs-from . presswerk -- plan
$ git push
```

A `🐛` commit plans a patch release, while `✨` plans the next minor release. Do not edit the generated
release commit or add a fixup on top of a failed candidate; use the
[recovery procedure](../recovery/).
