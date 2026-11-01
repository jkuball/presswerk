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
with `prepare --major` on a `0.y.z` version.

SemVer does not say how to bump within `0.x`. Presswerk follows the common convention, also
used by Cargo: on `0.y.z`, a breaking change bumps `y`, and everything else bumps `z`.

Without a flag, the analysis decides the bump, except that it never moves to `1.0.0`.
`prepare --major`, `--minor`, and `--patch` override it with an explicit level. The flags name
literal SemVer positions. `prepare` fails if the analysis requires a higher level than the one
selected; on `0.y.z`, a breaking change therefore rejects `--patch`.

## Release flow

Committing the version before building means a release has two steps:

1. `prepare` creates a commit with the new version and frozen notes.
1. That commit goes through CI, and only then does `publish` release it.

This is how Presswerk gets a pure release, where everything published is derived from one
commit. It costs one extra commit per release.

`publish` decides from the commit alone whether it is a release. Being a
[release commit](glossary.md#release-commit) is a structural property: the commit changes the
version compared to its parent. It does not depend on publication state. What `publish` does
with a commit depends on whether its version changed and where the tag for its version is:

| Version vs parent | Tag for the version | Classification | `publish` does |
| --- | --- | --- | --- |
| either | on a strict ancestor | ordinary commit | nothing |
| changed | missing | new release | validates, then publishes |
| unchanged | missing | inherits a [pending release](#pending-releases) | reports it and exits successfully |
| changed | on this commit | retry or republish | completes missing state |
| unchanged | on this commit | invalid candidate | stops |
| either | on a commit that is not an ancestor | conflict | stops |

"Parent" means the first parent for merge commits. An annotated tag is compared by the commit it
points to.

A tag on an ancestor covers ordinary development after a release, and the cases where the
version returns to one that is already released: a revert of a failed release commit, and
adopting a project whose current version was released before Presswerk. A commit that inherits
an unpublished version is a fixup after a failed release commit; under the strict rule below it
is reported, never published.

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
release commit is on the branch. Recovery is a revert. Who made the release commit does not
matter: `prepare`, a forge's squash merge, or a developer by hand all work, as long as the
commit is valid (see [scenario 12](scenarios.md#12-hand-made-release-commit)).

By default, `prepare` only creates the release commit locally, on the current branch.
`prepare --push` also pushes it. Pushing it to main and letting CI run `publish` is the
standard flow, and the only one in the first release.

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
- **Triggering CI.** On GitHub, pushes and pull requests made with the default `GITHUB_TOKEN`
  start no workflow runs, so automated preparation needs a GitHub App token or a personal
  token.

This rule is strict on purpose, and there is no loose mode. A version is released from exactly
its release commit and nothing else. A later commit that still carries an untagged version,
such as a fixup or an unrelated merge, is never published; `publish` reports it and stops.

Presswerk deliberately uses the version-changing commit as its release boundary: later changes
require a new `prepare`, so that the boundary and the notes are always reconsidered together.
Publishing a later commit would not in itself break purity; the rule exists for correctness of
what is released and described. The structural rule alone does not prove that the notes are
complete, since a hand-written release commit can have incomplete notes, so the validity checks
define what is actually guaranteed.

The release commit has to pass CI before it is published, and it can fail there, since the
version may be used by derivations and checks. Recovery is a revert: `presswerk revert`
reverts the failed release commit, the problem is fixed, and `prepare` runs again (see
[scenario 7](scenarios.md#7-release-commit-fails-ci)).

## Previous release

`plan` and `prepare` need the previous release: its version is the base for the bump, and its
tag starts the commit range for analysis and notes. `publish` does not need it.

The previous release comes from the commit itself, not from scanning tags:

1. evaluate `version` at `HEAD`, for example `1.3.0`;
1. the previous release is the tag rendered from `tag.format`, for example `v1.3.0`;
1. that tag must be an ancestor of `HEAD`. If it is missing, the release is
   [pending](#pending-releases); if it points elsewhere, `plan` fails;
1. the commit range is `v1.3.0..HEAD`, resolved by Git itself.

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

If the committed version has no tag, `HEAD` is a release commit whose version is not published
yet, or it sits on top of one. That release is *pending*. To name the release commit in its
report, Presswerk follows first parents back only until the version changes. Presswerk cannot tell whether it is
still on its way through CI or has failed, since that would require asking the CI system. It
does not guess:

- `publish` on a later commit reports the pending release and exits successfully;
- `prepare` and `plan` report the pending release and do nothing until it is published or
  reverted, also exiting successfully.

If the release commit failed, its own pipeline is already red; repeating that alarm on every
later commit would only add noise, and would raise false alarms while the release is still on
its way. Once the pending release is published, the next `prepare` picks up everything that
landed in the meantime.

```console
$ presswerk prepare --pre rc --push
pending release  1.4.0-rc.2 at 6e7f8a9 is not published yet
Nothing to prepare until it is published or reverted.
```

This follows a general principle: Presswerk detects and reports such states precisely, and
leaves decisions that need context, such as whether to wait or to revert, to the developer or
to an agent using the tool.

### The first release

A commit without a release declaration counts as version `0.0.0`, and so does the imaginary
parent of a root commit. The commit that introduces the declaration therefore changes the
version from `0.0.0`, and is a release commit like any other:

- a project created from the [flake template](roadmap.md#flake-template) releases from its
  very first commit;
- an existing flake that was never released releases from the commit that adds the
  declaration and the changelog section;
- an existing flake that was already released by other means tags that release first, so
  adding the declaration is an ordinary commit.

The first version is the project's own choice. Presswerk suggests `0.1.0`, as SemVer does,
but `0.0.1` or `0.3.0` are equally valid. Skipping versions from the implicit `0.0.0` is
allowed for the first release only; `publish` sees from the commit alone that the parent has no
declaration. After the first release, all checks apply as usual.

The first release commit is never made by `prepare`, which needs a declaration at `HEAD` to
work. It is a boundary commit, made by hand or by the flake template, that adds the
declaration, the version, and the changelog section together, cleanly separating the
repository's history before and after adoption. Committing the declaration alone would create
an invalid release commit, which `publish` rejects (see
[scenario 13](scenarios.md#13-adopting-an-existing-flake)).

### Adoption

A committed version without a tag is reported as a pending release. When a repository adopts
Presswerk, that report is where adoption starts. This is the whole adoption story: Presswerk has
an exact contract for what it needs to release the next version, and its error messages say
precisely what is missing. A developer adopting Presswerk only has to create that state, for
example by setting `tag.format` to match existing tags, or by tagging the commit that released
the committed version. There is no `adopt` command; a short guide in the documentation covers
it. An interactive `presswerk adopt` may come much later; it would create exactly that
boundary commit.

```console
$ presswerk plan
previous release   v1.3.0 (9c0ffee)    from committed version 1.3.0

$ presswerk plan
pending release  0.4.2 at 2b3c4d5 is not published yet (no tag v0.4.2)
If 0.4.2 was released before Presswerk, tag the commit that released it as v0.4.2.
```

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

Advanced users should be able to provide ordinary git-cliff parser configuration through the
git-cliff backend's options. The default behavior for unknown commits remains open: reject,
include as `Other`, or ignore.

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

Developers must be able to add their own text, not only commit-derived notes. Every changelog
therefore has a place where developers write notes for the next release while working, such
as an `Unreleased` section at the top. `prepare` moves that text into the new version's
section next to the generated notes and leaves the place empty for the next release. The
exact layout, and how the text is handed to templates, is still open.

### Rendering notes

Presswerk renders every changelog section, and tag names, itself with
[minijinja](https://github.com/mitsuhiko/minijinja) templates. The built-in default template
groups an unfiltered changelog's notes by the other changelogs, with an `Other` group for the
rest. Each changelog can bring its own template from the repository, so every project can
shape its notes freely.

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
