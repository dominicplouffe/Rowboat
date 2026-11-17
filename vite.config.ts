import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// https://tauri.app/start/frontend/vite/
export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  // Component tests mount real components, which needs Svelte's browser build.
  resolve: mode === 'test' ? { conditions: ['browser'] } : undefined,
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    // The system webviews Tauri 2 supports: WebView2 (Chromium), WKWebView on
    // macOS 10.15+, and WebKitGTK 4.1.
    target: ['chrome105', 'safari13'],
    sourcemap: false,
    reportCompressedSize: false,
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
  },
}));
