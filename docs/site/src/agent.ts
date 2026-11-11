import { getCollection } from 'astro:content';
import { referenceData } from './reference/data';
import { optionsMarkdown, referencePages } from './reference/markdown';

export interface AgentPage {
  slug: string;
  title: string;
  description: string;
  markdown: string;
}

const base = import.meta.env.BASE_URL.replace(/\/?$/, '/');

export const markdownPath = (slug: string) => `${base}${slug}.md`;

const htmlPath = (slug: string) => (slug === 'index' ? base : `${base}${slug}/`);
const markdownSlug = (id: string) => (id === 'index' ? id : id.replace(/\/index$/, ''));

/** Convert Starlight asides to portable blockquotes. */
function rewriteAsides(markdown: string): string {
  return markdown.replace(/^:::(note|caution)(?:\[([^\]]+)\])?\n([\s\S]*?)\n:::$/gm, (_aside, kind: string, title: string | undefined, body: string) => {
    const label = title ? `${kind[0]!.toUpperCase()}${kind.slice(1)}: ${title}` : `${kind[0]!.toUpperCase()}${kind.slice(1)}`;
    return `> **${label}**\n>\n${body
      .split('\n')
      .map((line) => `> ${line}`)
      .join('\n')}`;
  });
}

/** Rewrite links between documentation pages to their Markdown copies. */
function rewriteLinks(markdown: string, source: string, pages: AgentPage[]): string {
  const routes = new Map(pages.map((page) => [htmlPath(page.slug), markdownPath(page.slug)]));
  const sourceUrl = new URL(htmlPath(source), 'https://presswerk.invalid');
  const siteOrigin = new URL(import.meta.env.SITE!).origin;

  return markdown.replace(/(!?\[[^\]]*\]\()([^)]*)(\))/g, (link, opening: string, destination: string, closing: string) => {
    const match = destination.match(/^(\S+)(.*)$/);
    if (!match) return link;
    const [, target, suffix] = match;
    let url: URL;
    try {
      url = new URL(target, sourceUrl);
    } catch {
      return link;
    }
    if (url.origin !== sourceUrl.origin && url.origin !== siteOrigin) return link;
    const path = routes.get(url.pathname);
    return path ? `${opening}${path}${url.search}${url.hash}${suffix}${closing}` : link;
  });
}

/** Every published documentation page in its agent-facing Markdown form. */
export async function agentPages(): Promise<AgentPage[]> {
  const data = referenceData();
  const generated = new Set(referencePages.map((page) => page.slug));
  const docs = await getCollection('docs');
  const pages: AgentPage[] = docs
    .filter((entry) => !generated.has(entry.id))
    .map((entry) => {
      if (entry.body === undefined) throw new Error(`documentation page ${entry.id} has no Markdown body`);
      return {
        slug: markdownSlug(entry.id),
        title: entry.data.title,
        description: entry.data.description,
        markdown: `# ${entry.data.title}\n\n${entry.body.replaceAll('{{presswerk.version}}', data.version)}`,
      };
    });

  pages.push({
    slug: 'reference/options',
    title: 'Release options',
    description: 'Every option of the release module evaluated by lib.mkRelease.',
    markdown: optionsMarkdown([...data.options].sort((left, right) => left.name.localeCompare(right.name))),
  });
  pages.push(
    ...referencePages.map((page) => ({
      slug: page.slug,
      title: page.title,
      description: page.description,
      markdown: `# ${page.title}\n\n${page.markdown(data, base, 'text')}`,
    })),
  );
  pages.sort((left, right) => (left.slug === 'index' ? -1 : right.slug === 'index' ? 1 : left.slug.localeCompare(right.slug)));

  return pages.map((page) => ({
    ...page,
    markdown: rewriteLinks(`${rewriteAsides(page.markdown).trimEnd()}\n`, page.slug, pages),
  }));
}
