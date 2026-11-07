{
  perSystem =
    { lib
    , pkgs
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
        version = "0.1.0";
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
          pnpm build
          runHook postBuild
        '';

        installPhase = ''
          runHook preInstall
          mkdir -p $out
          cp -r dist/. $out/
          runHook postInstall
        '';
      });
    };
}
