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
version of that release. If your project does not match that yet:

- **Never released, but the version is not `0.0.0`**, for example a default from `cargo init` or
  `npm init`: reset the version to `0.0.0`. The project is then fresh and needs no adoption.
- **Released only to a registry, without Git tags**: tag the commit of the last release by hand,
  then trust it as described below.
- **Tags in another format**: set `tag.format` to that format, or tag the last release in
  Presswerk's format.

## 1. Add the declaration

Add Presswerk as a flake input and declare the release, as in the
[walkthrough](../walkthrough/#2-declare-the-package-and-release):

```nix title="flake.nix"
{
  inputs.presswerk.url = "github:jkuball/presswerk/v{{presswerk.version}}";

  outputs = { self, presswerk, ... }: {
    presswerk.default = presswerk.lib.mkRelease {
      sourceRoot = self.outPath;
      # At least one version file; here, Cargo.
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
whether that release is complete. They stop and print how to trust it:

```console
$ nix run --inputs-from . presswerk -- plan
error: previous release v0.38.2 at 1a2b3c4… has no Presswerk declaration
  If v0.38.2 was released completely by another tool, trust it in the release module:
    trustedPredecessor = { tag = "v0.38.2"; commit = "1a2b3c4…"; };
  or once with:
    presswerk prepare --trust-predecessor v0.38.2=1a2b3c4…
```

Once you have confirmed that the release is complete, add the option to the declaration and commit
it, for example together with the declaration itself:

```nix title="flake.nix"
presswerk.default = presswerk.lib.mkRelease {
  # …
  trustedPredecessor = {
    tag = "v0.38.2";
    commit = "1a2b3c4…"; # the full commit SHA from the error
  };
};
```

Presswerk checks that the tag exists on the remote, points to that commit, and is an ancestor of
`HEAD`, and that the version files committed today contain the tag's version. A lightweight tag is
accepted here, although Presswerk's own tags are annotated. Presswerk reads nothing else from the
old release: that it is complete, including the artifacts another tool published, is your word.
The release commit records the trust in a `Presswerk-Trusted-Predecessor` trailer, so `publish`
verifies the same predecessor.

With the option committed, `plan` passes and `prepare` runs wherever you usually run it, in CI or
locally. Until a commit since the last tag is release-worthy, `prepare` creates no release commit
and the option keeps waiting for it.

Instead of the option, you can pass the trust for a single run with the flag the error prints, such
as `prepare --trust-predecessor v0.38.2=1a2b3c4…`, locally or added once to the `prepare` step in
CI. `plan` accepts the flag too. Without the option, `plan` keeps failing until the first release
exists.

## 4. Release as usual

From the next release on, the predecessor is an ordinary Presswerk release. The option then names an
older release and has no effect: `plan` and `prepare` warn that it can be removed, and you remove it
whenever it suits you. Continue with the flow of your [release mode](../release-modes/).

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

## OCI image aliases

This applies only if the project publishes [OCI images](/reference/publishers/oci-image/) with
aliases such as `latest`. Presswerk moves an alias only when the index it points to carries a
version annotation (`org.opencontainers.image.version`), which other tools often do not write.
Such an alias would stay on the old release, so the first release takes it over: if the release
commit trusts a predecessor, an alias without a version annotation moves when it points to the
same digest as the predecessor's version tag in the same repository. The predecessor's tag is one of the publisher's `tags`, rendered with
the trusted version, such as `0.38.2`. Both digests are read from the registry. `publish` reports
the alias as `taken over`, and from then on it moves like any other alias.

`prepare --trust-predecessor` checks this before it writes the release commit:

```console
note: alias ghcr.io/owner/project:latest points to the trusted predecessor's tag 0.38.2; publish will take it over
```

If the alias points elsewhere, for example because the old tool tagged the release under another
name or someone retagged it by hand, Presswerk leaves it alone and reports it as `kept`.
`prepare --trust-predecessor` warns about it beforehand:

```console
warning: alias ghcr.io/owner/project:latest points to sha256:d753bc… without a version annotation, so another tool pushed it; publish will keep it
```

Retag such an alias once by hand after the first release, for example with
[skopeo](https://github.com/containers/skopeo), which copies the index unchanged:

```console
$ skopeo copy --all --preserve-digests docker://ghcr.io/owner/project:0.38.3 docker://ghcr.io/owner/project:latest
```

From the next release on, Presswerk moves it.
