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
      inherit (releaseConfig) version stable;

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
      ++ lib.concatLists
        (lib.mapAttrsToList
          (
            _name: workspace:
              [
                {
                  path = workspace.cargoToml;
                  toml = "workspace.package.version";
                }
                {
                  path = workspace.cargoLock;
                  cargoWorkspace = workspace.cargoToml;
                }
              ]
          )
          releaseConfig.versionFiles.cargoWorkspace)
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
      changelog = releaseConfig.changelog;
      # Each type decides what its instances write into the declaration; `after` and `claims` are
      # shared.
      publishers = lib.mapAttrs
        (_type: lib.mapAttrs (_name: publisher: publisher.declaration // { after = publisher.resolvedAfter; inherit (publisher) claims; }))
        (builtins.removeAttrs releaseConfig.publishers releaseConfig.publisherAliases);
    }
    # Only a declared trust is written, so declarations without one keep their shape.
    // lib.optionalAttrs (releaseConfig.trustedPredecessor != null) {
      trustedPredecessor = { inherit (releaseConfig.trustedPredecessor) tag commit; };
    };
}
