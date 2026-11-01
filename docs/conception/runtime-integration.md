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
- a Nix installation that can evaluate the flake, and from which every artifact of the
  candidate can be obtained without building: already in the local store, or substitutable;
- credentials for the configured forge and publishers;
- a way to start the CLI.

A local shell is a runtime like any other. The same command must work there.

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
that gate. Presswerk can still validate the candidate identity and reject a stale or unexpected
ref.

## Secrets

The release declaration may name the secrets a publisher needs, but never contains them.
Values arrive at runtime.

The baseline is environment variables and files, which every runtime supports. Beyond that,
[secretspec](https://secretspec.dev/) is the preferred candidate for an integration. It
declares a project's secrets once, in `secretspec.toml`, and resolves them from interchangeable
providers such as the OS keyring, dotenv files, environment variables, or password managers,
with profiles per environment. That matches Presswerk's need for one release that works locally
and in CI with different credential sources.

Possible integration depths, still open:

1. document running the CLI under secretspec, with no code dependency;
1. check the secrets named in the release declaration against `secretspec.toml`;
1. resolve secrets in-process through secretspec's Rust library.

Provider-native identity, such as OIDC or GitHub App tokens, must remain possible alongside
secretspec.

## GitHub Actions

GitHub Actions is the first CI system Presswerk is exercised against, because the first
consumers use it. A release job depends on all required build and test jobs and then runs the
CLI.

Illustrative only:

```yaml
jobs:
  build:
    # Project-specific build and test matrix.

  release:
    needs: [build]
    if: github.ref == 'refs/heads/main' || startsWith(github.ref, 'refs/heads/release/')
    concurrency:
      group: presswerk-${{ github.sha }}
      cancel-in-progress: false
    permissions:
      contents: write
      id-token: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: DeterminateSystems/determinate-nix-action@v3
      - run: nix develop --command presswerk publish
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

Permissions alone do not expose a token to the CLI; every secret a publisher names must be
passed explicitly, here through `env`.

The `concurrency` group is recommended: it keeps two `publish` runs for the same commit from
running at the same time, for example a retry while the first run is still going. Presswerk
has no lock of its own (see [publication](publication.md#concurrent-runs)).

A consumer's setup still has to settle permissions, environment approvals, OpenID Connect use,
and organization setups such as GitHub App tokens and organization secrets.

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
