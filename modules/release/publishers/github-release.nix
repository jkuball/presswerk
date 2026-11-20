{
  flake.modules.presswerk.default =
    { config, lib, publisherLib, ... }:
    let
      inherit (lib) mkOption types;
      # The release tag, rendered like the CLI does with the sample version.
      sampleTag = builtins.replaceStrings [ "{version}" ] [ publisherLib.sampleVersion ] config.tag.format;

      githubReleaseModule = { config, name, options, ... }: {
        imports = [
          (publisherLib.instance {
            type = "github-release";
            afterDefault = builtins.filter (address: !(lib.hasPrefix "github-release." address)) publisherLib.addresses;
            afterDefaultText = lib.literalMD "every publisher that is not a GitHub Release";
          })
        ];

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
            type = types.attrsOf publisherLib.artifactFile;
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

        config = {
          cycleHint = lib.mkIf
            (!options.after.isDefined || options.after.highestPrio >= 1500)
            (lib.mkDefault "github-release.${name}.after defaults to every other publisher; set it explicitly to change that");
          declaration = { inherit (config) apiUrl repository body token assets; };
          # One repository has one release per tag.
          claims = [{
            destination = "${lib.removeSuffix "/" config.apiUrl}/repos/${config.repository}";
            key = sampleTag;
          }];
        };
      };
    in
    {
      options.publishers.github-release = mkOption {
        type = types.attrsOf (types.submodule githubReleaseModule);
        default = { };
        example = lib.literalExpression ''
          {
            default.repository = "owner/project";
          }
        '';
        description = "GitHub Release publishers.";
      };
    };
}
