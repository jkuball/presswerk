# Product shape

Presswerk consists of a CLI that does all the work, a release declaration that lives in the
consumer's flake, and an optional flake-parts module that helps write that declaration.

## Rust CLI

The CLI is the product. It should eventually be usable locally and from different CI systems.
The first milestone prioritizes automatic CI-driven releases. Tentative commands are:

```text
presswerk plan
presswerk prepare
presswerk check
presswerk publish
presswerk revert
```

A likely interpretation is:

- `plan` finds the release baseline, runs commit analysis, proposes the next version, and
  renders notes. It writes nothing and is the dry run;
- `prepare` writes the version files and notes and creates a release commit. It is run
  automatically in CI for the first milestone. A local workflow, including push behavior,
  can follow later. Preparation analyzes the full range since the previous release, not only
  the latest commit. When the release chain is healthy and that range has nothing to release,
  preparation exits successfully without creating a commit;
- `check` runs the preflight that `publish` always runs before its first write, without
  publishing: candidate validity, credentials, remote state, and artifact availability (see
  [publication](publication.md#preflight));
- `publish` performs ordered, idempotent publication and verifies the result. It is meant to
  run on every push and any number of times per commit, so deciding that a commit is not a
  release must be fast;
- `revert` reverts a failed release commit. With `--impure-cleanup` it also deletes the tag to
  free the version (see [publication](publication.md#freeing-a-version)).

The exact command names and division remain open. For example, `plan` could also be a flag
on `prepare`, and the pressing-plant analogy suggests `record` or `cut` for `prepare` and
`press` for `publish`. A correctly idempotent `publish` should make
a separate `resume` command unnecessary.

### The press is not part of the master

By default, the CLI comes from outside the master record: the release commit pins *what* is
released (artifacts, version, notes, and the release declaration), not *how* it is pressed. A
newer Presswerk can therefore republish an older tag, which is how bugs in publishers are
fixed after the fact.

Consumers may still pin the CLI in their flake, for example in a dev shell, as a reproducible
default. A pinned CLI is not the only one allowed: a newer, externally installed Presswerk can
still republish the same tag. What cannot be fixed afterward is code that is part of the master
record itself, such as the declaration or the flake-parts module and library at that commit.

The CLI is always an impure process: it reads Git history, talks to forges and registries, and
receives credentials at runtime. How it is wired into a local shell or a CI system is described
in [runtime integration](runtime-integration.md).

### Nix configures, the CLI acts

Nix is the configuration surface: it declares what to release and where things live. The
logic lives in the CLI: writing and reading version files, cutting and reading changelog
sections, analyzing commits, and publishing. The declaration stays plain data wherever
possible.

## Release declaration in the flake

The consumer declares what to release in its flake: artifacts, publishers, version source,
branch policy, and commit analysis settings. The CLI reads that
[release declaration](glossary.md#release-declaration) by evaluating the flake, instead of
maintaining a separate configuration file. The CLI evaluates in-process with
[nix-bindings-rust](https://github.com/nixops4/nix-bindings-rust), which wraps the Nix C API.
It does not invoke `nix eval` or another Nix command.

Reasons:

- the flake is already the [master record](glossary.md#master-record), so the declaration is
  pinned to the same commit as the artifacts it describes;
- artifacts are referenced as ordinary derivations, and the CLI can evaluate and realise exactly
  those;
- there is no second configuration language.

A release declaration is always required. Presswerk does not offer a zero-configuration mode
that guesses the forge, the version files, or the publishers.

Consequences:

- Nix is the primary distribution channel. The CLI links against the Nix C libraries it was
  built with and uses the local Nix store, usually through the Nix daemon. The daemon may run a
  different Nix version, such as Determinate Nix;
- the declaration never contains secrets. It may name them, but their values only reach the
  CLI at runtime. Fetching private flake inputs during evaluation may still need
  authentication, configured in Nix itself (such as `access-tokens`), never in the
  declaration;
- the intended publication behavior is that evaluating the declaration does not trigger a
  build. Import from derivation, substitution-only evaluation, and clear errors for missing
  outputs need further validation and are not gates for the first asset-free self-release.

### Only commits are evaluated

Presswerk evaluates the flake at an exact commit (`git+file://…?rev=<sha>`), never the working
tree. Uncommitted or staged changes therefore never leak into a release.

- `plan` evaluates `HEAD`. On a dirty tree it still works, but warns that local changes are
  not part of what it shows.
- `prepare` works from an exact commit and creates a release commit. A future local workflow
  must handle a dirty working tree safely. CI checks the resulting candidate before publication.
- `publish` evaluates the candidate commit.

The release declaration is therefore always read from the commit being worked on.

Nix does not enforce this on its own: a `git+file://` reference without `rev` reads the
working tree, including uncommitted changes to tracked files. The CLI therefore always builds
the flake reference itself and pins `rev`.

The lock file is part of the commit as well. The CLI locks the flake in check mode: it never
updates or writes `flake.lock`, and a commit whose lock file is missing or out of date fails
instead of resolving inputs to whatever is current. The `nix` command behaves differently
here: without `--no-update-lock-file`, it locks missing inputs and writes the result into the
working tree.

The declaration is read by its exact output path, `presswerk.default`, starting from the flake
outputs. Nix's installable search through `packages.<system>` and `legacyPackages.<system>`
does not apply, so a package that happens to be named `presswerk` cannot shadow the
declaration.

```console
$ presswerk plan
warning: working tree is dirty; showing the plan for HEAD (4f2a91c) without local changes
...

$ presswerk prepare
error: working tree is dirty; commit or stash your changes first
```

### Shape of the declaration

The shape is undecided. In particular, Presswerk is not committed to a `lib.mkRelease`
function or to a runnable `packages.<system>.release` output. Options include:

1. a plain data output, such as `presswerk.<name>`, that the CLI evaluates. The CLI itself is
   installed separately, for example through a dev shell or `nix run`;
1. a library function that validates the declaration and returns it, still as data;
1. a library function that additionally wraps the CLI into a runnable package bound to the
   declaration.

Options 1 and 2 keep the flake purely declarative. Option 3 helps runtimes that can only run a
flake package, and could be added later on top of either of the others without changing the
CLI.

Illustrative only, using option 1:

```nix
{
  outputs = { self, ... }: {
    presswerk.default = {
      version = "1.2.3";
      publishers.github-release.default = {
        repository = "owner/project";
        assets."project-x86_64-linux" = "${self.packages.x86_64-linux.default}/bin/project";
      };
    };
  };
}
```

See [declaration scenarios](declaration-scenarios.md) for complete examples.

```sh
presswerk plan
presswerk publish
```

## flake-parts module

The optional flake-parts module provides typed options and produces the same release
declaration as a hand-written flake.

Illustrative only:

```nix
{
  imports = [ inputs.presswerk.flakeModules.default ];

  presswerk.default.publishers.github-release.default.repository = "owner/project";

  perSystem = { self', system, ... }: {
    presswerk.default.publishers.github-release.default.assets."project-${system}" =
      "${self'.packages.default}/bin/project";
  };
}
```

The module must not contain behavior unavailable to ordinary flakes.
