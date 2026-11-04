{
  perSystem =
    { config
    , lib
    , pkgs
    , self'
    , ...
    }:
    let
      nixLibraries = with pkgs.nix.libs; [
        nix-util
        nix-util-c.dev
        nix-store
        nix-store-c.dev
        nix-expr
        nix-expr-c.dev
        nix-fetchers
        nix-fetchers-c.dev
        nix-flake
        nix-flake-c.dev
      ];
      nixBindingsEnvironment = {
        LIBCLANG_PATH = lib.makeLibraryPath [ pkgs.llvmPackages.clang-unwrapped ];
        BINDGEN_EXTRA_CLANG_ARGS = "-x c++ -std=c++2a";
        PKG_CONFIG_PATH = lib.makeSearchPath "lib/pkgconfig" ([ pkgs.boehmgc.dev ] ++ nixLibraries);
        LIBRARY_PATH = lib.makeLibraryPath ([ pkgs.boehmgc ] ++ nixLibraries);
      };
    in
    {
      rust-project = {
        src = lib.cleanSourceWith {
          src = ../.;
          filter = path: type:
            lib.hasSuffix "/src/release-notes.md.j2" path
            || config.rust-project.crane-lib.filterCargoSources path type;
        };

        crates.presswerk.crane.args = nixBindingsEnvironment // {
          meta.mainProgram = "presswerk";
          nativeBuildInputs = with pkgs; [ pkg-config makeWrapper gitMinimal ];
          buildInputs = [ pkgs.boehmgc ] ++ nixLibraries;
          preCheck = ''
            export XDG_CACHE_HOME="$TMPDIR/cache"
          '';
        };
      };

      packages.default = self'.packages.presswerk;

      devshells.default = {
        packages = [ pkgs.pkg-config pkgs.boehmgc ] ++ nixLibraries;
        env = lib.mapAttrsToList (name: value: { inherit name value; }) nixBindingsEnvironment;
      };

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
