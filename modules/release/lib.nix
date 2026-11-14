{ config, lib, self, ... }:
let
  version = (builtins.fromTOML (builtins.readFile (self.outPath + "/Cargo.toml"))).package.version;
in
{
  flake.lib.mkRelease =
    releaseModule:
    let
      evaluated = lib.evalModules {
        class = "presswerk";
        modules = [
          config.flake.modules.presswerk.default
          releaseModule
        ];
        specialArgs.presswerk = { inherit version; };
      };
      releaseConfig = evaluated.config;
      failedAssertions = builtins.filter (assertion: !assertion.assertion) releaseConfig.assertions;
      assertionMessage = lib.concatMapStringsSep "\n" (assertion: "- ${assertion.message}") failedAssertions;
    in
    assert builtins.seq releaseConfig.sourceRoot true;
    assert lib.assertMsg (failedAssertions == [ ]) "Release module assertions failed:\n${assertionMessage}";
    {
      schemaVersion = 0;
      inherit (releaseConfig) version;

      versionFiles = lib.concatLists
        (lib.mapAttrsToList
          (
            _name: cargo:
              [
                {
                  path = cargo.cargoToml;
                  toml = "package.version";
                }
                {
                  path = cargo.cargoLock;
                  cargoLock = cargo.crate;
                }
              ]
          )
          releaseConfig.versionFiles.cargo)
      ++ lib.mapAttrsToList
        (_name: file: {
          inherit (file) path;
          toml = file.key;
        })
        releaseConfig.versionFiles.toml
      ++ lib.mapAttrsToList
        (_name: file: {
          inherit (file) path;
          json = file.key;
        })
        releaseConfig.versionFiles.json
      ++ lib.mapAttrsToList
        (_name: file: {
          inherit (file) path;
          plain = { };
        })
        releaseConfig.versionFiles.plain
      ++ lib.mapAttrsToList
        (_name: file: {
          inherit (file) path;
          regex = file.pattern;
        })
        releaseConfig.versionFiles.regex
      ++ lib.mapAttrsToList
        (_name: badge: {
          inherit (badge) path;
          badge = { inherit (badge) colorScheme; };
        })
        releaseConfig.versionFiles.badge;

      tag.format = releaseConfig.tag.format;
      analyzer =
        if releaseConfig.analyzer.conventional.enable then {
          convention = "conventional";
          conventional.catalog = lib.filterAttrs (_type: entry: entry != null) releaseConfig.analyzer.conventional.catalog;
        } else {
          convention = "gitmoji";
          gitmoji.catalog = lib.mapAttrs'
            (
              _identifier: entry:
                lib.nameValuePair entry.shortcode (builtins.removeAttrs entry [ "shortcode" ])
            )
            (lib.filterAttrs (_identifier: entry: entry != null) releaseConfig.analyzer.gitmoji.catalog);
        };
      changelog.path = releaseConfig.changelog.path;
      publishers.github-release = releaseConfig.publishers.github-release;
      publishers.oci = lib.mapAttrs
        (_name: oci: oci // {
          compression =
            if oci.compression.zstd.enable then { zstd.level = oci.compression.zstd.level; }
            else { gzip.level = oci.compression.gzip.level; };
          username = lib.filterAttrs (_source: value: value != null) oci.username;
        })
        releaseConfig.publishers.oci;
      publishers.helm = lib.mapAttrs
        (_name: helm: helm // {
          username = lib.filterAttrs (_source: value: value != null) helm.username;
        })
        releaseConfig.publishers.helm;
    };
}
