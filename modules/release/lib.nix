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

      versionFiles = lib.concatLists (lib.mapAttrsToList
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
        releaseConfig.versionFiles.cargo);

      tag.format = releaseConfig.tag.format;
      analyzer = {
        convention = releaseConfig.analyzer.convention;
        gitmoji.catalog = lib.mapAttrs'
          (
            _identifier: entry:
              lib.nameValuePair entry.shortcode (builtins.removeAttrs entry [ "shortcode" ])
          )
          (lib.filterAttrs (_identifier: entry: entry != null) releaseConfig.analyzer.gitmoji.catalog);
      };
      changelog.path = releaseConfig.changelog.path;
      publishers.github-release = releaseConfig.publishers.github-release;
    };
}
