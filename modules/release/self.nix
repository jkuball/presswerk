{ config, self, ... }:
{
  flake = {
    presswerkModules = config.flake.modules.presswerk;

    presswerk.default = config.flake.lib.mkRelease {
      sourceRoot = self.outPath;
      versionFiles.cargo.presswerk = { };
      publishers.github-release.default.repository = "jkuball/presswerk";
    };
  };
}
