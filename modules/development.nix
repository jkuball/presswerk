{
  perSystem =
    { config
    , lib
    , pkgs
    , self'
    , ...
    }:
    let
      gitmojis = pkgs.fetchurl {
        url = "https://gitmoji.dev/api/gitmojis";
        hash = "sha256-LL9wc+PodtzbpkMVMH6u2O2E698FFt6gnAnc4m9x7s4=";
      };

      gitmojiCommitCheck = pkgs.writers.writePython3Bin "check-gitmoji-commit"
        {
          libraries = [ pkgs.python3Packages.regex ];
        } ''
        import json
        import sys
        import unicodedata
        from pathlib import Path

        import regex


        GITMOJIS = Path("${gitmojis}")
        SHORTCODE = regex.compile(r":[A-Za-z0-9_+-]+:")
        EXTENDED_PICTOGRAPHIC = regex.compile(r"\p{Extended_Pictographic}")
        REGIONAL_FLAG = regex.compile(r"\p{Regional_Indicator}{2}")
        KEYCAP = regex.compile(r"[#*0-9]\ufe0f?\u20e3")


        def fail(message):
            print("error: " + message, file=sys.stderr)
            raise SystemExit(1)


        def normalize_emoji(value):
            without_variation_selectors = value.translate(
                {ord("\ufe0e"): None, ord("\ufe0f"): None}
            )
            return unicodedata.normalize("NFC", without_variation_selectors)


        def is_unicode_emoji(grapheme):
            return bool(
                EXTENDED_PICTOGRAPHIC.search(grapheme)
                or REGIONAL_FLAG.fullmatch(grapheme)
                or KEYCAP.fullmatch(grapheme)
            )


        if len(sys.argv) != 2:
            fail("expected the commit message file as the only argument")

        message_file = Path(sys.argv[1])
        try:
            lines = message_file.read_text(encoding="utf-8").splitlines()
        except OSError as error:
            fail("cannot read commit message file: " + str(error))

        title = lines[0] if lines else ""
        if not title:
            fail("commit title is empty")

        with GITMOJIS.open(encoding="utf-8") as gitmojis_file:
            known = json.load(gitmojis_file)["gitmojis"]

        known_shortcodes = {entry["code"] for entry in known}
        known_unicode = {normalize_emoji(entry["emoji"]) for entry in known}

        if title.startswith(":"):
            match = SHORTCODE.match(title)
            if match is None:
                fail("malformed Gitmoji shortcode; expected :shortcode:")
            intention = match.group()
            known_intention = intention in known_shortcodes
            intention_kind = "shortcode"
        else:
            match = regex.match(r"\X", title)
            intention = match.group() if match else ""
            if not is_unicode_emoji(intention):
                fail(
                    "commit title must start with a Unicode emoji "
                    "or :shortcode: intention"
                )
            known_intention = normalize_emoji(intention) in known_unicode
            intention_kind = "Unicode intention"

        remainder = title[len(intention):]
        if not remainder:
            fail("commit title message must not be empty")
        if not remainder[0].isspace():
            fail("Gitmoji intention must be followed by whitespace")
        if not remainder.strip():
            fail("commit title message must not be empty")

        if not known_intention:
            print(
                "warning: unknown Gitmoji " + intention_kind + " " + repr(intention)
                + "; accepting structurally valid intention",
                file=sys.stderr,
            )
      '';

      gitmojiCommitCheckTests = pkgs.runCommand "gitmoji-commit-check-tests"
        {
          nativeBuildInputs = [ gitmojiCommitCheck ];
        } ''
        message_file="$TMPDIR/commit-message"

        expect_silent() {
          description="$1"
          message="$2"
          printf '%s\n' "$message" > "$message_file"
          if ! output="$(check-gitmoji-commit "$message_file" 2>&1)"; then
            echo "FAIL: $description should pass" >&2
            echo "$output" >&2
            exit 1
          fi
          if [ -n "$output" ]; then
            echo "FAIL: $description should be silent" >&2
            echo "$output" >&2
            exit 1
          fi
        }

        expect_warning() {
          description="$1"
          message="$2"
          printf '%s\n' "$message" > "$message_file"
          if ! output="$(check-gitmoji-commit "$message_file" 2>&1)"; then
            echo "FAIL: $description should pass with a warning" >&2
            echo "$output" >&2
            exit 1
          fi
          if ! printf '%s\n' "$output" | grep -F 'warning: unknown Gitmoji' >/dev/null; then
            echo "FAIL: $description should warn about an unknown Gitmoji" >&2
            echo "$output" >&2
            exit 1
          fi
        }

        expect_failure() {
          description="$1"
          message="$2"
          printf '%s\n' "$message" > "$message_file"
          if output="$(check-gitmoji-commit "$message_file" 2>&1)"; then
            echo "FAIL: $description should fail" >&2
            exit 1
          fi
          if ! printf '%s\n' "$output" | grep -F 'error:' >/dev/null; then
            echo "FAIL: $description should explain the failure" >&2
            echo "$output" >&2
            exit 1
          fi
        }

        expect_silent 'known Unicode without variation selector' '⚡ Improve hook performance'
        expect_silent 'known canonical ZWJ grapheme' '🧑‍💻 Improve developer experience'
        expect_silent 'known shortcode with optional scope' ':bug: (hooks): Fix commit validation'
        expect_warning 'unknown Unicode emoji' '🦄 Preserve custom intentions'
        expect_warning 'unknown shortcode' ':party_parrot: Celebrate custom intentions'
        expect_failure 'ordinary title' 'Fix commit validation'
        expect_failure 'malformed shortcode' ':bug Fix commit validation'
        expect_failure 'missing separator' '🐛Fix commit validation'
        expect_failure 'empty message' '🐛   '

        touch "$out"
      '';
    in
    {
      treefmt.programs = {
        deadnix.enable = true;
        nixpkgs-fmt.enable = true;
        rustfmt.enable = true;
      };

      pre-commit.settings = {
        package = pkgs.prek;
        hooks = {
          gitmoji-commit = {
            enable = true;
            name = "gitmoji commit message";
            description = "Check the Gitmoji commit title shape.";
            package = gitmojiCommitCheck;
            entry = lib.getExe gitmojiCommitCheck;
            stages = [ "commit-msg" ];
            always_run = true;
          };
          treefmt = {
            enable = true;
            package = self'.formatter;
          };
        };
      };

      checks.gitmoji-commit = gitmojiCommitCheckTests;

      devshells.default = {
        packages = [
          config.rust-project.toolchain
          pkgs.prek
        ];

        devshell.startup.install-pre-commit.text = config.pre-commit.installationScript;
      };
    };
}
