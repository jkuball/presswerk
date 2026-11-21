{
  flake.modules.presswerk.default =
    { lib, publisherLib, ... }:
    let
      inherit (lib) mkOption types;

      # An HTTP(S) URL without credentials, query, or fragment.
      url = types.strMatching "https?://[^/@?#]+(/[^@?#]*)?";

      # The project name as the index compares it (PEP 503): lowercase, with every run of `-`,
      # `_`, and `.` as one `-`.
      normalize = name: lib.concatStringsSep "-" (builtins.filter builtins.isString (builtins.split "[-_.]+" (lib.toLower name)));

      pythonModule = { config, ... }: {
        imports = [ (publisherLib.instance { type = "python"; }) ];

        options = {
          project = mkOption {
            type = types.strMatching "[A-Za-z0-9]([A-Za-z0-9._-]*[A-Za-z0-9])?";
            example = "my-package";
            description = "The project name on the index, as in `pyproject.toml`. Every distribution's name must match it, compared in normalized form, so `my_package` matches `my-package`.";
          };

          distributions = mkOption {
            type = publisherLib.artifactFile;
            example = lib.literalExpression "self.packages.x86_64-linux.my-package-dist";
            description = "A directory in a flake output that holds the wheels (`.whl`) and the sdist (`.tar.gz`) of the release, and no other files. `publish` fetches it from the local store or a substituter without building. Indexes never replace a file, so the distributions must be reproducible byte for byte.";
          };

          repositoryUrl = mkOption {
            type = url;
            example = "https://upload.pypi.org/legacy/";
            description = "The index's legacy upload API, which `twine` and `uv publish` also upload through. Credentials never go here, only into `username` and `password`.";
          };

          indexUrl = mkOption {
            type = url;
            example = "https://pypi.org/simple/";
            description = "The index's simple API, which Presswerk reads the release's state from. It is read anonymously first, and with `username` and `password` only if the index asks for them.";
          };

          username = mkOption {
            type = publisherLib.credential { what = "index username"; valueExample = "__token__"; };
            example = { value = "__token__"; };
            description = "Index username: `{ value = \"…\"; }` writes it into the release declaration, for indexes where it is no secret, such as `__token__` for PyPI API tokens; `{ env = \"NAME\"; }` reads it from an environment variable at run time.";
          };

          password = mkOption {
            type = publisherLib.credential { what = "index password or token"; secretOnly = true; };
            example = { env = "PYPI_TOKEN"; };
            description = "Index password or API token, such as `{ env = \"PYPI_TOKEN\"; }`, read from an environment variable at run time.";
          };
        };

        config = {
          declaration = { inherit (config) project distributions repositoryUrl indexUrl username password; };
          # An index holds one release per project and version.
          claims = [{
            destination = "${lib.removeSuffix "/" config.indexUrl}/";
            key = "${normalize config.project}=={version}";
          }];
        };
      };
    in
    {
      options.publishers.python = mkOption {
        type = types.attrsOf (types.submodule pythonModule);
        default = { };
        example = lib.literalExpression ''
          {
            default = {
              project = "my-package";
              distributions = self.packages.x86_64-linux.my-package-dist;
              repositoryUrl = "https://upload.pypi.org/legacy/";
              indexUrl = "https://pypi.org/simple/";
              username.value = "__token__";
              password.env = "PYPI_TOKEN";
            };
          }
        '';
        description = "Python publishers.";
      };
    };
}
