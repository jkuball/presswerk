import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { satteri } from '@astrojs/markdown-satteri';
import pelePlugin from './src/plugins/pele.mjs';
import versionPlugin from './src/plugins/version.mjs';

export default defineConfig({
  site: 'https://jkuball.github.io',
  base: '/presswerk/',
  markdown: {
    processor: satteri({ mdastPlugins: [versionPlugin(), pelePlugin()] }),
  },
  integrations: [
    starlight({
      title: 'Presswerk',
      logo: {
        light: './src/assets/brand/lockup-light.svg',
        dark: './src/assets/brand/lockup-dark.svg',
        replacesTitle: true,
      },
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/jkuball/presswerk' }],
      customCss: [
        '@fontsource-variable/archivo/wdth.css',
        '@fontsource-variable/jetbrains-mono/wght.css',
        '@fontsource-variable/jetbrains-mono/wght-italic.css',
        './src/styles/presswerk.css',
      ],
      sidebar: [
        {
          label: 'Documentation',
          items: [
            { label: 'Overview', slug: 'index' },
            { label: 'Why Presswerk', slug: 'guide' },
            { label: 'Release model', slug: 'guide/model' },
            { label: 'Release modes', slug: 'guide/release-modes' },
            { label: 'Walkthrough', slug: 'guide/walkthrough' },
            { label: 'Adopt an existing project', slug: 'guide/adoption' },
            {
              label: 'CI integration',
              items: [
                { slug: 'guide/ci-integration' },
                { label: 'GitHub Actions', slug: 'guide/github-actions' },
                { label: 'NixCI', slug: 'guide/nixci' },
              ],
            },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Release options', link: '/reference/options/' },
            { slug: 'reference/cli' },
            {
              label: 'Version files',
              items: [{ slug: 'reference/version-files' }, { slug: 'reference/version-files/build-your-own' }],
            },
            {
              label: 'Commit analyzers',
              items: [
                { slug: 'reference/analyzers' },
                { slug: 'reference/analyzers/gitmoji' },
                { slug: 'reference/analyzers/build-your-own' },
              ],
            },
            {
              label: 'Release publishers',
              items: [
                { slug: 'reference/publishers' },
                { label: 'GitHub Release', slug: 'reference/publishers/github-release' },
                { slug: 'reference/publishers/build-your-own' },
              ],
            },
          ],
        },
        {
          label: 'More',
          items: [
            { label: 'Recovery', slug: 'guide/recovery' },
            { slug: 'reference/declaration-schema' },
          ],
        },
      ],
    }),
  ],
});
