import type { APIRoute, GetStaticPaths } from 'astro';
import { agentPages, type AgentPage } from '../agent';

export const getStaticPaths = (async () => {
  const pages = await agentPages();
  const ociImage = pages.find((page) => page.slug === 'reference/publishers/oci-image');
  if (!ociImage) throw new Error('OCI image Markdown page is missing');
  return [
    ...pages.map((page) => ({ params: { page: page.slug }, props: page })),
    { params: { page: 'reference/publishers/oci' }, props: ociImage },
  ];
}) satisfies GetStaticPaths;

export const GET: APIRoute<AgentPage> = ({ props }) =>
  new Response(props.markdown, { headers: { 'Content-Type': 'text/markdown; charset=utf-8' } });
