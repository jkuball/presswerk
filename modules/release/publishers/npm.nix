{
  flake.modules.presswerk.default =
    { config, lib, publisherLib, ... }:
    let
      inherit (lib) mkOption types;
      aliasLib = import ./_aliases.nix { inherit lib; };
      distTagRule = "https://docs.npmjs.com/cli/commands/npm-dist-tag#caveats";
      # npm refuses a dist-tag that semver reads as a version or range, such as `1.2`, `v1`, or
      # `x`. The CLI checks the same.
      looksLikeVersion = tag:
        builtins.match "[[:space:]]*" tag != null
        || builtins.match "[[:space:]]*([<>]=?|=|~>?|\\^)?[[:space:]]*[vV=]?[[:space:]]*([0-9].*|[xX*]([.[:space:]|-].*)?)" tag != null;
      renderAliases = npm: map publisherLib.renderTemplate npm.aliases;

      npmModule = { config, ... }: {
        imports = [ (publisherLib.instance { type = "npm"; }) ];

        options = {
          name = mkOption {
            type = types.strMatching "(@[^/]+/)?[^/@][^/]*";
            example = "@scope/package";
            description = "The package name, which the tarball's `package.json` must name too.";
          };

          package = mkOption {
            type = publisherLib.artifactFile;
            example = lib.literalExpression "self.packages.x86_64-linux.npm-tarball";
            description = "The package tarball, as `npm pack` writes it, or a directory that holds exactly one `.tgz`. Presswerk publishes it as it is and never packs; since npm never accepts a version twice, it should be byte-reproducible, so a retry finds the same tarball. `publish` fetches it from the local store or a substituter without building.";
          };

          registry = mkOption {
            type = types.strMatching "https?://[^@?#[:space:]]+";
            example = "https://registry.npmjs.org";
            description = "Base URL of the npm registry. There is no default, so a package never lands on a public registry by accident.";
          };

          access = mkOption {
            type = types.enum [ "public" "restricted" ];
            example = "public";
            description = "Who may install the package, as the registry records it with the first upload of a scoped package.";
          };

          tag = mkOption {
            type = types.nonEmptyStr;
            example = "presswerk";
            description = "The dist-tag sent with the upload of each version, as every npm publish sends one. It is never moved afterwards: a rerun for a version that already exists does not touch it. A dedicated tag such as `presswerk`, with `latest` among the `aliases`, keeps `latest` moving forward only.";
          };

          aliases = mkOption {
            type = types.listOf types.nonEmptyStr;
            default = [ ];
            example = [ "latest" "release-{major}.{minor}" ];
            description = "Dist-tags that follow the release line, such as `latest` or `release-{major}`, set through the registry's dist-tag API. ${aliasLib.rule} A registry without the dist-tag API leaves them unset with a warning.";
          };

          token = mkOption {
            type = types.nullOr (publisherLib.credential { what = "npm token"; secretOnly = true; });
            default = null;
            example = { env = "NPM_TOKEN"; };
            description = "The registry token, read from an environment variable at run time. Set either `token`, or `username` and `password`.";
          };

          username = mkOption {
            type = types.nullOr (publisherLib.credential { what = "registry username"; valueExample = "_json_key_base64"; });
            default = null;
            example = { value = "_json_key_base64"; };
            description = "The registry username for basic authentication, together with `password`: `{ env = \"NAME\"; }` reads it from an environment variable at run time; `{ value = \"…\"; }` writes it into the release declaration, for registries where it is no secret.";
          };

          password = mkOption {
            type = types.nullOr (publisherLib.credential { what = "registry password"; secretOnly = true; });
            default = null;
            example = { env = "REGISTRY_PASSWORD"; };
            description = "The registry password for basic authentication, together with `username`, read from an environment variable at run time.";
          };
        };

        config = {
          declaration = {
            inherit (config) name package registry access tag aliases;
          } // lib.filterAttrs (_option: credential: credential != null) {
            inherit (config) token username password;
          };
          # One registry holds each version of a package once, and each dist-tag once. A dist-tag
          # never looks like a version, so the keys cannot collide.
          claims = map
            (key: {
              destination = "npm:${lib.removeSuffix "/" config.registry}";
              key = "${config.name}@${key}";
            })
            ([ "{version}" ] ++ config.aliases);
        };
      };
    in
    {
      options.publishers.npm = mkOption {
        type = types.attrsOf (types.submodule npmModule);
        default = { };
        example = lib.literalExpression ''
          {
            default = {
              name = "@scope/package";
              package = self.packages.x86_64-linux.npm-tarball;
              registry = "https://registry.npmjs.org";
              access = "public";
              tag = "presswerk";
              aliases = [ "latest" ];
              token.env = "NPM_TOKEN";
            };
          }
        '';
        description = "npm package publishers.";
      };

      config.assertions = lib.concatLists (lib.mapAttrsToList
        (name: npm:
          let
            versionLike = builtins.filter looksLikeVersion (renderAliases npm);
          in
          [
            {
              assertion = (npm.token != null) != (npm.username != null && npm.password != null)
                && (npm.token == null || (npm.username == null && npm.password == null));
              message = "publishers.npm.${name} needs either token, or username and password";
            }
            {
              assertion = !looksLikeVersion npm.tag;
              message = "publishers.npm.${name}.tag ${builtins.toJSON npm.tag} would be read as a version or range, which npm refuses as a dist-tag (${distTagRule}); use a name that starts with a letter other than v";
            }
            (aliasLib.assertion {
              address = "npm.${name}";
              inherit (npm) aliases;
              hint = "the release's own dist-tag is tag";
            })
            {
              assertion = versionLike == [ ];
              message = "publishers.npm.${name}.aliases render to ${builtins.toJSON versionLike} for version ${publisherLib.sampleVersion}, which npm would read as a version or range and refuses as a dist-tag (${distTagRule}); use names that start with a letter other than v";
            }
            {
              assertion = !builtins.elem npm.tag (renderAliases npm);
              message = "publishers.npm.${name}.aliases must not contain the tag ${builtins.toJSON npm.tag}: the tag is set with the upload and never moved";
            }
            {
              assertion = lib.allUnique (renderAliases npm);
              message = "publishers.npm.${name}.aliases must not render to the same dist-tag twice";
            }
          ])
        config.publishers.npm);
    };
}
