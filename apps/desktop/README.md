# Halcyon Desktop

Tauri 2 + Svelte 5 + TypeScript.

```bash
pnpm install
pnpm tauri dev    # App mit echtem Kern und deinen Konten
pnpm dev          # Nur die Oberfläche im Browser, mit Testdaten
```

Im Browser (ohne Tauri) liefert `src/lib/api.ts` Testdaten aus
`src/lib/mock.ts`. So lässt sich die Oberfläche ohne Konten entwickeln.
