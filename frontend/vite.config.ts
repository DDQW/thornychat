import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Tauri's own reload handles the shell; the page only needs HMR.
    watch: { ignored: ['**/src-tauri/**', '**/crates/**'] },
  },
  build: {
    target: 'chrome120', // WebView2 is evergreen Chromium
    sourcemap: false,
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
  },
});
