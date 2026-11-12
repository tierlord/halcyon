# 03 – Einmalcodes und Sendungsnummern erkennen

## Warum

Zwei geplante Komfort-Features: Einmalcodes mit einem Klick kopieren und
Sendungen direkt verfolgen. Dafür brauchen wir reine Funktionen, die aus
Betreff und Text die Werte herausziehen – ohne Netzwerk, ohne KI, schnell und
gut getestet.

## Dateien

- neu: `crates/halcyon-core/src/extract/mod.rs`
- neu: `crates/halcyon-core/src/extract/otp.rs`
- neu: `crates/halcyon-core/src/extract/tracking.rs`
- in `crates/halcyon-core/src/lib.rs` **nur** die Zeile `pub mod extract;`
- `crates/halcyon-core/Cargo.toml`: nur falls `regex` gebraucht wird, per
  `cargo add regex`

## Teil A: Einmalcodes (`otp.rs`)

```rust
pub struct OneTimeCode {
    pub code: String,
    /// 0.0–1.0; the UI only offers codes above a threshold.
    pub confidence: f32,
}

/// Finds verification / one-time codes in a message.
pub fn find_codes(subject: &str, body_text: &str) -> Vec<OneTimeCode>;
```

- Codes: 4–8 Ziffern, auch mit Leerzeichen oder Bindestrich gruppiert
  (`482 913`, `482-913`), außerdem 6–8 Zeichen alphanumerisch in Großbuchstaben
  mit mindestens einer Ziffer (`K7Q2MX`). Ausgabe ohne Trennzeichen.
- Hohe Konfidenz nur mit Schlüsselwort in der Nähe (gleiche Zeile oder
  direkt davor), Deutsch und Englisch: Code, Bestätigungscode,
  Sicherheitscode, Einmalpasswort, TAN, PIN, Verifizierung, verification,
  security code, one-time, OTP, passcode, login code.
- Nicht als Code werten: Jahreszahlen 1900–2099 ohne Schlüsselwort, Uhrzeiten,
  Datumsangaben, Preise (`12,99 €`), Telefonnummern, PLZ in Adressen,
  Bestell- und Rechnungsnummern mit Schlüsselwort wie „Bestellnummer",
  „Rechnungsnr.", „Kundennummer".
- Treffer im Betreff zählen mehr als im Text. Sortiert nach Konfidenz,
  Duplikate zusammengefasst.

## Teil B: Sendungsnummern (`tracking.rs`)

```rust
pub enum Carrier { Dhl, DhlExpress, Dpd, Hermes, Gls, Ups, FedEx, Usps, DeutschePostBrief }

pub struct Shipment {
    pub carrier: Carrier,
    pub tracking_number: String,
    /// Public tracking page for this number.
    pub url: String,
}

pub fn find_shipments(subject: &str, body_text: &str) -> Vec<Shipment>;
```

- Erkennung über bekannte Nummernformate **und** Kontext (Name des
  Versanddienstes oder Wörter wie Sendungsnummer, Paketnummer, tracking
  number in der Nähe). Ohne Kontext nur Formate, die eindeutig sind (z. B. UPS
  `1Z` + 16 Zeichen).
- Wo es eine Prüfziffer gibt (UPS, DHL-Paket mit 12/20 Stellen, USPS),
  prüfen und bei falscher Prüfziffer verwerfen. Quelle der Regel im Kommentar
  angeben.
- Tracking-Links, die schon in der Mail stehen (z. B.
  `https://…/sendungsverfolgung?…=NUMMER`), ebenfalls auswerten.

## Tests

Pro Teil mindestens 20 Fälle in `#[cfg(test)]`, davon mindestens ein Drittel
Negativfälle (darf *nicht* erkannt werden). Alle Texte erfunden; Nummern so
erzeugen, dass Prüfziffern stimmen.

## Fertig, wenn

Alle Prüfbefehle grün und in den Kommentaren steht, woher die Formatregeln
stammen.

## Ergebnis

_(von Gemini auszufüllen)_
