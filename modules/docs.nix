{ config, lib, self, ... }:
let
  repositoryUrl = "https://github.com/jkuball/presswerk";
  sourceRevision = self.rev or "main";
  sourcePrefix = "${toString self.outPath}/";

  releaseOptions = (lib.evalModules {
    class = "presswerk";
    modules = [ config.flake.modules.presswerk.default ];
  }).options;

  # Follow a documented location like `publishers.github-release.<name>.repository` into submodules.
  optionAt = node: loc:
    if loc == [ ] then node
    else if lib.isOption node then
      optionAt (node.type.getSubOptions [ ]) (if lib.elem (lib.head loc) [ "<name>" "*" ] then lib.tail loc else loc)
    else optionAt node.${lib.head loc} (lib.tail loc);

  literal = value:
    if value == null then null
    else {
      kind = if value._type == "literalMD" then "markdown" else "nix";
      inherit (value) text;
    };

  sourceLink = position:
    let path = lib.removePrefix sourcePrefix position.file;
    in {
      inherit path;
      inherit (position) line;
      url = "${repositoryUrl}/blob/${sourceRevision}/${path}#L${toString position.line}";
    };

  # The JSON behind the options reference: every visible, non-internal release option.
  optionsReference = map
    (doc:
      let option = optionAt releaseOptions doc.loc;
      in {
        inherit (doc) name loc description readOnly type;
        anchor = lib.concatMapStringsSep "." (part: if part == "<name>" then "name" else part) doc.loc;
        default = if option ? default || option ? defaultText then literal doc.default else null;
        example = literal (doc.example or null);
        declarations = map sourceLink option.declarationPositions;
      })
    (lib.filter
      (doc: doc.visible == true && !doc.internal && lib.head doc.loc != "_module")
      (lib.optionAttrSetToDocList releaseOptions));

  # Presswerk's own declaration, evaluated with this revision's mkRelease, as the schema example.
  schemaExample = config.flake.lib.mkRelease {
    sourceRoot = self.outPath;
    versionFiles.cargo.presswerk = { };
    publishers.github-release.default.repository = "jkuball/presswerk";
  };
in
{
  flake.tests.docs-reference = {
    "test internal options are omitted" = {
      expr = lib.filter (name: lib.hasPrefix "_module" name || lib.hasPrefix "assertions" name) (map (option: option.name) optionsReference);
      expected = [ ];
    };

    "test submodule options resolve with defaults and source links" = {
      expr = lib.removeAttrs (lib.findFirst (option: option.name == "publishers.github-release.<name>.token.secret") null optionsReference) [ "description" "declarations" ];
      expected = {
        name = "publishers.github-release.<name>.token.secret";
        loc = [ "publishers" "github-release" "<name>" "token" "secret" ];
        anchor = "publishers.github-release.name.token.secret";
        readOnly = false;
        type = "non-empty string";
        default = {
          kind = "nix";
          text = ''"GITHUB_TOKEN"'';
        };
        example = {
          kind = "nix";
          text = ''"PRESSWERK_GITHUB_TOKEN"'';
        };
      };
    };

    "test options without a declared default have none" = {
      expr = (lib.findFirst (option: option.name == "analyzer.gitmoji.catalog.<name>.semver") null optionsReference).default;
      expected = null;
    };

    "test declarations link to repository paths" = {
      expr = map (declaration: declaration.path) (lib.findFirst (option: option.name == "tag.format") null optionsReference).declarations;
      expected = [ "modules/release/declaration-module.nix" ];
    };
  };

  perSystem =
    { config
    , lib
    , pkgs
    , self'
    , ...
    }:
    let
      site = lib.fileset.toSource {
        root = ../docs/site;
        fileset = ../docs/site;
      };

      cliReference = config.rust-project.crane-lib.mkCargoDerivation (
        config.rust-project.crates.presswerk.crane.args
        // {
          inherit (config.rust-project) src;
          pname = "presswerk-cli-reference";
          version = self.presswerk.default.version;
          cargoArtifacts = self'.packages.presswerk.cargoArtifacts;
          doInstallCargoArtifacts = false;
          buildPhaseCargoCommand = "cargoWithProfile run --locked --example cli-reference > cli.json";
          installPhaseCommand = "install -Dm644 cli.json $out/cli.json";
        }
      );
    in
    {
      # Generated data behind the reference pages and the documented version; the site reads it from
      # $PRESSWERK_REFERENCE.
      packages.docs-reference = pkgs.runCommand "presswerk-docs-reference" { } ''
        mkdir -p $out
        cp ${cliReference}/cli.json $out/cli.json
        cp ${pkgs.writeText "options.json" (builtins.toJSON optionsReference)} $out/options.json
        cp ${pkgs.writeText "declaration.json" (builtins.toJSON schemaExample)} $out/declaration.json
        cp ${./release/gitmoji/catalog.json} $out/gitmoji.json
        cp ${./release/gitmoji/LICENSE} $out/gitmoji-license.txt
        printf '%s' ${lib.escapeShellArg self.presswerk.default.version} > $out/version
      '';

      packages.docs = pkgs.stdenvNoCC.mkDerivation (finalAttrs: {
        pname = "presswerk-docs";
        version = self.presswerk.default.version;
        src = site;
        ASTRO_TELEMETRY_DISABLED = "1";
        PRESSWERK_REFERENCE = self'.packages.docs-reference;

        pnpmDeps = pkgs.fetchPnpmDeps {
          inherit (finalAttrs) pname version src;
          fetcherVersion = 5;
          hash = "sha256-qd4L9vGdG2/H1p0+7pNh3EsWfJsenJEyeBwyntDmBsI=";
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

      checks.docs-reference = pkgs.runCommand "docs-reference-check" { nativeBuildInputs = [ pkgs.gnugrep ]; } ''
        site=${self'.packages.docs}
        pages="
          404
          badges
          guide
          guide/adoption
          guide/ci-integration
          guide/github-actions
          guide/model
          guide/nixci
          guide/recovery
          guide/release-modes
          guide/walkthrough
          index
          reference/analyzers
          reference/cli
          reference/declaration-schema
          reference/options
          reference/publishers
          reference/version-files
        "
        for page in $pages; do
          test -s "$site/$page.md" || { echo "missing Markdown page $page.md" >&2; exit 1; }
        done
        for page in options declaration-schema cli publishers analyzers; do
          test -s "$site/reference/$page/index.html" || { echo "missing reference page $page" >&2; exit 1; }
        done
        test -s "$site/llms.txt"
        test ! -e "$site/llms-full.txt"
        for page in $pages; do
          test "$page" = 404 || grep -Fq "(https://jkuball.github.io/presswerk/$page.md)" "$site/llms.txt"
        done
        grep -Fq 'rel="alternate" type="text/markdown" href="/presswerk/index.md"' "$site/index.html"
        grep -Fq 'rel="describedby" href="/presswerk/llms.txt"' "$site/index.html"
        grep -Fq 'rel="alternate" type="text/markdown" href="/presswerk/guide/walkthrough.md"' "$site/guide/walkthrough/index.html"
        grep -q 'id="tag.format"' "$site/reference/options/index.html"
        # Release options in Nix examples link to their reference entries.
        grep -q 'href="/presswerk/reference/options/#versionFiles.cargo" class="pw-code-link"' "$site/index.html"
        grep -q 'id="publishers.github-release.name.token.secret"' "$site/reference/options/index.html"
        grep -q '^# Presswerk$' "$site/index.md"
        grep -q '^# Walkthrough: release automatically from GitHub Actions$' "$site/guide/walkthrough.md"
        grep -Fq '](/presswerk/reference/options.md#versionFiles.cargo)' "$site/reference/version-files.md"
        grep -q '^### `tag.format`$' "$site/reference/options.md"
        grep -q '^### presswerk publish$' "$site/reference/cli.md"
        grep -A20 '^### Minor$' "$site/reference/analyzers.md" | grep -q '`:sparkles:`'
        # The vendored catalog is MIT licensed; its notice must travel with the published copy.
        for copy in "$site/reference/analyzers.md" "$site/reference/analyzers/index.html"; do
          grep -q 'Copyright (c) 2016-2022 Carlos Cuesta' "$copy"
          grep -q 'copies or substantial portions of the Software' "$copy"
        done
        if grep -R -n -E '\{\{presswerk\.version\}\}|@@pw-options|<!-- reference:|^:::' "$site" --include='*.md' --include='llms.txt'; then
          echo "unresolved documentation placeholder" >&2
          exit 1
        fi
        if grep -q 'assertions' "$site/reference/options.md"; then
          echo "internal options leaked into the reference" >&2
          exit 1
        fi
        touch $out
      '';

      checks.docs-version =
        assert lib.assertMsg
          (self'.packages.docs.version == self.presswerk.default.version)
          "packages.docs.version must match presswerk.default.version";
        pkgs.runCommand "docs-version-check" { } "touch $out";
    };
}
