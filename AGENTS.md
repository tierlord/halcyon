# Hinweise für KI-Agenten (Claude, Gemini, …)

Halcyon ist ein E-Mail-Programm: Rust-Kern + Tauri 2 + Svelte 5/TypeScript.
Erst Linux, später Windows, eventuell Android. Ziele und Planung:
[README.md](README.md), [ROADMAP.md](ROADMAP.md).

## Aufbau

- `crates/halcyon-core` – UI-unabhängiger Kern. **Keine** Abhängigkeit zu Tauri
  oder anderen UI-Crates, damit er auf allen Plattformen läuft.
- `crates/halcyon-cli` – Entwickler-CLI `halcyon`.
- `apps/desktop` – Tauri-App (`src-tauri/` Rust, `src/` Svelte).
- `design/palette.md` – Farben und Kontrastregeln. Farben in der Oberfläche
  nur über die CSS-Variablen aus `apps/desktop/src/app.css`.

## Regeln

- Code, Bezeichner und Kommentare auf **Englisch**; Texte in der Oberfläche
  und Dokumentation für Nutzer auf **Deutsch**.
- Kommentare erklären das *Warum*, nicht das *Was*.
- Kontoeinrichtung und Sync bleiben **generisch**: Verhalten hängt von
  Server-Fähigkeiten ab, nie vom Anbieternamen (Ausnahme: Gmail-Erweiterungen,
  erkannt an `X-GM-EXT-1`).
- **Keine echten Mails, Adressen oder Zugangsdaten** in Code, Tests oder
  Fixtures. Testdaten nur synthetisch, Domains `example.org`, `example.com`,
  `example.net` oder `*.example` / `*.invalid`.
- Secrets nur über `halcyon_core::secrets` (System-Schlüsselbund), nie in der
  Datenbank, in Logs oder Fehlermeldungen.
- Neue Abhängigkeiten sparsam und begründet; mit `cargo add` hinzufügen.

## Prüfen vor der Übergabe

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Bei Änderungen an der Oberfläche zusätzlich in `apps/desktop`:

```bash
pnpm check
pnpm build
```

Die Oberfläche lässt sich mit `pnpm dev` im Browser mit Testdaten
(`src/lib/mock.ts`) ansehen, ohne Konten.

## Zusammenarbeit

Aufgaben für parallel arbeitende Agenten stehen in [docs/tasks/](docs/tasks/).
Dort steht auch der Ablauf (eigener Branch, eigene Dateien, Übergabenotiz).
