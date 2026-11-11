{
  flake.modules.presswerk.default =
    { config, lib, ... }:
    let
      inherit (lib) mkOption types;

      gitmojiCatalog = builtins.fromJSON (builtins.readFile ./gitmoji/catalog.json);
      variationSelector = builtins.fromJSON ''"\uFE0F"'';
      normalizedEmoji = emoji: builtins.replaceStrings [ variationSelector ] [ "" ] emoji;
      catalogEntries = builtins.filter (entry: entry != null) (builtins.attrValues config.analyzer.gitmoji.catalog);
      catalogEmojis = map (entry: normalizedEmoji entry.emoji) catalogEntries;
      catalogShortcodes = map (entry: entry.shortcode) catalogEntries;

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
          parts = builtins.split "[{]version[}]" format;
          expressions = builtins.filter builtins.isList parts;
          literals = builtins.filter builtins.isString parts;
        in
        builtins.length expressions == 1
        && !(lib.hasSuffix "{" (builtins.elemAt literals 0))
        && !(lib.hasPrefix "}" (builtins.elemAt literals 1))
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

      gitmojiModule = { name, ... }: {
        options = {
          shortcode = mkOption {
            type = types.str;
            default = name;
            description = "Serialized shortcode without surrounding colons; defaults to the catalog entry identifier.";
          };

          emoji = mkOption {
            type = types.nonEmptyStr;
            description = "Unicode spelling of the Gitmoji; U+FE0F is ignored when checking uniqueness.";
          };

          description = mkOption {
            type = types.nonEmptyStr;
            description = "Release-note group description for the Gitmoji.";
          };

          semver = mkOption {
            type = types.nullOr (types.enum [
              "major"
              "minor"
              "patch"
            ]);
            description = "Semantic-version level triggered by the Gitmoji, or null for no release.";
          };
        };
      };

      githubReleaseModule = {
        options = {
          apiUrl = mkOption {
            type = types.nonEmptyStr;
            default = "https://api.github.com";
            description = "Base URL of the GitHub API.";
          };

          repository = mkOption {
            type = types.strMatching "[^/]+/[^/]+";
            example = "owner/project";
            description = "GitHub repository in owner/name form.";
          };

          token.secret = mkOption {
            type = types.nonEmptyStr;
            default = "GITHUB_TOKEN";
            example = "PRESSWERK_GITHUB_TOKEN";
            description = "Name of the environment variable that holds the GitHub API token.";
          };
        };
      };
    in
    {
      options = {
        sourceRoot = mkOption {
          type = types.path;
          example = lib.literalExpression "self.outPath";
          description = "Source of the repository, from which the version files are read.";
        };

        version = mkOption {
          type = types.nonEmptyStr;
          defaultText = lib.literalMD "read from the [version files](https://presswerk.dev/reference/version-files/)";
          description = "Release version; every version file must declare the same one.";
        };

        versionFiles.cargo = mkOption {
          type = types.attrsOf (types.submodule cargoModule);
          default = { };
          example = lib.literalExpression ''
            {
              my-crate = { };
              my-other-crate.cargoToml = "crates/other/Cargo.toml";
            }
          '';
          description = "Cargo version sources and writers.";
        };

        tag.format = mkOption {
          type = tagFormat;
          default = "v{version}";
          example = "release-{version}";
          description = "Release tag format; must contain {version} exactly once, not adjacent to another brace.";
        };

        analyzer = {
          convention = mkOption {
            type = types.enum [ "gitmoji" ];
            default = "gitmoji";
            description = "Commit convention analyzed for releases.";
          };

          gitmoji.catalog = mkOption {
            type = types.attrsOf (types.nullOr (types.submodule gitmojiModule));
            default = { };
            defaultText = lib.literalMD "the [default Gitmoji catalog](https://presswerk.dev/reference/analyzers/#default-catalog)";
            example = lib.literalExpression ''
              {
                # Release CI changes as patches.
                construction_worker.semver = "patch";
                # Stop accepting 💩 commits.
                poop = lib.mkForce null;
              }
            '';
            description = "Gitmoji catalog keyed by entry identifier; each entry's shortcode defaults to its identifier. Setting an entry to `lib.mkForce null` removes it.";
          };
        };

        changelog.path = mkOption {
          type = relativePath;
          default = "CHANGELOG.md";
          example = "docs/NEWS.md";
          description = "Repository-relative changelog path.";
        };

        publishers.github-release = mkOption {
          type = types.attrsOf (types.submodule githubReleaseModule);
          default = { };
          example = lib.literalExpression ''
            {
              default.repository = "owner/project";
            }
          '';
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
          internal = true;
          visible = false;
          description = "Assertions checked while evaluating the release module.";
        };
      };

      config = {
        analyzer.gitmoji.catalog = lib.mapAttrs
          (
            _shortcode: lib.mapAttrs (_field: lib.mkDefault)
          )
          gitmojiCatalog;

        assertions = [
          {
            assertion = lib.all (shortcode: shortcode != "" && !(lib.hasInfix ":" shortcode)) catalogShortcodes;
            message = "analyzer.gitmoji.catalog shortcodes must be non-empty and contain no colons";
          }
          {
            assertion = builtins.length catalogShortcodes == builtins.length (lib.unique catalogShortcodes);
            message = "analyzer.gitmoji.catalog shortcodes must be unique";
          }
          {
            assertion = builtins.length catalogEmojis == builtins.length (lib.unique catalogEmojis);
            message = "analyzer.gitmoji.catalog emoji values must be unique after removing variation selector-16 (U+FE0F)";
          }
        ];

        version = lib.mkMerge (lib.mapAttrsToList
          (
            name: cargo:
              lib.mkDefinition {
                file = "versionFiles.cargo.${name} (${cargo.cargoToml})";
                value = (builtins.fromTOML (builtins.readFile (config.sourceRoot + "/${cargo.cargoToml}"))).package.version;
              }
          )
          config.versionFiles.cargo);
      };
    };
}
