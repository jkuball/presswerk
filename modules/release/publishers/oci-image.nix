{
  flake.modules.presswerk.default =
    { config, lib, publisherLib, ... }:
    let
      inherit (lib) mkOption types;
      oci = import ./_oci.nix { inherit lib publisherLib; };
      renderTags = image: map oci.renderTag (image.tags ++ image.aliases);

      ociImageModule = { config, ... }: {
        imports = [ (publisherLib.instance { type = "oci-image"; }) ];

        options = oci.registryOptions "images" // {
          platforms = mkOption {
            type = types.attrsOf publisherLib.artifactFile;
            example = lib.literalExpression ''
              {
                "linux/amd64" = self.packages.x86_64-linux.image;
                "linux/arm64" = self.packages.aarch64-linux.image;
              }
            '';
            description = "The image for each platform, keyed by `os/architecture[/variant]`, which must match the image's config. Each is a Docker image archive, gzip- or zstd-compressed or not, such as the output of `dockerTools.buildImage` or `buildLayeredImage`, or an executable that writes an uncompressed one to stdout, such as the output of `streamLayeredImage`. `publish` fetches them from the local store or a substituter without building. The images of all platforms form one image index.";
          };

          tags = mkOption {
            type = types.listOf oci.tag;
            default = [ "{version}" ];
            example = [ "{version}" "v{version}" ];
            description = "Tags of the release. Each contains `{version}`, which is replaced by the version, and may also use `{major}`, `{minor}`, and `{patch}`. A tag is never moved once it exists.";
          };

          aliases = mkOption {
            type = types.listOf oci.tag;
            default = [ ];
            example = [ "latest" "{major}" "{major}.{minor}" ];
            description = "Floating tags, also called rolling tags, that follow the release line, such as `latest`, `{major}`, or `{major}.{minor}`. An alias may use `{major}`, `{minor}`, and `{patch}`, but not `{version}`. It moves only to a higher final version than the one it points to, so `{major}` stays within its major version.";
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
        };

        config = {
          declaration = oci.registryDeclaration config // {
            inherit (config) platforms tags aliases;
            compression =
              if config.compression.zstd.enable then { zstd.level = config.compression.zstd.level; }
              else { gzip.level = config.compression.gzip.level; };
          };
          claims = map (oci.claim config.repository) (renderTags config);
        };
      };
    in
    {
      imports = [
        (lib.doRename {
          from = [ "publishers" "oci" ];
          to = [ "publishers" "oci-image" ];
          visible = false;
          warn = false;
          use = lib.id;
        })
      ];

      options.publishers.oci-image = mkOption {
        type = types.attrsOf (types.submodule ociImageModule);
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

      config = {
        publisherAliases = [ "oci" ];
        assertions = lib.concatLists (lib.mapAttrsToList
          (name: image: [
            {
              assertion = image.platforms != { };
              message = "publishers.oci-image.${name}.platforms must contain at least one image";
            }
            {
              assertion = image.tags != [ ] && lib.all (lib.hasInfix "{version}") image.tags;
              message = "publishers.oci-image.${name}.tags must contain {version} in every tag, and at least one tag, so every release has a tag of its own; declare tags that follow other versions under aliases";
            }
            {
              assertion = !lib.any (lib.hasInfix "{version}") image.aliases;
              message = "publishers.oci-image.${name}.aliases must not contain {version}, which names a single version; declare such tags under tags";
            }
            {
              assertion = image.compression.gzip.enable != image.compression.zstd.enable;
              message = "publishers.oci-image.${name}.compression needs exactly one of gzip and zstd enabled";
            }
            {
              assertion = lib.allUnique (renderTags image);
              message = "publishers.oci-image.${name}.tags and aliases must not render to the same tag";
            }
          ] ++ oci.registryAssertions "oci-image.${name}" image)
          config.publishers.oci-image);
      };
    };
}
