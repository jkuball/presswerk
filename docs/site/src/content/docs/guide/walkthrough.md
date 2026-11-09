---
title: "Walkthrough: release automatically from GitHub Actions"
sidebar:
  label: Walkthrough
description: Set up automatic releases with GitHub Actions, from an empty repository to the first GitHub Release.
---

This walkthrough sets up [automatic releases](../release-modes/#automatic-releases) with GitHub
Actions, from an empty repository to its first GitHub Release. Every push to `main` may prepare a
release, and CI publishes the release commit once it has built and checked it.

The example project is a minimal Cargo package, only because Cargo is the version file Presswerk
supports today. Nothing else here is specific to Rust.

The package starts at Presswerk's implicit `0.0.0` baseline, and a Gitmoji feature commit makes
the first version `0.1.0`.

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

Presswerk publishes a Git tag and a GitHub Release. This example has no release assets and needs no
binary cache.

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

Create `flake.nix`. Replace `YOUR-OWNER/pktest` with the repository you created.

```nix title="flake.nix"
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    presswerk.url = "github:jkuball/presswerk/v{{presswerk.version}}";
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

## 3. Add the workflow

Create `.github/workflows/ci.yml`:

```yaml title=".github/workflows/ci.yml"
name: CI

on:
  push:
    branches: [main]
  workflow_dispatch:
    inputs:
      candidate:
        type: string

permissions:
  actions: write
  contents: write

concurrency: presswerk-release

jobs:
  ci:
    runs-on: ubuntu-latest
    env:
      GH_TOKEN: ${{ github.token }}
      GITHUB_TOKEN: ${{ github.token }}
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0
          ref: ${{ inputs.candidate || github.sha }}
      - uses: DeterminateSystems/determinate-nix-action@v3
      - run: |
          git config user.name github-actions[bot]
          git config user.email 41898282+github-actions[bot]@users.noreply.github.com

      - name: Build and check
        run: |
          nix build --no-link
          nix flake check

      - name: Prepare release
        if: github.event_name == 'push'
        run: |
          nix run --inputs-from . presswerk -- prepare
          if test "$(git rev-parse HEAD)" != "$GITHUB_SHA"; then
            git push origin HEAD:main
            gh workflow run ci.yml --ref main -f candidate="$(git rev-parse HEAD)"
          fi

      - name: Publish release
        if: inputs.candidate
        run: nix run --inputs-from . presswerk -- publish HEAD
```

Every run builds and checks its commit first. After that:

- A push to `main` runs `prepare`. If it creates a release commit, the run pushes it and starts
  the workflow again with that commit as `candidate`.
- A run with a `candidate` checks out exactly that commit, builds and checks it, and publishes it.
- The shared concurrency group lets only one release run at a time.

[GitHub Actions](../github-actions/) explains why the workflow dispatches itself and how to extend
it, for example with release plans on pull requests.

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

The push run builds and checks the ordinary commit, runs `prepare`, and pushes a new
`🔖 Release v0.1.0` commit. Because pushes made with `GITHUB_TOKEN` do not start another push
workflow, the run dispatches CI with the release commit's full SHA. In the dispatched run, CI
builds and checks that exact commit and only then calls `publish HEAD`.

```mermaid The first push creates a release commit, then an explicit dispatch validates and publishes that exact commit.
gitGraph
  commit id: "✨ first command"
  commit id: "🔖 0.1.0" tag: "v0.1.0"
```

The tag appears only after CI succeeded on the release commit. If preparation finds no
release-worthy commit, it creates no release commit and dispatches nothing. If `main` moved on in
the meantime, the push of the release commit is rejected, nothing is released, and the newer run
takes over.

## 6. Verify the evidence

Fetch the result and compare the release commit across Git, Actions, and GitHub Releases:

```console
$ git pull --ff-only --tags
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
clean local `main` to it. Then use the [Gitmoji convention](/presswerk/reference/analyzers/gitmoji/) and pass your GitHub token
when planning. From the second release onward, `plan` verifies that the previous GitHub Release
completed:

```console
$ git pull --ff-only
$ # edit, stage, and commit the next change
$ GITHUB_TOKEN="$(gh auth token)" nix run --inputs-from . presswerk -- plan
$ git push
```

A `🐛` commit plans a patch release, while `✨` plans the next minor release. Do not edit the generated
release commit or add a fixup on top of a failed candidate; use the
[recovery procedure](../recovery/).

This workflow releases automatically. To decide yourself when to release, prepare on demand from
CI or run the same commands from your machine; see [Release modes](../release-modes/#manual-releases).
