import { defineConfig } from 'astro/config';
import tailwindcss from '@tailwindcss/vite';

// Static output (dist/), no UI framework: the only client script loads the WebAssembly demo on click.
export default defineConfig({
  output: 'static',
  // Keep /*! @license */ banners in the minified bundles: GSAP's licence forbids removing its notices.
  vite: { plugins: [tailwindcss()], build: { rolldownOptions: { output: { comments: { legal: true } } } } },
});
