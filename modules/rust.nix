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
            || lib.hasSuffix "/src/badge.json" path
            || config.rust-project.crane-lib.filterCargoSources path type;
        };

        crates.presswerk.crane.args = {
          meta = {
            description = "Controlled releases for software built with Nix flakes";
            homepage = "https://presswerk.dev/";
            license = with lib.licenses; [ mit asl20 ];
            mainProgram = "presswerk";
          };
          nativeBuildInputs = [ pkgs.gitMinimal ];
          nativeCheckInputs = [ pkgs.nix ];
          preCheck = ''
            export XDG_CACHE_HOME="$TMPDIR/cache"
          '';
        };
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
