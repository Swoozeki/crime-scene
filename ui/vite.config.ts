import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { viteSingleFile } from 'vite-plugin-singlefile';

// One self-contained index.html: embedded in the csi binary and reused for static export.
export default defineConfig({
  plugins: [svelte(), viteSingleFile()],
  server: { proxy: { '/api': 'http://127.0.0.1:7777' } },
  build: { target: 'es2022', chunkSizeWarningLimit: 2000 },
});
