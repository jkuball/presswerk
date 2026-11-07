# Publication

## Publishers

OCI, PyPI, npm, crates.io, Git tags, and forge releases have different identity, mutability,
and recovery rules. The core should not reduce them to a generic "upload file" operation.

Every destination type must work against both public and private instances, such as a private
OCI registry, an alternative Cargo registry, or a self-hosted Forgejo.

Publishers take their inputs directly as Nix values, each with its own typed options: files for
a forge release, one image per system for OCI, a packaged crate for crates.io. There is no
central artifact list. The CLI finds every store path a release needs by walking the evaluated
declaration, since each path carries its origin as string context. Sharing an artifact between
publishers is done with ordinary Nix: `let` bindings, or `config` in the flake-parts module.

### Publish never builds

`publish` realises artifacts only by fetching them: from the local store or from a
substituter. It never builds. If the recipe is substitutable, it is publishable: Presswerk
publishes what the local store and the configured substituters hold for each store path. It
relies on Nix's model of reproducible builds and does not claim a byte-level proof that these
are the bytes CI tested. The preflight before the tag checks that every artifact can be
obtained, and stops before any write if one cannot:

```console
error: presswerk-aarch64-darwin (/nix/store/…-presswerk-static) is not in any substituter
       did the aarch64-darwin build job push it?
```

How the artifacts get there is runtime wiring (see
[runtime integration](runtime-integration.md#artifacts-without-building)).

A later, optional feature can strengthen this with a trusted-signer requirement: every artifact
must carry a Nix signature from one of the keys listed in the declaration. A signature shows
that a key endorsed those store contents; that the signer built or tested them is only true if
the CI process guarantees it. It is not part of the core design.

### Publisher safety requirements

Every publisher must:

1. validate configuration and runtime credentials;
1. inspect the remote objects it is about to write and distinguish missing, matching,
   conflicting, and unknown [remote state](glossary.md#remote-state);
1. publish only missing state where the destination permits it;
1. verify the final result;
1. tolerate an exact-candidate retry, including one running at the same time: when a write
   fails because the object already exists, inspect again instead of failing;
1. avoid leaking credentials in output;
1. respect publication ordering.

Idempotency is the core guarantee. A failure at any point is repaired by rerunning the same
commit and version, so correctness does not depend on running every read-only check before the
first external write.

### When remote state matches

Whether an existing remote artifact is *matching* is decided per publisher, in this order:

1. **The content digest**, if the destination provides one, such as the file checksums of
   crates.io, PyPI, and npm, the `digest` of a GitHub release asset, or the manifest digest of
   an OCI image (the index digest for a multi-platform image). It is compared with the digest
   of exactly the representation the publisher uploads, which means fetching the local bytes.
   For OCI, a change in the pushing tool's compression can produce a different digest for the
   same content; that is a false conflict, which fails safe.
1. **Otherwise the digest of the downloaded remote artifact**, computed by Presswerk.
1. **Otherwise the state is *unknown***, and the run stops. Presswerk never assumes a match. A
   digest mismatch is *conflicting* and never falls back to weaker evidence.

The local side of the comparison should be cheap too. Where the build already records the
digests of what will be uploaded, such as the layer digests nix2container writes into its image
description, publishers read them from the local store path instead of fetching and hashing the
artifact. Only artifacts without such metadata are hashed. A retry of a release with many OCI
images then costs one manifest request and one metadata read per image. This only matters on
retries and republishes, not on the normal path.

The Nix store path is not evidence of a match. Publishers may still record it next to an
artifact, such as in an OCI annotation, for diagnostics. A store path identifies how an
artifact is built, not its bytes: two builders can produce different bytes under the same path,
and a remote label is only a claim. Each publisher documents which identity it uses.

Presentation metadata, such as a release's body and display title, is deliberately not
compared; a body edited by hand after publication is left alone. Metadata that changes behavior
is checked: the release's tag must point to the candidate, a draft release is not complete and
is continued rather than created again (for example after a crash), and the pre-release flag
must match the version. Which release is marked as latest belongs to the alias policy (see
[open decisions](open-decisions.md#extensions-and-publication)).

### Preflight

The intended full design runs a read-only preflight before its first write, the tag. The
asset-free first milestone does not need all future artifact and publisher checks. It still
must validate the candidate and relevant remote state, avoid silent overwrites, and permit a
safe retry after a partial publication.

The full preflight checks:

1. the candidate's classification, and for a new release, the policy validation of its
   [release commit](glossary.md#release-commit) and completion of its previous release (see
   [release flow](release-model.md#release-flow));
1. that every publisher's credentials are present and valid, using the strongest
   non-mutating check each destination offers. Not every destination can prove write
   permission without writing; publishers report what they could not verify;
1. the remote state of every pressing: *conflicting* or *unknown* stops the run;
1. that every artifact can be obtained without building.

Since the tag uses up the version, this keeps the common mistakes, such as a missing secret, a
binary missing from the cache, or a version already taken on a registry, from turning into a
partial release: the version stays free, and the fix is a rerun. What the preflight cannot
catch, such as a registry rejecting content or a network failure during a write, is what
idempotency and retries are for. Only release commits get a preflight, so the fast path for
ordinary commits is unaffected.

`presswerk check` runs the preflight alone, without publishing. It is to `publish` what `plan`
is to `prepare`. For example, `check` after a local `prepare` shows whether `publish` would
succeed before anything is pushed, as far as local credentials match those of CI.

### Unsupported predecessor boundary

Predecessor declarations are normally decoded strictly with the current release toolchain. Their
own version, tag format, and publisher destinations remain authoritative. Presswerk does not
project selected fields from an unsupported declaration or silently substitute the current
publishers.

Before 1.0, an intentional schema break may require a one-time crossing:

1. use the predecessor's original locked Presswerk toolchain to run `publish <tag>` again. This
   revalidates the tagged declaration, remote tag, and every old publisher without changing an
   already complete release;
1. record the annotated tag's peeled commit SHA and confirm the local and remote tag agree;
1. update the release toolchain, then run `prepare` normally. It must stop with the dedicated
   unsupported-predecessor error naming that exact tag and SHA;
1. rerun `prepare --trust-predecessor <tag>=<sha>`. The exception is accepted only when the local
   tag, remote tag, SHA, and ancestry all match that exact unsupported predecessor;
1. review and push the generated release commit. It records
   `Presswerk-Trusted-Predecessor: <tag> <sha>` as its final commit trailer. `publish` takes no
   trust flag; it consumes the immutable trailer and revalidates the same facts before writing.

The flag and trailer are rejected for a supported predecessor, the first release, a different
SHA or tag, a missing or moved remote tag, or a non-ancestor. They bypass only decoding and
rechecking the old publishers, which step 1 performed with the old tool. After 1.0, schema
evolution is expected to preserve backward compatibility. This mechanism remains an emergency
fallback, not the normal upgrade procedure.

### Extension shape

The extension shape remains open. Plausible options are:

- analyzer and publisher executables communicating through context JSON;
- a small lifecycle inspired by Semantic Release;
- built-in Rust adapters plus a generic executable escape hatch;
- a Rust library API for in-tree adapters and a process protocol for external adapters.

The first implementation defines extensible Rust trait boundaries for publishers and
version-file generators, with implementations kept in the main crate. It does not create separate
trait crates or stabilize an external plugin SDK. The publisher boundary supports a test double;
a materially different real publisher can inform the public extension design later.

Consumers must be able to add their own publishers without upstreaming them, since some
destinations will be internal to an organization.

The [analyzer](glossary.md#analyzer) for commit analysis and release notes faces the same
extension question and should use the same mechanism where practical.

## Forges

Git operations form the common core. GitHub and Forgejo behavior lives in adapters.

Common concepts include tags, releases, pull requests, and commit status links.
Authentication, API URLs, protected branches, and latest-release behavior differ. The core must
not assume a GitHub token or GitHub App when running against Forgejo. It must also not assume
`github.com` when running against GitHub, because GitHub Enterprise Server uses its own host
and API URL. The project has no Enterprise Server instance to test against, so that path
depends on consumer feedback.

Tag creation and forge release creation remain separately verifiable. A forge release must not
hide a Git tag pointing at the wrong commit.

## Security

- Secrets enter only at runtime through environment variables, files, secret managers, or
  provider-native identity (see [runtime integration](runtime-integration.md#secrets)).
- Secrets and generated authentication files never enter the Nix store.
- Authentication and ambiguous network failures fail closed.
- CI branch filters are not automatically secret access-control boundaries.
- Release jobs run only for trusted revisions and protected environments.
- Presswerk contacts only services the consumer configured. Telemetry is opt-in and exported
  only to endpoints the consumer chose (see [observability](observability.md)).

## Recovery

- A completed version belongs to one exact commit.
- Matching external state is retained.
- Missing state may be completed.
- Conflicting state is never overwritten silently.
- Retries use the same commit, version, and frozen notes.
- Publication never follows a branch to a newer commit implicitly.

### The tag as reservation and lock

The Git tag is pushed before any other pressing. Pushing a tag without force is atomic, so the
tag doubles as a version reservation and a concurrency lock:

- if the tag is missing, this run creates it and owns the release;
- if the tag exists on the candidate commit, the remote state is *matching* and the run
  continues completing missing state;
- if the tag exists on another commit, the state is *conflicting* and the run stops.

Once the tag exists, the version is used up by default. A partially published version is
completed if possible.

### Freeing a version

Once anything other than the tag has been published, a version is used up for good. A partial
release is fixed forward: the fix goes out as the next version (see
[scenario 9](scenarios.md#9-bug-in-the-master-record)). Cleaning up what was published, such as
yanking a PyPI release, is the consumer's choice.

`presswerk revert --impure-cleanup` is a narrow, explicitly unsafe administrative escape hatch
for the case where only the tag was written. It reverts the release commit and deletes the tag,
so the next `prepare` can assign the same version again. It:

- refuses if any destination holds a pressing of that version;
- refuses if the state of any destination is *unknown*;
- requires the operator to prevent any `publish` for that version from running or starting
  throughout inspection and cleanup, including queued retries, since finding no pressings does
  not prove that a publication will not write one a moment later;
- deletes the tag only if it is still exactly the object that was inspected;
- warns that clones and mirrors that already fetched the tag keep it.

The preflight makes this case rare: most failures that would leave only a tag are caught
before the tag is written.

### Partial releases

Publication can fail halfway in a way a retry cannot fix. What happens next depends on where
the bug is:

- **In the press**, such as a Presswerk publisher bug, wrong credentials, or a registry
  outage: update Presswerk or fix the wiring, then republish the tag (see
  [scenario 8](scenarios.md#8-bug-in-the-press)).
- **In the master record**, such as broken artifact metadata or a broken release declaration:
  the version cannot be completed. It stays a partial release, and the fix is released as the
  next version (see [scenario 9](scenarios.md#9-bug-in-the-master-record)).

Cleaning up a partial release is the consumer's responsibility. Some destinations, such as
PyPI, do not allow it at all.

The forge release is published last, and it is created as a draft first: its assets are
uploaded and verified, and only then is the draft published. GitHub and Forgejo both support
this. So Presswerk never initially publishes a forge release before its assets are uploaded
and verified; an asset deleted later is detected and restored by a republish where possible.
This guarantee is per destination: with several forges, the first can be published while the second fails. Other
pressings are visible as soon as they are written: an OCI image pushed before a later failure
can already be picked up by tools such as Renovate.

### Concurrent runs

The tag only locks out other commits. Two `publish` runs on the same release commit, such as a
CI retry while the first run is still going, both see the tag as theirs and continue.

Presswerk has no lock of its own: a lock left behind by a crashed run would block exactly the
retry that idempotency is meant to make easy. Instead:

- **Runtimes are recommended to serialize runs per commit**, such as a GitHub Actions
  `concurrency` group (see [runtime integration](runtime-integration.md#github-actions)).
- **Publishers tolerate the race where the destination allows it.** If a write fails because
  the object already exists, the publisher inspects again: *matching* is success,
  *conflicting* stops the run. Most destinations allow this, such as registries with immutable
  versions, forge releases that are unique per tag, and OCI content addressed by digest.
- **Publishers document where they cannot.** Destinations that are only read, modified, and
  written back, such as a Helm repository's `index.yaml` on plain HTTP, can lose an update
  under concurrent runs. Their publishers say so.

### Mutable aliases

Some destinations have pointers that move, unlike versions: GitHub's latest release, npm
dist-tags such as `latest` and `next`, and OCI tags such as `latest`, `1`, or `1.4`. They are
the only state Presswerk overwrites on purpose.

An alias is defined by the version order, not by the order of runs: it points to the highest
version in its scope.

| Alias | Scope |
| --- | --- |
| `latest` | highest stable version |
| `next` | highest pre-release |
| `1.4` | highest `1.4.x` |

On every attempt (new release, retry, or republish), the publisher reads the alias and moves it
only if this version is higher than its current target. So:

- republishing `1.3.0` after `1.5.0` leaves `latest` alone;
- a retry after a crash before the alias step still sets it;
- a slow `1.4.0` run finishing after `1.5.0` leaves `latest` alone;
- a maintenance release `1.2.1` after `2.0.0` leaves `latest` alone.

Where the destination decides itself, such as GitHub picking the latest release by SemVer,
Presswerk leaves the decision to it. Aliases are written last and are best-effort: a failed
alias update is a warning, not a failed release, and the next attempt repairs it. Between two
runs reading the alias at the same moment, the lower version can win; the next run on the
higher version corrects it. Which aliases a publisher supports depends on what its destination
can do.

### Republishing from a tag

Because a tag points at a master record, any existing release can be published again from its
tag, for example to restore a lost artifact. This is a central feature of Presswerk, not only a
recovery path. A republish only targets the destinations declared in the release commit;
destinations added to the flake later do not apply to older versions.

Republishing works as long as the release's artifacts can be obtained. Presswerk is not a CI
system and does not rebuild: if a path was garbage-collected from every cache, the consumer
refills the cache, for example by building that commit and pushing the result, and then
republishes. Restoring the original bytes from an archive and rebuilding from the same recipe
are different operations: a rebuild recovers the original bytes only if the build is
reproducible. Otherwise it conflicts with surviving pressings, and where both the original
bytes and their digest are gone, byte-identical restoration cannot be established at all.
Pinning release paths in caches that support it, such as Cachix, is a planned feature. The same
idempotency rules apply: matching state is kept, missing state is completed, conflicting state
is reported.
