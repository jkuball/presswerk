{ config, inputs, lib, self, ... }:
let
  mkBareRelease = config.flake.lib.mkRelease;
  # Most tests are about other options, so they analyze Gitmoji unless they declare an analyzer.
  mkRelease = module: mkBareRelease {
    imports = [
      module
      ({ config, ... }: {
        analyzer.gitmoji.enable = lib.mkIf (!config.analyzer.conventional.enable) (lib.mkDefault true);
      })
    ];
  };
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
  conventionalCatalog = builtins.fromJSON (builtins.readFile ./conventional/catalog.json);
  # The message of the first failed assertion for a release module with these settings.
  failedAssertion = settings: (lib.findFirst (assertion: !assertion.assertion) null
    (lib.evalModules {
      class = "presswerk";
      modules = [
        config.flake.modules.presswerk.default
        ({
          inherit sourceRoot;
          version = "1.2.3";
        } // settings)
      ];
      specialArgs.presswerk.version = presswerkVersion;
    }).config.assertions).message;
  succeeds = value: (builtins.tryEval (builtins.deepSeq value true)).success;

  publisher = {
    publishers.github-release.default.repository = "owner/project";
  };

  image = derivation {
    name = "image.tar.gz";
    system = "x86_64-linux";
    builder = "/bin/sh";
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
          assets = { };
        };
        publishers.oci = { };
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

    "test GitHub Release assets normalize to store paths" = {
      expr =
        let
          tool = derivation {
            name = "tool";
            system = "x86_64-linux";
            builder = "/bin/sh";
          };
        in
        (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          publishers.github-release.default = {
            repository = "owner/project";
            assets = {
              file = "${tool}/bin/tool";
              whole = tool;
            };
          };
        }).publishers.github-release.default.assets;
      expected =
        let
          out = (derivation {
            name = "tool";
            system = "x86_64-linux";
            builder = "/bin/sh";
          }).outPath;
        in
        {
          file = "${out}/bin/tool";
          whole = out;
        };
    };

    "test GitHub Release assets need a flake output" = {
      expr = succeeds (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.example.cargoToml = cargoToml;
        publishers.github-release.default = {
          repository = "owner/project";
          assets.fake = "/nix/store/00000000000000000000000000000000-fake/bin/fake";
        };
      });
      expected = false;
    };

    "test OCI images normalize to store paths with default tags" = {
      expr =
        (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.oci.default = {
            repository = "ghcr.io/owner/project";
            images."linux/amd64" = image;
            username.secret = "REGISTRY_USERNAME";
            password.secret = "REGISTRY_TOKEN";
          };
        }).publishers.oci.default;
      expected = {
        repository = "ghcr.io/owner/project";
        images."linux/amd64" = image.outPath;
        tags = [ "{version}" ];
        public = false;
        username.secret = "REGISTRY_USERNAME";
        password.secret = "REGISTRY_TOKEN";
      };
    };

    "test OCI publishers reject invalid declarations" = {
      expr = map
        (oci: succeeds (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.oci.default = {
            repository = "ghcr.io/owner/project";
            images."linux/amd64" = image;
            username.secret = "REGISTRY_USERNAME";
            password.secret = "REGISTRY_TOKEN";
          } // oci;
        }).publishers.oci)
        [
          { }
          { repository = "localhost:5000/project"; }
          { tags = [ "{version}" "latest" ]; }
          { public = true; }
          { repository = "[::1]:5000/project"; }
          { repository = "europe-docker.pkg.dev/project/repo/image"; }
          # Without a registry host.
          { repository = "owner/project"; }
          { repository = "ghcr.io/Owner/project"; }
          # Credentials, queries, and fragments never enter the declaration.
          { repository = "user:secret@ghcr.io/owner/project"; }
          { repository = "token@ghcr.io/owner/project"; }
          { repository = "ghcr.io/owner/project?x=1"; }
          { repository = "ghcr.io/owner/project#fragment"; }
          { repository = "ghcr.io?x=1/owner/project"; }
          { images = { }; }
          { images."linux/amd64" = "/nix/store/00000000000000000000000000000000-fake.tar.gz"; }
          # Every release needs a tag of its own.
          { tags = [ "latest" ]; }
          { tags = [ "{version}" "{version}" ]; }
          { tags = [ "{version}" "{major}" ]; }
          { tags = [ "-{version}" ]; }
        ];
      expected = [ true true true true true true false false false false false false false false false false false false false ];
    };

    "test a release needs no publishers" = {
      expr = (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.example.cargoToml = cargoToml;
      }).publishers;
      expected = {
        github-release = { };
        oci = { };
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

    "test the Conventional Commits analyzer normalizes" = {
      expr = (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        analyzer.conventional.enable = true;
      }).analyzer;
      expected = {
        convention = "conventional";
        conventional.catalog = conventionalCatalog;
      };
    };

    "test the default Conventional Commits catalog" = {
      expr = lib.mapAttrs (_type: entry: entry.semver) conventionalCatalog;
      expected = {
        build = null;
        chore = null;
        ci = null;
        docs = null;
        feat = "minor";
        fix = "patch";
        perf = "patch";
        refactor = null;
        revert = "patch";
        style = null;
        test = null;
      };
    };

    "test Conventional Commits catalog merges overrides, additions, and removals" = {
      expr =
        let
          catalog = (mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            analyzer.conventional.enable = true;
            analyzer.conventional.catalog = {
              docs.semver = "patch";
              deps = {
                title = "Dependencies";
                semver = "patch";
              };
              style = lib.mkForce null;
            };
          }).analyzer.conventional.catalog;
        in
        {
          inherit (catalog) docs deps feat;
          style = catalog ? style;
        };
      expected = {
        docs = conventionalCatalog.docs // { semver = "patch"; };
        deps = {
          title = "Dependencies";
          semver = "patch";
        };
        feat = conventionalCatalog.feat;
        style = false;
      };
    };

    "test Conventional Commits catalog can be replaced with mkForce" = {
      expr = builtins.attrNames (mkRelease {
        inherit sourceRoot;
        version = "1.2.3";
        analyzer.conventional.enable = true;
        analyzer.conventional.catalog = lib.mkForce {
          feat = conventionalCatalog.feat;
        };
      }).analyzer.conventional.catalog;
      expected = [ "feat" ];
    };

    "test Conventional Commits types must be lowercase names" = {
      expr = map
        (
          type:
          succeeds (mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            analyzer.conventional.enable = true;
            analyzer.conventional.catalog.${type} = {
              title = "Custom";
              semver = null;
            };
          })
        )
        [
          "Feat"
          "fix(cli)"
          "breaking!"
          "deps-dev"
        ];
      expected = [
        false
        false
        false
        true
      ];
    };

    "test each analyzer alone is accepted" = {
      expr = map
        (
          analyzer:
          (mkBareRelease {
            inherit sourceRoot;
            version = "1.2.3";
            analyzer.${analyzer}.enable = true;
          }).analyzer.convention
        )
        [
          "gitmoji"
          "conventional"
        ];
      expected = [
        "gitmoji"
        "conventional"
      ];
    };

    "test a release without an enabled analyzer fails" = {
      expr = succeeds (mkBareRelease {
        inherit sourceRoot;
        version = "1.2.3";
        # Configuring an analyzer does not enable it.
        analyzer.gitmoji.catalog.memo.semver = "patch";
      });
      expected = false;
    };

    "test a missing analyzer is reported" = {
      expr = failedAssertion { };
      expected = "enable a commit analyzer, for example analyzer.gitmoji.enable = true;";
    };

    "test two enabled analyzers fail" = {
      expr = succeeds (mkBareRelease {
        inherit sourceRoot;
        version = "1.2.3";
        analyzer.gitmoji.enable = true;
        analyzer.conventional.enable = true;
      });
      expected = false;
    };

    "test two enabled analyzers are named in the assertion" = {
      expr = failedAssertion {
        analyzer.gitmoji.enable = true;
        analyzer.conventional.enable = true;
      };
      expected = "enable exactly one commit analyzer; enabled: analyzer.gitmoji, analyzer.conventional";
    };

    "test a configured but disabled analyzer is ignored" = {
      expr = (mkBareRelease {
        inherit sourceRoot;
        version = "1.2.3";
        analyzer.conventional.enable = true;
        # Invalid as an enabled catalog: the shortcode repeats memo.
        analyzer.gitmoji.catalog.other_memo = {
          shortcode = "memo";
          emoji = "📚";
          description = "Duplicate the resolved memo shortcode.";
          semver = "major";
        };
      }).analyzer;
      expected = {
        convention = "conventional";
        conventional.catalog = conventionalCatalog;
      };
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
