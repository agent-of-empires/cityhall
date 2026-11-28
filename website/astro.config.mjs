import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  site: 'https://cityhall.agent-of-empires.com',
  vite: {
    plugins: [tailwindcss()],
  },
  redirects: { '/': '/docs/' },
  integrations: [sitemap({ changefreq: 'weekly', priority: 0.7 })],
});
