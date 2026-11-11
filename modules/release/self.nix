{ inputs, self, ... }:
{
  flake.presswerk.default = inputs.presswerk-release.lib.mkRelease {
    sourceRoot = self.outPath;
    versionFiles.cargo.presswerk = { };
    publishers.github-release.default.repository = "jkuball/presswerk";
    analyzer.gitmoji.catalog.memo.semver = "patch";
  };

  perSystem = { system, ... }: {
    packages.presswerk-release = inputs.presswerk-release.packages.${system}.presswerk;
  };
}
