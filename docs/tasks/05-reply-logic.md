# 05 – Antworten, Allen antworten, Weiterleiten: die Logik

## Warum

Für den Versand braucht Halcyon reine Funktionen, die aus einer empfangenen
Mail den Entwurf einer Antwort oder Weiterleitung machen. Klingt einfach, ist
aber eine Quelle klassischer Ärgernisse: „Re: AW: Re: Re:", die eigene Adresse
in „Allen antworten", Antworten an `noreply@` statt an `Reply-To`, kaputte
Threads beim Empfänger, zerschossene Zitate. Die Funktionen sollen davon
nichts falsch machen – und das mit vielen Tests belegen.

## Dateien

- neu: `crates/halcyon-core/src/reply.rs`
- in `crates/halcyon-core/src/lib.rs` **nur** die Zeile `pub mod reply;`
- `crates/halcyon-core/Cargo.toml`: nur falls `chrono` für die Datumszeile
  gebraucht wird, per `cargo add chrono --no-default-features --features std`

Nichts anderes ändern. Den Versand, die Oberfläche und SMTP baut Claude.

## Schnittstelle

```rust
use crate::model::MessageHeader;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipient {
    pub name: Option<String>,
    /// Kleingeschrieben, ohne spitze Klammern.
    pub address: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyKind {
    Reply,
    ReplyAll,
    Forward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftSeed {
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub subject: String,
    /// Message-ID der Originalmail (ohne `<>`), bei Weiterleitung `None`.
    pub in_reply_to: Option<String>,
    /// Vollständige `References`-Kette (ohne `<>`), bei Weiterleitung leer.
    pub references: Vec<String>,
    /// Vorbelegter Text: zwei Leerzeilen, dann Zitat bzw. Weiterleitungsblock.
    pub body: String,
}

/// Macht aus einer empfangenen Mail den Entwurf einer Antwort/Weiterleitung.
///
/// `own_addresses`: alle Adressen des Nutzers (auch die anderer Konten),
/// kleingeschrieben. `text`: der Textteil der Originalmail (bei HTML-Mails
/// der daraus abgeleitete Text). `utc_offset_minutes`: Zeitzone für die
/// Datumsangabe in der Zitatzeile (Tests sollen deterministisch sein).
pub fn prepare(
    kind: ReplyKind,
    original: &MessageHeader,
    text: &str,
    own_addresses: &[String],
    utc_offset_minutes: i32,
) -> DraftSeed;

/// "Max Muster <max@example.org>", "max@example.org", "\"Muster, Max\" <…>"
/// → Recipient. `None` bei unbrauchbarer Eingabe.
pub fn parse_recipient(s: &str) -> Option<Recipient>;
```

Hinweis: `MessageHeader.to`/`cc` enthalten Einträge als `"Name <adresse>"`
oder `"adresse"`; `from_name`/`from_addr`/`reply_to` sind getrennt bzw. nur
die Adresse. Siehe `crates/halcyon-core/src/model.rs`.

## Regeln

**Betreff**
- Antwort: `"Re: "` + Betreff, vorher **alle** führenden Antwort-Präfixe
  entfernen, beliebig oft und gemischt, ohne Rücksicht auf Groß-/Kleinschreibung:
  `Re`, `RE`, `AW`, `Antw`, `Antwort`, `SV`, `VS`, `R`, `Rif`, `Réf`, `Odp`,
  `Ynt`, `Vá`, `回复`, `答复`, auch mit Zähler (`Re[2]:`, `Re(3):`, `Re^2:`) und
  mit voll- oder halbbreitem Doppelpunkt.
- Weiterleitung: `"Fwd: "` + Betreff, vorher führende Weiterleitungs-Präfixe
  entfernen (`Fwd`, `Fw`, `WG`, `Wtr`, `TR`, `RV`, `Doorst`, `转发`). Antwort-
  Präfixe bleiben bei der Weiterleitung stehen.
- Mailinglisten-Kennungen in eckigen Klammern (`[team] Re: Plan`): Es gibt
  zwei übliche Ergebnisse – `"Re: [team] Plan"` (Thunderbird, Präfix vorne,
  Kennung bleibt) oder `"[team] Re: Plan"` (Kennung vorne). Entscheide dich
  begründet für eine Variante, dokumentiere sie im Code-Kommentar und teste
  sie. Wichtig ist nur: keine doppelten Präfixe und die Kennung geht nicht
  verloren.
- Leerer oder fehlender Betreff → `"Re: "` bzw. `"Fwd: "` ohne Doppel-Leerzeichen.

**Empfänger**
- Antwort an `Reply-To`, sonst an `From`.
- Ist der Absender der Originalmail eine eigene Adresse (Antwort auf eine
  selbst gesendete Mail), geht die Antwort an die ursprünglichen `To`.
- Allen antworten: `to` wie oben; `cc` = ursprüngliche `To` + `Cc`, ohne eigene
  Adressen, ohne Doppelte (Groß-/Kleinschreibung egal), ohne Adressen, die
  schon in `to` stehen. Namen bleiben erhalten.
- Weiterleitung: `to` und `cc` leer.
- `noreply`-Adressen nicht stillschweigend entfernen – aber testen, dass
  `Reply-To` sie richtig ersetzt.

**Thread**
- `in_reply_to` = Message-ID der Originalmail.
- `references` = `References` der Originalmail + deren Message-ID, ohne
  Doppelte. Bei mehr als 20 Einträgen: den ersten und die letzten 19 behalten
  (RFC 5322, 3.6.4 empfiehlt, die Kette zu kürzen, aber den Anfang zu behalten).
- Fehlt die Message-ID, bleiben die Felder leer – kein Absturz.

**Zitat (Antworten)**
- Text: zwei Leerzeilen, dann `"Am {Datum} schrieb {Name}:"`, dann jede Zeile
  der Originalmail mit `"> "` davor; bereits zitierte Zeilen bekommen nur
  `">"` (also `">> "`, nicht `"> > "`), leere Zeilen werden zu `">"`.
- Datum deutsch: `"Di., 10.11.2026 um 08:15"`. Name: `from_name`, sonst
  `from_addr`.
- Signatur der Originalmail (ab der Zeile `"-- "`) wird nicht zitiert.
- Zeilenenden: nur `\n` in der Ausgabe, auch wenn die Eingabe `\r\n` hat.

**Weiterleitung**
- Text: zwei Leerzeilen, dann
  ```
  -------- Weitergeleitete Nachricht --------
  Betreff: …
  Datum: …
  Von: Name <adresse>
  An: …
  ```
  eine Leerzeile, dann der Originaltext **unverändert** (nicht zitiert, mit
  Signatur).

## Tests

Mindestens 40 Fälle in `#[cfg(test)]`: Betreff-Präfixe in allen Varianten,
Empfänger-Kombinationen (Reply-To, eigene Mail, mehrere eigene Konten, Groß-/
Kleinschreibung, Gruppen, Namen mit Komma in Anführungszeichen), lange
`References`-Ketten, fehlende Felder, Zitate mit vorhandenen Zitaten,
Signaturen und `\r\n`. Alle Daten erfunden, nur `*.example`.

## Fertig, wenn

Alle Prüfbefehle aus AGENTS.md grün sind und die Entscheidung zu
Mailinglisten-Kennungen im Ergebnis begründet ist.

## Ergebnis

- `chrono` (`--no-default-features --features std`) zu `crates/halcyon-core/Cargo.toml` hinzugefügt und `pub mod reply;` in `crates/halcyon-core/src/lib.rs` eingetragen.
- `crates/halcyon-core/src/reply.rs`: `Recipient`, `ReplyKind`, `DraftSeed`, `prepare` und `parse_recipient` umgesetzt.
  - **Betreff**: Alle führenden Antwort-/Weiterleitungspräfixe (inkl. internationaler Kürzel, Zähler wie `Re[2]:`/`Re(3):`/`Re^2:` und vollbreitem Doppelpunkt) werden entfernt; bei Weiterleitungen bleiben Antwortpräfixe erhalten; leere Betreffs ergeben `"Re: "` bzw. `"Fwd: "`.
  - **Mailinglisten-Kennungen**: Variante Thunderbird (`"Re: [team] Plan"`). Begründung: Standardkonformes `Re:` am Betreff-Anfang hält Sortierung/Filterung in Clients sauber, während die Listenkennung unverändert erhalten bleibt und doppelte `Re:` vor/nach `[tag]` entfernt werden.
  - **Empfänger & Thread**: `Reply-To` vor `From`, eigene gesendete Mails antworten an `To`, `ReplyAll` dedupliziert `To`+`Cc` ohne eigene Adressen/`to`; `References` auf max. 20 Einträge (erster + letzte 19 nach RFC 5322 3.6.4) gekürzt.
  - **Zitate & Weiterleitung**: `\r\n` → `\n`, deutsches Datum mit `utc_offset_minutes`, Signatur ab `"-- "` beim Zitieren abgeschnitten, bestehende `>`-Zitate als `>>` fortgeführt; Weiterleitung mit unverändertem Text inkl. Signatur.
- 40 Unit-Tests in `reply::tests` geschrieben und geprüft.
- Prüfungen: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` und `cargo test --workspace` grün.
