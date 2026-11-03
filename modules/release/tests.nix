{ config, inputs, lib, self, ... }:
let
  mkRelease = config.flake.lib.mkRelease;
  sourceRoot = self.outPath;
  fixtureRoot = self.outPath + "/tests/fixtures/release";
  presswerkVersion = (builtins.fromTOML (builtins.readFile (self.outPath + "/Cargo.toml"))).package.version;
  succeeds = value: (builtins.tryEval (builtins.deepSeq value true)).success;

  publisher = {
    publishers.github-release.default.repository = "owner/project";
  };

  defaults = mkRelease {
    inherit sourceRoot;
    imports = [ publisher ];
    versionFiles.cargo.presswerk = { };
  };

  overrides = mkRelease {
    sourceRoot = fixtureRoot;
    imports = [ publisher ];
    versionFiles.cargo.example = {
      crate = "actual-crate";
      cargoToml = "nested/Cargo.toml";
      cargoLock = "nested/Cargo.lock";
    };
    changelog.path = "docs/NEWS.md";
  };
in
{
  flake.tests.release-declaration = {
    "test defaults normalize to plain declaration data" = {
      expr = defaults;
      expected = {
        schemaVersion = 1;
        version = presswerkVersion;
        versionFiles = [
          {
            path = "Cargo.toml";
            toml = "package.version";
          }
          {
            path = "Cargo.lock";
            cargoLock = "presswerk";
          }
        ];
        tag.format = "v{{ version }}";
        analyzer.convention = "gitmoji";
        changelog.path = "CHANGELOG.md";
        publishers.github-release.default = {
          repository = "owner/project";
          token.secret = "GITHUB_TOKEN";
        };
      };
    };

    "test sourceRoot is omitted from normalized data" = {
      expr = defaults ? sourceRoot;
      expected = false;
    };

    "test Cargo overrides normalize both writers" = {
      expr = {
        inherit (overrides) version versionFiles changelog;
      };
      expected = {
        version = "1.2.3";
        versionFiles = [
          {
            path = "nested/Cargo.toml";
            toml = "package.version";
          }
          {
            path = "nested/Cargo.lock";
            cargoLock = "actual-crate";
          }
        ];
        changelog.path = "docs/NEWS.md";
      };
    };

    "test matching version sources merge" = {
      expr = (mkRelease {
        sourceRoot = fixtureRoot;
        version = "1.2.3";
        versionFiles.cargo.example.cargoToml = "nested/Cargo.toml";
      }).version;
      expected = "1.2.3";
    };

    "test conflicting version sources fail" = {
      expr = succeeds (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.first.cargoToml = "nested/Cargo.toml";
        versionFiles.cargo.second.cargoToml = "nested/Other.toml";
      });
      expected = false;
    };

    "test schemaVersion cannot be authored" = {
      expr = succeeds (mkRelease {
        inherit sourceRoot;
        schemaVersion = 2;
        version = "1.2.3";
      });
      expected = false;
    };

    "test sourceRoot is required" = {
      expr = succeeds (mkRelease {
        version = "1.2.3";
      });
      expected = false;
    };

    "test compact tag version expression is accepted" = {
      expr = (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        tag.format = "v{{version}}";
      }).tag.format;
      expected = "v{{version}}";
    };

    "test wide tag version expression is accepted" = {
      expr = (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        tag.format = "v{{\t  version  \t}}";
      }).tag.format;
      expected = "v{{\t  version  \t}}";
    };

    "test missing tag version expression fails" = {
      expr = succeeds (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        tag.format = "release";
      });
      expected = false;
    };

    "test repeated mixed tag version expressions fail" = {
      expr = succeeds (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        tag.format = "{{version}}-{{  version\t}}";
      });
      expected = false;
    };

    "test invalid repository-relative paths fail" = {
      expr = map
        (
          path:
          succeeds (mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            changelog.path = path;
          })
        )
        [
          ""
          "/CHANGELOG.md"
          "../CHANGELOG.md"
          "docs\\CHANGELOG.md"
        ];
      expected = [
        false
        false
        false
        false
      ];
    };

    "test presswerk.version is available to assertions" = {
      expr = (mkRelease ({ presswerk, ... }: {
        inherit sourceRoot;
        version = presswerkVersion;
        assertions = [
          {
            assertion = presswerk.version == presswerkVersion;
            message = "unexpected presswerk.version module argument";
          }
        ];
      })).version;
      expected = presswerkVersion;
    };

    "test failed standard assertions fail evaluation" = {
      expr = succeeds (mkRelease ({ presswerk, ... }: {
        inherit sourceRoot;
        version = presswerkVersion;
        assertions = [
          {
            assertion = presswerk.version != presswerkVersion;
            message = "expected assertion failure";
          }
        ];
      }));
      expected = false;
    };

    "test public standard module evaluates with presswerk class" = {
      expr =
        let
          evaluated = lib.evalModules {
            class = "presswerk";
            modules = [
              self.presswerkModules.default
              {
                inherit sourceRoot;
                version = presswerkVersion;
              }
            ];
          };
        in
        {
          inherit (evaluated.config) version;
          changelog = evaluated.config.changelog.path;
        };
      expected = {
        version = presswerkVersion;
        changelog = "CHANGELOG.md";
      };
    };

    "test wrong module class fails" = {
      expr = succeeds (mkRelease {
        inherit sourceRoot;
        imports = [{ _class = "nixos"; }];
        version = "1.2.3";
      });
      expected = false;
    };

    "test normalized declaration round-trips through JSON" = {
      expr = builtins.fromJSON (builtins.toJSON defaults);
      expected = defaults;
    };

    "test Presswerk self declaration matches schema and Cargo version" = {
      expr = {
        schemaVersion = self.presswerk.default.schemaVersion;
        versionMatches = self.presswerk.default.version == presswerkVersion;
      };
      expected = {
        schemaVersion = 1;
        versionMatches = true;
      };
    };
  };

  perSystem = {
    nix-unit.inputs = {
      inherit (inputs)
        devshell
        flake-parts
        git-hooks
        import-tree
        nix-unit
        nixpkgs
        rust-flake
        treefmt-nix
        ;
    };
  };
}
