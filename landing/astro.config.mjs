// @ts-check

import { defineConfig } from 'astro/config';

import { satteri } from '@astrojs/markdown-satteri';
import starlight from '@astrojs/starlight';
import mermaid from 'astro-mermaid';

import { backendTabs, chooseBackend } from './src/backends.mjs';

export default defineConfig({
  site: 'https://guinea-rs.github.io',
  base: '/guinea',

  markdown: {
    processor: satteri({ mdastPlugins: [backendTabs()] }),
  },

  vite: {
    server: { fs: { allow: ['..'] } },
  },

  integrations: [
    mermaid({ autoTheme: true, enableLog: false, mermaidConfig: { layout: 'elk' } }),
    starlight({
      title: 'guinea',
      favicon: '/guinea.svg',
      customCss: ['./src/styles/starlight.css'],
      routeMiddleware: './src/starlightRouteData.ts',
      components: {
        SiteTitle: './src/components/SiteTitle.astro',
      },
      head: [
        { tag: 'link', attrs: { rel: 'preconnect', href: 'https://fonts.googleapis.com' } },
        { tag: 'link', attrs: { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: true } },
        {
          tag: 'link',
          attrs: {
            rel: 'stylesheet',
            href: 'https://fonts.googleapis.com/css2?family=Chakra+Petch:wght@500;700&display=swap',
          },
        },
        { tag: 'script', content: chooseBackend },
      ],
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/guinea-rs/guinea' }],

      defaultLocale: 'root',
      locales: {
        root: { label: 'English', lang: 'en' },
        ru: { label: 'Русский', lang: 'ru' },
      },

      sidebar: [
        { slug: 'introduction' },
        {
          label: 'Concepts',
          translations: { ru: 'Концепции' },
          items: [{ autogenerate: { directory: 'Concepts' } }],
        },
      ],
    }),
  ],
});
