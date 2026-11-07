import { defineEcConfig, ExpressiveCodeTheme } from '@astrojs/starlight/expressive-code';

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

export default defineEcConfig({
  themes: [
    noir({ name: 'presswerk-noir-dark', type: 'dark', ground: '#1b1b1b', ink: '#ececea', faint: '#969693', accent: '#f0705f' }),
    noir({ name: 'presswerk-noir-light', type: 'light', ground: '#e6e6e3', ink: '#111111', faint: '#666663', accent: '#b82e1c' }),
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
