import { defineConfig } from 'astro/config';
import tailwindcss from '@tailwindcss/vite';

// Static output (dist/), no UI framework: the only client script loads the WebAssembly demo on click.
export default defineConfig({
  output: 'static',
  vite: { plugins: [tailwindcss()] },
});
