{
  flake.modules.presswerk.default =
    { config, lib, ... }:
    let
      inherit (lib) mkOption types;

      relativePath = types.addCheck types.str (
        path:
        path != ""
        && !(lib.hasPrefix "/" path)
        && !(lib.hasInfix "\\" path)
        && lib.all (part: part != "" && part != "." && part != "..") (lib.splitString "/" path)
      );

      tagFormat = types.addCheck types.nonEmptyStr (
        format:
        let
          parts = builtins.split "\\{\\{[[:space:]]*version[[:space:]]*\\}\\}" format;
          expressions = builtins.filter builtins.isList parts;
          literals = builtins.filter builtins.isString parts;
          containsSyntax = literal: lib.any (syntax: lib.hasInfix syntax literal) [ "{{" "{%" "{#" ];
        in
        builtins.length expressions == 1 && lib.all (literal: !containsSyntax literal) literals
      );

      cargoModule = { name, ... }: {
        options = {
          crate = mkOption {
            type = types.nonEmptyStr;
            default = name;
            description = "Cargo package whose version is updated in Cargo.lock.";
          };

          cargoToml = mkOption {
            type = relativePath;
            default = "Cargo.toml";
            description = "Repository-relative path to Cargo.toml.";
          };

          cargoLock = mkOption {
            type = relativePath;
            default = "Cargo.lock";
            description = "Repository-relative path to Cargo.lock.";
          };
        };
      };

      githubReleaseModule = {
        options = {
          repository = mkOption {
            type = types.strMatching "[^/]+/[^/]+";
            description = "GitHub repository in owner/name form.";
          };

          token.secret = mkOption {
            type = types.nonEmptyStr;
            default = "GITHUB_TOKEN";
            description = "Logical name of the GitHub API token secret.";
          };
        };
      };
    in
    {
      options = {
        sourceRoot = mkOption {
          type = types.path;
          description = "Immutable consumer source used while evaluating the release declaration.";
        };

        version = mkOption {
          type = types.nonEmptyStr;
          description = "Effective release version shared by every version source.";
        };

        versionFiles.cargo = mkOption {
          type = types.attrsOf (types.submodule cargoModule);
          default = { };
          description = "Cargo version sources and writers.";
        };

        tag.format = mkOption {
          type = tagFormat;
          default = "v{{ version }}";
          description = "Template used to render release tags; must contain only literal text and {{ version }} exactly once.";
        };

        analyzer.convention = mkOption {
          type = types.enum [ "gitmoji" ];
          default = "gitmoji";
          description = "Commit convention analyzed for releases.";
        };

        changelog.path = mkOption {
          type = relativePath;
          default = "CHANGELOG.md";
          description = "Repository-relative changelog path.";
        };

        publishers.github-release = mkOption {
          type = types.attrsOf (types.submodule githubReleaseModule);
          default = { };
          description = "GitHub Release publishers.";
        };

        assertions = mkOption {
          type = types.listOf (types.submodule {
            options = {
              assertion = mkOption { type = types.bool; };
              message = mkOption { type = types.str; };
            };
          });
          default = [ ];
          description = "Assertions checked while evaluating the release module.";
        };
      };

      config.version = lib.mkMerge (lib.mapAttrsToList
        (
          name: cargo:
            lib.mkDefinition {
              file = "versionFiles.cargo.${name} (${cargo.cargoToml})";
              value = (builtins.fromTOML (builtins.readFile (config.sourceRoot + "/${cargo.cargoToml}"))).package.version;
            }
        )
        config.versionFiles.cargo);
    };
}
