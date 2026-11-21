{
  flake.modules.presswerk.default =
    { lib, publisherLib, ... }:
    let
      inherit (lib) mkOption types;

      # crates.io treats names that differ only in case or in `-` and `_` as the same crate.
      canonicalName = name: lib.toLower (builtins.replaceStrings [ "_" ] [ "-" ] name);

      cargoModule = { config, ... }: {
        imports = [ (publisherLib.instance { type = "cargo"; }) ];

        options = {
          name = mkOption {
            type = types.strMatching "[A-Za-z0-9_-]+";
            example = "my-crate";
            description = "The crate's name on the index. The packaged `Cargo.toml` must declare the same name.";
          };

          crate = mkOption {
            type = publisherLib.artifactFile;
            example = lib.literalExpression "self.packages.x86_64-linux.my-crate-package";
            description = "The packaged crate: a `.crate` file, or a derivation whose output holds exactly one, such as one that runs `cargo package`. Its name must be `name` and its version the release version. `publish` fetches it from the local store or a substituter without building.";
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
          declaration = { inherit (config) name crate index token; };
          # One version of a crate per index.
          claims = [{
            destination = config.index;
            key = "${canonicalName config.name}@{version}";
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
            default = {
              name = "my-crate";
              crate = self.packages.x86_64-linux.my-crate-package;
              index = "sparse+https://index.crates.io/";
              token.env = "CARGO_REGISTRY_TOKEN";
            };
          }
        '';
        description = "Cargo publishers, one per crate.";
      };
    };
}
