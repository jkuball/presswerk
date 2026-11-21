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
  workspaceTomlFixture = builtins.toFile "Workspace.toml" ''
    [workspace]
    members = [ "crates/*" ]

    [workspace.package]
    version = "1.2.3"
  '';
  workspaceToml = builtins.baseNameOf workspaceTomlFixture;
  pyprojectFixture = builtins.toFile "pyproject.toml" ''
    [project]
    version = "1.2.3"
  '';
  packageJsonFixture = builtins.toFile "package.json" ''{ "nested": { "version": "1.2.3" } }'';
  # Holds a version under every key of the TOML dotted-key test inputs.
  packageLockFixture = builtins.toFile "package-lock.json" (builtins.toJSON {
    version = "1.2.3";
    packages = { "".version = "1.2.3"; "a.b".version = "1.2.3"; };
    a = { "x.y".b = "1.2.3"; b = "1.2.3"; };
    "q\"z" = "1.2.3";
    dependencies."@scope/pkg".version = "1.2.3";
    "é" = "1.2.3";
  });
  # The same inputs as the Rust test `parses_keys_as_toml_dotted_keys`.
  validKeys = [ "version" ''packages."".version'' ''packages."a.b".version'' "a.'x.y'.b" ''"q\"z"'' ''dependencies."@scope/pkg".version'' " a . b " ''"\u00e9"'' ];
  malformedKeys = {
    empty = "";
    empty-segment = "a..b";
    trailing-dot = "a.";
    second-line = "a = 1\nb";
    smuggled-value = "a = 0 #";
    bare-at = "@scope";
    unclosed-quote = ''"a'';
    space = "a b";
    table-header = "[a]\nb";
    comment = "a#";
  };
  releaseWithKey = key: mkRelease {
    sourceRoot = fixtureRoot;
    versionFiles.json.package = {
      path = builtins.baseNameOf packageLockFixture;
      inherit key;
    };
  };
  versionFixture = builtins.toFile "VERSION" "1.2.3\n";
  fixtureRoot = builtins.dirOf cargoTomlFixture;
  cargoToml = builtins.baseNameOf cargoTomlFixture;
  otherToml = builtins.baseNameOf otherTomlFixture;
  cargoLock = builtins.baseNameOf cargoLockFixture;
  presswerkVersion = (builtins.fromTOML (builtins.readFile (self.outPath + "/Cargo.toml"))).package.version;
  vendoredCatalog = builtins.fromJSON (builtins.readFile ./gitmoji/catalog.json);
  conventionalCatalog = builtins.fromJSON (builtins.readFile ./conventional/catalog.json);
  defaultChangelogTemplate = builtins.readFile ../../src/release-notes.md.j2;
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

  imagePublisher = {
    repository = "ghcr.io/owner/project";
    platforms."linux/amd64" = image;
    username.env = "REGISTRY_USERNAME";
    password.env = "REGISTRY_TOKEN";
  };

  pythonPublisher = {
    name = "My_Package";
    distributions = image;
    repositoryUrl = "https://upload.pypi.org/legacy/";
    indexUrl = "https://pypi.org/simple";
    username.value = "__token__";
    password.env = "PYPI_TOKEN";
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
        stable = false;
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
        changelog = {
          path = "CHANGELOG.md";
          template = defaultChangelogTemplate;
          heading = "## [{version}] - {date}";
          unreleasedHeading = "## [Unreleased]";
        };
        publishers.github-release.default = {
          after = [ ];
          apiUrl = "https://api.github.com";
          repository = "owner/project";
          body = "{{ notes }}";
          token.env = "GITHUB_TOKEN";
          assets = { };
          claims = [{
            destination = "https://api.github.com/repos/owner/project";
            key = "v{version}";
          }];
        };
        publishers.oci-image = { };
        publishers.oci-helm = { };
        publishers.python = { };
      };
    };

    "test sourceRoot is omitted from normalized data" = {
      expr = defaults ? sourceRoot;
      expected = false;
    };

    "test stable defaults from the version's major component" = {
      expr = map
        (version: (mkRelease { inherit sourceRoot version; }).stable)
        [ "0.4.2" "1.2.3" ];
      expected = [ false true ];
    };

    "test stable can be enabled before version 1" = {
      expr = (mkRelease {
        inherit sourceRoot;
        version = "0.4.2";
        stable = true;
      }).stable;
      expected = true;
    };

    "test stable cannot be disabled from version 1" = {
      expr = map
        (version: failedAssertion {
          inherit version;
          stable = false;
          analyzer.gitmoji.enable = true;
        })
        [ "1.2.3" "1.0.0-rc.1" ];
      expected = [
        "stable may be false only while the version's major component is 0"
        "stable may be false only while the version's major component is 0"
      ];
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
        changelog = {
          path = "docs/NEWS.md";
          template = defaultChangelogTemplate;
          heading = "## [{version}] - {date}";
          unreleasedHeading = "## [Unreleased]";
        };
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

    "test Cargo workspaces read the workspace version and normalize both writers" = {
      expr =
        let
          release = mkRelease {
            sourceRoot = fixtureRoot;
            versionFiles.cargoWorkspace = {
              default.cargoToml = workspaceToml;
              backend = {
                cargoToml = workspaceToml;
                cargoLock = "backend/Cargo.lock";
              };
            };
          };
        in
        { inherit (release) version versionFiles; };
      expected = {
        version = "1.2.3";
        versionFiles = [
          {
            path = workspaceToml;
            toml = "workspace.package.version";
          }
          {
            path = "backend/Cargo.lock";
            cargoWorkspace = workspaceToml;
          }
          {
            path = workspaceToml;
            toml = "workspace.package.version";
          }
          {
            path = "Cargo.lock";
            cargoWorkspace = workspaceToml;
          }
        ];
      };
    };

    "test a Cargo workspace's lock file defaults to the one next to its manifest" = {
      expr = (mkRelease {
        sourceRoot = fixtureRoot;
        # An explicit stable keeps the default, and with it the version files, unevaluated.
        stable = true;
        versionFiles.cargoWorkspace = {
          root = { };
          backend.cargoToml = "services/backend/Cargo.toml";
        };
      }).versionFiles;
      expected = [
        {
          path = "services/backend/Cargo.toml";
          toml = "workspace.package.version";
        }
        {
          path = "services/backend/Cargo.lock";
          cargoWorkspace = "services/backend/Cargo.toml";
        }
        {
          path = "Cargo.toml";
          toml = "workspace.package.version";
        }
        {
          path = "Cargo.lock";
          cargoWorkspace = "Cargo.toml";
        }
      ];
    };

    "test a disagreeing Cargo workspace fails" = {
      expr = succeeds (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.example.cargoToml = otherToml;
        versionFiles.cargoWorkspace.default.cargoToml = workspaceToml;
      }).version;
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

    "test keys parse as TOML dotted keys" = {
      expr =
        let
          release = mkRelease {
            sourceRoot = fixtureRoot;
            versionFiles.json = lib.listToAttrs (lib.imap0
              (index: key: lib.nameValuePair "key${toString index}" {
                path = builtins.baseNameOf packageLockFixture;
                inherit key;
              })
              validKeys);
          };
        in
        {
          inherit (release) version;
          keys = map (file: file.json) release.versionFiles;
        };
      expected = {
        version = "1.2.3";
        keys = validKeys;
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
          publishers.oci-image.default = {
            repository = "ghcr.io/owner/project";
            platforms."linux/amd64" = image;
            username.env = "REGISTRY_USERNAME";
            password.env = "REGISTRY_TOKEN";
          };
        }).publishers.oci-image.default;
      expected = {
        after = [ ];
        repository = "ghcr.io/owner/project";
        platforms."linux/amd64" = image.outPath;
        tags = [ "{version}" ];
        aliases = [ ];
        public = false;
        compression.gzip.level = 6;
        username.env = "REGISTRY_USERNAME";
        password.env = "REGISTRY_TOKEN";
        claims = [{
          destination = "oci://ghcr.io/owner/project";
          key = "{version}";
        }];
      };
    };

    "test OCI Helm charts normalize to store paths with registry authentication" = {
      expr =
        (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.oci-helm.default = {
            repository = "ghcr.io/owner/charts/project";
            chart = image;
            public = true;
            username.env = "REGISTRY_USERNAME";
            password.env = "REGISTRY_TOKEN";
          };
        }).publishers.oci-helm.default;
      expected = {
        after = [ ];
        repository = "ghcr.io/owner/charts/project";
        chart = image.outPath;
        public = true;
        username.env = "REGISTRY_USERNAME";
        password.env = "REGISTRY_TOKEN";
        claims = [{
          destination = "oci://ghcr.io/owner/charts/project";
          key = "{version}";
        }];
      };
    };

    "test OCI Helm publishers reject invalid declarations" = {
      expr = map
        (helm: succeeds (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.oci-helm.default = {
            repository = "ghcr.io/owner/charts/project";
            chart = image;
            username.env = "REGISTRY_USERNAME";
            password.env = "REGISTRY_TOKEN";
          } // helm;
        }).publishers.oci-helm)
        [
          { }
          { public = true; }
          { username = { value = "oauth2accesstoken"; }; }
          { repository = "owner/charts/project"; }
          { repository = "ghcr.io/Owner/project"; }
          { repository = "user:secret@ghcr.io/owner/charts/project"; }
          { chart = "/nix/store/00000000000000000000000000000000-chart.tgz"; }
          { username = { env = "REGISTRY_USERNAME"; value = "user"; }; }
        ];
      expected = [ true true true false false false false false ];
    };

    "test Python publishers normalize to store paths and claim the normalized name" = {
      expr =
        (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.python.default = pythonPublisher;
        }).publishers.python.default;
      expected = {
        after = [ ];
        name = "My_Package";
        distributions = image.outPath;
        repositoryUrl = "https://upload.pypi.org/legacy/";
        indexUrl = "https://pypi.org/simple";
        username.value = "__token__";
        password.env = "PYPI_TOKEN";
        claims = [{
          destination = "https://pypi.org/simple/";
          key = "my-package=={version}";
        }];
      };
    };

    "test Python publishers reject invalid declarations" = {
      expr = map
        (python: succeeds (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.python.default = pythonPublisher // python;
        }).publishers.python)
        [
          { }
          { username.env = "PYPI_USERNAME"; }
          { name = "-my-package"; }
          { name = "my package"; }
          { distributions = "/nix/store/00000000000000000000000000000000-dist"; }
          { repositoryUrl = "https://user:secret@upload.pypi.org/legacy/"; }
          { indexUrl = "ftp://pypi.org/simple/"; }
          { indexUrl = "https://pypi.org/simple/?token=secret"; }
          { password.value = "secret"; }
        ];
      expected = [ true true false false false false false false false ];
    };

    "test Python publishers of one name on one index collide" = {
      expr = map
        (other: succeeds (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.python.default = pythonPublisher;
          publishers.python.other = pythonPublisher // other;
        }).publishers)
        [
          { name = "other"; }
          { indexUrl = "https://test.pypi.org/simple/"; }
          { name = "my.package"; }
          { indexUrl = "https://pypi.org/simple/"; }
          { indexUrl = "HTTPS://PyPI.org:443/simple"; }
          { indexUrl = "https://pypi.org:8443/simple/"; }
          { indexUrl = "http://[::1]:8080/simple"; }
          { indexUrl = "https://pypi.org/proxy/../simple/"; }
          { indexUrl = "https://pypi.org/x/%2E%2E/simple/"; }
          { indexUrl = "https://pypi.org:0443/simple/"; }
        ];
      expected = [ true true false false false true true false false false ];
    };

    "test OCI Helm and OCI image publishers do not share a release tag" = {
      expr = map
        (publishers: succeeds (mkRelease (lib.recursiveUpdate
          {
            sourceRoot = fixtureRoot;
            versionFiles.cargo.example.cargoToml = cargoToml;
            imports = [ publisher ];
            publishers.oci-helm.default = {
              repository = "ghcr.io/owner/charts/project";
              chart = image;
              username.env = "REGISTRY_USERNAME";
              password.env = "REGISTRY_TOKEN";
            };
            publishers.oci-image.default = {
              repository = "ghcr.io/owner/images/project";
              platforms."linux/amd64" = image;
              username.env = "REGISTRY_USERNAME";
              password.env = "REGISTRY_TOKEN";
            };
          }
          publishers)).publishers)
        [
          { }
          {
            publishers.oci-helm.other = {
              repository = "ghcr.io/owner/charts/other";
              chart = image;
              username.env = "REGISTRY_USERNAME";
              password.env = "REGISTRY_TOKEN";
            };
          }
          {
            publishers.oci-helm.other = {
              repository = "ghcr.io/owner/charts/project";
              chart = image;
              username.env = "REGISTRY_USERNAME";
              password.env = "REGISTRY_TOKEN";
            };
          }
          { publishers.oci-image.default.repository = "ghcr.io/owner/charts/project"; }
          {
            publishers.oci-image.default = {
              repository = "ghcr.io/owner/charts/project";
              tags = [ "v{version}" ];
            };
          }
        ];
      expected = [ true true false false true ];
    };

    "test a token is a secret source, never a value or an old shape" = {
      expr = map
        (token: succeeds (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          publishers.github-release.default = {
            repository = "owner/project";
            inherit token;
          };
        }).publishers)
        [
          { env = "PRESSWERK_GITHUB_TOKEN"; }
          { value = "ghp_written-into-the-declaration"; }
          { secret = "GITHUB_TOKEN"; }
          { secret.env = "GITHUB_TOKEN"; }
          { }
          { env = ""; }
          { env = "GITHUB_TOKEN"; value = "token"; }
        ];
      expected = [ true false false false false false false ];
    };

    "test a literal OCI username normalizes without the secret" = {
      expr =
        (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.oci-image.default = {
            repository = "europe-docker.pkg.dev/project/repo/image";
            platforms."linux/amd64" = image;
            username.value = "oauth2accesstoken";
            password.env = "GAR_TOKEN";
          };
        }).publishers.oci-image.default.username;
      expected.value = "oauth2accesstoken";
    };

    "test OCI image publishers reject invalid declarations" = {
      expr = map
        (oci: succeeds (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.oci-image.default = {
            repository = "ghcr.io/owner/project";
            platforms."linux/amd64" = image;
            username.env = "REGISTRY_USERNAME";
            password.env = "REGISTRY_TOKEN";
          } // oci;
        }).publishers.oci-image)
        [
          { }
          { repository = "localhost:5000/project"; }
          { aliases = [ "latest" "{major}" "{minor}" "{major}.{minor}" ]; }
          { public = true; }
          { username = { value = "oauth2accesstoken"; }; }
          { compression.zstd = { enable = true; level = 19; }; }
          { compression.gzip.level = 9; }
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
          { platforms = { }; }
          { platforms."linux/amd64" = "/nix/store/00000000000000000000000000000000-fake.tar.gz"; }
          # Every release needs a tag of its own, and every tag names the version.
          { tags = [ ]; }
          { tags = [ "latest" ]; }
          { tags = [ "{version}" "latest" ]; }
          { tags = [ "{version}" "{version}" ]; }
          { tags = [ "-{version}" ]; }
          # An alias follows a line of versions, never a single one.
          { aliases = [ "{version}-slim" ]; }
          { aliases = [ "{unknown}" ]; }
          { aliases = [ "latest" "latest" ]; }
          { aliases = [ "{major}.{minor}.{patch}" ]; }
          # A password is always a secret.
          { password.value = "written-into-the-declaration"; }
          # A username comes from exactly one source.
          { username = { env = "REGISTRY_USERNAME"; value = "user"; }; }
          # Exactly one compression, with a level in range.
          { compression = { gzip.enable = true; zstd.enable = true; }; }
          { compression.gzip.enable = false; }
          { compression.zstd = { enable = true; level = 23; }; }
        ];
      expected = [ true true true true true true true true true false false false false false false false false false false false false false false false false false false false false false false false ];
    };

    "test OCI image publishers of one repository share no image tag" = {
      expr = map
        (others: succeeds (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.oci-image = lib.mapAttrs
            (_name: oci: {
              repository = "ghcr.io/owner/project";
              platforms."linux/amd64" = image;
              username.env = "REGISTRY_USERNAME";
              password.env = "REGISTRY_TOKEN";
            } // oci)
            ({ default.aliases = [ "latest" ]; } // others);
        }).publishers.oci-image)
        [
          { debug.tags = [ "{version}-debug" ]; }
          { debug = { tags = [ "{version}-debug" ]; aliases = [ "debug" ]; }; }
          { other = { repository = "ghcr.io/owner/other"; aliases = [ "latest" ]; }; }
          { debug = { tags = [ "{version}-debug" ]; aliases = [ "latest" ]; }; }
          { debug = { }; }
          # `{major}.{minor}.{patch}` renders the same tag as `{version}`.
          { debug = { tags = [ "{version}-debug" ]; aliases = [ "{major}.{minor}.{patch}" ]; }; }
        ];
      expected = [ true true true false false false ];
    };

    "test publishers that claim the same remote object fail with both names" = {
      expr = {
        image = failedAssertion {
          publishers.oci-image.image = imagePublisher;
          publishers.oci-helm.chart = {
            inherit (imagePublisher) repository username password;
            chart = image;
          };
        };
        # Different templates that name the same object.
        alias = failedAssertion {
          publishers.oci-image.image = imagePublisher // {
            tags = [ "v{version}" ];
            aliases = [ "1.2.3" ];
          };
          publishers.oci-helm.chart = {
            inherit (imagePublisher) repository username password;
            chart = image;
          };
        };
        github = failedAssertion {
          publishers.github-release.a.repository = "owner/project";
          publishers.github-release.b = {
            repository = "owner/project";
            after = [ ];
          };
        };
      };
      expected = {
        image = "publishers.oci-helm.chart and publishers.oci-image.image both claim {version} in oci://ghcr.io/owner/project";
        alias = "publishers.oci-helm.chart and publishers.oci-image.image both claim the same object in oci://ghcr.io/owner/project: {version} and 1.2.3";
        github = "publishers.github-release.a and publishers.github-release.b both claim v{version} in https://api.github.com/repos/owner/project";
      };
    };

    "test GitHub Releases of other repositories or API URLs claim nothing in common" = {
      expr = map
        (other: succeeds (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          publishers.github-release.default.repository = "owner/project";
          publishers.github-release.other = { after = [ ]; } // other;
        }).publishers)
        [
          { repository = "owner/other"; }
          { repository = "owner/project"; apiUrl = "https://github.example/api/v3"; }
          { repository = "owner/project"; }
          # GitHub names are case-insensitive.
          { repository = "Owner/Project"; }
        ];
      expected = [ true true false false ];
    };

    "test publisher dependencies normalize to after edges" = {
      expr = {
        githubDefault = (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.oci-image.image = imagePublisher;
          publishers.oci-helm.chart = {
            repository = "ghcr.io/owner/charts/project";
            chart = image;
            username.env = "REGISTRY_USERNAME";
            password.env = "REGISTRY_TOKEN";
          };
        }).publishers.github-release.default.after;
        githubOverride = (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.github-release.default.after = [ ];
          publishers.oci-image.image = imagePublisher;
        }).publishers.github-release.default.after;
        rewrittenBefore = (mkRelease {
          sourceRoot = fixtureRoot;
          versionFiles.cargo.example.cargoToml = cargoToml;
          imports = [ publisher ];
          publishers.github-release.default.after = [ ];
          publishers.oci-image.image = imagePublisher // {
            before = [ "github-release.default" ];
          };
        }).publishers.github-release.default.after;
      };
      expected = {
        githubDefault = [ "oci-helm.chart" "oci-image.image" ];
        githubOverride = [ ];
        rewrittenBefore = [ "oci-image.image" ];
      };
    };

    "test publisher dependencies reject unknown references" = {
      expr = map
        (edge: failedAssertion {
          publishers.github-release.default = {
            repository = "owner/project";
          } // edge;
        })
        [
          { after = [ "oci-image.missing" ]; }
          { before = [ "oci-image.missing" ]; }
        ];
      expected = [
        ''publishers.github-release.default.after references unknown publisher "oci-image.missing"''
        ''publishers.github-release.default.before references unknown publisher "oci-image.missing"''
      ];
    };

    "test publisher dependencies reject self references" = {
      expr = map
        (edge: failedAssertion {
          publishers.oci-image.image = imagePublisher // edge;
        })
        [
          { after = [ "oci-image.image" ]; }
          { before = [ "oci-image.image" ]; }
        ];
      expected = [
        "publishers form a cycle: oci-image.image → oci-image.image"
        "publishers form a cycle: oci-image.image → oci-image.image"
      ];
    };

    "test publisher dependency cycles include the default hint only while it applies" = {
      expr = {
        default = failedAssertion {
          publishers.github-release.default.repository = "owner/project";
          publishers.oci-image.image = imagePublisher // {
            after = [ "github-release.default" ];
          };
        };
        explicit = failedAssertion {
          publishers.github-release.default = {
            repository = "owner/project";
            after = [ "oci-image.image" ];
          };
          publishers.oci-image.image = imagePublisher // {
            after = [ "github-release.default" ];
          };
        };
        mkDefault = failedAssertion {
          publishers.github-release.default = {
            repository = "owner/project";
            after = lib.mkDefault [ "oci-image.image" ];
          };
          publishers.oci-image.image = imagePublisher // {
            after = [ "github-release.default" ];
          };
        };
        unrelated = failedAssertion {
          publishers.github-release.default.repository = "owner/project";
          publishers.oci-image.a = imagePublisher // { after = [ "oci-image.b" ]; };
          publishers.oci-image.b = imagePublisher // { after = [ "oci-image.a" ]; };
        };
      };
      expected = {
        default = "publishers form a cycle: github-release.default → oci-image.image → github-release.default (github-release.default.after defaults to every other publisher; set it explicitly to change that)";
        explicit = "publishers form a cycle: github-release.default → oci-image.image → github-release.default";
        mkDefault = "publishers form a cycle: github-release.default → oci-image.image → github-release.default";
        unrelated = "publishers form a cycle: oci-image.a → oci-image.b → oci-image.a";
      };
    };

    "test a release needs no publishers" = {
      expr = (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.example.cargoToml = cargoToml;
      }).publishers;
      expected = {
        github-release = { };
        oci-image = { };
        oci-helm = { };
        python = { };
      };
    };

    "test publishers.oci aliases publishers.oci-image without entering the declaration" = {
      expr =
        let
          ociPublisher = repository: {
            inherit repository;
            platforms."linux/amd64" = image;
            username.env = "REGISTRY_USERNAME";
            password.env = "REGISTRY_TOKEN";
          };
          publishers = (mkRelease {
            sourceRoot = fixtureRoot;
            versionFiles.cargo.example.cargoToml = cargoToml;
            publishers.oci.legacy = ociPublisher "ghcr.io/owner/legacy";
            publishers.oci-image.current = ociPublisher "ghcr.io/owner/current";
          }).publishers;
        in
        {
          names = builtins.attrNames publishers.oci-image;
          hasLegacyKey = publishers ? oci;
        };
      expected = {
        names = [ "current" "legacy" ];
        hasLegacyKey = false;
      };
    };

    "test duplicate publishers.oci and publishers.oci-image names fail normally" = {
      expr = succeeds (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.example.cargoToml = cargoToml;
        publishers.oci.default = {
          repository = "ghcr.io/owner/legacy";
          platforms."linux/amd64" = image;
          username.env = "REGISTRY_USERNAME";
          password.env = "REGISTRY_TOKEN";
        };
        publishers.oci-image.default.repository = "ghcr.io/owner/current";
      }).publishers;
      expected = false;
    };

    "test publishers.helm is not an alias" = {
      expr = succeeds (mkRelease {
        sourceRoot = fixtureRoot;
        versionFiles.cargo.example.cargoToml = cargoToml;
        publishers.helm.default = {
          repository = "ghcr.io/owner/charts/project";
          chart = image;
          username.env = "REGISTRY_USERNAME";
          password.env = "REGISTRY_TOKEN";
        };
      }).publishers;
      expected = false;
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

    "test changelog headings are validated" = {
      expr = map
        (
          heading:
          succeeds (mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            changelog.heading = heading;
          })
        )
        [
          "## v{version} ({date})"
          "### {version}"
          "v{version}"
          "## {version} {version}"
          "## {version} {date} {date}"
          "## {version}\n{date}"
          "## {version} ({date:%d.%m.%Y})"
          "## {version} - {date:%B %e, %Y (100%%)}"
          "## {version} {date:}"
          "## {version} {date:%Y"
          "## {version} {date} {date:%Y}"
        ];
      expected = [
        true
        true
        false
        false
        false
        false
        true
        true
        false
        false
        false
      ];
    };

    "test unreleased headings are validated" = {
      expr = map
        (
          heading:
          succeeds (mkRelease {
            inherit sourceRoot;
            version = "1.2.3";
            changelog.unreleasedHeading = heading;
          })
        )
        [
          "## Upcoming"
          "Unreleased"
          "## [Unreleased]\n"
        ];
      expected = [
        true
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
  }
  # Keys that are not valid TOML fail in fromTOML, which tryEval cannot catch; the rest throw.
  // lib.mapAttrs'
    (name: key: lib.nameValuePair "test malformed key is rejected: ${name}" {
      expr = (releaseWithKey key).version;
      expectedError.msg =
        if builtins.elem name [ "second-line" "smuggled-value" "table-header" ] then "is not a TOML dotted key" else "parsing TOML";
    })
    malformedKeys;

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
