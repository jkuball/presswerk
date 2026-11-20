{
  flake.modules.presswerk.default =
    { config, lib, ... }:
    let
      inherit (lib) mkOption types;

      gitmojiCatalog = builtins.fromJSON (builtins.readFile ./gitmoji/catalog.json);
      conventionalCatalog = builtins.fromJSON (builtins.readFile ./conventional/catalog.json);
      variationSelector = builtins.fromJSON ''"\uFE0F"'';
      normalizedEmoji = emoji: builtins.replaceStrings [ variationSelector ] [ "" ] emoji;
      versionMajor = lib.versions.major config.version;
      # The segments of a TOML dotted key, which also selects JSON keys. TOML parses it, as the CLI
      # does: the key must be one dotted key, so a newline or a value smuggled in with `=` fails.
      keySegments = option: key:
        let
          invalid = throw "${option} ${builtins.toJSON key} is not a TOML dotted key";
          walk = leaf: value:
            if builtins.isAttrs value && builtins.length (builtins.attrNames value) == 1 then
              let name = builtins.head (builtins.attrNames value); in [ name ] ++ walk leaf value.${name}
            else if value == leaf then [ ]
            else invalid;
          parse = leaf: walk leaf (builtins.addErrorContext "while parsing ${option} as a TOML dotted key"
            (builtins.fromTOML "${key} = ${toString leaf}"));
          segments = parse 0;
        in
        if lib.hasInfix "\n" key || lib.hasInfix "\r" key || parse 1 != segments then invalid else segments;
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
      canonicalPublishers = builtins.removeAttrs config.publishers config.publisherAliases;
      publisherAddresses = lib.concatLists (lib.mapAttrsToList
        (type: publishers: map (name: "${type}.${name}") (builtins.attrNames publishers))
        canonicalPublishers);
      publisherNodes = builtins.listToAttrs (lib.concatLists (lib.mapAttrsToList
        (type: publishers: lib.mapAttrsToList
          (name: publisher: lib.nameValuePair "${type}.${name}" publisher)
          publishers)
        canonicalPublishers));
      resolvePublisherAfter = address:
        publisherNodes.${address}.after
        ++ builtins.filter
          (other: builtins.elem address publisherNodes.${other}.before)
          publisherAddresses;
      publisherReferences = lib.concatLists (lib.mapAttrsToList
        (address: publisher:
          map (reference: { inherit address reference; option = "after"; }) publisher.after
          ++ map (reference: { inherit address reference; option = "before"; }) publisher.before)
        publisherNodes);
      unknownPublisherReferences = builtins.filter
        ({ reference, ... }: !(builtins.elem reference publisherAddresses))
        publisherReferences;
      selfDependentPublishers = lib.unique (map
        ({ address, ... }: address)
        (builtins.filter ({ address, reference, ... }: address == reference) publisherReferences));
      sortedPublishers = lib.toposort
        (dependency: publisher: builtins.elem dependency publisherNodes.${publisher}.resolvedAfter)
        publisherAddresses;
      publisherLoop = if sortedPublishers ? loops then builtins.head sortedPublishers.loops else null;
      # `toposort.cycle` includes the path leading into the loop; diagnostics name only the loop.
      publisherCycle =
        if publisherLoop == null then [ ]
        else
          lib.drop
            (lib.lists.findFirstIndex (address: address == publisherLoop) 0 sortedPublishers.cycle)
            sortedPublishers.cycle;
      publisherCyclePath = publisherCycle ++ lib.optionals (publisherCycle != [ ]) [ publisherLoop ];
      publisherCycleHints = lib.unique (builtins.filter (hint: hint != null)
        (map (address: publisherNodes.${address}.cycleHint) publisherCycle));
      # Two publishers that write the same remote object would only conflict once the tag is
      # pushed, so every instance claims what it writes, rendered with the sample version. The CLI
      # renders the same templates again with the version it releases.
      sampleVersion = "1.2.3";
      renderTemplate = builtins.replaceStrings [ "{version}" "{major}" "{minor}" "{patch}" ]
        ([ sampleVersion ] ++ lib.splitString "." sampleVersion);
      publisherClaims = lib.concatLists (lib.mapAttrsToList
        (address: publisher: map
          (claim: claim // {
            inherit address;
            key = renderTemplate claim.key;
          })
          publisher.claims)
        publisherNodes);
      sharedClaims = lib.concatLists (lib.imap0
        (index: claim: map
          (other: { inherit claim other; })
          (builtins.filter
            (other: other.destination == claim.destination && other.key == claim.key && other.address != claim.address)
            (lib.drop (index + 1) publisherClaims)))
        publisherClaims);

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

      # Mirrors the CLI: one line, a Markdown heading, {version} exactly once, {date} at most once.
      placeholderCount = placeholder: format:
        builtins.length (builtins.filter builtins.isList (builtins.split "[{]${placeholder}[}]" format));
      # `{date}` or `{date:<format>}` at most once. The CLI checks the strftime format itself.
      validDateFormat = dateFormat: dateFormat != "" && !(lib.hasInfix "{" dateFormat);
      validDatePlaceholder = format:
        let
          parts = builtins.split "[{]date(:[^}]*)?[}]" format;
          placeholders = builtins.filter builtins.isList parts;
          spec = builtins.elemAt (builtins.head placeholders) 0;
        in
        builtins.length placeholders <= 1
        && lib.all (literal: !(lib.hasInfix "{date:" literal)) (builtins.filter builtins.isString parts)
        && (placeholders == [ ] || spec == null || validDateFormat (lib.removePrefix ":" spec));
      changelogHeading = types.addCheck types.str
        (
          format:
          lib.hasPrefix "#" format
            && !(lib.hasInfix "\n" format)
            && !(lib.hasInfix "\r" format)
            && placeholderCount "version" format == 1
            && validDatePlaceholder format
        ) // { description = "Markdown heading line containing {version} exactly once and {date} or {date:<format>} at most once"; };
      unreleasedHeading = types.addCheck types.str
        (
          line: lib.hasPrefix "#" line && !(lib.hasInfix "\n" line) && !(lib.hasInfix "\r" line)
        ) // { description = "Markdown heading line"; };

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

      cargoWorkspaceModule = { config, ... }: {
        options = {
          cargoToml = mkOption {
            type = relativePath;
            default = "Cargo.toml";
            description = "Repository-relative path to the workspace root's Cargo.toml.";
          };

          cargoLock = mkOption {
            type = relativePath;
            default =
              if lib.hasInfix "/" config.cargoToml
              then "${builtins.dirOf config.cargoToml}/Cargo.lock"
              else "Cargo.lock";
            defaultText = lib.literalMD "`Cargo.lock` next to `cargoToml`";
            description = "Repository-relative path to the workspace's Cargo.lock.";
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
            type = types.str;
            example = "project.version";
            description = ''
              TOML dotted key of the version string in the ${format} file. Bare segments consist of
              `A-Za-z0-9_-`; quote any other segment as in TOML, such as `packages."".version`,
              `dependencies."@scope/pkg".version`, or `a.'x.y'.b`.
            '';
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

      # The options every publisher instance has, whatever its type: its place in the dependency
      # graph, and what it writes into the declaration and claims on its remote.
      publisherInstance =
        { type
        , afterDefault ? [ ]
        , afterDefaultText ? null
        }:
        { name, ... }: {
          options = {
            after = mkOption ({
              type = types.listOf types.nonEmptyStr;
              default = afterDefault;
              example = [ "oci-image.api" ];
              description = "Publishers that must complete before this publisher starts. Defining this option replaces its default.";
            } // lib.optionalAttrs (afterDefaultText != null) { defaultText = afterDefaultText; });

            before = mkOption {
              type = types.listOf types.nonEmptyStr;
              default = [ ];
              example = [ "github-release.default" ];
              description = "Publishers that start only after this publisher completes.";
            };

            cycleHint = mkOption {
              type = types.nullOr types.str;
              default = null;
              internal = true;
              visible = false;
              description = "Hint appended when this publisher participates in a dependency cycle.";
            };

            resolvedAfter = mkOption {
              type = types.listOf types.nonEmptyStr;
              readOnly = true;
              internal = true;
              visible = false;
              description = "Publisher dependencies after rewriting `before` edges.";
            };

            declaration = mkOption {
              type = types.attrsOf types.anything;
              readOnly = true;
              internal = true;
              visible = false;
              description = "What the release declaration holds for this publisher, apart from `after`; set by its type.";
            };

            claims = mkOption {
              type = types.listOf (types.submodule {
                options = {
                  destination = mkOption {
                    type = types.str;
                    description = "The remote, such as `oci://ghcr.io/owner/project`.";
                  };
                  key = mkOption {
                    type = types.str;
                    description = "The object within it, as a version template such as the tag `{major}.{minor}`.";
                  };
                };
              });
              readOnly = true;
              internal = true;
              visible = false;
              description = "The remote objects this publisher writes, set by its type. No two publishers may claim the same one, rendered with the sample version 1.2.3 here and with the released version by the CLI.";
            };
          };

          config.resolvedAfter = resolvePublisherAfter "${type}.${name}";
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

        stable = mkOption {
          type = types.bool;
          default = versionMajor != "0";
          defaultText = lib.literalMD "`false` for `0.x` versions, `true` from major version 1";
          description = "Whether the public API is stable. When true, analyzer levels follow plain SemVer, so a major level on `0.y.z` selects `1.0.0`; when false, major and minor levels both advance the minor component.";
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

        versionFiles.cargoWorkspace = mkOption {
          type = types.attrsOf (types.submodule cargoWorkspaceModule);
          default = { };
          example = lib.literalExpression ''
            {
              my-workspace = { };
              backend.cargoToml = "backend/Cargo.toml";
            }
          '';
          description = ''
            Cargo workspace version sources and writers, keyed by a label. The flake reads
            `workspace.package.version`. Presswerk writes it and the `Cargo.lock` entry of every
            workspace member with `version.workspace = true`.
          '';
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
            description = "MiniJinja template for the changelog section that `prepare` writes. The context contains `version`; `unreleased`, the hand-written text under `changelog.unreleasedHeading`, or an empty string; and `groups`, each with `title` and `commits`; every commit has `text`, `message`, `scope`, `breaking`, and `id`.";
          };

          heading = mkOption {
            type = changelogHeading;
            default = "## [{version}] - {date}";
            example = "## v{version} ({date})";
            description = "Version heading of a changelog section, in which `{version}` stands for the release version and `{date}` for the release date: one line that starts with `#`, with `{version}` exactly once and `{date}` at most once; other text is literal and trailing whitespace is ignored. `{date:<format>}` writes the date in a strftime format, such as `%d.%m.%Y` or `%B %e, %Y`; plain `{date}` means `{date:%Y-%m-%d}`. The format must write a full date that can be read back, and existing headings match only when their date parses in that format. The same format renders new headings and finds existing ones, for the duplicate check, the insertion point, and the section that publishers use as release notes.";
          };

          unreleasedHeading = mkOption {
            type = unreleasedHeading;
            default = "## [Unreleased]";
            example = "## Upcoming";
            description = "Literal heading line of an optional section above the first version heading for hand-written release notes; trailing whitespace is ignored. `prepare` moves its text into the new version section through the template's `unreleased` variable and removes the section, heading included. It never adds the heading, and the text alone does not cause a release. A template that does not use `unreleased` leaves the section untouched.";
          };
        };

        publisherAliases = mkOption {
          type = types.listOf types.str;
          default = [ ];
          internal = true;
          visible = false;
          description = "Earlier names of publisher types under `publishers`, which forward to the current one.";
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
        # What publisher types share. Each type lives in `publishers/<type>.nix`.
        _module.args.publisherLib = {
          instance = publisherInstance;
          addresses = publisherAddresses;
          # A file in a flake output: a derivation, or a string built from one, such as
          # "${pkg}/bin/tool". The string context proves that the path comes from a derivation the
          # flake evaluates.
          artifactFile = types.coercedTo types.package (package: "${package}") (
            types.addCheck types.str (path: builtins.hasContext path && lib.hasPrefix "${builtins.storeDir}/" path)
            // { description = "file in a flake output"; }
          );
          # A credential, in the shape every publisher type shares: exactly one of the value
          # itself or a secret source to read it from at run time. More sources, siblings of
          # `env`, may follow.
          # `secretOnly` is for what is always a secret, such as a token or password: it may never
          # be written into the declaration.
          credential = { what, valueExample ? null, secretOnly ? false }: types.attrTag ({
            env = mkOption {
              type = types.nonEmptyStr;
              example = "PRESSWERK_TOKEN";
              description = "Name of the environment variable that holds the ${what}, read at run time. `publish` reads every declared secret before it writes anything and stops if one is missing.";
            };
          } // lib.optionalAttrs (!secretOnly) {
            value = mkOption ({
              type = types.nonEmptyStr;
              description = "The ${what} itself, written into the release declaration; only for what is no secret.";
            } // lib.optionalAttrs (valueExample != null) { example = valueExample; });
          });
          # The version that claims and templates are rendered with; its parts differ.
          inherit sampleVersion renderTemplate;
        };

        analyzer.gitmoji.catalog = lib.mapAttrs (_shortcode: lib.mapAttrs (_field: lib.mkDefault)) gitmojiCatalog;
        analyzer.conventional.catalog = lib.mapAttrs (_type: lib.mapAttrs (_field: lib.mkDefault)) conventionalCatalog;

        assertions = map
          ({ address, option, reference }: {
            assertion = false;
            message = "publishers.${address}.${option} references unknown publisher ${builtins.toJSON reference}";
          })
          unknownPublisherReferences
        ++ map
          (address: {
            assertion = false;
            message = "publishers form a cycle: ${address} → ${address}";
          })
          selfDependentPublishers
        ++ [
          {
            assertion = publisherCycle == [ ];
            message = "publishers form a cycle: ${lib.concatStringsSep " → " publisherCyclePath}${lib.optionalString (publisherCycleHints != [ ]) " (${lib.concatStringsSep "; " publisherCycleHints})"}";
          }
        ]
        ++ [
          {
            assertion = sharedClaims == [ ];
            message =
              let shared = builtins.head sharedClaims;
              in "publishers.${shared.claim.address} and publishers.${shared.other.address} both push to ${shared.claim.destination} and both render ${shared.claim.key} for version 1.2.3";
          }
          {
            assertion = config.stable || versionMajor == "0";
            message = "stable may be false only while the version's major component is 0";
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
                    value = lib.getAttrFromPath (keySegments "versionFiles.${kind}.${name}.key" file.key) (parse (read file.path));
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
            ++ lib.mapAttrsToList
              (
                name: workspace:
                  lib.mkDefinition {
                    file = "versionFiles.cargoWorkspace.${name} (${workspace.cargoToml})";
                    value = (builtins.fromTOML (read workspace.cargoToml)).workspace.package.version;
                  }
              )
              config.versionFiles.cargoWorkspace
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
