---
title: GitHub Actions
description: Wire preparation and publication as two serialized runs around one exact release candidate.
---

This page shows one way to run Presswerk from CI: [automatic releases](../release-modes/#automatic-releases)
from GitHub Actions. The integration is runtime wiring, not code inside the CLI. The workflow must
keep ordinary branch preparation separate from exact-candidate publication. The complete working
file is in the [walkthrough](../walkthrough/#3-add-the-two-phase-workflow), and the
[CLI reference](/presswerk/reference/cli/) documents the invoked commands.

## Trigger matrix

The workflow uses the same build job for three events:

| Event | Revision checked out | Release action |
| --- | --- | --- |
| `pull_request` | pull request revision | run `plan` after checks |
| push to `main` | `github.sha` | run `prepare` only if `main` is still there |
| `workflow_dispatch` with `candidate` | exact candidate SHA | run `publish HEAD` after checks |

Every checkout uses `fetch-depth: 0`. Commit analysis needs the applicable Git history and release
tags.

## Phase 1: prepare from a fresh push

A push run first builds and checks the ordinary commit. Immediately before preparation it fetches
`origin/main` and tags, then compares the remote tip with the immutable `GITHUB_SHA` that started the
run.

```sh
git fetch origin main --tags
if test "$(git rev-parse origin/main)" != "$GITHUB_SHA"; then
  echo "main advanced; leaving release work to the newer run"
  exit 0
fi
```

If `main` moved, this run performs no release mutation. The newer push has its own run and owns the
next attempt.

```mermaid When B reaches main before A prepares, A's stale run exits and only B can gain a release commit.
gitGraph
  commit id: "previous" tag: "v0.2.2"
  commit id: "push A"
  commit id: "push B"
  commit id: "release from B"
```

After the guard, the workflow configures a bot identity and runs `prepare`. It compares `HEAD` before
and after. If the SHA did not change, there was nothing to release. If a release commit was created,
the workflow pushes that commit to `main` and records its full SHA as `candidate`.

```sh
before=$(git rev-parse HEAD)
nix run --inputs-from . presswerk -- prepare
candidate=$(git rev-parse HEAD)
if test "$candidate" = "$before"; then
  exit 0
fi

git push origin HEAD:main
gh workflow run ci.yml --ref main -f candidate="$candidate"
```

Branch protection must permit the workflow's push or use repository policy that provides an
equivalent trusted path.

## Why explicit dispatch is required

GitHub suppresses new push workflows for pushes made with the repository's `GITHUB_TOKEN`. That
prevents recursive automation, but it also means the pushed release commit does not receive an
ordinary push run. GitHub explicitly allows `workflow_dispatch` calls made with that token, so phase
1 dispatches the same workflow with the candidate SHA.

The workflow file must already exist on the default branch. `actions: write` permits the dispatch,
and `GH_TOKEN: ${{ github.token }}` exposes the token to the `gh` command.

## Phase 2: check and publish the exact candidate

The dispatched run checks out the input rather than the current branch tip:

```yaml
- uses: actions/checkout@v7
  with:
    fetch-depth: 0
    ref: ${{ inputs.candidate || github.sha }}
```

It repeats the project's build and checks against the release commit, including any version-dependent
outputs. Only after those steps pass does it assert the checkout identity and publish:

```sh
test "$(git rev-parse HEAD)" = "${{ inputs.candidate }}"
nix run --inputs-from . presswerk -- publish HEAD
```

```mermaid A dispatched candidate remains the publication target after a later ordinary commit reaches main.
gitGraph
  commit id: "previous" tag: "v0.2.2"
  commit id: "change"
  commit id: "candidate 0.3.0" tag: "v0.3.0"
  commit id: "later main"
```

The tag in the diagram appears only when the candidate run publishes. The run never substitutes
`later main` for its input SHA.

## Serialize release work

Preparation and publication share one concurrency group:

```yaml
concurrency:
  group: ${{ github.event_name == 'pull_request' && github.ref || 'presswerk-release' }}
  cancel-in-progress: false
```

Pull requests use their own ref as the group and do not block release work. Push and dispatched runs
use `presswerk-release`, so only one release-related run executes at a time. The dispatched candidate
waits until the push run that created it has finished, then acquires the same slot.

```mermaid Serialized runs let preparation finish before the exact candidate run publishes its release commit.
gitGraph
  commit id: "ordinary push"
  commit id: "prepared candidate" tag: "published after CI"
```

GitHub concurrency retains at most one pending run for a group and may replace an older pending run
with a newer one. The stale-main guard makes superseded push work harmless, but it does not make the
queue a release scheduler. After a failed run or recovery, an operator may need to rerun or dispatch
the appropriate revision. Never make an old run follow `main` to compensate.

## Release on demand

For [manual releases](../release-modes/#manual-releases) from CI, prepare only when someone starts the
workflow by hand without a candidate, instead of on every push. Change the condition of the
**Prepare release** step:

```yaml
      - name: Prepare release
        if: github.event_name == 'workflow_dispatch' && inputs.candidate == ''
```

Start a release with **Run workflow** and an empty candidate, or with
`gh workflow run ci.yml --ref main`. The run checks out the branch tip, prepares, pushes the release
commit, and dispatches the candidate run as before. Pushes to `main` then only build and check.

## Permissions and secrets

The minimal workflow in this guide requests:

```yaml
permissions:
  actions: write
  contents: write
```

`contents: write` permits the release commit push, annotated tag push, and GitHub Release writes.
`actions: write` permits the explicit dispatch. Repository or organization policy may reduce those
permissions; the workflow then needs an approved credential or policy change rather than broader
Presswerk configuration.

Pass the runtime token explicitly:

- `GH_TOKEN` is consumed by the `gh workflow run` command;
- `GITHUB_TOKEN` is consumed by Presswerk because that is the default logical secret name in the
  GitHub Release publisher declaration.

The declaration stores only the name `GITHUB_TOKEN`, never its value.

## Pull request plans

Run `plan` after pull request checks and place its output in the job log or step summary. It evaluates
the committed pull request revision without writing. A plan can still fail, for example if the
release chain is pending or the flake lock is inconsistent. Treat that as release-readiness feedback,
not as permission to publish from the pull request.

## Operational checks

For every release, retain evidence of:

1. the ordinary push SHA and its successful preparation step;
2. the release commit SHA pushed by that step;
3. the dispatched run input and checkout at that exact SHA;
4. successful candidate build and checks;
5. the annotated tag resolving to the same release commit;
6. the published GitHub Release for that tag.

The [recovery guide](../recovery/) starts from those identities when a phase fails.

## Recovery

The recovery guide describes each case with Git and the Presswerk CLI. In this workflow, its steps
map to runs:

- **Checking and publishing a candidate again.** Rerun the candidate's dispatched run, or dispatch the
  same full SHA again. The run checks out that commit, repeats the build and checks, and publishes
  only if they pass:

  ```console
  $ gh workflow run ci.yml --ref main -f candidate=RELEASE_COMMIT_SHA
  ```

- **Preparing again after a transient failure.** Rerun the push run if `main` still points to its
  commit. If the log says `main advanced`, the newer push run owns preparation; do not revive the
  stale one.

`gh run list --workflow=ci.yml` lists the runs and their events, and `gh release view v0.3.0` shows
the GitHub Release state for the evidence.
