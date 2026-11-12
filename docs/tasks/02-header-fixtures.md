# 02 – Testkorpus für Mail-Header

## Warum

Die geplanten Komfort-Features (Kategorien, Vorgänge, Rechnungen, Newsletter)
hängen davon ab, dass Halcyon Kopfzeilen aus der echten Welt zuverlässig
versteht. Dafür brauchen wir eine Sammlung **synthetischer**, aber
realistischer Header, gegen die wir testen.

## Dateien

- neu: `crates/halcyon-core/tests/fixtures/headers/*.eml`
- neu: `crates/halcyon-core/tests/headers.rs`

Nichts anderes ändern.

## Umsetzung

1. 25–40 Header-Blöcke als einzelne `.eml`-Dateien (nur Kopfzeilen, mit
   `\r\n`-Zeilenenden, Leerzeile am Ende). Dateiname beschreibt den Fall, z. B.
   `marketplace-reply.eml`, `invoice-telecom.eml`.
2. **Ausschließlich erfundene Daten.** Firmen als `*.example`, Personen frei
   erfunden, Message-IDs zufällig. Keine echten Domains außer `example.*`.
3. Abdecken (jeweils mindestens ein Fall):
   - Kleinanzeigen-artige Plattform: mehrere Mails *eines* Gesprächs mit
     gleicher, gesprächsspezifischer `Reply-To`-Adresse und Anzeigentitel im
     Betreff; dazu ein zweites Gespräch zu einer anderen Anzeige
   - Rechnung (Telekom-, Strom-, Online-Shop-artig), Mahnung
   - Bestellung → Versandbestätigung → Zustellung derselben Bestellnummer
   - Einmalcode / 2FA / Login-Bestätigung
   - Newsletter mit `List-Unsubscribe` und `List-Unsubscribe-Post`, einer nur
     mit `mailto:`, einer ganz ohne
   - Antwortketten mit `In-Reply-To` und `References`
   - Kodierungen: `=?UTF-8?B?…?=`, `=?UTF-8?Q?…?=`, `=?ISO-8859-1?Q?…?=`,
     über mehrere Zeilen gefaltete Header, Umlaute und Emoji im Betreff
   - Gruppenadressen (`team: a@x.example, b@x.example;`), mehrere Empfänger,
     `Cc`
   - kaputte Fälle: fehlendes `Date`, ungültiges Datum, `From` ohne Adresse,
     doppelte Header, sehr langer Betreff
4. `tests/headers.rs`: ein Test pro Datei (oder pro Gruppe), der die Datei mit
   `include_bytes!` lädt, `halcyon_core::imap::parse_header_block` aufruft und
   die erwarteten Felder prüft (`subject`, `from_name`, `from_addr`, `to`,
   `date`, `message_id`, `in_reply_to`, `references`, `reply_to`,
   `list_unsubscribe`, `list_unsubscribe_post`).
5. Wenn der Parser einen Fall **falsch** behandelt: den Test trotzdem mit dem
   *richtigen* Erwartungswert schreiben, mit `#[ignore = "…Grund…"]`
   markieren und in der Übergabenotiz auflisten. Nicht den Parser ändern.

## Fertig, wenn

Alle Prüfbefehle grün (ignorierte Tests sind ok) und die Liste der
Abweichungen im Ergebnis steht.

## Ergebnis

- 31 synthetische `.eml`-Header-Blöcke mit `\r\n`-Zeilenenden und abschließender Leerzeile unter `crates/halcyon-core/tests/fixtures/headers/` angelegt (alle Domains ausschließlich `*.example`).
- Abgedeckt: Kleinanzeigen-Konversationen (2 getrennte Gespräche über `Reply-To`), Rechnungen & Mahnung, Bestell-/Versand-/Zustellkette, Einmalcodes & Login-Warnung, Newsletter (`One-Click`, nur `mailto:`, ohne `List-Unsubscribe`), Antwortketten (`In-Reply-To`/`References`), RFC-2047-Kodierungen (`UTF-8` B/Q, `ISO-8859-1` Q), gefaltete Header, Gruppenadressen, Mehrfachempfänger/`Cc` sowie fehlerhafte Header (fehlendes/ungültiges Datum, fehlende Adresse in `From`, doppelte Header, sehr langer Betreff).
- Integrationstests in `crates/halcyon-core/tests/headers.rs` (10 Tests) gegen `parse_header_block`: Alle 10 Tests bestehen ohne `#[ignore]`; der bestehende Parser hat alle 31 Fälle sauber verarbeitet.
- Prüfungen: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` und `cargo test --workspace` (13 Unit- + 10 Integrationstests) grün.
