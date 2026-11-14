---
title: GitHub Actions
description: How the walkthrough's workflow releases from GitHub Actions, and how to extend it.
---

The [walkthrough](../walkthrough/#3-add-the-workflow) sets up a minimal workflow for
[automatic releases](../release-modes/#automatic-releases) from GitHub Actions. This page explains
how it works and how to extend it. The [CLI reference](/reference/cli/) documents the
commands it runs.

## How the workflow works

Every run builds and checks its commit first. After that, a release takes two runs:

1. **The push run prepares.** A push to `main` runs `prepare` on the pushed commit. If that creates
   a release commit, the run pushes it to `main` and dispatches the workflow again with the commit's
   full SHA as `candidate`.
2. **The candidate run publishes.** The dispatched run checks out exactly that commit, builds and
   checks it, and runs `publish HEAD`.

```mermaid The push run of ✨ feature creates 🔖 0.3.0; the candidate run checks it and tags it, even after main has moved on to 🐛 fix.
gitGraph
  commit id: "🔖 0.2.2" tag: "v0.2.2"
  commit id: "✨ feature"
  commit id: "🔖 0.3.0" tag: "v0.3.0"
  commit id: "🐛 fix"
```

The candidate run publishes the commit it was given, never the tip of `main`. If `🐛 fix` reaches
`main` before it starts, it still checks and publishes `🔖 0.3.0`.

### Why the workflow dispatches itself

Pushes made with the workflow's `GITHUB_TOKEN` do not start new workflow runs, so the pushed release
commit gets no push run of its own. Dispatching with that token is allowed, so the push run starts
the candidate run itself. The workflow file must be on the default branch for the dispatch to work.

### Stale runs

The push run pushes the release commit without force. If `main` moved on while it ran, the push is
rejected, nothing is released, and the push run of the newer commit prepares instead.

### One release at a time

All runs share the concurrency group `presswerk-release`, so only one of them runs at a time. The
candidate run waits until the push run that dispatched it has finished.

GitHub keeps at most one waiting run per group and cancels an older waiting run when a newer one
arrives. If another push arrives while the candidate run waits, the candidate run is cancelled. The
newer push run then fails, because `prepare` refuses to start while a release is pending. Dispatch
the candidate again, as described under [Recovery](#recovery).

## Permissions and tokens

```yaml
permissions:
  actions: write
  contents: write
```

`contents: write` allows pushing the release commit and the tag, and creating the GitHub Release.
`actions: write` allows the dispatch. The job passes the workflow's token twice: `gh` reads it from
`GH_TOKEN`, and Presswerk reads it from `GITHUB_TOKEN`, the default secret of the
[GitHub Release publisher](/reference/publishers/github-release/#token).

Branch protection on `main` must allow the workflow to push the release commit. To require pull
requests for everyone else, push with a GitHub App as described under
[Protected main](#protected-main).

## Extensions

### Release on demand

For [manual releases](../release-modes/#manual-releases), prepare only when someone starts the
workflow by hand without a candidate, instead of on every push:

```diff lang="yaml" title=".github/workflows/ci.yml"
       - name: Prepare release
-        if: github.event_name == 'push'
+        if: github.event_name == 'workflow_dispatch' && !inputs.candidate
```

Start a release with **Run workflow** and an empty candidate, or with
`gh workflow run ci.yml --ref main`. The run prepares on the tip of `main` and dispatches the
candidate run as before. Pushes to `main` then only build and check.

### Release plans on pull requests

Run `plan` on pull requests to see what merging them would release. Pull requests get their own
concurrency group, so they never wait for release work:

```diff lang="yaml" title=".github/workflows/ci.yml"
 on:
+  pull_request:
   push:
     branches: [main]

-concurrency: presswerk-release
+concurrency:
+  group: ${{ github.event_name == 'pull_request' && github.ref || 'presswerk-release' }}

       - name: Build and check
         run: nix flake check --build-all --keep-going --print-build-logs

+      - name: Plan release
+        if: github.event_name == 'pull_request'
+        run: |
+          set -o pipefail
+          nix run --inputs-from . presswerk -- plan | tee "$RUNNER_TEMP/plan.txt"
+          { echo '```text'; cat "$RUNNER_TEMP/plan.txt"; echo '```'; } >> "$GITHUB_STEP_SUMMARY"
```

GitHub checks out the merge of the pull request into `main`, so the plan shows what would be released
after merging. The plan also appears in the run's summary. A failing plan, for example while a release
is pending, fails the check. `plan` never writes anything.

### Skip stale preparation

A push run whose commit is no longer the tip of `main` cannot release, because its push would be
rejected. To skip its preparation right away, compare the tip before `prepare`:

```diff lang="yaml" title=".github/workflows/ci.yml"
       - name: Prepare release
         if: github.event_name == 'push'
         run: |
+          git fetch origin main
+          if test "$(git rev-parse origin/main)" != "$GITHUB_SHA"; then
+            echo "main moved on; the newer run prepares"
+            exit 0
+          fi
           nix run --inputs-from . presswerk -- prepare
```

This only saves work. Without it, the rejected push keeps stale runs from releasing.

### Protected main

A ruleset that requires pull requests on `main` also blocks the workflow's own push of the release
commit. On personal repositories, GitHub Actions cannot be exempted from a ruleset, but a GitHub
App can. Create your own app for your repositories:

1. Create a GitHub App under **Settings → Developer settings → GitHub Apps**. It needs no callback
   URL and no webhook, and only the repository permission **Contents: Read and write**.
2. Install it on the repository.
3. Store its app ID as the variable `RELEASE_APP_ID` and a generated private key as the secret
   `RELEASE_APP_PRIVATE_KEY`.
4. Add the app as a bypass actor to the ruleset on `main`.

The workflow then creates a token for the app, checks out with it so that `git push` uses it, and
commits as the app's bot user. Find the bot user's ID with
`gh api '/users/YOUR-APP[bot]' --jq .id`:

```diff lang="yaml" title=".github/workflows/ci.yml"
 permissions:
   actions: write
-  contents: write
+  contents: read

 jobs:
   ci:
     runs-on: ubuntu-latest
     env:
       GH_TOKEN: ${{ github.token }}
       GITHUB_TOKEN: ${{ github.token }}
     steps:
+      - name: Create release app token
+        id: app
+        uses: actions/create-github-app-token@v2
+        with:
+          app-id: ${{ vars.RELEASE_APP_ID }}
+          private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}
       - uses: actions/checkout@v7
         with:
           fetch-depth: 0
           ref: ${{ inputs.candidate || github.sha }}
+          token: ${{ steps.app.outputs.token }}
       - uses: DeterminateSystems/determinate-nix-action@v3
       - run: |
-          git config user.name github-actions[bot]
-          git config user.email 41898282+github-actions[bot]@users.noreply.github.com
+          git config user.name YOUR-APP[bot]
+          git config user.email BOT-USER-ID+YOUR-APP[bot]@users.noreply.github.com

       - name: Prepare release
-        if: github.event_name == 'push'
+        if: github.event_name == 'push' && github.actor != 'YOUR-APP[bot]'

       - name: Publish release
         if: inputs.candidate
+        env:
+          GITHUB_TOKEN: ${{ steps.app.outputs.token }}
         run: nix run --inputs-from . presswerk -- publish HEAD
```

Presswerk now pushes, tags, and creates the GitHub Release as the app, so the workflow's own token
only needs `contents: read`; it still dispatches the candidate run. Unlike pushes made with
`GITHUB_TOKEN`, the app's push of the release commit starts a push run of its own. That run skips
`prepare`, because a release is pending.

## Recovery

The [recovery guide](../recovery/) describes what to do when a release fails. To check and publish a
candidate again, rerun its candidate run. GitHub allows reruns for 30 days; dispatching the
candidate's full SHA again works at any time:

```console
$ gh workflow run ci.yml --ref main -f candidate=RELEASE_COMMIT_SHA
```

The run checks out that commit, repeats the build and checks, and publishes only if they pass.
