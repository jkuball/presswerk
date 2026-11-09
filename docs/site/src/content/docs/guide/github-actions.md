---
title: GitHub Actions
description: How the walkthrough's workflow releases from GitHub Actions, and how to extend it.
---

The [walkthrough](../walkthrough/#3-add-the-workflow) sets up a minimal workflow for
[automatic releases](../release-modes/#automatic-releases) from GitHub Actions. This page explains
how it works and how to extend it. The [CLI reference](/presswerk/reference/cli/) documents the
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
[GitHub Release publisher](/presswerk/reference/publishers/github-release/#token).

Branch protection on `main` must allow the workflow to push the release commit.

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

## Recovery

The [recovery guide](../recovery/) describes each case with Git and the Presswerk CLI. In this
workflow, its steps map to runs:

- **Checking and publishing a candidate again.** Rerun the candidate run, or dispatch the same full
  SHA again. The run checks out that commit, repeats the build and checks, and publishes only if
  they pass:

  ```console
  $ gh workflow run ci.yml --ref main -f candidate=RELEASE_COMMIT_SHA
  ```

- **Preparing again after a transient failure.** Rerun the push run if `main` still points to its
  commit. If `main` moved on, the push run of the newer commit prepares instead.
