import { defineConfig } from 'astro/config';
import tailwindcss from '@tailwindcss/vite';

// Static output (dist/), no UI framework: the only client script loads the WebAssembly demo on click.
export default defineConfig({
  output: 'static',
  // GitHub Pages serves the site under /ai-ui-rewrite-rust/ (scripts/deploy-pages.sh sets SITE_BASE); '/' otherwise.
  base: process.env.SITE_BASE ?? '/',
  // Keep /*! @license */ banners in the minified bundles: GSAP's licence forbids removing its notices.
  vite: { plugins: [tailwindcss()], build: { rolldownOptions: { output: { comments: { legal: true } } } } },
});
