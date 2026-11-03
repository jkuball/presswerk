# Release model

## Candidate and release context

A release run works on exactly one [candidate](glossary.md#candidate): a
[master record](glossary.md#master-record) that is about to be released, identified by
repository, commit, and version.

Everything Presswerk derives about that candidate is collected in one immutable
[release context](glossary.md#release-context). Conceptually it contains:

```json
{
  "repository": "forge.example/owner/project",
  "commit": "full commit SHA",
  "ref": "refs/heads/release/v1.2.x",
  "version": "1.2.3",
  "previousRelease": {
    "version": "1.2.2",
    "commit": "previous release SHA",
    "tag": "v1.2.2"
  },
  "commits": [],
  "notes": "frozen Markdown",
  "artifacts": []
}
```

The schema remains open. It must:

- identify one exact candidate;
- contain no credentials;
- retain frozen notes for retries;
- carry enough artifact identity for publishers to detect conflicts.

It should not force every registry into one false universal artifact model.

## Versions

The version is the first thing a release commits. This is a hard requirement. The effective
version must be known and committed before final builds of version-dependent derivations, and
publication never calculates a different version afterward.

Commit analysis may recommend the next version. Preparation can accept that recommendation,
allow an explicit choice, or verify a manually selected version against the analyzed changes.

Presswerk does not require a `VERSION` file. Where the version lives is the consumer's choice,
for example `Cargo.toml`, `flake.nix`, or a plugin manifest. The consumer configures which
files hold the version and how `prepare` replaces it in each of them. The exact shape of that
configuration is open. It must produce one effective version and explicit consistency checks.

### Versions below 1.0

Presswerk follows [SemVer](https://semver.org/) strictly. Item 4 of the specification says
major version zero is for initial development and anything may change at any time; item 5 says
`1.0.0` defines the public API. When that point is reached is a human decision, so Presswerk
never bumps to `1.0.0` on its own. It only happens when the developer asks for it explicitly,
possibly with `prepare --major` on a `0.y.z` version; the command interface is not settled.

SemVer does not prescribe how to bump within `0.x`. Presswerk maps the analyzer's bump levels
as follows:

| Analyzer recommendation | `0.4.2` becomes |
| --- | --- |
| patch | `0.4.3` |
| minor | `0.5.0` |
| major or breaking | `0.5.0` |

For Gitmoji, this uses the catalog's SemVer levels: a bug fix is a patch, a feature is a minor
change, and a breaking change also increases the minor component until `1.0.0` is explicitly
chosen.

Without an override, analysis decides the bump, except that it never moves to `1.0.0` on its
own. Explicit overrides are a later interface question; they must not select a lower bump than
the analysis requires.

## Release flow

Committing the version before building means a release has two steps:

1. `prepare` creates a commit with the new version and frozen notes.
1. That commit goes through CI, and only then does `publish` release it.

This is how Presswerk gets a pure release, where everything published is derived from one
commit. It costs one extra commit per release. In the first milestone CI performs preparation
automatically; no separate local preparation command is required.

`publish` decides from the commit alone whether it is a release. Being a
[release commit](glossary.md#release-commit) is a structural property: the commit changes the
version compared to its parent. It does not depend on publication state. What `publish` does
with a commit depends on whether its version changed and where the tag for its version is:

| Version vs parent | Tag for the version | Classification | `publish` does |
| --- | --- | --- | --- |
| either | on a strict ancestor | ordinary commit | nothing |
| changed | missing | new release | validates, then publishes |
| unchanged | missing | inherits a [pending release](#pending-releases), except for the initial baseline | fails and reports it |
| changed | on this commit | retry or republish | completes missing state |
| unchanged | on this commit | invalid candidate | stops |
| either | on a commit that is not an ancestor | conflict | stops |

"Parent" means the first parent for merge commits. An annotated tag is compared by the commit it
points to.

A tag on an ancestor covers ordinary development after a release, and the cases where the
version returns to one that is already released: a revert of a failed release commit, and
adopting a project whose current version was released before Presswerk. It classifies the commit
but does not prove the previous release completed. A commit that inherits an unpublished version
is a fixup after a failed release commit; under the strict rule below it is reported, never
published.

Classification comes first. Two kinds of checks follow:

- **Policy validation** applies only to a new release: the validity checks below.
- **Checks on every attempt** apply to new releases, retries, and republishes alike: the
  candidate's classification, credentials, artifact availability, and remote state.

A retry or republish is therefore never rejected by a newer CLI that would analyze the old
commits differently.

`publish` needs no state from `prepare`, so it can run on every push, any number of times. See
[scenarios](scenarios.md) for how this plays out.

A new release is only published if its release commit is also valid:

1. every changelog has a section for that version. A changelog without matching commits still
   gets one, which by default says "No recorded changes."; the template decides the wording;
1. the version is a valid successor of the version committed in the parent: the next patch,
   minor, or major version, or a valid pre-release step;
1. the version is at least what commit analysis of the range requires. `publish` runs the
   analysis again, which is deterministic from the commits and the pinned configuration. A
   larger bump than required is allowed.

If any check fails, `publish` fails loudly instead of skipping the commit, since a broken
release commit is on the branch. A revert is one possible recovery; M1 does not perform
automatic recovery. Who made the release commit does not matter in this proposed model:
`prepare`, a forge's squash merge, or a developer by hand all work if the commit is valid
(see [scenario 12](scenarios.md#12-hand-made-release-commit)).

The first milestone runs preparation automatically for changes on `main`, then runs CI on the
release commit and publishes only after that candidate passes. Each run is pinned to the commit
it was started for: it must not follow `main` to a newer commit. If `B` reaches `main` before
`A` can push its release commit, `A` fails cleanly and `B` is handled by its own CI run. If
`A` has already pushed its release commit, `A` completes that release and `B`'s own CI run can
release `B` separately afterward if its changes warrant it. The CI runtime serializes releases;
Presswerk does not add its own lock.
A configurable stale-run check could stop preparation when `main` has already advanced. Local
preparation and its push behavior remain goals, but are not needed for the first milestone.

### Preparation branches (later)

A second flow is planned, but deliberately not designed to the end yet: `prepare` puts the
release commit on a separate preparation branch, CI builds it and runs a dry-run `publish`,
and the developer merges it into main once everything is green. It gives notes a place to be
reviewed and works with protected branches. What is known so far:

- **Recognizing the release on main.** After a squash merge, the forge creates the commit on
  main, not Presswerk. The rule still holds, because it is structural: whichever commit changes
  the version compared to its parent is the release commit.
- **No supported workaround until then.** Pushing a release commit to a branch and merging it
  by hand produces a structurally valid release commit, but without the freshness check below
  its notes can silently miss changes that reached main in the meantime. It is outside the
  supported workflow of the first release.
- **Main moving in the meantime.** If main moved after `prepare`, the merged commit contains
  code the notes do not describe. The preparation branch's dry run must fail in that case. One
  way is to record the base the notes were rendered from in the release commit, and to require
  that it equals the release commit's first parent on main. That would also make the merge
  strategy irrelevant: squash, merge commit, and rebase all leave main's previous tip as the
  first parent.
- **Human commits on the preparation branch.** Either `prepare` can incorporate them again
  (something like `prepare --reuse`), or the branch is reserved for Presswerk's own commits,
  as Renovate does with its branches. Open.
- **Triggering CI.** Pushes made with the default `GITHUB_TOKEN` do not start a push workflow.
  An explicit `workflow_dispatch` or `repository_dispatch` is a documented exception and may
  allow automated candidate CI without a separate GitHub App or personal token. The exact
  wiring and permissions need testing.

The rule is strict: a version is released from exactly its release commit and nothing else. A
later commit that still carries an untagged version, such as a fixup or an unrelated merge, is
not published directly. A failed, untagged release commit can be reverted before a new candidate
is prepared.

Presswerk deliberately uses the version-changing commit as its release boundary: later changes
require a new `prepare`, so that the boundary and the notes are always reconsidered together.
Publishing a later commit would not in itself break purity; the rule exists for correctness of
what is released and described. The structural rule alone does not prove that the notes are
complete, since a hand-written release commit can have incomplete notes, so the validity checks
define what is actually guaranteed.

The release commit has to pass CI before it is published, and it can fail there, since the
version may be used by derivations and checks. The documented recovery is to revert the failed,
untagged release commit, fix the problem, and prepare again (see
[scenario 7](scenarios.md#7-release-commit-fails-ci)). Recovery is manual in the first
milestone. Whether automatic recovery should ever be added is undecided.

## Previous release

The previous release provides the base version and the tag that starts the commit range for
analysis and notes. A new release must not proceed while that previous release is incomplete.
Preparation and publication of a new candidate both need to enforce this release-chain
invariant. `prepare` therefore needs read access to the forge too, for example a GitHub token
in CI, not only `publish`. A retry of an existing candidate completes that candidate rather than selecting a
new one.

For release selection, Presswerk identifies the latest applicable release tag and analyzes the
whole Git range from that tag to the input commit, for example `v1.3.0..HEAD`. It never decides
that there is nothing to release merely because the latest commit does not bump the version.
The previous release must be complete before another release may proceed.

In the ordinary post-release state, the committed version identifies the baseline. The intended
lookup is:

1. evaluate the committed version, for example `1.3.0`, at the input commit when preparing, or
   at the parent when validating a new release commit;
1. find the tag rendered from `tag.format`, for example `v1.3.0`, and check that it is an
   ancestor of that commit. Outside the initial `0.0.0` baseline, a missing tag can mean the
   release is [pending](#pending-releases); a tag pointing elsewhere is a conflict;
1. verify that the previous release is fully published. For the first milestone, its GitHub
   Release must exist and be published, not missing or a draft. A network or authentication
   error must not be mistaken for a complete release;
1. analyze the applicable commit range, such as `v1.3.0..HEAD`, using Git history. For the first
   release there is no physical tag, so the range includes the root commit.

An exact-candidate retry completes the frozen candidate; it does not select a new version or
regenerate its notes.

Presswerk does not implement its own commit walking, and it never reads more history than the
task at hand needs. It does the minimum work required for exactly what it is doing. Checking
that a tag exists, or that it is an ancestor of `HEAD`, is a lookup, not a scan. Maintenance lines need no special handling: on `release/v1.2.x` the committed version is `1.2.0`, so the base is `v1.2.0`
regardless of newer tags on main.

For a stable release after pre-releases, the base is the last stable release instead: notes
for `1.4.0` cover everything since `v1.3.0`, not only since `v1.4.0-rc.2`, and a breaking
change anywhere in that range still leads to a major bump.

A pre-release version does not say which stable version its series started from: `1.4.0-rc.2`
could build on `1.3.0` or `1.3.7`. So when the committed version is a pre-release, Presswerk
finds the stable base from the tags, without walking history and without recording anything
extra:

1. list the release tags, parsed through `tag.format`;
1. keep only stable versions whose tags are ancestors of `HEAD`. Each check is a lookup Git
   answers itself. This drops releases from other lines, such as `v1.3.1` on
   `release/v1.3.x`;
1. of those, take the highest version below the committed one. This drops newer releases such
   as `v2.0.0`.

This is the same mechanism as the consistency check below. It picks by version, not by commit
distance, so it does not depend on the shape of the history.

Looking at other tags is only a consistency check: if a tag reachable from `HEAD` parses to a
higher version than the committed one, `plan` stops. This lists tag names and checks ancestry;
it does not walk history.

### Pending releases

After preparation has created a release commit, its version can be untagged while CI is running
or after it fails. That release is *pending*. The untagged initial `0.0.0` baseline is not a
pending release. To name a pending release commit, Presswerk can follow first parents back until
the version changes. Presswerk itself cannot tell whether CI is still running or has failed
without asking the CI system, so the diagnostic says *pending*, not *failed*.

A release attempt on a later commit that inherits the untagged version reports the pending
release and exits with failure. A revert which restores the previous tagged version clears this
condition even though the reverted release commit remains in history. The previous design's
successful exit is superseded.

A tag alone does not prove that publication completed. If the tag exists but its GitHub Release
is missing or still a draft, the release chain is also broken and a new release must not proceed.
Retrying the exact candidate can complete a partial publication. A revert or force-push of a
branch does not remove an existing tag.

For automatic releases, a later commit is considered by its own CI run, not by advancing an
earlier run to a newer commit. The runtime queues later work while a release is in progress.
After a failed release is fixed, an operator may need to rerun or dispatch the appropriate
later CI run. M1 has no autoheal or recovery scheduler.

```console
$ presswerk prepare --pre rc --push
pending release  1.4.0-rc.2 at 6e7f8a9 is not published yet
Nothing to prepare until it is published or reverted.
```

Detecting the state belongs in the release logic. The CI integration and operator coordinate
retries without changing a run's candidate.

### The first release

A fresh, never-released repository has an implicit `0.0.0` baseline. It needs no `v0.0.0`
tag, manual boundary commit, or special bootstrap command. Normal automated preparation
analyzes the history including the root commit, selects a version when a commit warrants a
release, and creates the first release commit with its version and frozen notes. The same CI
and publication flow then applies as for later releases.

There is no release until the configured commit convention finds a release-worthy change. For
example, a first Gitmoji feature commit bumps the implicit `0.0.0` baseline to `0.1.0`.

For the first milestone, the fresh baseline is recognized by a simple rule: the committed
version is `0.0.0` and no release tag exists. Such a commit is neither a pending release nor a
previous release whose completion must be checked. A release commit whose parent is at this
baseline skips the previous-release check. Adopting a repository with releases made by other
tools remains open. See [scenario 13](scenarios.md#13-first-automatic-release).

### Adoption

Fresh repositories require no special adoption flow. For an already released or versioned
repository, existing tags, committed versions, and possibly incomplete releases need to be
reconciled without mistaking them for a fresh `0.0.0` baseline. The exact workflow is not part
of the first self-release milestone.

## Branches

Releases are made from the main branch. Maintenance releases for older major or minor versions are made from
[release lines](glossary.md#release-line) such as `release/v1.2.x`, branched off the release
tag they maintain. A maintenance release lives only on its release line and is not merged back
to main.

A release line can only be started where the contract holds: the tag must exist, and the
version committed at the tagged commit must match it. The release declaration must exist on
the line too. For releases made before Presswerk was adopted, the first commit on the line has
to add the declaration. Arbitrarily old versions are therefore only maintainable if their tags
and version files are clean. Pre-releases are made on main as well (see [pre-releases](#pre-releases)).

## Pre-releases

Pre-releases such as `1.4.0-rc.1` are ordinary versions in a line on main. They get release
commits, tags, and publication like any other version; revert, republishing, and the tag as
lock work unchanged.

Whether `prepare` produces a pre-release is decided by how it is called, not by the branch or
the analyzer. The analyzer decides which number is bumped; the call decides whether the result
is a pre-release:

| Call | Analyzer bump | Result |
| --- | --- | --- |
| `prepare --pre rc` | minor | `1.3.0` → `1.4.0-rc.1` |
| `prepare --pre rc` | patch | `1.4.0-rc.1` → `1.4.0-rc.2` |
| `prepare --pre rc` | major | `1.4.0-rc.2` → `2.0.0-rc.1` |
| `prepare` | up to minor since `v1.3.0` | `1.4.0-rc.2` → `1.4.0` |
| `prepare` | major since `v1.3.0` | `1.4.0-rc.2` → `2.0.0` |

During a pre-release series, the analysis always covers the whole range since the last stable
release. The resulting version is the higher of the series' current target (`1.4.0`) and what
the analysis requires from the stable base (`1.3.0`). A breaking change that lands after
`rc.2` therefore leads to `2.0.0` or `2.0.0-rc.1`, never to `1.4.0`.

This allows a fully automated flow: CI runs `prepare --pre rc` on every push to main, and a
human promotes to a stable release by running `prepare` without `--pre` (see
[scenario 11](scenarios.md#11-continuous-release-candidates)).

Pre-releases on separate branches would leave main's committed version behind, so the next
release on main would propose the same version again. Release branches therefore stay
reserved for maintenance. The cost: whatever is merged during the release-candidate phase ends
up in the next candidate and the final release. A freeze is the team's own choice.

How each destination marks pre-releases differs (npm dist-tags, the GitHub pre-release flag,
no `latest` OCI tag, PEP 440 spelling such as `1.4.0rc1` on PyPI). That is publisher behavior
and does not change the model.

## Commit analysis

Commit analysis and release-note rendering are mandatory: every release gets a version
recommendation and release-note data from the commits since the previous release.

They sit behind an [analyzer](glossary.md#analyzer) adapter, so the implementation can be
replaced. The first analyzer uses [git-cliff](https://git-cliff.org/) instead of implementing
commit parsing and SemVer bump calculation again. Rendering is not the analyzer's job: the
analyzer returns structured data, and Presswerk renders it (see
[rendering notes](#rendering-notes)).

The analyzer is configured in two parts (see
[declaration scenarios](declaration-scenarios.md#analyzer-convention-and-backend)):

- the [convention](glossary.md#convention) the project's commits follow, independent of any
  tool;
- the backend that analyzes them, git-cliff by default, with its own options.

This keeps Presswerk from being married to git-cliff. Useful conventions are:

- Conventional Commits;
- Gitmoji followed by Conventional Commits, such as `✨ feat(ui): add dark mode`;
- pure Gitmoji, where the emoji determines category and bump behavior.

For the self-release, the [Gitmoji specification](https://gitmoji.dev/specification) and its
[catalog](https://github.com/carloscuesta/gitmoji/blob/master/packages/gitmojis/src/gitmojis.json)
are the reference. The catalog contains SemVer levels for some Gitmojis and `null` for others;
the specification accepts Unicode and shortcode spellings. For Presswerk's self-release, notes
include commits with a Gitmoji even when that emoji does not trigger a version bump. Notes group
commits by emoji, with Unicode and shortcode spellings in the same group. Groups with more
commits come first; equal-sized groups are ordered by Gitmoji shortcode. Each entry includes at
least the commit subject; whether the default also includes the body is an implementation
choice. Commits without a Gitmoji are not included in those notes.

Release commits use `🔖`. A range contains at most one release commit, its previous release's,
on its boundary; it is never grouped as an ordinary `🔖` entry. A release commit cannot list
itself either: its notes are rendered before it exists. How the template refers to the
previous release, if at all, is up to the template.

Analysis uses the commits actually reachable in Git history, not forge pull-request metadata.
After a squash merge the squash commit is analyzed, not the original commits that were not
retained in the target branch's history.

Advanced users should eventually be able to provide ordinary git-cliff parser configuration
through the backend's options.

The analyzer interface is open. It must at least accept a commit range and return a bump
recommendation plus structured commit data, without depending on git-cliff's types.

## Changelogs

Release notes are committed, like the version. The repository keeps `CHANGELOG.md` files with
one section per version, newest on top, in the commonly understood Keep a Changelog style.
`prepare` renders the notes for the new version with the analyzer, writes them as a new section,
and commits them in the release commit.

The flake declares where the changelogs live, just as it declares the version files.
`publish` reads the section for the candidate's version back from the evaluated commit.
Republishing from a tag therefore reads the notes exactly as they were committed.

A release can have several changelogs. A product with several OCI images and a Helm chart can
keep one changelog for the service and another for the chart, all under one version. A commit
lands in a changelog if its scope or the paths it touches match that changelog's filters (see
[declaration scenarios](declaration-scenarios.md#changelogs-per-service)). Which changelog each
publisher uses is open.

[`changelog-merge`](https://github.com/cachix/changelog-merge) provides semantic Git merging
for concurrent changelog additions and release cuts, which helps when several branches touch
the same `CHANGELOG.md`. It does not select versions or publish releases.

Developers should be able to add their own text, not only commit-derived notes. A changelog
can therefore have an optional place where developers write notes for the next release while
working, such as an `Unreleased` section at the top. `prepare` moves that text into the new
version's section next to the generated notes and leaves the place empty for the next release.
A changelog without that place works with commit-derived notes alone. The exact layout, and how
the text is handed to templates, is still open; the first milestone does not need this feature.

### Rendering notes

Presswerk renders changelog sections and tag names with
[minijinja](https://github.com/mitsuhiko/minijinja) templates. Its self-release notes group
Gitmoji commits by emoji and order the groups by cardinality. The default should be sensible
for Presswerk without trying to satisfy every project. Custom templates let projects shape
their notes; their configuration and template context are still open. For several changelogs,
another possible layout groups an overall changelog by the other changelogs, with an `Other`
group for the rest.

The data passed to templates (version, date, commits, groups) is a public interface: changing
it breaks consumer templates.

Release notes of private repositories may be internal. Nothing in either model may assume the
notes are meant for the public.

### Why notes are committed files

Frozen notes have to survive the gap between `prepare` and `publish`, which may run on another
machine much later. Committing them as files is the only option that fits the model:

- **Not in the commit message:** squash and merge commits replace the message, so the notes
  would be lost; a message cannot be reviewed in a pull request; and Nix cannot see it.
- **Not regenerated at publication:** human edits would be lost.
- **Not stored outside the repository:** Presswerk keeps
  [no state of its own](overview.md#no-state-of-its-own).
