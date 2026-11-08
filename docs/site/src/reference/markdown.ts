import cliPage from './pages/cli.md?raw';
import declarationSchemaPage from './pages/declaration-schema.md?raw';
import githubReleasePage from './pages/github-release.md?raw';
import gitmojiPage from './pages/gitmoji.md?raw';
import type { CliArgument, CliCommand, Literal, ReferenceData, ReleaseOption } from './data';

/**
 * Generated reference pages as Markdown. The same Markdown is rendered into the site and served
 * as plain text next to each page, so both are always built from the same data.
 */

export interface ReferencePage {
  /** Route slug below the site base, without leading or trailing slash. */
  slug: string;
  title: string;
  description: string;
  markdown: (data: ReferenceData, base: string) => string;
}

/**
 * Replace `<!-- reference:name -->` markers in a hand-written page with generated Markdown, and
 * resolve its site links like `](/reference/options/)` against the base, so they work from both
 * the page and its plain-text copy.
 */
function fill(source: string, base: string, sections: Record<string, string>): string {
  return source.replaceAll('](/', `](${base}`).replace(/<!-- reference:([a-z-]+) -->/g, (marker, name: string) => {
    if (!(name in sections)) throw new Error(`unknown reference section ${marker}`);
    return sections[name]!.trimEnd();
  });
}

const fence = (language: string, text: string) => {
  const ticks = '`'.repeat(Math.max(3, ...[...text.matchAll(/`+/g)].map(([run]) => run.length + 1)));
  return `${ticks}${language}\n${text.trimEnd()}\n${ticks}`;
};
const code = (text: string) => (text.includes('`') ? `\`\` ${text} \`\`` : `\`${text}\``);
const cell = (text: string) => text.replace(/\|/g, '\\|').replace(/\n+/g, ' ');
/** Escape Markdown syntax in plain text such as option types, like the pattern `[^/]+/[^/]+`. */
const plain = (text: string) => text.replace(/[\\`*_[\]<>|]/g, '\\$&');

export const optionHref = (base: string, option: Pick<ReleaseOption, 'anchor'>) =>
  `${base}reference/options/#${option.anchor}`;

function literalMarkdown(literal: Literal): string {
  if (literal.kind === 'markdown') return literal.text;
  return literal.text.includes('\n') ? fence('nix', literal.text) : code(literal.text);
}

/** A list item for a literal; multi-line values go into an indented block below the label. */
function literalItem(label: string, literal: Literal): string {
  const value = literalMarkdown(literal);
  return value.includes('\n') ? `- ${label}:\n\n  ${value.replace(/\n/g, '\n  ')}\n` : `- ${label}: ${value}`;
}

export function optionMarkdown(option: ReleaseOption): string {
  const lines = [`### ${code(option.name)}`, '', option.description.trim(), '', `- Type: ${plain(option.type)}`];
  lines.push(option.default ? literalItem('Default', option.default) : '- Default: none, the option must be set');
  if (option.example) lines.push(literalItem('Example', option.example));
  if (option.readOnly) lines.push('- Read-only');
  for (const declaration of option.declarations) {
    lines.push(`- Declared in: [${declaration.path}:${declaration.line}](${declaration.url})`);
  }
  return lines.join('\n');
}

export function optionsMarkdown(options: ReleaseOption[]): string {
  return [
    '# Release options',
    '',
    'Options of the release module evaluated by `lib.mkRelease`.',
    '',
    ...options.map((option) => optionMarkdown(option) + '\n'),
  ].join('\n');
}

function optionsTable(options: ReleaseOption[], base: string): string {
  const rows = options.map((option) => {
    const value = option.default ? cell(option.default.kind === 'nix' ? code(option.default.text) : option.default.text) : 'required';
    return `| [${code(option.name)}](${optionHref(base, option)}) | ${plain(option.type)} | ${value} | ${cell(option.description)} |`;
  });
  return ['| Option | Type | Default | Description |', '| --- | --- | --- | --- |', ...rows].join('\n');
}

function argumentSignature(argument: CliArgument): string {
  const names = [argument.short && `-${argument.short}`, argument.long && `--${argument.long}`].filter(Boolean).join(', ');
  const values = argument.valueNames.map((value) => `<${value}>`).join(' ');
  if (!names) return argument.required ? `<${argument.valueNames[0]}>` : `[${argument.valueNames[0]}]`;
  return values ? `${names} ${values}` : names;
}

function argumentList(title: string, argumentsOrOptions: CliArgument[]): string[] {
  if (argumentsOrOptions.length === 0) return [];
  return [
    `${title}:`,
    '',
    ...argumentsOrOptions.map((argument) => {
      const details = [argument.help ? `${argument.help}.` : '', argument.defaults.length ? `Default: ${argument.defaults.map(code).join(', ')}.` : '']
        .filter(Boolean)
        .join(' ');
      return `- ${code(argumentSignature(argument))}${details ? `: ${details}` : ''}`;
    }),
    '',
  ];
}

function commandMarkdown(command: CliCommand, depth: number): string[] {
  return [
    `${'#'.repeat(depth)} ${command.name}`,
    '',
    ...(command.about ? [`${command.about}.`, ''] : []),
    fence('console', `$ ${command.usage}`),
    '',
    ...argumentList('Arguments', command.arguments),
    ...argumentList('Options', command.options),
    ...command.subcommands.flatMap((subcommand) => commandMarkdown(subcommand, depth + 1)),
  ];
}

const shorten = (declaration: Record<string, unknown>, keep: number) => {
  const { analyzer } = declaration as { analyzer: { gitmoji: { catalog: Record<string, unknown> } } };
  const catalog = Object.fromEntries(Object.entries(analyzer.gitmoji.catalog).slice(0, keep));
  return { ...declaration, analyzer: { ...analyzer, gitmoji: { catalog } } };
};

export const referencePages: ReferencePage[] = [
  {
    slug: 'reference/declaration-schema',
    title: 'Declaration schema',
    description: 'The plain release declaration that lib.mkRelease produces and the CLI reads.',
    markdown: ({ declaration, gitmoji }, base) =>
      fill(declarationSchemaPage, base, {
        example: fence('json', JSON.stringify(shorten(declaration, 2), null, 2)),
        'catalog-size': String(Object.keys(gitmoji).length),
      }),
  },
  {
    slug: 'reference/cli',
    title: 'CLI',
    description: 'Commands, arguments, and options of the presswerk CLI.',
    markdown: ({ cli }, base) =>
      fill(cliPage, base, {
        commands: commandMarkdown(cli, 2).join('\n'),
      }),
  },
  {
    slug: 'reference/publishers/github-release',
    title: 'GitHub Release publisher',
    description: 'How Presswerk publishes GitHub Releases, and the options of the publisher.',
    markdown: ({ options }, base) =>
      fill(githubReleasePage, base, {
        options: optionsTable(
          options.filter((option) => option.name.startsWith('publishers.github-release.')),
          base,
        ),
      }),
  },
  {
    slug: 'reference/analyzers/gitmoji',
    title: 'Gitmoji',
    description: 'Write analyzable commit subjects, configure which Gitmojis change versions, and the default catalog.',
    markdown: ({ gitmoji, gitmojiLicense }, base) =>
      fill(gitmojiPage, base, {
        catalog: [
          '| Gitmoji | Shortcode | Description | Release |',
          '| --- | --- | --- | --- |',
          ...Object.entries(gitmoji)
            .sort(([left], [right]) => left.localeCompare(right))
            .map(
              ([shortcode, entry]) =>
                `| ${entry.emoji} | ${code(`:${shortcode}:`)} | ${cell(entry.description)} | ${entry.semver ?? 'none'} |`,
            ),
        ].join('\n'),
        'catalog-size': String(Object.keys(gitmoji).length),
        license: fence('text', gitmojiLicense),
      }),
  },
];
