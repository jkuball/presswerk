{
  perSystem =
    { config
    , lib
    , pkgs
    , self'
    , ...
    }:
    let
      gitmojiSource = pkgs.fetchFromGitHub {
        owner = "carloscuesta";
        repo = "gitmoji";
        rev = "c7e067f43a33c5d9d42450314977efa409101308";
        hash = "sha256-M7A7a+kYAgeMz7twthGqZaauBaGwMJqPw6StaV/bYmE=";
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
          elif printf '%s\n' "$intention" | grep -Pq '(*UTF)(*UCP)^(?:\p{Extended_Pictographic}(?:\p{Grapheme_Extend}|\x{200D}\p{Extended_Pictographic})*|\p{Regional_Indicator}{2}|[#*0-9]\x{FE0F}?\x{20E3})$'; then
            intention_kind='Unicode intention'
            # shellcheck disable=SC2016 # jq expands $intention.
            known_filter='any(.gitmojis[]; (.emoji | gsub("\uFE0F"; "")) == ($intention | gsub("\uFE0F"; "")))'
          elif printf '%s\n' "$intention" | grep -Pq '(*UTF)(*UCP)^\p{Extended_Pictographic}'; then
            fail "Gitmoji intention must be followed by whitespace"
          else
            fail "commit title must start with a Unicode emoji or :shortcode: intention"
          fi

          if ! jq -e --arg intention "$intention" "$known_filter" "${gitmojiSource}/packages/gitmojis/src/gitmojis.json" >/dev/null; then
            echo "warning: unknown Gitmoji $intention_kind '$intention'; accepting structurally valid intention" >&2
          fi
        '';
      };
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

      devshells.default = {
        packages = [
          config.rust-project.toolchain
          pkgs.prek
        ];

        devshell.startup.install-pre-commit.text = config.pre-commit.installationScript;
      };
    };
}
