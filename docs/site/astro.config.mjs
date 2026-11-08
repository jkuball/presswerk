import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { satteri } from '@astrojs/markdown-satteri';
import pelePlugin from './src/plugins/pele.mjs';

export default defineConfig({
  site: 'https://jkuball.github.io',
  base: '/presswerk/',
  markdown: {
    processor: satteri({ mdastPlugins: [pelePlugin()] }),
  },
  integrations: [
    starlight({
      title: 'Presswerk',
      logo: {
        light: './src/assets/brand/lockup-light.svg',
        dark: './src/assets/brand/lockup-dark.svg',
        replacesTitle: true,
      },
      customCss: [
        '@fontsource-variable/archivo/wdth.css',
        '@fontsource-variable/jetbrains-mono/wght.css',
        '@fontsource-variable/jetbrains-mono/wght-italic.css',
        './src/styles/presswerk.css',
      ],
      sidebar: [
        { label: 'Documentation', items: [{ label: 'Overview', slug: 'index' }] },
        {
          label: 'Guide',
          items: [
            { label: 'Why Presswerk', slug: 'guide' },
            { label: 'Release model', slug: 'guide/model' },
            { label: 'Walkthrough', slug: 'guide/walkthrough' },
            { label: 'GitHub Actions', slug: 'guide/github-actions' },
            { label: 'Recovery', slug: 'guide/recovery' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Release options', link: '/reference/options/' },
            { slug: 'reference/declaration-schema' },
            { slug: 'reference/cli' },
            { label: 'Analyzers', items: [{ slug: 'reference/analyzers/gitmoji' }] },
            { label: 'Publishers', items: [{ label: 'GitHub Release', slug: 'reference/publishers/github-release' }] },
          ],
        },
      ],
    }),
  ],
});
