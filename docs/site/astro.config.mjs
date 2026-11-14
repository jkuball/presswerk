import { readFileSync } from 'node:fs';
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { satteri } from '@astrojs/markdown-satteri';
import pelePlugin from './src/plugins/pele.mjs';
import versionPlugin from './src/plugins/version.mjs';

export default defineConfig({
  site: 'https://presswerk.dev',
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
      routeMiddleware: './src/routeData.ts',
      components: { Head: './src/components/Head.astro' },
      head: [
        ...['option-filter', 'catalog-filter', 'filter-clear'].map((script) => ({
          tag: 'script',
          attrs: { type: 'module' },
          content: readFileSync(new URL(`./src/scripts/${script}.js`, import.meta.url), 'utf8'),
        })),
      ],
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
            { slug: 'reference/version-files' },
            {
              label: 'Commit analyzers',
              items: [
                { label: 'Overview', slug: 'reference/analyzers' },
                { label: 'Conventional Commits', slug: 'reference/analyzers/conventional-commits' },
                { label: 'Gitmoji', slug: 'reference/analyzers/gitmoji' },
              ],
            },
            {
              label: 'Release publishers',
              items: [
                { label: 'Overview', slug: 'reference/publishers' },
                { label: 'GitHub Release', slug: 'reference/publishers/github-release' },
                { label: 'Helm', slug: 'reference/publishers/helm' },
                { label: 'OCI', slug: 'reference/publishers/oci' },
              ],
            },
          ],
        },
        {
          label: 'More',
          items: [
            { label: 'Recovery', slug: 'guide/recovery' },
            { slug: 'reference/declaration-schema' },
            { slug: 'badges' },
            { slug: 'agents' },
          ],
        },
      ],
    }),
  ],
});
