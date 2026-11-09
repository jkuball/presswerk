---
title: Version files
description: The files in your repository that carry the version, and how Presswerk reads and writes them.
sidebar:
  label: Overview
---

A version file is a file in your repository that carries the project's version, such as
`Cargo.toml`. Presswerk uses version files in two directions:

- **Reading.** Each version file defines the release's [`version`](/presswerk/reference/options/#version)
  with the version it carries. The module system merges these definitions like any others, so every
  version file must carry the same version: if two disagree, their definitions conflict and the flake
  fails to evaluate, naming both files.
- **Writing.** `prepare` writes the new version into every version file and commits them together in
  the release commit.

Because the version comes from committed files, it is an ordinary input of your flake, and your
build can use it like any other value.

## Available version files

| Version file | Option | Reads | Writes |
| --- | --- | --- | --- |
| Cargo | [`versionFiles.cargo`](/presswerk/reference/options/#versionFiles.cargo) | `package.version` in `Cargo.toml` | `Cargo.toml` and the crate's entry in `Cargo.lock` |

## Your own

Version files for other formats are meant to be declared in the flake itself. See
[Build your own](./build-your-own/).
