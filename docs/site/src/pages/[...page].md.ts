import type { APIRoute, GetStaticPaths } from 'astro';
import { agentPages, type AgentPage } from '../agent';

export const getStaticPaths = (async () =>
  (await agentPages()).map((page) => ({ params: { page: page.slug }, props: page }))) satisfies GetStaticPaths;

export const GET: APIRoute<AgentPage> = ({ props }) =>
  new Response(props.markdown, { headers: { 'Content-Type': 'text/markdown; charset=utf-8' } });
