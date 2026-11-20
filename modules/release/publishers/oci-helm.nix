{
  flake.modules.presswerk.default =
    { lib, publisherLib, ... }:
    let
      inherit (lib) mkOption types;
      oci = import ./_oci.nix { inherit lib publisherLib; };

      ociHelmModule = { config, ... }: {
        imports = [ (publisherLib.instance { type = "oci-helm"; }) ];

        options = oci.registryOptions "chart" // {
          chart = mkOption {
            type = publisherLib.artifactFile;
            example = lib.literalExpression "self.packages.x86_64-linux.chart";
            description = "Chart archive written by `helm package`. `publish` fetches it from the local store or a substituter without building.";
          };
        };

        config = {
          declaration = oci.registryDeclaration config // { inherit (config) chart; };
          # A chart is pushed under the release version itself.
          claims = [ (oci.claim config.repository publisherLib.sampleVersion) ];
        };
      };
    in
    {
      options.publishers.oci-helm = mkOption {
        type = types.attrsOf (types.submodule ociHelmModule);
        default = { };
        example = lib.literalExpression ''
          {
            default = {
              repository = "ghcr.io/owner/charts/project";
              chart = self.packages.x86_64-linux.chart;
              public = true;
              username.secret.env = "GITHUB_ACTOR";
              password.secret.env = "GITHUB_TOKEN";
            };
          }
        '';
        description = "OCI Helm publishers.";
      };

    };
}
