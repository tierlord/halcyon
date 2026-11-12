# 01 – Modifiziertes UTF-7 für Ordnernamen

## Warum

IMAP überträgt Ordnernamen in „modified UTF-7" (RFC 3501, Abschnitt 5.1.3).
Ein Ordner „Entwürfe" kommt als `Entw&APw-rfe` an, „Gelöscht" als
`Gel&APY-scht`. Halcyon speichert bisher den Rohnamen und zeigt ihn so an.
Server mit `UTF8=ACCEPT` (RFC 6855) liefern dagegen direkt UTF-8.

## Dateien

- neu: `crates/halcyon-core/src/utf7.rs`

Nicht ändern: `lib.rs`, `imap.rs`, `store.rs` – die Einbindung macht Claude.
Damit das Modul kompiliert und getestet werden kann, darf **lokal zum Testen**
`pub mod utf7;` in `lib.rs` ergänzt werden; diese eine Zeile gehört mit in den
Commit.

## Umsetzung

```rust
/// Decodes a mailbox name from modified UTF-7. Invalid input is returned
/// unchanged (servers do send broken names; we must still show something).
pub fn decode(name: &str) -> String;

/// Encodes a mailbox name for use in IMAP commands (CREATE, RENAME, SELECT).
pub fn encode(name: &str) -> String;
```

Regeln aus RFC 3501 5.1.3:

- Druckbare ASCII-Zeichen 0x20–0x7e stehen für sich selbst, außer `&`.
- `&` wird als `&-` kodiert.
- Alles andere: UTF-16BE, Base64 mit `,` statt `/`, ohne Padding, zwischen
  `&` und `-`.
- Beim Kodieren: aufeinanderfolgende Nicht-ASCII-Zeichen in **einen**
  Base64-Block; Surrogatpaare (Emoji) korrekt.
- Beim Dekodieren gilt als ungültig: nicht abgeschlossener Block, falsche
  Base64-Zeichen, ungerade Byteanzahl, einzelne Surrogate, ein Block, der nur
  druckbares ASCII enthält. Dann den **ganzen** Namen unverändert zurückgeben.

Keine neue Abhängigkeit nötig; Base64 für die paar Zeichen selbst schreiben.

## Tests (im Modul, `#[cfg(test)]`)

Mindestens:

| Kodiert | Dekodiert |
|---------|-----------|
| `INBOX` | `INBOX` |
| `Entw&APw-rfe` | `Entwürfe` |
| `Gel&APY-scht` | `Gelöscht` |
| `Tom &- Jerry` | `Tom & Jerry` |
| `&ZeVnLIqe-` | `日本語` |
| `&2D3eAA-` | `😀` |
| `Ordner/&AMQA1gDc-` | `Ordner/ÄÖÜ` |

Plus: Rundreise `decode(encode(x)) == x` für alle Beispiele, und ungültige
Eingaben (`&`, `&AP`, `&APw`, `&AGE-` = nur ASCII im Block, `&2D0-` =
einzelnes Surrogat) kommen unverändert zurück.

## Fertig, wenn

Alle Prüfbefehle aus AGENTS.md grün sind und die Tests oben enthalten sind.

## Ergebnis

- Umgesetzt in `crates/halcyon-core/src/utf7.rs` (`decode`, `encode`, modifiziertes Base64 ohne externe Abhängigkeit, UTF-16BE inkl. Surrogatpaaren, Validierung gemäß RFC 3501 5.1.3) sowie `pub mod utf7;` in `crates/halcyon-core/src/lib.rs`.
- Alle geforderten Testfälle (Bekannte Namen, Rundreise, ungültige Eingaben) ergänzt.
- Prüfungen: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` und `cargo test --workspace` (17/17 Tests) grün.
- Offen: Einbindung in `imap.rs` / `store.rs` (wie vorgesehen durch Claude).
