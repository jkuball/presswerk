{ config, self, ... }:
{
  # Presswerk releases itself with the module and CLI of the commit being released.
  flake.presswerk.default = config.flake.lib.mkRelease {
    sourceRoot = self.outPath;
    versionFiles.cargo.presswerk = { };
    # The README badge follows the viewer's colour scheme; the docs show the variant for their theme.
    versionFiles.badge = {
      default = { };
      light = {
        colorScheme = "light";
        path = ".github/presswerk-badge-light.svg";
      };
      dark = {
        colorScheme = "dark";
        path = ".github/presswerk-badge-dark.svg";
      };
    };
    publishers.github-release.default = {
      repository = "jkuball/presswerk";
      # The CLI links Nix libraries from /nix/store, so it runs where Nix is installed.
      assets."presswerk-x86_64-linux" = "${self.packages.x86_64-linux.presswerk}/bin/presswerk";
    };
    analyzer.gitmoji.catalog.memo.semver = "patch";
  };
}
