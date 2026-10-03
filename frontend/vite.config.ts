import { searchForWorkspaceRoot } from 'vite';
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Tauri's own reload handles the shell; the page only needs HMR.
    watch: { ignored: ['**/src-tauri/**', '**/crates/**'] },
    // Tests may read the Rust test data they cross-check (the autocorrect
    // corpus, alone in its directory); the dev server serves nothing outside
    // the frontend.
    fs: mode === 'test' ? { allow: [searchForWorkspaceRoot(process.cwd()), '../crates/desktop/src/spellcheck'] } : undefined,
  },
  build: {
    target: 'chrome120', // WebView2 is evergreen Chromium
    sourcemap: false,
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
  },
}));
