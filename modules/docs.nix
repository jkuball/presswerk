{ self, ... }:
{
  perSystem =
    { lib
    , pkgs
    , self'
    , ...
    }:
    let
      site = lib.fileset.toSource {
        root = ../docs/site;
        fileset = ../docs/site;
      };
    in
    {
      packages.docs = pkgs.stdenvNoCC.mkDerivation (finalAttrs: {
        pname = "presswerk-docs";
        version = self.presswerk.default.version;
        src = site;
        ASTRO_TELEMETRY_DISABLED = "1";

        pnpmDeps = pkgs.fetchPnpmDeps {
          inherit (finalAttrs) pname version src;
          fetcherVersion = 5;
          hash = "sha256-l/UDZw4t/uW9e2c4baIIfL6xXSaM0ao2EzbjHRYSezg=";
        };

        nativeBuildInputs = [
          pkgs.nodejs
          pkgs.pnpm
          pkgs.pnpmConfigHook
        ];

        buildPhase = ''
          runHook preBuild
          pnpm build --outDir "$out"
          runHook postBuild
        '';

        installPhase = ''
          runHook preInstall
          runHook postInstall
        '';
      });

      checks.docs-version =
        assert lib.assertMsg
          (self'.packages.docs.version == self.presswerk.default.version)
          "packages.docs.version must match presswerk.default.version";
        pkgs.runCommand "docs-version-check" { } "touch $out";
    };
}
