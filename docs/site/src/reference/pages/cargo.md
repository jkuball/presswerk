The Cargo publisher uploads a packaged crate to a Cargo registry: crates.io or an alternative
registry such as [kellnr](https://kellnr.io/). Declare it under
[`publishers.cargo`](/reference/options/#publishers.cargo), keyed by a name of your choice, with
the crate's `name` on the index:

```nix presswerk-module
{
  publishers.cargo.default = {
    name = "my-crate";
    crate = self.packages.x86_64-linux.my-crate-package;
    index = "sparse+https://index.crates.io/";
    token.env = "CARGO_REGISTRY_TOKEN";
  };
}
```

There is no default registry. An alternative registry is declared the same way, with its own
index:

```nix presswerk-module
{
  publishers.cargo.default = {
    name = "my-crate";
    crate = self.packages.x86_64-linux.my-crate-package;
    index = "sparse+https://kellnr.example.com/api/v1/crates/";
    token.env = "KELLNR_TOKEN";
  };
}
```

`index` is the registry's sparse index, as Cargo's `registries.<name>.index` names it, ending in a
slash and written canonically, with a lowercase host and no default port, so that two spellings of
one index cannot hide a duplicate [claim](#claims). Git indexes are not supported. Presswerk reads the publish API's URL from the index's
`config.json` and uploads through `PUT /api/v1/crates/new`, the request `cargo publish` sends.

`presswerk plan` shows the one crate version:

```console
publishers
  cargo.default  sparse+https://index.crates.io/
    crate    my-crate@1.2.0
```

A first publication reports the crate as `uploaded`; a retry of the same archive reports
`already uploaded`.

## The crate archive

The flake builds the `.crate`; Presswerk never packs. Cargo itself cannot upload an archive that
was built before, which is why Presswerk speaks the publish API. `crate` is a `.crate` file or a
derivation whose output holds exactly one.

Crate versions are immutable, and Presswerk recognizes an earlier upload by its checksum, so the
archive must be byte-for-byte reproducible: a retry that builds different bytes is conflicting.
`cargo package` writes reproducible archives from the same sources. It can run in the Nix sandbox
against vendored dependencies:

```nix
{
  packages.my-crate-package = pkgs.stdenv.mkDerivation {
    pname = "my-crate-package";
    version = "1.2.0";
    src = ./.;
    cargoDeps = pkgs.rustPlatform.importCargoLock { lockFile = ./Cargo.lock; };
    nativeBuildInputs = [ pkgs.cargo pkgs.rustPlatform.cargoSetupHook ];
    buildPhase = ''
      cargo package --offline --no-verify --package my-crate
    '';
    installPhase = ''
      mkdir $out
      cp target/package/my-crate-*.crate $out/
    '';
  };
}
```

`--no-verify` skips building the packaged crate again; the flake's own checks build and test it.

Presswerk reads the archive only as far as the upload needs it: the packaged `Cargo.toml` and the
readme it names become the request's metadata. The crate's name must be `name` and its version the
release version; Presswerk checks both before it pushes the Git tag, so a mismatched
archive leaves no release state behind. Nothing else is checked, such as `publish` in
`Cargo.toml` or missing metadata: the registry decides what it accepts. Docs are not uploaded,
including kellnr's rustdoc uploads.

Keep the crate's `Cargo.toml` among the release's [version files](/reference/version-files/), so
the package always carries the release version.

## Behavior

Before anything is written, Presswerk reads the index entry of the crate's version and compares
its `cksum`, the SHA-256 of the uploaded archive, with the local archive:

- **Missing:** the index does not list the version. Presswerk checks that the index names an
  HTTP(S) publish API before the Git tag is pushed, then uploads the version.
- **Matching:** the index lists the version with the same checksum. It is left as it is and
  reported as `already uploaded`. A yanked version is matching too and reported as `kept`;
  Presswerk never republishes or unyanks it.
- **Conflicting:** the index lists the version with another checksum. Presswerk stops before the
  Git tag is pushed.
- **Unknown:** the index cannot be read, including a response with a line that is no index
  entry, or it does not answer within 30 seconds. Presswerk stops.

After its own upload, Presswerk waits about two minutes until the index lists the new version, so
publishers that run after it, such as the next crate of a workspace, find it there. A version that
was already listed is not waited for. If the index does not list it in time, the run fails, and a
rerun finds the upload as matching.

A release's crate is complete once the index lists its version. Before `plan` and `prepare` start
the next release, they check the previous release's own Cargo publishers, without the archive, so
only that the version exists. A missing version stops them, so the interrupted release is published
again first.

## Workspaces

Each crate of a workspace is its own publisher. Order them with
[`after`](/reference/options/#publishers.cargo.name.after) along their path dependencies, so a
crate is uploaded only once the crates it depends on are on the index. One derivation can package
the whole workspace, and each publisher takes its archive from it:

```nix presswerk-module
{ config, ... }:
let
  # `cargo package --offline --no-verify --workspace`, with every `.crate` in the output.
  crates = self.packages.x86_64-linux.crates;
in
{
  publishers.cargo.my-core = {
    name = "my-core";
    crate = "${crates}/my-core-${config.version}.crate";
    index = "sparse+https://index.crates.io/";
    token.env = "CARGO_REGISTRY_TOKEN";
  };
  publishers.cargo.my-cli = {
    name = "my-cli";
    crate = "${crates}/my-cli-${config.version}.crate";
    index = "sparse+https://index.crates.io/";
    token.env = "CARGO_REGISTRY_TOKEN";
    after = [ "cargo.my-core" ];
  };
}
```

A [Cargo workspace version file](/reference/version-files/) keeps every member that inherits the
workspace version on the release version.

## Claims

Each publisher claims `<name>@{version}` on its index. crates.io treats names that differ only in
case or in `-` and `_` as the same crate, so the claim does too: two publishers with the names
`my-crate` and `My_Crate` on one index fail module evaluation.

## Credentials

`token` names the environment variable that holds the registry's API token, read at run time. A
crates.io token needs the `publish-update` scope, and `publish-new` for a crate's first version. Secrets never enter the declaration
or Nix store and are never printed.

`publish` always needs the token. Indexes are read anonymously, unless the index's `config.json`
requires authentication, as private registries can; Presswerk then reads it with the token, as
Cargo does. Trusted publishing, without a long-lived token, is planned.

## Options

<!-- reference:options -->
