import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

// Set by `tauri android dev` / `tauri ios dev`: the phone or emulator loads
// the dev server over the network, so it must listen beyond localhost.
const host = process.env.TAURI_DEV_HOST

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
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 5174 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
    // Transform the app before the window asks for it.
    warmup: { clientFiles: ['./src/main.ts', './src/App.svelte'] },
  },
}))
