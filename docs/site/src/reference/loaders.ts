import type { Loader } from 'astro/loaders';
import { docsLoader } from '@astrojs/starlight/loaders';
import { referenceData } from './data';
import { referencePages } from './markdown';

/** Starlight's docs loader plus the generated reference pages, rendered from Markdown. */
export function docsWithReferenceLoader(): Loader {
  const docs = docsLoader();
  return {
    name: 'presswerk-docs-with-reference',
    async load(context) {
      await docs.load(context);
      const data = referenceData();
      for (const page of referencePages) {
        const body = page.markdown(data, context.config.base.replace(/\/?$/, '/'));
        context.store.set({
          id: page.slug,
          data: await context.parseData({
            id: page.slug,
            data: { title: page.title, description: page.description, editUrl: false },
          }),
          body,
          // Render as if the page lived in the docs directory, so Starlight's Markdown features apply.
          rendered: await context.renderMarkdown(body, {
            fileURL: new URL(`content/docs/${page.slug}.md`, context.config.srcDir),
          }),
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
