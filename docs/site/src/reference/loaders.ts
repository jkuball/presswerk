import type { Loader } from 'astro/loaders';
import { docsLoader } from '@astrojs/starlight/loaders';
import { referenceData, type Literal, type ReleaseOption } from './data';
import { optionListPlaceholder, referencePages } from './markdown';

type RenderMarkdown = (markdown: string) => Promise<{ html: string }>;

const escape = (text: string) =>
  text.replace(/[&<>"]/g, (character) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[character]!);

async function literalHtml(literal: Literal, render: RenderMarkdown): Promise<string> {
  if (literal.kind === 'markdown') return (await render(literal.text)).html;
  const ticks = '`'.repeat(Math.max(3, ...[...literal.text.matchAll(/`+/g)].map(([run]) => run.length + 1)));
  return (await render(`${ticks}nix\n${literal.text}\n${ticks}`)).html;
}

/** An option the way the options page shows it, with a heading of the given depth. */
async function optionHtml(option: ReleaseOption, depth: number, render: RenderMarkdown): Promise<string> {
  const field = (term: string, value: string) => `<dt>${term}</dt><dd>${value}</dd>`;
  return [
    `<section class="pw-option" data-option="${escape(`${option.name} ${option.description}`.toLowerCase())}"><h${depth} id="${escape(option.anchor)}"><code>${escape(option.name)}</code></h${depth}>`,
    (await render(option.description)).html,
    '<dl>',
    field('Type', escape(option.type)),
    field('Default', option.default ? await literalHtml(option.default, render) : 'none, the option must be set'),
    option.example ? field('Example', await literalHtml(option.example, render)) : '',
    option.readOnly ? field('Read-only', 'Set by Presswerk; it cannot be defined in a release module.') : '',
    field(
      'Declared in',
      option.declarations
        .map((declaration) => `<a href="${escape(declaration.url)}"><code>${escape(`${declaration.path}:${declaration.line}`)}</code></a>`)
        .join(' '),
    ),
    '</dl></section>',
  ].join('');
}

/** Starlight's docs loader plus the generated reference pages, rendered from Markdown. */
export function docsWithReferenceLoader(): Loader {
  const docs = docsLoader();
  return {
    name: 'presswerk-docs-with-reference',
    async load(context) {
      await docs.load(context);
      const data = referenceData();
      const render: RenderMarkdown = (markdown) => context.renderMarkdown(markdown);
      const options = [...data.options].sort((left, right) => left.name.localeCompare(right.name));
      for (const page of referencePages) {
        const body = page.markdown(data, context.config.base.replace(/\/?$/, '/'), 'site');
        const rendered = await context.renderMarkdown(body, {
          fileURL: new URL(`content/docs/${page.slug}.md`, context.config.srcDir),
        });
        for (const [placeholder, depth, prefix] of [...rendered.html.matchAll(optionListPlaceholder)]) {
          const sections = await Promise.all(
            options.filter((option) => option.name.startsWith(prefix!)).map((option) => optionHtml(option, Number(depth), render)),
          );
          rendered.html = rendered.html.replace(placeholder, `<div class="pw-option-list">${sections.join('')}</div>`);
        }
        context.store.set({
          id: page.slug,
          data: await context.parseData({
            id: page.slug,
            data: { title: page.title, description: page.description, editUrl: false },
          }),
          body,
          // Rendered as if the page lived in the docs directory, so Starlight's Markdown features apply.
          rendered,
          digest: context.generateDigest(body),
        });
      }
    },
  };
}

/** One entry per release option, keyed by its stable anchor, with the description rendered. */
export function optionsLoader(): Loader {
  return {
    name: 'presswerk-release-options',
    async load(context) {
      context.store.clear();
      for (const option of referenceData().options) {
        const data = {
          ...option,
          defaultHtml:
            option.default?.kind === 'markdown' ? (await context.renderMarkdown(option.default.text)).html : null,
        };
        context.store.set({
          id: option.anchor,
          data: await context.parseData({ id: option.anchor, data }),
          body: option.description,
          rendered: await context.renderMarkdown(option.description),
          digest: context.generateDigest(data),
        });
      }
    },
  };
}
