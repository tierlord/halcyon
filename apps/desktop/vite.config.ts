import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

// Tauri expects a fixed port and must see Rust errors in the terminal.
export default defineConfig(({ command }) => ({
  plugins: [
    svelte({
      // In development, component styles travel inside the component's
      // JavaScript instead of as separate CSS requests. A dependency
      // re-optimisation reloads the page while it is still loading; separate
      // CSS requests could then get lost and leave a component unstyled
      // (seen as a broken layout after start). Builds keep real CSS files.
      emitCss: command === 'build',
    }),
  ],
  clearScreen: false,
  // Every library the app imports, bundled up front. Otherwise Vite finds
  // them only while the page loads and reloads it mid-way.
  optimizeDeps: {
    include: [
      'svelte',
      'svelte/transition',
      'svelte/easing',
      '@tauri-apps/api/core',
      '@tauri-apps/api/event',
    ],
  },
  server: {
    port: 5173,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
    // Transform the app before the window asks for it.
    warmup: { clientFiles: ['./src/main.ts', './src/App.svelte'] },
  },
}))
