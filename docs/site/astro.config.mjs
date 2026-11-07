import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://jkuball.github.io',
  base: '/presswerk/',
  integrations: [
    starlight({
      title: 'Presswerk',
      sidebar: [{ label: 'Documentation', items: [{ label: 'Overview', slug: 'index' }] }],
    }),
  ],
});
