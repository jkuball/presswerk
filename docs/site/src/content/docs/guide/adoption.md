---
title: Adopt an existing project
description: Start releasing a project with Presswerk that already has releases made by another tool.
sidebar:
  label: Adopt an existing project
---

A project that already has releases does not start from the implicit `0.0.0` baseline. Instead,
its last release becomes the baseline: you trust it once, and Presswerk makes its first release on
top of it. From then on, every release is a Presswerk release.

This needs a tag for the last release on the remote, and version files that still contain the
version of that release. If the project has a committed version without a tag, create the tag by
hand first.

## 1. Add the declaration

Add Presswerk as a flake input and declare the release, as in the
[walkthrough](../walkthrough/#2-declare-the-package-and-release):

```nix title="flake.nix"
{
  inputs.presswerk.url = "github:jkuball/presswerk/v{{presswerk.version}}";

  outputs = { self, presswerk, ... }: {
    presswerk.default = presswerk.lib.mkRelease {
      sourceRoot = self.outPath;
      versionFiles.cargo.YOUR-CRATE = { };
      publishers.github-release.default.repository = "YOUR-OWNER/YOUR-REPO";
      analyzer.gitmoji.enable = true;
      # Only if your tags are not `v{version}`:
      # tag.format = "release-{version}";
    };
  };
}
```

`tag.format` must render the existing tags, so Presswerk finds the last release. See the
[options reference](/reference/options/) for every field.

## 2. Keep the version files

Leave the version files at the last released version; do not reset them to `0.0.0`. Presswerk
reads the current version from them and computes the next one from the commits since the last tag.

Lock the flake and commit the declaration with your other changes:

```console
$ nix flake lock
$ git add flake.nix flake.lock
$ git commit -m "👷 Adopt Presswerk"
```

Use a subject that does not select a version level, such as 👷 with Gitmoji or `ci:` with
Conventional Commits, so adopting Presswerk does not change the next version by itself.

## 3. Trust the last release once

The commit of the last release has no Presswerk declaration, so `plan` and `prepare` cannot check
whether that release is complete. They stop and print the command to adopt it:

```console
$ nix run --inputs-from . presswerk -- prepare
error: previous release v0.38.2 at 1a2b3c4… has no Presswerk declaration
  If v0.38.2 was released completely by another tool, adopt it once:
    presswerk prepare --trust-predecessor v0.38.2=1a2b3c4…
```

Once you have confirmed that the release is complete, run that command:

```console
$ nix run --inputs-from . presswerk -- prepare --trust-predecessor v0.38.2=1a2b3c4…
```

Presswerk checks that the tag exists on the remote, points to that commit, and is an ancestor of
`HEAD`, and that the version files committed today contain the tag's version. A lightweight tag is
accepted here, although Presswerk's own tags are annotated. Presswerk reads nothing else from the
old release: that it is complete, including the artifacts another tool published, is your word.
The release commit records the trust in a `Presswerk-Trusted-Predecessor` trailer, so `publish`
verifies the same predecessor.

Run it wherever you usually run `prepare`: locally, then push the release commit, or once in CI
with the flag added to the `prepare` step for a single run. `prepare` creates no release commit if
no commit since the last tag is release-worthy; pass the flag again on the first release.

## 4. Release as usual

From the next release on, the predecessor is an ordinary Presswerk release and `prepare` needs no
flag. Continue with the flow of your [release mode](../release-modes/).

## Changelog

An existing changelog keeps its older sections as they are. If Presswerk recognizes none of its
version headings, it inserts its first section before the first level-2 heading:

```markdown title="CHANGELOG.md"
# Changelog

## [0.39.0] - 2026-11-20
…

## v0.38.2 (2026-11-10)
…
```

From then on, the newest heading is Presswerk's and new sections go above it. To have Presswerk
recognize the older headings too, declare their format with `changelog.heading`.
