---
title: NixCI
description: Release automatically from NixCI with one deploy job that prepares ordinary commits and publishes checked release commits.
---

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
  commit id: "🔖 0.2.2" tag: "v0.2.2"
  commit id: "✨ feature"
  commit id: "🔖 0.3.0" tag: "v0.3.0"
```

GitHub Actions needs two phases and an explicit dispatch for the same flow, because pushes made with
its own token start no new runs. With NixCI, every push starts a suite.

## The GitHub App

The deploy job pushes the release commit and its tag, and creates the GitHub Release, as a GitHub
App. Its pushes start NixCI suites, and the app can bypass a ruleset that requires pull requests on
`main`:

1. Create a GitHub App under **Settings → Developer settings → GitHub Apps**. It needs no callback
   URL and no webhook, and only the repository permission **Contents: Read and write**.
2. Install it on the repository.
3. Note its client ID, and generate a private key.
4. Add the app as a bypass actor to the ruleset on `main`, if there is one.

The job commits as the app's bot user. Find its ID with
`gh api '/users/YOUR-APP[bot]' --jq .id`.

## The deploy package

Add two packages to the flake: one prints a short-lived installation token of the app, the other
runs the release script with it. This example extends the `packages` of the flake from the
[walkthrough](../walkthrough/#2-declare-the-package-and-release):

```nix title="flake.nix"
packages = eachSystem (system:
  let
    pkgs = nixpkgs.legacyPackages.${system};
    repository = "YOUR-OWNER/YOUR-REPO";
  in
  rec {
    # default = … as in the walkthrough

    # Prints an installation token of the app, scoped to the repository.
    github-app-token = pkgs.writeShellApplication {
      name = "github-app-token";
      runtimeInputs = [ pkgs.coreutils pkgs.curl pkgs.jq pkgs.openssl ];
      text = # bash
        ''
          key="$(mktemp)"
          trap 'rm -f "$key"' EXIT
          printf '%s\n' "$RELEASE_APP_PRIVATE_KEY" > "$key"

          base64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }
          api() {
            curl --fail-with-body --silent --show-error --retry 3 \
              --header "Accept: application/vnd.github+json" \
              --header "Authorization: Bearer $jwt" \
              "$@"
          }

          now="$(date +%s)"
          header="$(printf '%s' '{"alg":"RS256","typ":"JWT"}' | base64url)"
          payload="$(printf '{"iat":%d,"exp":%d,"iss":"%s"}' \
            "$((now - 60))" "$((now + 540))" "YOUR-CLIENT-ID" | base64url)"
          signature="$(printf '%s.%s' "$header" "$payload" \
            | openssl dgst -sha256 -sign "$key" -binary | base64url)"
          jwt="$header.$payload.$signature"

          installation="$(api "https://api.github.com/repos/${repository}/installation" | jq -er .id)"
          api --request POST \
            --data '{"repositories":["${baseNameOf repository}"]}' \
            "https://api.github.com/app/installations/$installation/access_tokens" \
            | jq -er .token
        '';
    };

    presswerk-deploy = pkgs.writeShellApplication {
      name = "presswerk-deploy";
      runtimeInputs = [
        pkgs.coreutils
        pkgs.git
        pkgs.nix
        presswerk.packages.${system}.default
        github-app-token
      ];
      text = # bash
        ''
          commit="$NIX_CI_GIT_COMMIT_SHA"

          # Deploy jobs have no nix on PATH. The one from runtimeInputs talks to the host's
          # daemon, which holds the outputs the suite just built.
          export NIX_REMOTE="''${NIX_REMOTE:-daemon}"

          # Authenticate git and Presswerk as the app. The token stays in the environment,
          # never in the checkout's Git configuration.
          GITHUB_TOKEN="$(github-app-token)"
          export GITHUB_TOKEN
          export GIT_CONFIG_COUNT=1
          export GIT_CONFIG_KEY_0="http.https://github.com/.extraheader"
          GIT_CONFIG_VALUE_0="AUTHORIZATION: basic $(printf 'x-access-token:%s' "$GITHUB_TOKEN" | base64 -w0)"
          export GIT_CONFIG_VALUE_0

          if git remote get-url origin > /dev/null 2>&1; then
            git remote set-url origin "https://github.com/${repository}.git"
          else
            git remote add origin "https://github.com/${repository}.git"
          fi
          git config user.name "YOUR-APP[bot]"
          git config user.email "BOT-USER-ID+YOUR-APP[bot]@users.noreply.github.com"

          # Commit analysis needs the full history and the release tags.
          if test "$(git rev-parse --is-shallow-repository)" = true; then
            git fetch --unshallow --tags origin
          fi
          git fetch --tags origin main

          # A release commit whose suite succeeded: tag and publish it.
          presswerk publish HEAD

          if test "$(git rev-parse origin/main)" != "$commit"; then
            echo "main advanced; leaving release work to the newer suite"
            exit 0
          fi

          presswerk prepare
          if test "$(git rev-parse HEAD)" != "$commit"; then
            # Without force: a concurrent push to main rejects this one.
            git push origin HEAD:refs/heads/main
          fi
        '';
    };
  });
```

Replace `YOUR-OWNER/YOUR-REPO`, `YOUR-CLIENT-ID`, `YOUR-APP`, and `BOT-USER-ID`. The Presswerk
package comes from the flake's `presswerk` input, so the CLI matches the declaration. The `in-repo`
checkout is shallow unless the [configuration](https://nix-ci.com/documentation/configuration) sets
`shallow-clone = false`, so the script deepens it if needed. It also points `origin` at GitHub
instead of relying on the remote of the checkout.

## The NixCI configuration

Declare the deploy job in `nix-ci.nix` at the repository root:

```nix title="nix-ci.nix"
{
  deploy.presswerk = {
    package = "packages.x86_64-linux.presswerk-deploy";
    branches = ":default";
    in-repo = true;
    secrets = [ "RELEASE_APP_PRIVATE_KEY" ];
  };
}
```

`in-repo = true` runs the job in a checkout of the repository at the commit, which Presswerk needs
to analyze history and create commits. `branches = ":default"` keeps releases on the default branch.

## Secrets

Set the secret `RELEASE_APP_PRIVATE_KEY` for the repository in NixCI to the app's private key. The
script turns it into a `GITHUB_TOKEN` for the push, the tag, and the
[GitHub Release publisher](/reference/publishers/github-release/#token). `plan` and `prepare` also
use it to check that the previous release's GitHub Release is complete.

Add the secrets of other publishers, such as registry credentials, to `secrets` as well.

## With a deploy key instead

:::caution[Untested]
This variant has not been run yet.
:::

Instead of an app, the job can push with an SSH deploy key that has write access and may push to
`main` past the ruleset. It then needs a separate `GITHUB_TOKEN` secret for the GitHub Release
publisher. Declare the key in `nix-ci.nix`:

```nix title="nix-ci.nix"
{
  deploy.presswerk = {
    # package, branches, in-repo as above
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

In the deploy script, replace the app authentication with:

```bash
export GIT_SSH_COMMAND="ssh -i $PRESSWERK_DEPLOY_KEY -o IdentitiesOnly=yes"
git remote set-url --push origin git@github.com:YOUR-OWNER/YOUR-REPO.git
```

and add `pkgs.openssh` to `runtimeInputs`. Pin GitHub's SSH host keys in a `known_hosts` file.

NixCI writes the key to a file and sets `PRESSWERK_DEPLOY_KEY` to its path. GitHub sends push
events for pushes made with a deploy key, but whether NixCI starts a suite for them has not been
confirmed. Without it, the release commit is never published automatically.

## Concurrency

NixCI documents no way to run the deploys of one repository one at a time. The push without force
keeps a stale deploy from releasing, but two deploys of release commits could publish at once.
