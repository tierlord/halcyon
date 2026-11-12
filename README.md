# Halcyon

Ein E-Mail-Programm mit Komfort-Features, die man anderswo vergeblich sucht:
Vorgänge statt loser Einzelmails, kontenübergreifende Kategorien, automatische
Rechnungsablage, Newsletter-Aufräumen, Einmalcodes per Klick – mit lokaler KI,
ohne Cloud.

## Aufbau

| Pfad | Inhalt |
|------|--------|
| `crates/halcyon-core` | UI-unabhängiger Kern: SQLite-Speicher mit Volltextsuche, Kontoerkennung, IMAP-Sync, Schlüsselbund |
| `crates/halcyon-cli`  | Entwickler-CLI (`halcyon`) zum Testen des Kerns mit echten Konten |
| `apps/desktop`        | Desktop-App: Tauri 2 + Svelte 5 + TypeScript |
| `design/`             | Farbpalette und Gestaltungsregeln |
| `docs/tasks/`         | Abgegrenzte Aufgaben für parallel arbeitende KI-Agenten |

## Ausprobieren

```bash
cargo build
./target/debug/halcyon discover max@example.org   # Servereinstellungen ermitteln
./target/debug/halcyon account add max@example.org
./target/debug/halcyon sync
./target/debug/halcyon inbox
./target/debug/halcyon search rechnung telekom
```

Desktop-App (nutzt dieselbe Datenbank und dieselben Konten wie das CLI):

```bash
cd apps/desktop && pnpm install && pnpm tauri dev
```

Passwörter liegen im System-Schlüsselbund (Secret Service), nie in der
Datenbank. Die Datenbank liegt unter `~/.local/share/halcyon/halcyon.db`
(`--db` oder `HALCYON_DB` zum Überschreiben). Mehr Log-Ausgabe mit
`HALCYON_LOG=debug`.

## Stand

Die ausführliche Planung steht in [ROADMAP.md](ROADMAP.md).

- [x] Kontoerkennung: Provider-Autoconfig, Thunderbird-ISPDB, MX, SRV, Raten
- [x] IMAP mit TLS/STARTTLS, Login per Passwort (XOAUTH2 vorbereitet)
- [x] Sync neuer Nachrichten (nur Header), neueste zuerst, UIDVALIDITY-sicher
- [x] Kontenübergreifende Suche und gemeinsamer Posteingang
- [ ] Flag-Änderungen und Löschungen (CONDSTORE/QRESYNC)
- [ ] Nachrichtentext und Anhänge laden
- [ ] OAuth2 (Gmail, Microsoft)
- [ ] SMTP-Versand
- [x] Erste Desktop-Oberfläche: Posteingänge, Konten, Suche, Abrufen
- [ ] Desktop: Mailtext, Aktionen, Kategorien
