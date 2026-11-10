import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { defineEcConfig, ExpressiveCodeAnnotation, ExpressiveCodeTheme, InlineStyleAnnotation } from '@astrojs/starlight/expressive-code';
import { h } from '@astrojs/starlight/expressive-code/hast';

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
const INK = { dark: '#ececea', light: '#111111' };
const SHELL_LANGUAGES = new Set(['console', 'sh', 'bash', 'shell', 'shellsession', 'zsh']);

/** Paint a column range of a line in one of the colours above, in every theme. */
function paint(line, colors, columnStart, columnEnd, style = {}) {
  // Style variants follow the order of `themes` below: dark first, then light.
  [colors.dark, colors.light].forEach((color, styleVariantIndex) =>
    line.addAnnotation(
      new InlineStyleAnnotation({
        color,
        ...style,
        styleVariantIndex,
        inlineRange: { columnStart, columnEnd },
        renderPhase: 'latest',
      }),
    ),
  );
}

/** Paint a column range of a line with the accent, in every theme. */
function accent(line, columnStart, columnEnd) {
  paint(line, ACCENT, columnStart, columnEnd);
}

/**
 * Colour the `$` prompt at the start of shell lines with the accent, and draw the command after it
 * in bold. The grammar finds the command only after an environment assignment, so the command is
 * taken from the text: the first word after the prompt and any `NAME=value` assignments.
 */
function shellPrompt() {
  return {
    name: 'presswerk-shell-prompt',
    hooks: {
      postprocessAnnotations: ({ codeBlock }) => {
        if (!SHELL_LANGUAGES.has(codeBlock.language)) return;
        for (const line of codeBlock.getLines()) {
          if (!/^\$(\s|$)/.test(line.text)) continue;
          accent(line, 0, 1);
          const command = /^\$\s+(?:\w+=(?:"[^"]*"|'[^']*'|\S*)\s+)*([^\s#]\S*)/.exec(line.text);
          if (command) {
            const end = command[0].length;
            paint(line, INK, end - command[1].length, end, { bold: true });
          }
        }
      },
    },
  };
}

/**
 * Mark GitHub Actions expressions in YAML: every `${{ … }}` expression and every `if:` condition,
 * which is an expression without the delimiters. The workflow's `on:` key is drawn as a key, not as
 * the YAML 1.1 boolean the grammar takes it for.
 */
function actionsExpressions() {
  return {
    name: 'presswerk-actions-expressions',
    hooks: {
      postprocessAnnotations: ({ codeBlock }) => {
        if (!['yaml', 'yml'].includes(codeBlock.language)) return;
        for (const line of codeBlock.getLines()) {
          if (/^on:/.test(line.text)) paint(line, INK, 0, 2, { bold: true });
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

const OPTIONS_PAGE = '/presswerk/reference/options/';

/** Wrap a column range of a line in a link. */
class LinkAnnotation extends ExpressiveCodeAnnotation {
  constructor({ href, ...options }) {
    super(options);
    this.href = href;
  }

  render({ nodesToTransform }) {
    const last = nodesToTransform.length - 1;
    return nodesToTransform.map((node, index) =>
      h('a', { href: this.href, class: 'pw-code-link' }, index === last ? [node, h('span', { class: 'pw-code-link-icon', 'aria-hidden': 'true' })] : [node]),
    );
  }
}

/** The documented release options from the generated reference data, if the build provides it. */
function releaseOptions() {
  const directory = process.env.PRESSWERK_REFERENCE;
  if (!directory) return [];
  const options = JSON.parse(readFileSync(join(directory, 'options.json'), 'utf8'));
  return options.map((option) => ({ path: option.loc, anchor: option.anchor }));
}

/** The longest documented option that the attribute path starts with; `<name>` matches any name. */
function matchOption(options, path) {
  let best;
  for (const option of options) {
    if (option.path.length > path.length) continue;
    if (!option.path.every((part, index) => part === '<name>' || part === path[index])) continue;
    if (!best || option.path.length > best.path.length) best = option;
  }
  return best;
}

/**
 * Link release options in Nix code to the options reference. Only attributes set inside a
 * `mkRelease { … }` call are linked, with the paths of enclosing attribute sets taken into
 * account, so a `version` attribute of a package elsewhere in the flake stays plain. A block marked
 * `presswerk-module` is a release module, or part of one, as a whole.
 */
function optionLinks() {
  const options = releaseOptions();
  return {
    name: 'presswerk-option-links',
    hooks: {
      postprocessAnnotations: ({ codeBlock }) => {
        if (codeBlock.language !== 'nix' || options.length === 0) return;
        // The attribute set paths currently open inside the release declaration; null outside it.
        let scopes = /(?:^|\s)presswerk-module(?:\s|$)/.test(codeBlock.meta) ? [[]] : null;
        for (const line of codeBlock.getLines()) {
          const text = line.text;
          if (scopes === null) {
            const call = /\bmkRelease\b/.exec(text);
            if (!call) continue;
            line.addAnnotation(
              new LinkAnnotation({ href: OPTIONS_PAGE, inlineRange: { columnStart: call.index, columnEnd: call.index + call[0].length } }),
            );
            if (/\{\s*$/.test(text)) scopes = [[]];
            continue;
          }
          const definition = /^(\s*)([A-Za-z_][\w'-]*(?:\.[A-Za-z_][\w'-]*)*)\s*=(?!=)/.exec(text);
          if (definition) {
            const parts = definition[2].split('.');
            const prefix = scopes[scopes.length - 1];
            const option = matchOption(options, [...prefix, ...parts]);
            if (option) {
              // Link the part of the written path that belongs to the option.
              const own = parts.slice(0, Math.max(0, option.path.length - prefix.length)).join('.');
              if (own) {
                const start = definition[1].length;
                line.addAnnotation(
                  new LinkAnnotation({ href: `${OPTIONS_PAGE}#${option.anchor}`, inlineRange: { columnStart: start, columnEnd: start + own.length } }),
                );
              }
            }
            if (/=\s*\{\s*$/.test(text)) scopes.push([...prefix, ...parts]);
            continue;
          }
          if (/^\s*\}/.test(text)) {
            scopes.pop();
            if (scopes.length === 0) scopes = null;
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
  plugins: [shellPrompt(), actionsExpressions(), optionLinks()],
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
