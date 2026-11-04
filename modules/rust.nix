{
  perSystem =
    { config
    , lib
    , pkgs
    , self'
    , ...
    }:
    {
      rust-project = {
        src = lib.cleanSourceWith {
          src = ../.;
          filter = path: type:
            lib.hasSuffix "/src/release-notes.md.j2" path
            || config.rust-project.crane-lib.filterCargoSources path type;
        };

        crates.presswerk.crane.args.meta.mainProgram = "presswerk";
        crates.presswerk.crane.args.nativeBuildInputs = [ pkgs.gitMinimal ];
      };

      packages.default = self'.packages.presswerk;

      checks.presswerk-test = config.rust-project.crane-lib.cargoTest (
        config.rust-project.crates.presswerk.crane.args
        // {
          inherit (config.rust-project) src;
          cargoArtifacts = self'.packages.presswerk.cargoArtifacts;
          cargoExtraArgs = "-p presswerk";
          pname = "presswerk";
          strictDeps = true;
        }
      );
    };
}
