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
        PKG_CONFIG_PATH = lib.makeSearchPath "lib/pkgconfig" ([ pkgs.boehmgc.dev ] ++ nixLibraries);
        LIBRARY_PATH = lib.makeLibraryPath ([ pkgs.boehmgc ] ++ nixLibraries);
      };
      configureBindgen = ''
        export BINDGEN_EXTRA_CLANG_ARGS="''${BINDGEN_EXTRA_CLANG_ARGS:-}${lib.optionalString pkgs.stdenv.hostPlatform.isDarwin " -isysroot ${pkgs.apple-sdk.sdkroot}"} -x c++ -std=c++2a"
      '';
      craneLib = config.rust-project.crane-lib;
      presswerk = config.rust-project.crates.presswerk;
      presswerkArgs = presswerk.crane.args // {
        inherit (config.rust-project) src;
        cargoExtraArgs = "-p presswerk";
        pname = "presswerk";
        strictDeps = true;
      };
      # The dependency-only build must not see Presswerk's own version. Otherwise every release
      # commit gets a new presswerk-deps derivation and rebuilds every crate. It builds from a
      # dummy source whose Cargo.toml has no version, which Cargo reads as 0.0.0, and whose
      # Cargo.lock says 0.0.0 for presswerk.
      cargoLock = builtins.fromTOML (builtins.readFile ../Cargo.lock);
      withoutOwnVersion = package:
        if package.name == "presswerk" && !(package ? source)
        then package // { version = "0.0.0"; }
        else package;
      cargoArtifacts = craneLib.buildDepsOnly (
        # Without src: it would bring back the real Cargo.toml and Cargo.lock.
        builtins.removeAttrs presswerkArgs [ "src" ]
        // {
          version = "0.0.0";
          cargoVendorDir = craneLib.vendorCargoDeps presswerkArgs;
          dummySrc = craneLib.mkDummySrc {
            # The real Cargo.lock stays out; the normalized one below takes its place.
            src = lib.cleanSourceWith {
              src = config.rust-project.src;
              filter = path: type:
                craneLib.filterCargoSources path type
                && !lib.hasSuffix "/Cargo.lock" path;
            };
            cargoLock = (pkgs.formats.toml { }).generate "Cargo.lock" (
              cargoLock // { package = map withoutOwnVersion cargoLock.package; }
            );
            cleanCargoTomlFilter = path:
              path != [ "package" "version" ]
              && craneLib.filters.cargoTomlDefault path;
          };
        }
      );
    in
    {
      rust-project = {
        src = lib.cleanSourceWith {
          src = ../.;
          filter = path: type:
            lib.hasSuffix "/src/release-notes.md.j2" path
            || lib.hasSuffix "/src/badge.json" path
            || config.rust-project.crane-lib.filterCargoSources path type;
        };

        crates.presswerk.crane = {
          args = nixBindingsEnvironment // {
            meta = {
              description = "Controlled releases for software built with Nix flakes";
              homepage = "https://presswerk.dev/";
              license = with lib.licenses; [ mit asl20 ];
              mainProgram = "presswerk";
            };
            nativeBuildInputs = with pkgs; [ rustPlatform.bindgenHook pkg-config makeWrapper gitMinimal ];
            buildInputs = [ pkgs.boehmgc ] ++ nixLibraries;
            postConfigure = configureBindgen;
            preCheck = ''
              export XDG_CACHE_HOME="$TMPDIR/cache"
            '';
          };
          extraBuildArgs = {
            inherit cargoArtifacts;
            # The docs' CLI reference comes from this build rather than from compiling the crate once
            # more. Crane installs the example next to the binary; it leaves as cli.json.
            cargoBuildExtraArgs = "--bins --example cli-reference";
            postInstall = ''
              mkdir -p "$out/share/presswerk"
              "$out/bin/cli-reference" > "$out/share/presswerk/cli.json"
              rm "$out/bin/cli-reference"
            '';
          };
          outputs.drv = {
            clippy = craneLib.cargoClippy (presswerkArgs // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets --all-features -- --deny warnings";
              meta = presswerkArgs.meta // {
                description = "Clippy check for the presswerk crate";
              };
            });
            doc = craneLib.cargoDoc (presswerkArgs // {
              inherit cargoArtifacts;
              RUSTDOCFLAGS = "-D warnings";
              meta = presswerkArgs.meta // {
                description = "Rust docs for the presswerk crate";
              };
            });
          };
        };
      };

      packages.default = self'.packages.presswerk;

      devshells.default = {
        packages = [ pkgs.pkg-config pkgs.boehmgc ] ++ nixLibraries;
        env = lib.mapAttrsToList (name: value: { inherit name value; }) nixBindingsEnvironment;
        devshell.startup.configure-bindgen.text = ''
          source ${pkgs.rustPlatform.bindgenHook}/nix-support/setup-hook
          populateBindgenEnv
          ${configureBindgen}
        '';
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
