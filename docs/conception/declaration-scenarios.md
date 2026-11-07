# Declaration scenarios

The [scenarios](scenarios.md) show the developer experience of the CLI. This document shows
the other side: the person who writes the [release declaration](glossary.md#release-declaration)
in the flake. The CLI always reads standardized plain data. Consumers may write that data
directly, but the primary authoring interface is the Nix module evaluated by `lib.mkRelease`
(see [product shape](product-shape.md#shape-of-the-declaration)).

## The example: Presswerk's own release

All sketches below illustrate the same possible future release. They are not the scope of the
asset-free [self-release milestone](roadmap.md#self-release-milestone):

- the version lives in `Cargo.toml` and is read by the Nix package;
- `prepare` updates `Cargo.toml` and `Cargo.lock`, which also records the crate's own version;
- this example analyzes Conventional Commits with git-cliff; Presswerk's self-release uses
  Gitmoji instead;
- the release is an annotated tag `v<version>` plus a GitHub release;
- the GitHub release carries binaries for `x86_64-linux` and `aarch64-darwin` (later work);
- the GitHub token is named, never contained.

The tag is not configured as a publisher. It is always created first, because it is the
[reservation and lock](publication.md#the-tag-as-reservation-and-lock); only its format is a
setting.

## A. Plain data output

The flake exposes an attribute set. Presswerk is not a flake input at all; the CLI comes from
outside and evaluates `presswerk.default`.

```nix
{
  outputs = { self, nixpkgs, ... }: {
    packages = { /* presswerk-static for x86_64-linux, presswerk for aarch64-darwin */ };

    presswerk.default = {
      schemaVersion = 2;
      version = (nixpkgs.lib.importTOML ./Cargo.toml).package.version;

      versionFiles = [
        { path = "Cargo.toml"; toml = "package.version"; }
        { path = "Cargo.lock"; cargoLock = "presswerk"; }
        # Escape hatch: a generator derivation, built in an isolated build environment.
        # { generator = { system, version }: let pkgs = nixpkgs.legacyPackages.${system}; in
        #     pkgs.runCommand "bump-${version}" { nativeBuildInputs = [ pkgs.jq ]; } ''
        #       mkdir -p $out
        #       jq --arg v '${version}' '.version = $v' ${./package.json} > $out/package.json
        #     ''; }
      ];

      tag.format = "v{version}";  # the default

      analyzer = {
        convention = "conventional-commits";
        backend = "git-cliff";  # the default
      };

      changelog.path = "CHANGELOG.md";

      publishers.github-release.default = {
        repository = "jkuball/presswerk";
        token.secret = "GITHUB_TOKEN";
        assets = {
          "presswerk-x86_64-linux" = "${self.packages.x86_64-linux.presswerk-static}/bin/presswerk";
          "presswerk-aarch64-darwin" = "${self.packages.aarch64-darwin.presswerk}/bin/presswerk";
        };
      };
    };
  };
}
```

- Nothing to learn beyond Nix itself. No dependency on Presswerk in the lock file.
- `versionFiles` entries are writers built into the CLI: `toml` sets a key in a TOML file,
  `cargoLock` sets the version of one package in `Cargo.lock`. `generator` is the escape hatch
  for everything else (see [version generators](#version-generators)). Afterward, the CLI
  evaluates `version` again and fails unless it equals the new version, whichever writers were
  used.
- `tag.format` contains the exact `{version}` placeholder once, not adjacent to another brace.
  Everything around it is literal. The CLI renders a tag by replacing the placeholder and reads
  tags back by matching the literal
  prefix and suffix; what is in between is the version. Tags that do not match are not releases.
  This deliberately is not a Minijinja template because tag formats must be reversible.
- Interpolating `self.packages...` only computes the store path. Nothing is built during
  evaluation (import from derivation is only satisfied by substitution); the CLI realises the
  paths only when it publishes.
- Typos and missing fields surface only when the CLI evaluates the declaration.
- Every default has to be spelled out or known by heart.

## Version generators

The escape hatch for version bumps the built-in writers cannot do is a generator: a function
`{ system, version }: derivation`. It replaces the earlier idea of a script running in the
checkout, which could sweep stray files such as credentials or build output into the release
commit.

- **Evaluation and build:** `prepare` evaluates the generator from the commit, never the working
  tree, applies it to the new version and the system of the machine running `prepare`, and
  realises it. That may build locally, substitute, or use remote builders. This is an explicit
  realisation in `prepare`, not import from derivation; `publish` never invokes generators.
  `plan` realises generators too, so it can show the real diff, including merge conflicts,
  without touching Git or the checkout. Generators should depend only on files, so this stays
  cheap.
- **Isolation:** the generator is built in an isolated build environment, subject to the
  builder's sandbox policy. This is not a promise of secure execution: sandboxing can be
  disabled or relaxed, and is off by default on macOS, where a generator may read files beyond
  its inputs. Generators are trusted code, like the rest of the declaration. Fixed-output
  derivations, which have network access, are rejected. What holds regardless of the sandbox:
  Presswerk copies only validated generator outputs and never discovers release changes by
  sweeping the working tree.
- **Output:** every regular file in `$out` replaces the file at the same repository-relative
  path. Whatever is in `$out` is the change; there is no list of allowed paths. Generators can
  use the usual tools, such as `substituteInPlace` or `jq`. Narrowing inputs to the files a
  generator needs avoids needless rebuilds.
- **Import:** the CLI accepts only regular files at validated paths. It rejects symlinks,
  anything under `.git`, paths escaping the repository, directory/file collisions, and paths
  inside submodules. Files are copied, not linked into the store, stay writable, and keep their
  executable bit. Deletions are not supported.
- **Several writers on one file:** every writer, built-in or generator, starts from the old
  commit and never sees the others' changes. The CLI combines their results per file with Git's
  three-way merge, using the old file as the base. Changes in different places combine, and
  identical changes combine as one; overlapping different changes fail `prepare` with a conflict
  naming both writers. No union merging or custom merge drivers resolve such conflicts. The
  promise is textual: a clean merge can still produce an invalid file, which the consistency
  checks and CI catch. Files that must change together, such as `package.json` and its
  lockfile, belong in one generator.
- **Committing:** `prepare` builds the release commit from Git objects only, `HEAD` plus the
  merged files, without reading or writing the index, so files staged meanwhile by an editor or
  another process can never enter it. It then moves the branch atomically, and only if this
  worktree is still on that branch and the branch still points to the expected `HEAD`.
- **Checkout synchronization:** afterward, `prepare` updates the index and the working tree to
  the new commit for the changed files, each checked on its own: an index entry or file is only
  updated if it is unchanged since `prepare` started. Otherwise the user's change is kept and
  reported. Leaving the index behind would show a staged reversal of the release, which the next
  ordinary commit could pick up.
- **Interruption:** the branch move is atomic, `prepare` as a whole is not. If it stops after
  moving the branch but before synchronizing the checkout, the next `presswerk` command detects
  the mismatch and reports the created release commit and the incomplete checkout update,
  instead of claiming completion.

## B. Release module evaluated by `mkRelease`

`mkRelease` evaluates a Nix module with Presswerk's standard options and returns the normalized
plain declaration. The consumer can compose defaults, local policy, and modules exposed by
other flakes.

```nix
{
  inputs.presswerk.url = "github:jkuball/presswerk";

  outputs = { self, presswerk, ... }: {
    presswerk.default = presswerk.lib.mkRelease {
      sourceRoot = ./.;

      imports = [
        ({ lib, ... }: {
          changelog.path = lib.mkDefault "CHANGELOG.md";
        })
        {
          versionFiles.cargo.presswerk = {};

          publishers.github-release.default = {
            repository = "jkuball/presswerk";
            assets = {
              "presswerk-x86_64-linux" = "${self.packages.x86_64-linux.presswerk-static}/bin/presswerk";
              "presswerk-aarch64-darwin" = "${self.packages.aarch64-darwin.presswerk}/bin/presswerk";
            };
          };
        }
      ];
    };
  };
}
```

- `mkRelease` includes `presswerk.presswerkModules.default` automatically and emits the current
  renderer-owned `schemaVersion`; consumers cannot override it.
- Errors appear during evaluation with messages written for the declaration author.
- Standard module features provide imports, typed options, defaults, priorities, `mkIf`, and
  `mkMerge`. Version sources contribute to one typed effective-version option, so conflicting
  versions fail during module evaluation and matching definitions merge.
- Other flakes can expose reusable modules under `presswerkModules`; consumers import them
  explicitly. Modules receive `presswerk.version` and can use standard assertions for version
  requirements, while option-presence checks remain preferable for simple capabilities.
- `sourceRoot` is the immutable consumer source used only during module evaluation. It is not
  emitted into the declaration; normalized file paths remain repository-relative strings.
- Authoring conveniences can normalize into a smaller standardized declaration. The Cargo
  instance name defaults its crate name, so `versionFiles.cargo.presswerk = {}` reads
  `Cargo.toml` below `sourceRoot` and emits both Cargo writers shown in the plain-data example.
  Relative `cargoToml` and `cargoLock` options select other files, and an explicit `crate`
  option overrides the instance name when needed.
- The standard defaults are `v{version}` tags, Gitmoji analysis for the first milestone,
  `GITHUB_TOKEN` as the logical GitHub Release secret name, conventional root Cargo filenames,
  and `CHANGELOG.md` as the changelog path.
- M1 always emits the changelog path. Generated notes are committed there as part of the master
  record and the GitHub Release publisher reads them back for publication.
- Only the normalized plain result is exposed at `presswerk.default` and read by the CLI.
- Presswerk becomes a flake input. The module version is pinned in the master record, but the
  [press is not](product-shape.md#the-press-is-not-part-of-the-master): only declaration
  authoring and normalization are frozen with the commit, not publishing.

## C. Possible flake-parts adapter

A future flake-parts integration may provide a thin adapter for composing flake-level and
per-system values. It reuses the same release modules and produces the same standardized plain
data as `lib.mkRelease`; it does not define a separate option schema. Whether that adapter is
useful remains open and it is not part of the first milestone.

## Publishers: implementation, then name

Publishers are declared as `publishers.<implementation>.<name>`, the same pattern NixOS uses
for named instances such as `services.nginx.virtualHosts.<name>`. There are no type strings,
and one implementation can be used several times:

```nix
publishers.oci.ghcr     = { registry = "ghcr.io/jkuball"; /* ... */ };
publishers.oci.internal = { registry = "registry.corp.example"; /* ... */ };
```

Each publisher points at what it publishes directly, as Nix values in its own typed options.
There is no central artifact list; shared artifacts use `let` bindings, or `config` in the
flake-parts module (see [publication](publication.md#publishers)).

In the module system, each implementation is one typed `attrsOf (submodule …)` option. Since
attribute sets have no order, publication order cannot come from the declaration order.

## Analyzer: convention and backend

Exactly one analyzer is active, so it is not a set of named instances. The
[convention](glossary.md#convention) a project follows is independent of the tool that
analyzes it, so it is a setting of its own. The tool is a checked choice, with per-backend
options next to it, like `virtualisation.oci-containers.backend` in NixOS:

```nix
presswerk.default.analyzer = {
  convention = "conventional-commits";
  backend = "git-cliff";             # the default
  git-cliff.config = ./cliff.toml;   # only for raw git-cliff configuration
};
```

In the common case only `analyzer.convention` is written. Options for a backend that is not
selected produce a warning. Backend-specific options remain open.

For the built-in Gitmoji convention, the release module provides the vendored catalog as a
mergeable default. Outer keys identify entries; `shortcode` defaults to that key but can differ:

```nix
presswerk.default.analyzer = {
  convention = "gitmoji";
  gitmoji.catalog.memo.semver = "patch";
  gitmoji.catalog.flying_saucer = {
    shortcode = "flying-saucer";
    emoji = "🛸";
    description = "Experimental feature behind a flag.";
    semver = null;
  };
};
```

Each entry declares its Unicode emoji, release-note description, and `major`, `minor`, `patch`,
or `null` SemVer level. Resolved shortcodes must be non-empty, colon-free, and unique. The plain
release declaration is keyed by resolved shortcode, so the example serializes `flying-saucer`
without carrying the authoring-only `flying_saucer` identifier. Standard module merging handles
overrides and additions; `lib.mkForce` can replace the default catalog when a project needs
removals.

## Changelogs per service

A shop product in one repository: two services built as OCI images, and a Helm chart. All
three share one version, but each keeps its own changelog next to its code, and the product
has an overall changelog for the GitHub release.

```text
CHANGELOG.md                      product, all commits
services/api/CHANGELOG.md         only commits touching services/api
services/worker/CHANGELOG.md      only commits touching services/worker
charts/shop/CHANGELOG.md          only commits touching charts/shop
libs/auth/                        shared library, no changelog of its own
```

Declared with the flake-parts module. A commit lands in a changelog if its scope is one of the
changelog's `scopes` **or** it touches one of its `paths` (like `git log -- <path>`). A
changelog without `scopes` and `paths` receives every commit; the name `default` has no special
meaning. The filters only decide where notes go; the version is always bumped from all
commits. `scopes` need a convention that has scopes, such as Conventional Commits, and produce
a warning otherwise.

```nix
{ config, inputs, ... }: {
  imports = [ inputs.presswerk.flakeModules.default ];

  presswerk.default = {
    analyzer.convention = "conventional-commits";

    changelogs = {
      default.path = "CHANGELOG.md";
      api    = { path = "services/api/CHANGELOG.md";    scopes = [ "api" ];    paths = [ "services/api" ]; };
      worker = { path = "services/worker/CHANGELOG.md"; scopes = [ "worker" ]; paths = [ "services/worker" ]; };
      chart  = { path = "charts/shop/CHANGELOG.md";     scopes = [ "chart" ];  paths = [ "charts/shop" ]; };
    };

    publishers.github-release.default = {
      repository = "acme/shop";
      changelog = config.presswerk.default.changelogs.default;
    };

    publishers.oci.api    = { /* image per system */ };
    publishers.oci.worker = { /* image per system */ };
    publishers.helm.shop  = { /* chart */ };
  };
}
```

The commits since `v1.3.0`:

```text
a1  feat(api): bulk order export          services/api/…
a2  fix(api): timeout on large orders     services/api/…
a3  fix(worker): retry on HTTP 503        services/worker/…
a4  feat(auth): token refresh             libs/auth/…
```

`plan` shows one version and what each changelog receives:

```console
$ presswerk plan
presswerk.default  github:acme/shop  main @ 4f2a91c

previous release   v1.3.0 (9c0ffee)
commits since      4  (2 feat, 2 fix)
next version       1.4.0  (minor, because of feat)

changelogs
  default   CHANGELOG.md                   4 commits
  api       services/api/CHANGELOG.md      2 commits  (1 feat, 1 fix)
  worker    services/worker/CHANGELOG.md   1 commit   (1 fix)
  chart     charts/shop/CHANGELOG.md       no changes

Nothing written. Run `presswerk prepare` to commit 1.4.0.
```

The release commit adds one section to each changelog:

```markdown
<!-- services/api/CHANGELOG.md -->
## [1.4.0] - 2026-10-31

### Features
- bulk order export

### Fixes
- timeout on large orders
```

```markdown
<!-- charts/shop/CHANGELOG.md -->
## [1.4.0] - 2026-10-31

No recorded changes. Released as part of shop 1.4.0.
```

`publish` uses the `default` changelog for the GitHub release body. The per-service changelogs
live in the repository, next to the code they describe.

Questions this example raises:

- `a4` changes `libs/auth`, which both services build from, yet it only reaches the product
  changelog. Path rules cannot see dependencies; a matching scope is the manual way to include
  such a commit. Nix only helps here in some cases: a derivation often receives a whole source
  tree, so its inputs do not say which files a program actually uses. Explicit scopes and paths
  are the starting point.
- Whether publishers other than the forge release use changelogs, such as Artifact Hub's
  `artifacthub.io/changes` annotation for Helm charts.

## What all three share

- The result the CLI reads is plain data in the same schema. B and C are ways to write A.
- Artifacts for other systems are referenced, not built, during evaluation. `publish` never
  builds them either; it fetches what CI built (see
  [publication](publication.md#publish-never-builds)).
- Secrets are referenced by name only.
