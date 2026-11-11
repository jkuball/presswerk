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

[`versionFiles.badge`](/reference/options/#versionFiles.badge) declares a version badge: a small
SVG with the Presswerk icon and the version, in the style of the [badges](/badges/). By default,
the badge follows the viewer's system colour scheme; set `colorScheme` to `"light"` or `"dark"` for
a badge made for one scheme, for example to choose between two with `<picture>`.

```nix presswerk-module
versionFiles.badge.default = { };
```

![Version badge](/version-badge-auto.svg)

`prepare` generates the badge at `.github/presswerk-badge.svg`; set `path` to put it elsewhere.
Presswerk only writes it: the badge does not carry the version, so a release needs another version
file, and editing or deleting the badge never blocks a release.

## Other formats

Cargo and the badge are the only built-in version files today. Declaring version files for other
formats in the flake itself is planned.
