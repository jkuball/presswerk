A commit analyzer reads the commits since the previous release and decides two things:

- **The version bump.** Each commit selects a level, and the highest level selects the next version.
  If no commit selects one, there is nothing to release.
- **The release notes.** The analyzer groups the commits into the notes that `prepare` writes into
  the [changelog](/reference/options/#changelog.path) and that the publishers publish.

## Choose an analyzer

Presswerk has two built-in analyzers:

- [Conventional Commits](/reference/analyzers/conventional-commits/), enabled by
  [`analyzer.conventional.enable`](/reference/options/#analyzer.conventional.enable)
- [Gitmoji](/reference/analyzers/gitmoji/), enabled by
  [`analyzer.gitmoji.enable`](/reference/options/#analyzer.gitmoji.enable)

A release enables exactly one of them, and none is enabled by default. A release without an
enabled analyzer, or with more than one, fails evaluation with a message that names them.

Both analyzers return the same structured release-note context. The
[changelog template](/reference/changelog/#template) decides how those groups and commits become
the section that `prepare` commits.

## Version levels

Each analyzer has a catalog that assigns every kind of commit a level: `"major"`, `"minor"`,
`"patch"`, or `null`. Presswerk takes the highest level in the commit range. Before `1.0.0`, both
`minor` and `major` advance the minor component; `patch` advances the patch component.

Commits with the level `null` still appear in the release notes when another commit in the range
selects a level. If none does, `plan` reports that there is nothing to prepare.

No commit moves a project to `1.0.0`. That step must always be acknowledged explicitly; a way to
choose the version for it is planned.

## History

Both analyzers treat history the same way:

- A commit that does not follow the convention, or whose intention is missing from the catalog,
  contributes neither a bump nor a release note.
- Merge commits are skipped, but the commits they merge are analyzed.
- A revert inside the range cancels the commit it reverts, so neither of them contributes a level
  to the version bump or an entry to the release notes.
- The release commit that `prepare` creates follows the convention.
