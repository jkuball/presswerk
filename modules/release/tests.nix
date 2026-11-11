{ config, inputs, lib, self, ... }:
let
  mkRelease = config.flake.lib.mkRelease;
  sourceRoot = self.outPath;
  cargoTomlFixture = builtins.toFile "Cargo.toml" ''
    [package]
    version = "1.2.3"
  '';
  otherTomlFixture = builtins.toFile "Other.toml" ''
    [package]
    version = "2.0.0"
  '';
  cargoLockFixture = builtins.toFile "Cargo.lock" "";
  pyprojectFixture = builtins.toFile "pyproject.toml" ''
    [project]
    version = "1.2.3"
  '';
  packageJsonFixture = builtins.toFile "package.json" ''{ "nested": { "version": "1.2.3" } }'';
  versionFixture = builtins.toFile "VERSION" "1.2.3\n";
  fixtureRoot = builtins.dirOf cargoTomlFixture;
  cargoToml = builtins.baseNameOf cargoTomlFixture;
  otherToml = builtins.baseNameOf otherTomlFixture;
  cargoLock = builtins.baseNameOf cargoLockFixture;
  presswerkVersion = (builtins.fromTOML (builtins.readFile (self.outPath + "/Cargo.toml"))).package.version;
  vendoredCatalog = builtins.fromJSON (builtins.readFile ./gitmoji/catalog.json);
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
      inherit cargoToml;
      inherit cargoLock;
    };
    changelog.path = "docs/NEWS.md";
  };
in
{
  flake.tests.release-declaration = {
    "test defaults normalize to plain declaration data" = {
      expr = defaults;
      expected = {
        schemaVersion = 0;
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
        tag.format = "v{version}";
        analyzer = {
          convention = "gitmoji";
          gitmoji.catalog = vendoredCatalog;
        };
        changelog.path = "CHANGELOG.md";
        publishers.github-release.default = {
          apiUrl = "https://api.github.com";
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
            path = cargoToml;
            toml = "package.version";
          }
          {
            path = cargoLock;
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
        versionFiles.cargo.example.cargoToml = cargoToml;
      }).version;
      expected = "1.2.3";
    };

    "test conflicting version sources fail" = {
      expr = succeeds (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.first.cargoToml = cargoToml;
        versionFiles.cargo.second.cargoToml = otherToml;
      });
      expected = false;
    };

    "test badges normalize to written-only version files" = {
      expr = (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.example.cargoToml = cargoToml;
        versionFiles.badge = {
          default = { };
          dark = {
            colorScheme = "dark";
            path = "docs/version.svg";
          };
        };
      }).versionFiles;
      expected = [
        {
          path = cargoToml;
          toml = "package.version";
        }
        {
          path = "Cargo.lock";
          cargoLock = "example";
        }
        {
          path = "docs/version.svg";
          badge.colorScheme = "dark";
        }
        {
          path = ".github/presswerk-badge.svg";
          badge.colorScheme = "auto";
        }
      ];
    };

    "test TOML, JSON, and plain files define the version and normalize" = {
      expr =
        let
          release = mkRelease {
            sourceRoot = fixtureRoot;
            versionFiles = {
              toml.pyproject = {
                path = builtins.baseNameOf pyprojectFixture;
                key = "project.version";
              };
              json.package = {
                path = builtins.baseNameOf packageJsonFixture;
                key = "nested.version";
              };
              plain.default.path = builtins.baseNameOf versionFixture;
            };
          };
        in
        {
          inherit (release) version;
          kinds = map (file: builtins.attrNames (builtins.removeAttrs file [ "path" ])) release.versionFiles;
        };
      expected = {
        version = "1.2.3";
        kinds = [ [ "toml" ] [ "json" ] [ "plain" ] ];
      };
    };

    "test a disagreeing JSON file fails" = {
      expr = succeeds (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.example.cargoToml = otherToml;
        versionFiles.json.package = {
          path = builtins.baseNameOf packageJsonFixture;
          key = "nested.version";
        };
      }).version;
      expected = false;
    };

    "test regex files normalize without defining the version" = {
      expr =
        let
          release = mkRelease {
            sourceRoot = fixtureRoot;
            versionFiles.cargo.example.cargoToml = cargoToml;
            versionFiles.regex.chart = {
              path = "Chart.yaml";
              pattern = "^version: (?<version>\\S+)$";
            };
          };
        in
        {
          inherit (release) version;
          regex = builtins.elemAt release.versionFiles 2;
        };
      expected = {
        version = "1.2.3";
        regex = {
          path = "Chart.yaml";
          regex = "^version: (?<version>\\S+)$";
        };
      };
    };

    "test a badge does not define the version" = {
      expr = succeeds (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.badge.default = { };
      }).version;
      expected = false;
    };

    "test schemaVersion cannot be authored" = {
      expr = succeeds (mkRelease {
        inherit sourceRoot;
        schemaVersion = 3;
        version = "1.2.3";
      });
      expected = false;
    };

    "test Gitmoji catalog merges overrides and additions" = {
      expr =
        let
          declaration = mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            analyzer.gitmoji.catalog = {
              memo.semver = "patch";
              flying_saucer = {
                shortcode = "flying-saucer";
                emoji = "🛸";
                description = "Experimental feature behind a flag.";
                semver = null;
              };
            };
          };
        in
        {
          memo = declaration.analyzer.gitmoji.catalog.memo;
          flying_saucer = declaration.analyzer.gitmoji.catalog."flying-saucer";
        };
      expected = {
        memo = vendoredCatalog.memo // { semver = "patch"; };
        flying_saucer = {
          emoji = "🛸";
          description = "Experimental feature behind a flag.";
          semver = null;
        };
      };
    };

    "test Gitmoji shortcode defaults to the entry identifier" = {
      expr = (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        analyzer.gitmoji.catalog.space_ship = {
          emoji = "🛰️";
          description = "Operate a satellite.";
          semver = null;
        };
      }).analyzer.gitmoji.catalog.space_ship;
      expected = {
        emoji = "🛰️";
        description = "Operate a satellite.";
        semver = null;
      };
    };

    "test duplicate resolved Gitmoji shortcodes fail" = {
      expr = succeeds (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        analyzer.gitmoji.catalog.other_memo = {
          shortcode = "memo";
          emoji = "📚";
          description = "Duplicate the resolved memo shortcode.";
          semver = null;
        };
      });
      expected = false;
    };

    "test Gitmoji shortcodes must be non-empty and colon-free" = {
      expr = map
        (
          shortcode:
          succeeds (mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            analyzer.gitmoji.catalog.custom = {
              inherit shortcode;
              emoji = "🧿";
              description = "Test invalid shortcode validation.";
              semver = null;
            };
          })
        )
        [
          ""
          ":custom:"
        ];
      expected = [
        false
        false
      ];
    };

    "test Gitmoji catalog can be replaced with mkForce" = {
      expr = builtins.attrNames (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        analyzer.gitmoji.catalog = lib.mkForce {
          memo = vendoredCatalog.memo;
        };
      }).analyzer.gitmoji.catalog;
      expected = [ "memo" ];
    };

    "test Gitmoji catalog entries can be removed with mkForce null" = {
      expr =
        let
          catalog = (mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            analyzer.gitmoji.catalog.memo = lib.mkForce null;
          }).analyzer.gitmoji.catalog;
        in
        {
          memo = catalog ? memo;
          sparkles = catalog ? sparkles;
        };
      expected = {
        memo = false;
        sparkles = true;
      };
    };

    "test Gitmoji emojis must be unique without variation selectors" = {
      expr = succeeds (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        analyzer.gitmoji.catalog.custom_zap = {
          emoji = "⚡";
          description = "Duplicate the upstream zap without its variation selector.";
          semver = null;
        };
      });
      expected = false;
    };

    "test sourceRoot is required" = {
      expr = succeeds (mkRelease {
        version = "1.2.3";
      });
      expected = false;
    };

    "test tag version placeholder with literal surroundings is accepted" = {
      expr = map
        (
          format:
          (mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            tag.format = format;
          }).tag.format
        )
        [
          "release/{version}/source"
          "{release}-{version}-{source}"
        ];
      expected = [
        "release/{version}/source"
        "{release}-{version}-{source}"
      ];
    };

    "test missing tag version placeholder fails" = {
      expr = succeeds (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        tag.format = "release";
      });
      expected = false;
    };

    "test repeated tag version placeholders fail" = {
      expr = succeeds (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        tag.format = "{version}-{version}";
      });
      expected = false;
    };

    "test delimiter-adjacent tag version placeholders fail" = {
      expr = map
        (
          format:
          succeeds (mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            tag.format = format;
          })
        )
        [
          "v{{version}}"
          "v{%raw%}{{version}}{%endraw%}"
          "v{{version}"
          "v{version}}"
        ];
      expected = [
        false
        false
        false
        false
      ];
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

    "test Presswerk self release uses the current schema" = {
      expr = {
        schemaMatches = self.presswerk.default.schemaVersion == defaults.schemaVersion;
        versionMatches = self.presswerk.default.version == presswerkVersion;
      };
      expected = {
        schemaMatches = true;
        versionMatches = true;
      };
    };
  };

  perSystem = {
    # nix-unit re-evaluates the full root flake in a network-disabled build sandbox,
    # so every direct input is overridden to make its source available without fetching.
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
