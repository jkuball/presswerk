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

      gitmojiCommitCheck = pkgs.writeShellApplication {
        name = "check-gitmoji-commit";
        runtimeInputs = [
          pkgs.gnugrep
          pkgs.jq
        ];
        text = ''
          fail() {
            echo "error: $1" >&2
            exit 1
          }

          [ "$#" -eq 1 ] || fail "expected the commit message file as the only argument"
          [ -r "$1" ] || fail "cannot read commit message file: $1"

          title=
          IFS= read -r title < "$1" || true
          [ -n "$title" ] || fail "commit title is empty"

          intention="''${title%%[[:space:]]*}"
          remainder="''${title#"$intention"}"
          [ -n "$remainder" ] || fail "Gitmoji intention must be followed by whitespace"

          message="''${remainder#"''${remainder%%[![:space:]]*}"}"
          [ -n "$message" ] || fail "commit title message must not be empty"

          if printf '%s\n' "$intention" | grep -Pq '^:[A-Za-z0-9_+-]+:$'; then
            intention_kind=shortcode
            # shellcheck disable=SC2016 # jq expands $intention.
            known_filter='any(.gitmojis[]; .code == $intention)'
          elif [[ "$intention" == :* ]]; then
            fail "malformed Gitmoji shortcode; expected :shortcode:"
          elif printf '%s\n' "$intention" | grep -Pq '(*UTF)(*UCP)^\p{Extended_Pictographic}(?:\p{Grapheme_Extend}|\x{200D}\p{Extended_Pictographic})*$'; then
            intention_kind='Unicode intention'
            # shellcheck disable=SC2016 # jq expands $intention.
            known_filter='any(.gitmojis[]; (.emoji | gsub("\uFE0F"; "")) == ($intention | gsub("\uFE0F"; "")))'
          elif printf '%s\n' "$intention" | grep -Pq '(*UTF)(*UCP)^\p{Extended_Pictographic}'; then
            fail "Gitmoji intention must be followed by whitespace"
          else
            fail "commit title must start with a Unicode emoji or :shortcode: intention"
          fi

          if ! jq -e --arg intention "$intention" "$known_filter" "${gitmojis}" >/dev/null; then
            echo "warning: unknown Gitmoji $intention_kind '$intention'; accepting structurally valid intention" >&2
          fi
        '';
      };

      gitmojiCommitCheckTests = pkgs.runCommand "gitmoji-commit-check-tests"
        {
          nativeBuildInputs = [ gitmojiCommitCheck ];
        } ''
        message_file="$TMPDIR/commit-message"

        check_case() {
          expected="$1"
          message="$2"
          printf '%s\n' "$message" > "$message_file"
          status=0
          output="$(check-gitmoji-commit "$message_file" 2>&1)" || status=$?

          case "$expected" in
            silent) [ "$status" -eq 0 ] && [ -z "$output" ] ;;
            warning) [ "$status" -eq 0 ] && printf '%s\n' "$output" | grep -Fq 'warning: unknown Gitmoji' ;;
            failure) [ "$status" -ne 0 ] && printf '%s\n' "$output" | grep -Fq 'error:' ;;
          esac || {
            echo "FAIL: $expected case: $message" >&2
            echo "$output" >&2
            exit 1
          }
        }

        check_case silent '⚡ Improve hook performance'
        check_case silent '🧑‍💻 Improve developer experience'
        check_case silent ':bug: (hooks): Fix commit validation'
        check_case warning '🦄 Preserve custom intentions'
        check_case warning ':party_parrot: Celebrate custom intentions'
        check_case failure 'Fix commit validation'
        check_case failure ':bug Fix commit validation'
        check_case failure '🐛Fix commit validation'
        check_case failure '🐛   '

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
            verbose = true;
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
