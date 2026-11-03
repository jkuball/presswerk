{ lib, standardModule, version }:
releaseModule:
let
  evaluated = lib.evalModules {
    class = "presswerk";
    modules = [
      standardModule
      releaseModule
    ];
    specialArgs.presswerk = { inherit version; };
  };
  config = evaluated.config;
  failedAssertions = builtins.filter (assertion: !assertion.assertion) config.assertions;
  assertionMessage = lib.concatMapStringsSep "\n" (assertion: "- ${assertion.message}") failedAssertions;
in
assert builtins.seq config.sourceRoot true;
assert lib.assertMsg (failedAssertions == [ ]) "Release module assertions failed:\n${assertionMessage}";
{
  schemaVersion = 1;
  inherit (config) version;

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
    config.versionFiles.cargo);

  tag.format = config.tag.format;
  analyzer.convention = config.analyzer.convention;
  changelog.path = config.changelog.path;
  publishers.github-release = config.publishers.github-release;
}
