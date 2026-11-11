import type { APIRoute } from 'astro';
import { agentPages, markdownPath } from '../agent';

export const GET: APIRoute = async () => {
  const site = import.meta.env.SITE;
  if (!site) throw new Error('Astro site URL is required to build llms.txt');
  const pages = (await agentPages()).filter((page) => page.slug !== '404');
  const markdown = [
    '# Presswerk',
    '',
    '> Release tooling for projects whose artifacts are built with Nix flakes.',
    '',
    'Presswerk records versions and release notes in a release commit, then publishes exactly the commit that was built and checked.',
    '',
    '## Documentation',
    '',
    ...pages.map((page) => `- [${page.title}](${new URL(markdownPath(page.slug), site)}): ${page.description}`),
    '',
  ].join('\n');

  return new Response(markdown, { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
