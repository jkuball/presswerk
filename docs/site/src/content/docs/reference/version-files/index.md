---
title: Version files
description: The files in your repository that carry the version, and how Presswerk reads and writes them.
---

A version file is a file in your repository that carries the project's version, such as
`Cargo.toml`. Presswerk uses version files in two directions:

- **Reading.** Each version file defines the release's [`version`](/presswerk/reference/options/#version)
  with the version it carries. The module system merges these definitions like any others, so every
  version file must carry the same version: if two disagree, their definitions conflict and the flake
  fails to evaluate, naming both files. The CLI also checks files that the flake does not read, such
  as `Cargo.lock`, and refuses to plan or prepare if one of them disagrees.
- **Writing.** `prepare` writes the new version into every version file and commits them together in
  the release commit.

Because the version comes from committed files, it is an ordinary input of your flake, and your
build can use it like any other value.

:::caution[Every release rebuilds what reads the version]
A derivation that reads the version is rebuilt for every release, and so is everything that depends
on it. Where an artifact carries the version, such as `--version` output or package metadata, that
rebuild is what makes it match the release. Elsewhere, keep the version out, especially out of
derivations that much of your build depends on.
:::

## Cargo

[`versionFiles.cargo`](/presswerk/reference/options/#versionFiles.cargo) declares one entry per
crate. The flake reads `package.version` from the crate's `Cargo.toml`. Presswerk writes that field
and the crate's package entry in `Cargo.lock`, and checks that both carry the same version.

```nix
versionFiles.cargo.my-crate = { };
versionFiles.cargo.my-other-crate.cargoToml = "crates/other/Cargo.toml";
```

## Other formats

Cargo is the only built-in version file today. Declaring version files for other formats in the
flake itself is planned.
