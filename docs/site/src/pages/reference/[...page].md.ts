import type { APIRoute, GetStaticPaths } from 'astro';
import { referenceData } from '../../reference/data';
import { optionsMarkdown, referencePages } from '../../reference/markdown';

/** Plain-text Markdown copies of the generated reference pages, next to their HTML pages. */
export const getStaticPaths = (() => {
  const data = referenceData();
  const base = import.meta.env.BASE_URL.replace(/\/?$/, '/');
  return [
    { params: { page: 'options' }, props: { markdown: optionsMarkdown(data.options) } },
    ...referencePages.map((page) => ({
      params: { page: page.slug.replace(/^reference\//, '') },
      props: { markdown: `# ${page.title}\n\n${page.markdown(data, base)}` },
    })),
  ];
}) satisfies GetStaticPaths;

export const GET: APIRoute = ({ props }) =>
  new Response(props.markdown, { headers: { 'Content-Type': 'text/markdown; charset=utf-8' } });
