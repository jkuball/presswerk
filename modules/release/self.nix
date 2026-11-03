{ config, lib, self, ... }:
let
  version = (builtins.fromTOML (builtins.readFile ../../Cargo.toml)).package.version;
in
{
  flake = {
    presswerkModules = config.flake.modules.presswerk;

    presswerk.default = config.flake.lib.mkRelease {
      sourceRoot = ../..;
      versionFiles.cargo.presswerk = { };
      publishers.github-release.default.repository = "jkuball/presswerk";
    };
  };

  perSystem =
    { pkgs, self', ... }:
    let
      declaration = import ../../tests/nix/release-declaration.nix {
        inherit (lib) evalModules;
        mkRelease = config.flake.lib.mkRelease;
        presswerkModule = self.presswerkModules.default;
        presswerkVersion = version;
        sourceRoot = ../..;
      };
      checkedDeclarations = builtins.deepSeq declaration (builtins.deepSeq self.presswerk.default {
        fixture = declaration;
        presswerk = self.presswerk.default;
      });
    in
    {
      checks.release-declaration =
        assert declaration.version == self'.packages.presswerk.version;
        assert self.presswerk.default.schemaVersion == 1;
        assert self.presswerk.default.version == self'.packages.presswerk.version;
        pkgs.writeText "release-declaration-check" (builtins.toJSON checkedDeclarations);
    };
}
