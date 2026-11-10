{
  perSystem =
    { config
    , lib
    , pkgs
    , ...
    }:
    let
      # The variable Archivo font (SIL Open Font License), pinned to a google/fonts revision.
      archivo = pkgs.fetchurl {
        name = "archivo-variable.ttf";
        url = "https://raw.githubusercontent.com/google/fonts/86aced91de05cd4f96a06838124cca45be337b2b/ofl/archivo/Archivo%5Bwdth%2Cwght%5D.ttf";
        hash = "sha256-DglKfTx8TCXPExDEswAU8drpMyIgscLIj0+plvCwUFM=";
      };

      python = pkgs.python3.withPackages (ps: [
        ps.fonttools
        ps.shapely
      ]);

      generator = ../docs/brand/generate.py;

      brandAssets = pkgs.writeShellApplication {
        name = "brand-assets";
        runtimeInputs = [
          pkgs.gitMinimal
          python
        ];
        text = ''
          root=$(git rev-parse --show-toplevel)
          python3 ${generator} ${archivo} "$root"
          echo "brand assets written to $root/docs/site/src/assets/brand and $root/docs/site/public"
        '';
      };
    in
    {
      packages.brand-assets = brandAssets;

      checks.brand-assets = pkgs.runCommand "brand-assets-check" { nativeBuildInputs = [ python ]; } ''
        python3 ${generator} ${archivo} generated
        diff -r ${../docs/site/src/assets/brand} generated/docs/site/src/assets/brand
        diff ${../docs/site/public/favicon.svg} generated/docs/site/public/favicon.svg
        diff ${../docs/site/public/badge.svg} generated/docs/site/public/badge.svg
        diff ${../docs/site/public/badge-light.svg} generated/docs/site/public/badge-light.svg
        touch $out
      '';

      devshells.default.commands = [
        {
          name = "brand-assets";
          help = "Regenerate the logo, icon, lockup, favicon and badge";
          command = lib.getExe config.packages.brand-assets;
        }
      ];
    };
}
