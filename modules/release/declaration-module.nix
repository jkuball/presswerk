{
  flake.modules.presswerk.default =
    { config, lib, ... }:
    let
      inherit (lib) mkOption types;

      gitmojiCatalog = builtins.fromJSON (builtins.readFile ./gitmoji/catalog.json);
      conventionalCatalog = builtins.fromJSON (builtins.readFile ./conventional/catalog.json);
      variationSelector = builtins.fromJSON ''"\uFE0F"'';
      normalizedEmoji = emoji: builtins.replaceStrings [ variationSelector ] [ "" ] emoji;
      # Every commit analyzer lives under `analyzer` and has an `enable` option; a release enables
      # exactly one of them. A disabled analyzer's settings are ignored.
      analyzerNames = [ "gitmoji" "conventional" ];
      enabledAnalyzers = builtins.filter (name: config.analyzer.${name}.enable) analyzerNames;
      catalogEntries = lib.optionals config.analyzer.gitmoji.enable
        (builtins.filter (entry: entry != null) (builtins.attrValues config.analyzer.gitmoji.catalog));
      conventionalTypes = lib.optionals config.analyzer.conventional.enable
        (builtins.attrNames (lib.filterAttrs (_type: entry: entry != null) config.analyzer.conventional.catalog));
      catalogEmojis = map (entry: normalizedEmoji entry.emoji) catalogEntries;
      catalogShortcodes = map (entry: entry.shortcode) catalogEntries;
      # OCI tag templates rendered with a stand-in version whose parts differ, as the CLI checks them.
      renderOciTags = oci:
        map (builtins.replaceStrings [ "{version}" "{major}" "{minor}" "{patch}" ] [ "1.2.3" "1" "2" "3" ])
          (oci.tags ++ oci.aliases);
      # Image tags that two OCI publishers of one repository would both push.
      sharedOciTags = lib.concatLists (lib.mapAttrsToList
        (name: oci: lib.concatLists (lib.mapAttrsToList
          (other: otherOci:
            map (tag: { inherit name other tag; inherit (oci) repository; })
              (lib.optionals (name < other && oci.repository == otherOci.repository)
                (lib.intersectLists (renderOciTags oci) (renderOciTags otherOci))))
          config.publishers.oci))
        config.publishers.oci);

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

      keyedFileModule = format: {
        options = {
          path = mkOption {
            type = relativePath;
            description = "Repository-relative path to the ${format} file.";
          };

          key = mkOption {
            type = types.strMatching "[^.]+([.][^.]+)*";
            example = "project.version";
            description = "Dotted key of the version string in the ${format} file.";
          };
        };
      };

      plainModule = {
        options.path = mkOption {
          type = relativePath;
          default = "VERSION";
          description = "Repository-relative path to the file that contains nothing but the version.";
        };
      };

      regexModule = {
        options = {
          path = mkOption {
            type = relativePath;
            description = "Repository-relative path to the file.";
          };

          pattern = mkOption {
            type = types.nonEmptyStr;
            example = "^version: (?<version>\\S+)$";
            description = "Rust regular expression that matches the file exactly once, with a capture group named `version`; `^` and `$` match at line boundaries.";
          };
        };
      };

      badgeModule = {
        options = {
          path = mkOption {
            type = relativePath;
            default = ".github/presswerk-badge.svg";
            description = "Repository-relative path of the generated badge.";
          };

          colorScheme = mkOption {
            type = types.enum [ "auto" "light" "dark" ];
            default = "auto";
            description = "Page colour scheme the badge is made for: `light` or `dark` pages, or `auto` to follow the viewer's system colour scheme.";
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

      gitmojiAnalyzerOptions = {
        enable = lib.mkEnableOption "the Gitmoji analyzer; a release enables exactly one commit analyzer";

        catalog = mkOption {
          type = types.attrsOf (types.nullOr (types.submodule gitmojiModule));
          default = { };
          defaultText = lib.literalMD "the [default Gitmoji catalog](https://presswerk.dev/reference/analyzers/gitmoji/#default-gitmoji-catalog)";
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

      conventionalTypeModule = {
        options = {
          title = mkOption {
            type = types.nonEmptyStr;
            description = "Release-note group heading for the type.";
          };

          semver = mkOption {
            type = types.nullOr (types.enum [
              "major"
              "minor"
              "patch"
            ]);
            description = "Semantic-version level triggered by the type, or null for no release. A breaking change is major whatever its type.";
          };
        };
      };

      conventionalAnalyzerOptions = {
        enable = lib.mkEnableOption "the Conventional Commits analyzer; a release enables exactly one commit analyzer";

        catalog = mkOption {
          type = types.attrsOf (types.nullOr (types.submodule conventionalTypeModule));
          default = { };
          defaultText = lib.literalMD "the [default Conventional Commits catalog](https://presswerk.dev/reference/analyzers/conventional-commits/#default-type-catalog)";
          example = lib.literalExpression ''
            {
              # Release documentation changes as patches.
              docs.semver = "patch";
              # Accept a project-specific type.
              deps = {
                title = "Dependencies";
                semver = "patch";
              };
              # Stop accepting style commits.
              style = lib.mkForce null;
            }
          '';
          description = "Conventional Commits types keyed by their lowercase name. Setting a type to `lib.mkForce null` removes it.";
        };
      };

      # A file in a flake output: a derivation, or a string built from one, such as "${pkg}/bin/tool".
      # The string context proves that the path comes from a derivation the flake evaluates.
      artifactFile = types.coercedTo types.package (package: "${package}") (
        types.addCheck types.str (path: builtins.hasContext path && lib.hasPrefix "${builtins.storeDir}/" path)
        // { description = "file in a flake output"; }
      );

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

          body = mkOption {
            type = types.str;
            default = "{{ notes }}";
            example = lib.literalExpression ''
              builtins.readFile ./github-release.md.j2
            '';
            description = "MiniJinja template for the GitHub Release body. The context contains `notes`, the committed changelog section; `version`, the release version; and `tag`, the release tag.";
          };

          token.secret = mkOption {
            type = types.nonEmptyStr;
            default = "GITHUB_TOKEN";
            example = "PRESSWERK_GITHUB_TOKEN";
            description = "Name of the environment variable that holds the GitHub API token.";
          };

          assets = mkOption {
            type = types.attrsOf artifactFile;
            default = { };
            example = lib.literalExpression ''
              {
                "my-tool-x86_64-linux" = "''${self.packages.x86_64-linux.default}/bin/my-tool";
                "my-tool-aarch64-darwin" = "''${self.packages.aarch64-darwin.default}/bin/my-tool";
              }
            '';
            description = "Files attached to the release, keyed by asset name. Each is a file in a flake output, which `publish` fetches from the local store or a substituter without building.";
          };
        };
      };

      secretModule = what: example: {
        options.secret = mkOption {
          type = types.nonEmptyStr;
          inherit example;
          description = "Name of the environment variable that holds the ${what}.";
        };
      };

      # Rendered with a stand-in version, a tag must be a valid OCI tag.
      ociTag = types.addCheck types.nonEmptyStr
        (
          template: builtins.match "[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}" (builtins.replaceStrings [ "{version}" "{major}" "{minor}" "{patch}" ] [ "0.0.0" "0" "0" "0" ] template) != null
        ) // { description = "OCI tag, optionally containing {version}, {major}, {minor}, or {patch}"; };

      ociModule = { config, ... }: {
        options = {
          repository = mkOption {
            # A plain host and optional port: no credentials, query, or fragment in the declaration.
            type = types.strMatching "(([A-Za-z0-9-]+[.])+[A-Za-z0-9-]+(:[0-9]+)?|[A-Za-z0-9.-]+:[0-9]+|localhost|\\[[0-9A-Fa-f:]+](:[0-9]+)?)/[a-z0-9._-]+(/[a-z0-9._-]+)*";
            example = "ghcr.io/owner/project";
            description = "Registry host, with an optional port, and repository path the images are pushed to. Credentials never go here, only into `username` and `password`. Loopback registries such as `localhost:5000` use plain HTTP, every other registry HTTPS.";
          };

          platforms = mkOption {
            type = types.attrsOf artifactFile;
            example = lib.literalExpression ''
              {
                "linux/amd64" = self.packages.x86_64-linux.image;
                "linux/arm64" = self.packages.aarch64-linux.image;
              }
            '';
            description = "The image for each platform, keyed by `os/architecture[/variant]`, which must match the image's config. Each is a Docker image archive, gzip- or zstd-compressed or not, such as the output of `dockerTools.buildImage` or `buildLayeredImage`, or an executable that writes an uncompressed one to stdout, such as the output of `streamLayeredImage`. `publish` fetches them from the local store or a substituter without building. The images of all platforms form one image index.";
          };

          tags = mkOption {
            type = types.listOf ociTag;
            default = [ "{version}" ];
            example = [ "{version}" "v{version}" ];
            description = "Tags of the release. Each contains `{version}`, which is replaced by the version, and may also use `{major}`, `{minor}`, and `{patch}`. A tag is never moved once it exists.";
          };

          aliases = mkOption {
            type = types.listOf ociTag;
            default = [ ];
            example = [ "latest" "{major}" "{major}.{minor}" ];
            description = "Floating tags, also called rolling tags, that follow the release line, such as `latest`, `{major}`, or `{major}.{minor}`. An alias may use `{major}`, `{minor}`, and `{patch}`, but not `{version}`. It moves only to a higher stable version than the one it points to, so `{major}` stays within its major version.";
          };

          public = mkOption {
            type = types.bool;
            default = false;
            description = "Whether anyone may pull the images without credentials. `plan` and `prepare` then check the previous release's tags anonymously and never read `username` or `password`; otherwise they need the credentials for that check. `publish` always needs the credentials.";
          };

          compression = {
            gzip = {
              enable = mkOption {
                type = types.bool;
                default = !config.compression.zstd.enable;
                defaultText = lib.literalExpression "!compression.zstd.enable";
                description = "Compress layers with gzip, which every client can pull. This is the default.";
              };
              level = mkOption {
                type = types.ints.between 1 9;
                default = 6;
                description = "The gzip level, from 1 (fastest) to 9 (smallest).";
              };
            };
            zstd = {
              enable = lib.mkEnableOption "zstd compression of layers, which is smaller and faster to unpack, but needs Docker 23, containerd 1.5, or a similarly recent client to pull";
              level = mkOption {
                type = types.ints.between 1 22;
                default = 3;
                description = "The zstd level, from 1 (fastest) to 22 (smallest).";
              };
            };
          };

          username = mkOption {
            type = types.submodule {
              options.secret = mkOption {
                type = types.nullOr types.nonEmptyStr;
                default = null;
                example = "REGISTRY_USERNAME";
                description = "Name of the environment variable that holds the registry username.";
              };
              options.value = mkOption {
                type = types.nullOr types.nonEmptyStr;
                default = null;
                example = "oauth2accesstoken";
                description = "The registry username itself, for registries where it is no secret, such as `oauth2accesstoken` for Google Artifact Registry. It is written into the release declaration.";
              };
            };
            description = "Registry username: either `secret`, read from an environment variable at run time, or `value`.";
          };

          password = mkOption {
            type = types.submodule (secretModule "registry password or token" "REGISTRY_TOKEN");
            description = "Registry password or token, read from an environment variable at run time.";
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

        versionFiles.toml = mkOption {
          type = types.attrsOf (types.submodule (keyedFileModule "TOML"));
          default = { };
          example = lib.literalExpression ''
            {
              pyproject = {
                path = "pyproject.toml";
                key = "project.version";
              };
            }
          '';
          description = "Version sources and writers for a dotted key in a TOML file.";
        };

        versionFiles.json = mkOption {
          type = types.attrsOf (types.submodule (keyedFileModule "JSON"));
          default = { };
          example = lib.literalExpression ''
            {
              package = {
                path = "package.json";
                key = "version";
              };
            }
          '';
          description = "Version sources and writers for a dotted key in a JSON file.";
        };

        versionFiles.plain = mkOption {
          type = types.attrsOf (types.submodule plainModule);
          default = { };
          example = lib.literalExpression ''
            {
              default = { };
            }
          '';
          description = "Version sources and writers for files that contain nothing but the version.";
        };

        versionFiles.regex = mkOption {
          type = types.attrsOf (types.submodule regexModule);
          default = { };
          example = lib.literalExpression ''
            {
              chart = {
                path = "chart/Chart.yaml";
                pattern = "^version: (?<version>\\S+)$";
              };
            }
          '';
          description = "Version writers for any text file, found by a regular expression; the CLI checks them, but the flake does not read them.";
        };

        versionFiles.badge = mkOption {
          type = types.attrsOf (types.submodule badgeModule);
          default = { };
          example = lib.literalExpression ''
            {
              light = {
                colorScheme = "light";
                path = ".github/badge-light.svg";
              };
              dark = {
                colorScheme = "dark";
                path = ".github/badge-dark.svg";
              };
            }
          '';
          description = "Version badges that `prepare` generates for every release; they do not carry the version.";
        };

        tag.format = mkOption {
          type = tagFormat;
          default = "v{version}";
          example = "release-{version}";
          description = "Release tag format; must contain {version} exactly once, not adjacent to another brace. The release commit subject names the tag, such as `🔖 Release v1.2.0` with Gitmoji or `chore(release): v1.2.0` with Conventional Commits.";
        };

        analyzer = {
          gitmoji = gitmojiAnalyzerOptions;
          conventional = conventionalAnalyzerOptions;
        };

        changelog = {
          path = mkOption {
            type = relativePath;
            default = "CHANGELOG.md";
            example = "docs/NEWS.md";
            description = "Repository-relative changelog path.";
          };

          template = mkOption {
            type = types.str;
            default = builtins.readFile ../../src/release-notes.md.j2;
            defaultText = lib.literalMD "the [built-in changelog template](https://github.com/jkuball/presswerk/blob/main/src/release-notes.md.j2)";
            example = lib.literalExpression ''
              builtins.readFile ./release-notes.md.j2
            '';
            description = "MiniJinja template for the changelog section that `prepare` writes. The context contains `version` and `groups`, each with `title` and `commits`; every commit has `text`, `message`, `scope`, `breaking`, and `id`.";
          };
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

        publishers.oci = mkOption {
          type = types.attrsOf (types.submodule ociModule);
          default = { };
          example = lib.literalExpression ''
            {
              default = {
                repository = "ghcr.io/owner/project";
                platforms."linux/amd64" = self.packages.x86_64-linux.image;
                tags = [ "{version}" ];
                aliases = [ "latest" "{major}" ];
                public = true;
                username.secret = "GITHUB_ACTOR";
                password.secret = "GITHUB_TOKEN";
              };
            }
          '';
          description = "OCI image publishers.";
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
        analyzer.gitmoji.catalog = lib.mapAttrs (_shortcode: lib.mapAttrs (_field: lib.mkDefault)) gitmojiCatalog;
        analyzer.conventional.catalog = lib.mapAttrs (_type: lib.mapAttrs (_field: lib.mkDefault)) conventionalCatalog;

        assertions = lib.concatLists
          (lib.mapAttrsToList
            (name: oci: [
              {
                assertion = oci.platforms != { };
                message = "publishers.oci.${name}.platforms must contain at least one image";
              }
              {
                assertion = oci.tags != [ ] && lib.all (lib.hasInfix "{version}") oci.tags;
                message = "publishers.oci.${name}.tags must contain {version} in every tag, and at least one tag, so every release has a tag of its own; declare tags that follow other versions under aliases";
              }
              {
                assertion = !lib.any (lib.hasInfix "{version}") oci.aliases;
                message = "publishers.oci.${name}.aliases must not contain {version}, which names a single version; declare such tags under tags";
              }
              {
                assertion = oci.compression.gzip.enable != oci.compression.zstd.enable;
                message = "publishers.oci.${name}.compression needs exactly one of gzip and zstd enabled";
              }
              {
                assertion = (oci.username.secret == null) != (oci.username.value == null);
                message = "publishers.oci.${name}.username needs exactly one of secret and value";
              }
              {
                assertion = lib.allUnique (renderOciTags oci);
                message = "publishers.oci.${name}.tags and aliases must not render to the same tag";
              }
            ])
            config.publishers.oci) ++ [
          {
            assertion = sharedOciTags == [ ];
            message =
              let shared = builtins.head sharedOciTags;
              in "publishers.oci.${shared.name} and publishers.oci.${shared.other} both push to ${shared.repository} and must not share a tag or alias; both render ${shared.tag} for version 1.2.3";
          }
          {
            assertion = builtins.length enabledAnalyzers == 1;
            message =
              if enabledAnalyzers == [ ] then
                "enable a commit analyzer, for example analyzer.gitmoji.enable = true;"
              else
                "enable exactly one commit analyzer; enabled: ${lib.concatMapStringsSep ", " (name: "analyzer.${name}") enabledAnalyzers}";
          }
          {
            assertion = lib.all (type: builtins.match "[a-z0-9][a-z0-9_-]*" type != null) conventionalTypes;
            message = "analyzer.conventional.catalog types must be lowercase letters, digits, hyphens, or underscores";
          }
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

        version =
          let
            read = path: builtins.readFile (config.sourceRoot + "/${path}");
            keyed = kind: parse: lib.mapAttrsToList
              (
                name: file:
                  lib.mkDefinition {
                    file = "versionFiles.${kind}.${name} (${file.path})";
                    value = lib.getAttrFromPath (lib.splitString "." file.key) (parse (read file.path));
                  }
              )
              config.versionFiles.${kind};
          in
          lib.mkMerge (
            lib.mapAttrsToList
              (
                name: cargo:
                  lib.mkDefinition {
                    file = "versionFiles.cargo.${name} (${cargo.cargoToml})";
                    value = (builtins.fromTOML (read cargo.cargoToml)).package.version;
                  }
              )
              config.versionFiles.cargo
            ++ keyed "toml" builtins.fromTOML
            ++ keyed "json" builtins.fromJSON
            ++ lib.mapAttrsToList
              (
                name: plain:
                  lib.mkDefinition {
                    file = "versionFiles.plain.${name} (${plain.path})";
                    value = lib.removeSuffix "\r" (lib.removeSuffix "\n" (read plain.path));
                  }
              )
              config.versionFiles.plain
          );
      };
    };
}
