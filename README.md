<p align="center">
  <img src="images/halcyon-icon.svg" alt="Halcyon-Logo: ein Eisvogel mit Briefumschlag" width="160">
</p>

<h1 align="center">Halcyon</h1>

<p align="center">
  E-Mail mit Komfort-Features, die man anderswo vergeblich sucht.
</p>

Vorgänge statt loser Einzelmails, kontenübergreifende Kategorien, automatische
Rechnungsablage, Newsletter-Aufräumen, Einmalcodes per Klick – auf Wunsch mit
lokaler KI, ohne Cloud. Der Name kommt vom Eisvogel (lat. *halcyon*) und den
„halkyonischen Tagen": Ruhe im Posteingang.

## Aufbau

| Pfad | Inhalt |
|------|--------|
| `crates/halcyon-core` | UI-unabhängiger Kern: SQLite-Speicher mit Volltextsuche, Kontoerkennung, IMAP-Sync, Schlüsselbund |
| `crates/halcyon-cli`  | Entwickler-CLI (`halcyon`) zum Testen des Kerns mit echten Konten |
| `apps/desktop`        | Desktop-App: Tauri 2 + Svelte 5 + TypeScript |
| `design/`             | Farbpalette und Gestaltungsregeln |
| `images/`             | Logo (`halcyon-logo.svg`) und zugeschnittenes App-Icon (`halcyon-icon.svg`) |
| `scripts/`            | `install-desktop.sh`: App mit Starter-Icon für den eigenen Benutzer installieren |
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
- [x] Konto-Dialog: Passwort erst nach erfolgreicher Serverprüfung
- [x] IMAP mit TLS/STARTTLS, Login per Passwort (XOAUTH2 vorbereitet)
- [x] Sync neuer Nachrichten, neueste zuerst, UIDVALIDITY-sicher
- [x] Abgleich in beide Richtungen: gelesen, markiert, gelöscht, verschoben
- [x] Mailinhalte (90 Tage vorgeladen, ältere bei Bedarf), Volltextsuche im Text
- [x] Sichere Anzeige: HTML-Filter, CSP, abgeschotteter Rahmen, externe Bilder blockiert, Phishing-Warnung bei Links
- [x] Anhänge speichern
- [x] Desktop-Oberfläche mit hellem und dunklem Design, App-Starter
- [ ] OAuth2 (Gmail, Microsoft)
- [ ] SMTP-Versand
- [ ] Sofortige Zustellung (IDLE)
- [ ] Kategorien, Vorgänge, Spam-Erkennung
