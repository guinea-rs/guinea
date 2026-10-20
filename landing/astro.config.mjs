// @ts-check

import { defineConfig } from 'astro/config';

import { satteri } from '@astrojs/markdown-satteri';
import starlight from '@astrojs/starlight';
import mermaid from 'astro-mermaid';

import { backendTabs, chooseBackend } from './src/backends.mjs';
import { codeThemes } from './src/code-themes.mjs';

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
    mermaid({ theme: 'base', autoTheme: false, enableLog: false, mermaidConfig: { layout: 'elk' } }),
    starlight({
      title: 'guinea',
      favicon: '/guinea.svg',
      customCss: ['./src/styles/starlight.css'],
      expressiveCode: {
        themes: codeThemes,
        useStarlightUiThemeColors: true,
      },
      routeMiddleware: './src/starlightRouteData.ts',
      components: {
        SiteTitle: './src/components/SiteTitle.astro',
      },
      head: [
        { tag: 'link', attrs: { rel: 'preconnect', href: 'https://fonts.googleapis.com' } },
        { tag: 'link', attrs: { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: '' } },
        {
          tag: 'link',
          attrs: {
            rel: 'stylesheet',
            href: 'https://fonts.googleapis.com/css2?family=Chakra+Petch:wght@600&display=swap',
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
          label: 'Application layer',
          translations: { ru: 'Слой приложения' },
          items: [
            { slug: 'concepts/scopes' },
            { slug: 'concepts/features' },
            { slug: 'concepts/plugins' },
            { slug: 'concepts/tasks' },
            { slug: 'concepts/feeds' },
            { slug: 'concepts/event-bus' },
            { slug: 'concepts/rpc' },
            { slug: 'concepts/streams' },
            { slug: 'concepts/lifecycle' },
            { slug: 'concepts/advanced-tasks' },
            { slug: 'concepts/periods' },
          ],
        },
        {
          label: 'Backend integration',
          translations: { ru: 'Интеграция с бэкендами' },
          items: [
            { slug: 'concepts/pages' },
            { slug: 'concepts/composition' },
          ],
        },
      ],
    }),
  ],
});
