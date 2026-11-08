import { defineEcConfig, ExpressiveCodeTheme, InlineStyleAnnotation } from '@astrojs/starlight/expressive-code';

/**
 * Noir code themes, after vim-256noir (https://github.com/andreasvc/vim-256noir): code is drawn in
 * ink and greys, and one accent marks what is worth a second look.
 *
 * - Structure is ink, weight and grey. Keywords, functions, attribute names and keys are bold;
 *   punctuation and operators are grey; comments are grey italic.
 * - Values take the accent: quoted strings, paths, numbers and language constants (booleans, null).
 *   Unquoted words (shell arguments, YAML scalars) and object keys stay ink. In YAML nearly every
 *   value is a string, so YAML strings stay ink too and only its literals take the accent.
 * - Meta features take the accent: Rust macros, GitHub Actions expressions (`${{ … }}` and `if:`
 *   conditions) and the `$` prompt of shell sessions. See the plugins below.
 * - Diffs keep Expressive Code's added and removed line markers, the only colour besides the accent.
 *
 * The greys and the accent match src/styles/presswerk.css.
 */
function noir({ name, type, ground, ink, faint, accent }) {
  return new ExpressiveCodeTheme({
    name,
    type,
    colors: {
      'editor.background': ground,
      'editor.foreground': ink,
    },
    settings: [
      { settings: { foreground: ink } },
      {
        scope: ['comment', 'punctuation.definition.comment'],
        settings: { foreground: faint, fontStyle: 'italic' },
      },
      {
        scope: ['punctuation', 'meta.brace', 'keyword.operator', 'punctuation.definition.string'],
        settings: { foreground: faint },
      },
      { scope: ['string', 'string.quoted', 'string.unquoted.path'], settings: { foreground: accent } },
      { scope: ['string.unquoted', 'support.type.property-name', 'entity.name.tag'], settings: { foreground: ink } },
      // In YAML nearly every value is a string, so strings stay ink and the accent marks literals.
      { scope: ['source.yaml string', 'source.yaml string.quoted'], settings: { foreground: ink } },
      // Numbers and language constants (booleans, null) carry the accent everywhere, as in vim-256noir.
      { scope: ['constant.numeric', 'constant.language'], settings: { foreground: accent } },
      // Rust macros expand at compile time, much like GitHub Actions expressions at run time.
      { scope: ['entity.name.function.macro.rust'], settings: { foreground: accent } },
      // Commands substituted inside shell strings are code, not string content.
      { scope: ['meta.scope.subshell'], settings: { foreground: ink } },
      {
        scope: [
          'keyword',
          'storage',
          'entity.name.function',
          'support.function',
          'entity.other.attribute-name',
          'entity.name.tag',
          'support.type.property-name',
          'meta.object-literal.key',
        ],
        settings: { fontStyle: 'bold' },
      },
    ],
  });
}

const ACCENT = { dark: '#f0705f', light: '#b82e1c' };
const SHELL_LANGUAGES = new Set(['console', 'sh', 'bash', 'shell', 'shellsession', 'zsh']);

/** Paint a column range of a line with the accent, in every theme. */
function accent(line, columnStart, columnEnd) {
  // Style variants follow the order of `themes` below: dark first, then light.
  [ACCENT.dark, ACCENT.light].forEach((color, styleVariantIndex) =>
    line.addAnnotation(
      new InlineStyleAnnotation({
        color,
        styleVariantIndex,
        inlineRange: { columnStart, columnEnd },
        renderPhase: 'latest',
      }),
    ),
  );
}

/** Colour the `$` prompt at the start of shell lines with the accent. */
function shellPrompt() {
  return {
    name: 'presswerk-shell-prompt',
    hooks: {
      postprocessAnnotations: ({ codeBlock }) => {
        if (!SHELL_LANGUAGES.has(codeBlock.language)) return;
        for (const line of codeBlock.getLines()) {
          if (/^\$(\s|$)/.test(line.text)) accent(line, 0, 1);
        }
      },
    },
  };
}

/**
 * Mark GitHub Actions expressions in YAML: every `${{ … }}` expression and every `if:` condition,
 * which is an expression without the delimiters.
 */
function actionsExpressions() {
  return {
    name: 'presswerk-actions-expressions',
    hooks: {
      postprocessAnnotations: ({ codeBlock }) => {
        if (!['yaml', 'yml'].includes(codeBlock.language)) return;
        for (const line of codeBlock.getLines()) {
          const condition = /^(\s*(?:-\s+)?if:\s*)(.+)$/.exec(line.text);
          if (condition) accent(line, condition[1].length, line.text.length);
          for (const expression of line.text.matchAll(/\$\{\{.*?\}\}/g)) {
            accent(line, expression.index, expression.index + expression[0].length);
          }
        }
      },
    },
  };
}

/**
 * Highlight a Nix indented string as shell when a `# bash` comment precedes it, the way editors
 * with tree-sitter injections do:
 *
 *   text = # bash
 *     ''
 *       echo hello
 *     '';
 *
 * The closing `''` must start its own line, so the shell grammar cannot swallow it.
 */
const nixShellInjection = {
  name: 'nix-shell-injection',
  scopeName: 'source.nix.shell-injection',
  injectTo: ['source.nix'],
  injectionSelector: 'L:source.nix -comment -string',
  embeddedLangsLazy: ['shellscript'],
  patterns: [
    {
      begin: '#\\s*(?:bash|sh|shell)\\s*$',
      beginCaptures: { 0: { name: 'comment.line.number-sign.nix' } },
      end: "(?<='')",
      patterns: [
        {
          begin: "''",
          beginCaptures: { 0: { name: 'string.quoted.other.nix punctuation.definition.string.other.start.nix' } },
          end: "^\\s*''(?![$'\\\\])",
          endCaptures: { 0: { name: 'string.quoted.other.nix punctuation.definition.string.other.end.nix' } },
          contentName: 'meta.embedded.block.shell',
          patterns: [{ include: 'source.shell' }],
        },
      ],
    },
  ],
};

export default defineEcConfig({
  plugins: [shellPrompt(), actionsExpressions()],
  shiki: { langs: [nixShellInjection] },
  themes: [
    noir({ name: 'presswerk-noir-dark', type: 'dark', ground: '#1b1b1b', ink: '#ececea', faint: '#969693', accent: ACCENT.dark }),
    noir({ name: 'presswerk-noir-light', type: 'light', ground: '#e6e6e3', ink: '#111111', faint: '#666663', accent: ACCENT.light }),
  ],
  useStarlightUiThemeColors: true,
  styleOverrides: {
    borderRadius: '3px',
    borderColor: 'transparent',
    codeFontFamily: 'var(--sl-font-mono)',
    uiFontFamily: 'var(--sl-font)',
    frames: {
      shadowColor: 'transparent',
    },
  },
});
