---
title: NixCI
description: Release automatically from NixCI with one deploy job that prepares ordinary commits and publishes checked release commits.
---

:::caution[Untested]
This setup follows the [NixCI documentation](https://nix-ci.com/documentation) but has not been run
yet. Check the [open points](#open-points) before relying on it.
:::

[NixCI](https://nix-ci.com/) builds every output of the flake for each pushed commit. Its
[deploy jobs](https://nix-ci.com/documentation/deploy) run a package from the flake, but only after
the suite for that commit succeeded. That is the guarantee Presswerk needs before `publish`, so
[automatic releases](../release-modes/#automatic-releases) fit into a single deploy job.

## How it works

The deploy job runs the same script on every commit of the default branch:

1. `publish HEAD` tags and publishes the commit if it is a release commit. For any other commit
   it does nothing.
2. If `main` has moved on since this commit, the job stops and leaves release work to the newer
   suite.
3. `prepare` creates a release commit when the new commits contain a release-worthy change. The job
   pushes it to `main`.
4. The push starts a new suite for the release commit. NixCI builds and checks that exact commit,
   and its deploy job publishes it in step 1.

```mermaid The deploy job of an ordinary commit pushes the release commit; the deploy job of the release commit publishes it after its suite succeeded.
gitGraph
  commit id: "0.2.2" tag: "v0.2.2"
  commit id: "✨ feature"
  commit id: "🔖 release 0.3.0" tag: "v0.3.0"
```

GitHub Actions needs two phases and an explicit dispatch for the same flow, because pushes made with
its own token start no new runs. With NixCI, every push starts a suite.

## The deploy package

Add a package to the flake that runs the script. This example extends the `packages` of the flake
from the [walkthrough](../walkthrough/#2-declare-the-package-and-release):

```nix title="flake.nix"
packages = eachSystem (system:
  let
    pkgs = nixpkgs.legacyPackages.${system};
  in
  {
    # default = … as in the walkthrough

    presswerk-deploy = pkgs.writeShellApplication {
      name = "presswerk-deploy";
      runtimeInputs = [ pkgs.git pkgs.openssh ];
      text = # bash
        ''
          # Commit analysis needs the full history and the release tags.
          if test "$(git rev-parse --is-shallow-repository)" = true; then
            git fetch --unshallow --tags origin
          fi
          git fetch --tags origin main

          # Push the release commit and its tag with the deploy key.
          export GIT_SSH_COMMAND="ssh -i $PRESSWERK_DEPLOY_KEY -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"
          git remote set-url --push origin git@github.com:YOUR-OWNER/YOUR-REPO.git
          git config user.name "Presswerk on NixCI"
          git config user.email "presswerk@users.noreply.github.com"

          # A release commit whose suite succeeded: tag and publish it.
          nix run --inputs-from . presswerk -- publish HEAD

          if test "$(git rev-parse origin/main)" != "$NIX_CI_GIT_COMMIT_SHA"; then
            echo "main advanced; leaving release work to the newer suite"
            exit 0
          fi

          before=$(git rev-parse HEAD)
          nix run --inputs-from . presswerk -- prepare
          if test "$(git rev-parse HEAD)" != "$before"; then
            git push origin HEAD:main
          fi
        '';
    };
  });
```

`nix run --inputs-from . presswerk` uses the Presswerk input locked in the flake, so the CLI matches
the declaration. Replace `YOUR-OWNER/YOUR-REPO` with the repository.

## The NixCI configuration

Declare the deploy job in `nix-ci.nix` at the repository root:

```nix title="nix-ci.nix"
{
  deploy.presswerk = {
    package = "packages.x86_64-linux.presswerk-deploy";
    branches = ":default";
    in-repo = true;
    secrets = [ "GITHUB_TOKEN" ];
    ssh-keys = [
      {
        secret = "PRESSWERK_DEPLOY_KEY";
        public-key = "ssh-ed25519 AAAA… presswerk deploy key";
      }
    ];
  };
}
```

`in-repo = true` runs the job in a checkout of the repository at the commit, which Presswerk needs
to analyze history and create commits. `branches = ":default"` keeps releases on the default branch.

## Secrets and keys

Set two secrets for the repository in NixCI:

- `GITHUB_TOKEN`: a token that can write the repository's contents, for the
  [GitHub Release publisher](/presswerk/reference/publishers/github-release/#token). `plan` and
  `prepare` also use it to check that the previous release is complete.
- `PRESSWERK_DEPLOY_KEY`: the private half of an SSH deploy key with write access, used to push the
  release commit and its tag. NixCI writes it to a file and puts the path in the variable. Add the
  public half as a deploy key with write access in the repository settings, and allow it to push to
  `main` past branch protection.

Pin GitHub's SSH host keys instead of `StrictHostKeyChecking=accept-new` once the setup works.

## Open points

These depend on how NixCI behaves and must be confirmed in a first run:

- **History and tags.** Whether the `in-repo` checkout is shallow. The script deepens it if needed.
- **The `origin` remote.** Whether the checkout has an `origin` remote that Presswerk can read tags
  from with `git ls-remote`.
- **New suites from pushes.** Whether the push from the deploy job starts a suite for the release
  commit. Without it the release commit is never published automatically.
- **Serialization.** Whether NixCI runs deploys of one repository one at a time. If not, the check
  against `origin/main` before `prepare` is the only guard against two release runs racing.
- **Nix in the job.** Whether `nix` is on the deploy job's `PATH`. The script runs the CLI through it.
