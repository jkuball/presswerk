{
  perSystem =
    { config, pkgs, self', ... }:
    {
      rust-project.crates.presswerk.crane.args.meta.mainProgram = "presswerk";

      packages.default = self'.packages.presswerk;

      checks.presswerk-test = config.rust-project.crane-lib.cargoTest (
        config.rust-project.crates.presswerk.crane.args
        // {
          inherit (config.rust-project) src;
          cargoArtifacts = self'.packages.presswerk.cargoArtifacts;
          cargoExtraArgs = "-p presswerk";
          nativeBuildInputs = [ pkgs.gitMinimal ];
          pname = "presswerk";
          strictDeps = true;
        }
      );
    };
}
