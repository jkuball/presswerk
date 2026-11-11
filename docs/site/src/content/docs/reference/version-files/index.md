---
title: Version files
description: The files in your repository that carry the version, and how Presswerk reads and writes them.
---

A version file is a file in your repository that carries the project's version, such as
`Cargo.toml`. Presswerk uses version files in two directions:

- **Reading.** Each version file defines the release's [`version`](/reference/options/#version)
  with the version it carries. The module system merges these definitions like any others, so every
  version file must carry the same version: if two disagree, their definitions conflict and the flake
  fails to evaluate, naming both files. The CLI also checks files that the flake does not read, such
  as `Cargo.lock`, and refuses to plan or prepare if one of them disagrees.
- **Writing.** `prepare` writes the new version into every version file and commits them together in
  the release commit.

A [badge](#badge) is the exception: Presswerk only writes it and never reads it.

Because the version comes from committed files, it is an ordinary input of your flake, and your
build can use it like any other value.

:::caution[Every release rebuilds what reads the version]
A derivation that reads the version is rebuilt for every release, and so is everything that depends
on it. Where an artifact carries the version, such as `--version` output or package metadata, that
rebuild is what makes it match the release. Elsewhere, keep the version out, especially out of
derivations that much of your build depends on.
:::

## Cargo

[`versionFiles.cargo`](/reference/options/#versionFiles.cargo) declares one entry per
crate. The flake reads `package.version` from the crate's `Cargo.toml`. Presswerk writes that field
and the crate's package entry in `Cargo.lock`, and checks that both carry the same version.

```nix presswerk-module
versionFiles.cargo.my-crate = { };
versionFiles.cargo.my-other-crate.cargoToml = "crates/other/Cargo.toml";
```

## Badge

[`versionFiles.badge`](/reference/options/#versionFiles.badge) declares a version badge, a small
SVG image for your README in the style of the [Presswerk badges](/badges/): the Presswerk icon on
the left and the version on the right.

```nix presswerk-module
versionFiles.badge.default = { };
```

![Version badge for Presswerk itself](/version-badge.svg)

`prepare` generates the badge from scratch at `.github/presswerk-badge.svg` and commits it in the
release commit; set `path` to put it elsewhere, or declare several badges under different names.
Embed it in your README with a relative path:

```md title="README.md"
![Version](.github/presswerk-badge.svg)
```

A badge does not carry the version. The flake does not read it, and the CLI does not check it, so
editing or deleting it never blocks a release; the next release writes it again. A release therefore
needs at least one other version file, such as Cargo.

Like every version file, the badge shows the version of the commit it is in. Between `prepare` and
`publish`, it names a version that is not published yet.

## Other formats

Cargo and the badge are the only built-in version files today. Declaring version files for other
formats in the flake itself is planned.
