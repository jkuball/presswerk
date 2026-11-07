import { defineEcConfig, ExpressiveCodeTheme, InlineStyleAnnotation } from '@astrojs/starlight/expressive-code';

/**
 * Noir code themes: structure through greys and weight, colour only for strings.
 * Attribute names, keywords and functions are bold; punctuation and comments are grey;
 * only comments are italic. The greys and the accent match src/styles/presswerk.css.
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

/** Colour the `$` prompt at the start of shell lines with the accent. */
function shellPrompt() {
  return {
    name: 'presswerk-shell-prompt',
    hooks: {
      postprocessAnnotations: ({ codeBlock }) => {
        if (!SHELL_LANGUAGES.has(codeBlock.language)) return;
        for (const line of codeBlock.getLines()) {
          if (!/^\$(\s|$)/.test(line.text)) continue;
          // Style variants follow the order of `themes` below: dark first, then light.
          [ACCENT.dark, ACCENT.light].forEach((color, styleVariantIndex) =>
            line.addAnnotation(
              new InlineStyleAnnotation({
                color,
                styleVariantIndex,
                inlineRange: { columnStart: 0, columnEnd: 1 },
                renderPhase: 'latest',
              }),
            ),
          );
        }
      },
    },
  };
}

export default defineEcConfig({
  plugins: [shellPrompt()],
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
