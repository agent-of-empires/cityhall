import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';

export default defineConfig({
  site: 'https://cityhall.example.com',
  redirects: { '/': '/docs/' },
  integrations: [sitemap({ changefreq: 'weekly', priority: 0.7 })],
});
