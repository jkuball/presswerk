{ config, self, ... }:
{
  # Presswerk releases itself with the module and CLI of the commit being released.
  flake.presswerk.default = config.flake.lib.mkRelease {
    sourceRoot = self.outPath;
    versionFiles.cargo.presswerk = { };
    # The README badge follows the viewer's colour scheme; the docs show the variant for their theme.
    versionFiles.badge = {
      default = { };
      light = {
        colorScheme = "light";
        path = ".github/presswerk-badge-light.svg";
      };
      dark = {
        colorScheme = "dark";
        path = ".github/presswerk-badge-dark.svg";
      };
    };
    publishers.github-release.default.repository = "jkuball/presswerk";
    # Presswerk's own commits use Gitmoji.
    analyzer.gitmoji.enable = true;
    # The hand-written highlights carry the release; the generated commits follow as one flat list,
    # each marked with its Gitmoji, folded into a <details> block so they do not bury them. A
    # release without highlights shows the list unfolded. GitHub needs the blank lines around the
    # Markdown inside <details>.
    changelog.template = builtins.readFile ./release-notes.md.j2;
  };
}
