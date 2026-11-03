{
  perSystem =
    { config, pkgs, self', ... }:
    {
      treefmt.programs = {
        deadnix.enable = true;
        nixpkgs-fmt.enable = true;
        rustfmt.enable = true;
      };

      pre-commit.settings = {
        package = pkgs.prek;
        hooks.treefmt = {
          enable = true;
          package = self'.formatter;
        };
      };

      devshells.default = {
        packages = [
          config.rust-project.toolchain
          pkgs.prek
        ];

        devshell.startup.install-pre-commit.text = config.pre-commit.installationScript;
      };
    };
}
