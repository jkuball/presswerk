import changelogPage from './pages/changelog.md?raw';
import cliPage from './pages/cli.md?raw';
import declarationSchemaPage from './pages/declaration-schema.md?raw';
import githubReleasePage from './pages/github-release.md?raw';
import analyzersPage from './pages/analyzers.md?raw';
import conventionalCommitsPage from './pages/conventional-commits.md?raw';
import gitmojiPage from './pages/gitmoji.md?raw';
import ociPage from './pages/oci.md?raw';
import publishersPage from './pages/publishers.md?raw';
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
  /** The site variant leaves option lists as placeholders that the loader renders like the options page. */
  markdown: (data: ReferenceData, base: string, target: 'site' | 'text') => string;
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
/** Escape Markdown syntax in plain text such as option types, like the pattern `[^/]+/[^/]+`. */
const plain = (text: string) => text.replace(/[\\`*_[\]<>|]/g, '\\$&');

function literalMarkdown(literal: Literal): string {
  if (literal.kind === 'markdown') return literal.text;
  return literal.text.includes('\n') ? fence('nix', literal.text) : code(literal.text);
}

/** A list item for a literal; multi-line values go into an indented block below the label. */
function literalItem(label: string, literal: Literal): string {
  const value = literalMarkdown(literal);
  return value.includes('\n') ? `- ${label}:\n\n  ${value.replace(/\n/g, '\n  ')}\n` : `- ${label}: ${value}`;
}

export function optionMarkdown(option: ReleaseOption, depth = 3): string {
  const lines = [`${'#'.repeat(depth)} ${code(option.name)}`, '', option.description.trim(), '', `- Type: ${plain(option.type)}`];
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

/** Placeholder paragraph for an option list; the loader replaces it in the rendered site page. */
export const optionListPlaceholder = /<p>@@pw-options (\d) (\S+)@@<\/p>/g;

/** The options whose name starts with a prefix, as on the options page, with headings at a depth. */
function optionList(options: ReleaseOption[], prefix: string, depth: number, target: 'site' | 'text'): string {
  if (target === 'site') return `@@pw-options ${depth} ${prefix}@@`;
  return options
    .filter((option) => option.name.startsWith(prefix))
    .map((option) => optionMarkdown(option, depth))
    .join('\n\n');
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

/** Fields shown first in the example, before the longer lists. */
const leadingFields = ['schemaVersion', 'version', 'tag', 'changelog'];

const shorten = (declaration: Record<string, unknown>, keep: number) => {
  const { analyzer } = declaration as { analyzer: { gitmoji: { catalog: Record<string, unknown> } } };
  const catalog = Object.fromEntries(Object.entries(analyzer.gitmoji.catalog).slice(0, keep));
  const leading = Object.fromEntries(leadingFields.map((field) => [field, declaration[field]]));
  return { ...leading, ...declaration, analyzer: { ...analyzer, gitmoji: { catalog } } };
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
    slug: 'reference/changelog',
    title: 'Changelog',
    description: 'How Presswerk renders and commits frozen release notes.',
    markdown: ({ options }, base, target) =>
      fill(changelogPage, base, {
        options: optionList(options, 'changelog.', 3, target),
      }),
  },
  {
    slug: 'reference/publishers',
    title: 'Release publishers',
    description: 'How Presswerk publishes releases outside the repository, and its built-in publishers.',
    markdown: (_, base) => fill(publishersPage, base, {}),
  },
  {
    slug: 'reference/publishers/github-release',
    title: 'GitHub Release publisher',
    description: 'The GitHub Release publisher: behavior, assets, token, and options.',
    markdown: ({ options }, base, target) =>
      fill(githubReleasePage, base, {
        options: optionList(options, 'publishers.github-release.', 3, target),
      }),
  },
  {
    slug: 'reference/publishers/oci',
    title: 'OCI publisher',
    description: 'The OCI publisher for container images: images, tags and aliases, behavior, credentials, and options.',
    markdown: ({ options }, base, target) =>
      fill(ociPage, base, {
        options: optionList(options, 'publishers.oci.', 3, target),
      }),
  },
  {
    slug: 'reference/analyzers',
    title: 'Commit analyzers',
    description: 'How Presswerk turns commits into a version bump and release notes, and how to choose an analyzer.',
    markdown: (_, base) => fill(analyzersPage, base, {}),
  },
  {
    slug: 'reference/analyzers/conventional-commits',
    title: 'Conventional Commits analyzer',
    description: 'The Conventional Commits analyzer, its options, and its default type catalog.',
    markdown: ({ conventional, options }, base, target) =>
      fill(conventionalCommitsPage, base, {
        options: optionList(options, 'analyzer.conventional.', 3, target),
        catalog: [
          '| Type | Default level | Release-note group |',
          '| --- | --- | --- |',
          ...Object.entries(conventional)
            .sort(([left], [right]) => left.localeCompare(right))
            .map(([type, entry]) => `| ${code(type)} | ${code(entry.semver ?? 'null')} | ${plain(entry.title)} |`),
        ].join('\n'),
      }),
  },
  {
    slug: 'reference/analyzers/gitmoji',
    title: 'Gitmoji analyzer',
    description: 'The Gitmoji commit analyzer, its options, and its default catalog.',
    markdown: ({ gitmoji, gitmojiLicense, options }, base, target) =>
      fill(gitmojiPage, base, {
        options: optionList(options, 'analyzer.gitmoji.', 3, target),
        // One list per release level instead of a wide table, so the catalog reads on a phone too.
        catalog: (
          [
            ['major', 'Major'],
            ['minor', 'Minor'],
            ['patch', 'Patch'],
            [null, 'No release'],
          ] as const
        )
          .map(([level, heading]) => {
            const entries = Object.entries(gitmoji)
              .filter(([, entry]) => entry.semver === level)
              .sort(([left], [right]) => left.localeCompare(right));
            return [
              `### ${heading}`,
              '',
              ...entries.map(([shortcode, entry]) => `- ${entry.emoji} ${code(`:${shortcode}:`)}\\\n  ${plain(entry.description)}`),
            ].join('\n');
          })
          .join('\n\n'),
        'catalog-size': String(Object.keys(gitmoji).length),
        license: fence('text', gitmojiLicense),
      }),
  },
];
