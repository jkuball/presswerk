{
  flake.modules.presswerk.default =
    { config, lib, publisherLib, ... }:
    let
      inherit (lib) mkOption types;

      # crates.io treats names that differ only in case or in `-` and `_` as the same crate.
      canonicalName = name: lib.toLower (builtins.replaceStrings [ "_" ] [ "-" ] name);

      cargoModule = { config, name, ... }: {
        imports = [ (publisherLib.instance { type = "cargo"; }) ];

        options = {
          crate = mkOption {
            type = publisherLib.artifactFile;
            example = lib.literalExpression "self.packages.x86_64-linux.my-crate-package";
            description = "The packaged crate: a `.crate` file, or a derivation whose output holds exactly one, such as one that runs `cargo package`. Its version must be the release version and its name the publisher's name. `publish` fetches it from the local store or a substituter without building.";
          };

          index = mkOption {
            type = types.strMatching "sparse[+]https?://[^/?#@[:space:]]+(/[^?#[:space:]]*)?/";
            example = "sparse+https://index.crates.io/";
            description = "Sparse index of the registry, ending in a slash, such as `sparse+https://index.crates.io/` for crates.io. The publish API is the `api` URL in the index's `config.json`.";
          };

          token = mkOption {
            type = publisherLib.credential { what = "registry token"; secretOnly = true; };
            example = { env = "CARGO_REGISTRY_TOKEN"; };
            description = "Registry API token, such as `{ env = \"CARGO_REGISTRY_TOKEN\"; }`, read from an environment variable at run time. Indexes that need authentication are read with it too.";
          };
        };

        config = {
          declaration = { inherit name; inherit (config) crate index token; };
          # One version of a crate per index.
          claims = [{
            destination = config.index;
            key = "${canonicalName name}@{version}";
          }];
        };
      };
    in
    {
      options.publishers.cargo = mkOption {
        type = types.attrsOf (types.submodule cargoModule);
        default = { };
        example = lib.literalExpression ''
          {
            my-crate = {
              crate = self.packages.x86_64-linux.my-crate-package;
              index = "sparse+https://index.crates.io/";
              token.env = "CARGO_REGISTRY_TOKEN";
            };
          }
        '';
        description = "Cargo publishers, keyed by the name of the crate each one uploads.";
      };

      config.assertions = map
        (name: {
          assertion = builtins.match "[A-Za-z0-9_-]+" name != null;
          message = "publishers.cargo.${builtins.toJSON name} must be named after its crate: ASCII letters, digits, `-`, and `_`";
        })
        (builtins.attrNames config.publishers.cargo);
    };
}
