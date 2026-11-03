{ mkRelease, presswerkModule, presswerkVersion, sourceRoot }:
let
  publisher = {
    publishers.github-release.default.repository = "owner/project";
  };

  defaults = mkRelease {
    inherit sourceRoot;
    imports = [ publisher ];
    versionFiles.cargo.presswerk = { };
  };

  fixtureRoot = ../fixtures/release;
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

  matchingVersions = mkRelease {
    sourceRoot = fixtureRoot;
    imports = [ publisher ];
    version = "1.2.3";
    versionFiles.cargo.example.cargoToml = "nested/Cargo.toml";
  };

  conflictingVersions = builtins.tryEval (
    (mkRelease {
      sourceRoot = fixtureRoot;
      imports = [ publisher ];
      versionFiles.cargo.first.cargoToml = "nested/Cargo.toml";
      versionFiles.cargo.second.cargoToml = "nested/Other.toml";
    }).version
  );

  consumerSchemaVersion = builtins.tryEval (
    (mkRelease {
      inherit sourceRoot;
      imports = [ publisher ];
      schemaVersion = 2;
      versionFiles.cargo.presswerk = { };
    }).schemaVersion
  );

  missingSourceRoot = builtins.tryEval (
    (mkRelease {
      version = "1.2.3";
    }).schemaVersion
  );

  missingTagPlaceholder = builtins.tryEval (
    (mkRelease {
      inherit sourceRoot;
      version = "1.2.3";
      tag.format = "release";
    }).tag.format
  );

  repeatedTagPlaceholder = builtins.tryEval (
    (mkRelease {
      inherit sourceRoot;
      version = "1.2.3";
      tag.format = "{{ version }}-{{ version }}";
    }).tag.format
  );

  backslashPath = builtins.tryEval (
    (mkRelease {
      inherit sourceRoot;
      version = "1.2.3";
      changelog.path = "docs\\NEWS.md";
    }).changelog.path
  );

  moduleArgument = mkRelease ({ presswerk, ... }: {
    inherit sourceRoot;
    imports = [ publisher ];
    versionFiles.cargo.presswerk = { };
    assertions = [
      {
        assertion = presswerk.version == presswerkVersion;
        message = "unexpected presswerk.version module argument";
      }
    ];
  });

  publishedStandardModule = mkRelease {
    inherit sourceRoot;
    imports = [
      presswerkModule
      publisher
    ];
    version = "0.0.0";
  };

  wrongModuleClass = builtins.tryEval (
    (mkRelease {
      inherit sourceRoot;
      imports = [{ _class = "nixos"; }];
      version = "0.0.0";
    }).version
  );
in
assert defaults.schemaVersion == 1;
assert !(defaults ? sourceRoot);
assert defaults.version == "0.0.0";
assert defaults.versionFiles == [
  {
    path = "Cargo.toml";
    toml = "package.version";
  }
  {
    path = "Cargo.lock";
    cargoLock = "presswerk";
  }
];
assert defaults.tag.format == "v{{ version }}";
assert defaults.analyzer.convention == "gitmoji";
assert defaults.changelog == { path = "CHANGELOG.md"; };
assert defaults.publishers.github-release.default.token.secret == "GITHUB_TOKEN";
assert overrides.version == "1.2.3";
assert overrides.versionFiles == [
  {
    path = "nested/Cargo.toml";
    toml = "package.version";
  }
  {
    path = "nested/Cargo.lock";
    cargoLock = "actual-crate";
  }
];
assert overrides.changelog == { path = "docs/NEWS.md"; };
assert matchingVersions.version == "1.2.3";
assert !conflictingVersions.success;
assert !consumerSchemaVersion.success;
assert !missingSourceRoot.success;
assert !missingTagPlaceholder.success;
assert !repeatedTagPlaceholder.success;
assert !backslashPath.success;
assert moduleArgument.version == presswerkVersion;
assert publishedStandardModule.version == "0.0.0";
assert publishedStandardModule.schemaVersion == 1;
assert !wrongModuleClass.success;
builtins.deepSeq (builtins.fromJSON (builtins.toJSON defaults)) defaults
