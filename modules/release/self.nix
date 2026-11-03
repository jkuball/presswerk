{ config, self, ... }:
{
  flake.presswerk.default = config.flake.lib.mkRelease {
    sourceRoot = self.outPath;
    versionFiles.cargo.presswerk = { };
    publishers.github-release.default.repository = "jkuball/presswerk";
  };
}
