---
title: "Walkthrough: release automatically from GitHub Actions"
sidebar:
  label: Walkthrough
description: Set up automatic releases with GitHub Actions, from an empty repository to the first GitHub Release.
---

This walkthrough sets up [automatic releases](../release-modes/#automatic-releases) with GitHub
Actions, from an empty repository to its first GitHub Release. Every push to `main` may prepare a
release, and CI publishes the release commit once it has built and checked it.

The example project is a minimal Cargo package. Nothing else here is specific to Rust: the
[version files](../../reference/version-files/) also cover TOML, JSON, plain `VERSION` files,
and any file a regular expression can find the version in.

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

Create a Cargo binary:

```console
$ mkdir YOUR-REPO && cd YOUR-REPO
$ git init -b main
$ cargo init --name YOUR-CRATE .
```

Cargo starts the package at `0.1.0`. Set it to `0.0.0` instead: nothing has been released yet,
and the first release is the one that gives the project its first version.

```diff lang="toml" title="Cargo.toml"
 [package]
 name = "YOUR-CRATE"
-version = "0.1.0"
+version = "0.0.0"
 edition = "2024"
```

Regenerate `Cargo.lock` after changing the package version:

```console
$ cargo generate-lockfile
```

Your generated `src/main.rs` is enough for the exercise.

## 2. Declare the package and release

Create `flake.nix`. Replace `YOUR-OWNER/YOUR-REPO` with the repository you created.

```nix title="flake.nix"
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    presswerk.url = "github:jkuball/presswerk/v{{presswerk.version}}";
  };

  outputs = { self, nixpkgs, presswerk }:
    let
      eachSystem = nixpkgs.lib.genAttrs [
        "aarch64-darwin"
        "x86_64-linux"
      ];
    in
    {
      presswerk.default = presswerk.lib.mkRelease {
        sourceRoot = self.outPath;
        versionFiles.cargo.YOUR-CRATE = { };
        publishers.github-release.default.repository = "YOUR-OWNER/YOUR-REPO";
        analyzer.gitmoji = { };
      };

      packages = eachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "YOUR-CRATE";
            version = self.presswerk.default.version;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
          };
        });
    };
}
```

`mkRelease` reads the version from `Cargo.toml`, updates that file and the matching `Cargo.lock`
package during preparation, analyzes Gitmoji commits with the default catalog (declare
`analyzer.conventional = { };` instead for [Conventional Commits](/reference/analyzers/#conventional-commits)),
writes `CHANGELOG.md`, and configures
the default `GITHUB_TOKEN` secret for publication. The package takes its version from the release
with `self.presswerk.default.version` instead of reading `Cargo.toml` again. The [options reference](/reference/options/)
documents every supported authoring field.

Lock and check the flake:

```console
$ nix flake lock
$ nix build
$ nix flake check
```

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
        run: nix flake check --build-all --keep-going --print-build-logs

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
  the workflow again with that commit as `candidate`. It has to start that run itself, because
  pushes made with `GITHUB_TOKEN` do not trigger workflows.
- A run with a `candidate` checks out exactly that commit, builds and checks it, and publishes it.
- The shared concurrency group lets only one release run at a time.
- `nix run --inputs-from . presswerk` runs the CLI from your flake's `presswerk` input, so it is
  always the version locked in `flake.lock`, the same one that provides `mkRelease`.

[GitHub Actions](../github-actions/) shows how to extend the workflow, for example with release plans
on pull requests.

## 4. Plan the first release

Add your repository remote, then commit the complete project with a release-worthy Gitmoji subject:

```console
$ git remote add origin git@github.com:YOUR-OWNER/YOUR-REPO.git
$ git add .gitignore Cargo.toml Cargo.lock src flake.nix flake.lock .github/workflows/ci.yml
$ git commit -m "✨ Add the first command"
$ nix run --inputs-from . presswerk -- plan
commit             3f621b7d7e693013a68f2849465f43193f474967
previous release   none (implicit 0.0.0 baseline)
analyzed commits   1
  3f621b7  ✨ Add the first command
next version       0.1.0

version files
  Cargo.toml  0.0.0 -> 0.1.0
  Cargo.lock  0.0.0 -> 0.1.0

notes
  ### ✨ Introduce new features

  - Add the first command

Nothing written. Run `presswerk prepare` to commit 0.1.0.
```

`plan` reads committed `HEAD`. If the working tree is dirty, it warns and still excludes those local
changes. See the [CLI reference](/reference/cli/) for command arguments and defaults.

## 5. Push and let both phases run

Push the ordinary commit:

```console
$ git push -u origin main
```

The push run builds and checks the ordinary commit, runs `prepare`, pushes a new
`🔖 Release v0.1.0` commit, and dispatches CI with its full SHA. The dispatched run builds and checks
that exact commit and only then calls `publish HEAD`.

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

Fetch the release commit and its tag, then look at the runs and the release:

```console
$ git pull --ff-only --tags
$ git log --oneline -2
9c4e1d2 (HEAD -> main, tag: v0.1.0, origin/main) 🔖 Release v0.1.0
3f621b7 ✨ Add the first command
$ gh run list --workflow ci.yml --limit 2 --json event,conclusion
[
  {
    "conclusion": "success",
    "event": "workflow_dispatch"
  },
  {
    "conclusion": "success",
    "event": "push"
  }
]
$ gh release view v0.1.0 --json tagName,isDraft,url
{
  "isDraft": false,
  "tagName": "v0.1.0",
  "url": "https://github.com/YOUR-OWNER/YOUR-REPO/releases/tag/v0.1.0"
}
```

The tag points to the release commit, the push run prepared it, the dispatched run published it, and
the GitHub Release is not a draft.

Open `CHANGELOG.md` at the release commit as a final check. Its `0.1.0` section is the frozen release
body that publication used.

## Next change

The workflow pushed the release commit, so start the next change only after fast-forwarding your
clean local `main` to it. Then use the [commit convention](/reference/analyzers/) your release declares and pass your GitHub token
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
