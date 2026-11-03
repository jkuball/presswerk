# Runtime integration

The release declaration and the artifacts are pure. Releasing is not: it reads full Git
history, pushes commits and tags, calls forge and registry APIs, and needs credentials.
Presswerk therefore always runs as an impure process in some
[runtime](glossary.md#runtime), and every consumer has to do a small amount of wiring in each
runtime it releases from.

## What the core relies on

The core contains no code specific to one CI system. It relies only on what every runtime can
provide:

- a checkout of the candidate commit with enough Git history for commit analysis;
- a Nix store, usually reached through the Nix daemon, that the CLI's embedded evaluator can
  use, and from which every artifact of the candidate can be obtained without building: already
  in the local store, or substitutable;
- credentials for the configured forge and publishers;
- a way to start the CLI.

A local shell is a runtime like any other. Supporting the same release flow there is a goal,
but it is not required for the first, CI-driven milestone.

## What the consumer wires

- when a release runs: trigger refs, manual dispatch, or a local invocation;
- that it runs only after the required checks passed;
- which credentials it receives, and from where;
- concurrency and stale-run protection;
- getting the artifacts of every system to the publishing machine, usually through a binary
  cache, including private caches.

### Artifacts without building

`publish` never builds anything (see
[publication](publication.md#publish-never-builds)). The CI build jobs build every system's
artifacts, and the release job only fetches them. The usual way is a binary cache that the
build jobs push to.

Without a binary cache, the runtime's own artifact storage works too. In GitHub Actions, each
build job can export its store paths, for example with `nix copy --to file://…`, and upload
them with `actions/upload-artifact`; the release job downloads them and imports them into its
local store before running `publish`.

Presswerk cannot prove that an arbitrary workflow included every required job. The runtime owns
that gate. Presswerk needs the artifacts being released, not every unrelated output of a flake;
consumers decide which extra builds and checks must pass. Presswerk can still validate the
candidate identity and reject a stale or unexpected ref.

## Secrets

The release declaration may name the secrets a publisher needs, but never contains them.
Values arrive at runtime.

The baseline is environment variables and files, which every runtime supports. Declaration
fields such as `token.secret = "GITHUB_TOKEN"` name a logical secret; for the first milestone
the runtime resolves that name from an environment variable of the same name. The declaration
does not encode the provider as `secret.env`, so another resolver can supply the same logical
name later.

Beyond that, [secretspec](https://secretspec.dev/) is the preferred candidate for an
integration. It declares a project's secrets once, in `secretspec.toml`, and resolves them from
interchangeable providers such as the OS keyring, dotenv files, environment variables, or
password managers,
with profiles per environment. That matches Presswerk's need for one release that works locally
and in CI with different credential sources.

Possible integration depths, still open:

1. document running the CLI under secretspec, with no code dependency;
1. check the secrets named in the release declaration against `secretspec.toml`;
1. resolve secrets in-process through secretspec's Rust library.

Provider-native identity, such as OIDC or GitHub App tokens, must remain possible alongside
secretspec.

## GitHub Actions

GitHub Actions is the first CI system exercised. A push to `main` can lead to automatic
preparation of a release commit; that exact commit must pass the required build and test jobs
before it is published. A run for one commit never silently advances to a later `main` commit.
A configurable stale-run check may stop preparation instead; the later commit has its own CI
run. Presswerk's own CI builds and checks its relevant buildable `x86_64-linux` flake outputs,
including packages, checks, dev shells, and the formatter. The workflow is tested in a separate
private repository before it is enabled for Presswerk itself.

GitHub documents that pushes made with `GITHUB_TOKEN` do not trigger a new push workflow, but
[`workflow_dispatch` and `repository_dispatch` are exceptions](https://docs.github.com/en/actions/concepts/security/github_token#when-github_token-triggers-workflow-runs).
An explicit dispatch using `GITHUB_TOKEN` is therefore a candidate for starting CI on the
release commit without a separate GitHub App or personal token. The dispatch ref, exact
candidate checkout, permissions, and behavior when `main` advances must be tested. A
`workflow_dispatch` workflow must exist on the default branch.

A CI run on a release commit publishes it; it does not prepare again. How the workflow tells
the two apart, for example from `presswerk plan --json` or from the dispatch that started it,
is explored during the first milestone.

GitHub Actions [concurrency groups](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency)
can serialize runs, but default queueing retains only one pending run, replacing the previous
pending run. The optional `queue: max` permits up to 100 pending runs, and queue order is based
on when jobs or workflows began waiting. Neither setting by itself proves that every candidate
is checked or that later commits are eventually analyzed. The integration must handle those
cases without switching a run to a newer commit or deadlocking a workflow that waits on another
workflow in the same group. GitHub Actions owns that serialization for the self-release;
other CI runtimes or a developer invoking the CLI must coordinate their own release runs.
Presswerk does not introduce a separate locking feature. A failed run releases its concurrency
slot, so later runs may stop on the broken chain. After recovery, an operator may need to rerun
or dispatch the appropriate CI run; M1 does not schedule that automatically.

Permissions alone do not expose a token to the CLI: it must be passed at runtime. The workflow
should request only the permissions it needs. Branch protection, environment approvals, and
organization credential policies may require different wiring.

Presswerk may ship a reusable GitHub Action that installs and invokes the CLI. It would be a
thin convenience outside the core.

## Forgejo Actions

Forgejo Actions is close to GitHub Actions and needs the same wiring. Differences in tokens and
API URLs are handled by the Forgejo forge adapter, not by runtime-specific code.

## Known integrations

Any runtime that can start a command works without changes to Presswerk. Some runtimes can only
run a flake output instead of an arbitrary command. [NixCI
deployments](https://nix-ci.lector.ai/documentation/deploy) are one example: they run a named
package from the consumer flake, and need `in-repo = true` for full Git history and tagging.
Such runtimes are the main argument for a runnable output (see
[product shape](product-shape.md#shape-of-the-declaration)). They are documented here as
compatibility notes, not as core dependencies.
