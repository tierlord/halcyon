# 04 – Bösartige HTML-Mails als Testkorpus

## Warum

HTML-Mails sind der größte Angriffsweg in einem Mailprogramm. Halcyon zeigt
sie in drei Schutzschichten an (siehe Kopfkommentar in
`crates/halcyon-core/src/content.rs`): HTML-Filter, Content-Security-Policy
im Dokument, abgeschotteter iframe ohne Skripte. Diese Aufgabe greift die
**ersten beiden Schichten** gezielt an.

## Dateien

- neu: `crates/halcyon-core/tests/fixtures/html/*.html`
- neu: `crates/halcyon-core/tests/html_safety.rs`

Nichts anderes ändern. **Nicht** `content.rs` anpassen, auch nicht bei
gefundenen Lücken (siehe unten).

## Schnittstelle

```rust
use halcyon_core::content::{sanitize_html, render, parse, MessageContent};

// Bereinigter <body>-Inhalt + Anzahl blockierter Fremdinhalte
let (body, blocked) = sanitize_html(html, &HashMap::new(), /* allow_remote */ false);

// Vollständiges Dokument für den iframe (enthält die CSP)
let rendered = render(&parse(raw_mail_bytes), false);
```

## Umsetzung

1. 40–60 kleine Dateien unter `fixtures/html/`, eine pro Angriffsidee,
   Dateiname beschreibt sie (`script-uppercase.html`,
   `href-javascript-tab.html`, …). Nur erfundene Domains (`*.example`).
2. Abdecken, jeweils mehrere Varianten:
   - `<script>` in allen Schreibweisen, verschachtelt, in `<svg>`/`<math>`,
     nach Kommentar-Tricks (`<!--><script>`), in `<noscript>`, `<template>`
   - Event-Handler auf beliebigen Elementen (`onerror`, `onload`,
     `onmouseover`, `onfocus` + `autofocus`, …), auch mit Großbuchstaben
     und ohne Anführungszeichen
   - Gefährliche URLs in `href`/`src`: `javascript:`, `vbscript:`,
     `data:text/html`, mit Entities (`&#106;avascript:`), Tabs/Zeilenumbrüchen
     (`java\tscript:`), führenden Leerzeichen
   - `<meta http-equiv="refresh">`, `<base href>`, `<link rel=stylesheet>`,
     `<iframe>`, `<frame>`, `<object>`, `<embed>`, `<applet>`, `<form>`,
     `<input>`, `<button formaction>`, `<a ping>`
   - CSS: `expression()`, `-moz-binding`, `behavior`, `@import` in allen
     Varianten, `url()` mit CSS-Escapes (`\75\72\6c`), Kommentaren und
     Leerzeichen, `</style>`-Ausbruch aus einem Style-Block
   - Mutations-XSS-Klassiker (`<svg><style><img src=x onerror=…>`,
     `<math><mtext><table><mglyph><style><img …>`, `<noscript><p title="</noscript><img …>">`)
   - Tracking: Pixel per `<img>`, `srcset`, `background`-Attribut,
     `style="background:url(…)"`, `<style>`-Block, `<picture><source>`,
     `poster` – jeweils prüfen, dass **nichts** davon ohne Erlaubnis lädt
     und `blocked` > 0 ist
   - Robustheit: sehr tiefe Verschachtelung (10 000 `<div>`), sehr lange
     Attribute, kaputtes HTML, NUL-Bytes – darf nicht abstürzen und muss
     unter 2 Sekunden bleiben
3. `tests/html_safety.rs`: eine Hilfsfunktion, die für jede Ausgabe prüft,
   dass sie **keine** ausführbaren Konstrukte enthält (keine `<script`,
   keine `on…=`-Attribute, kein `javascript:`/`vbscript:`/`data:text`, keine
   der verbotenen Tags, keine Fremd-URL in `src`/`srcset`/`poster`, kein
   `@import`, kein `expression(`). Prüfen **nach** dem Parsen der Ausgabe
   (z. B. mit `scraper` oder `html5ever` als `dev-dependency`) und nicht nur
   per Textsuche – Textsuche übersieht Entities und findet Harmloses im Text.
   Eine neue `dev-dependency` ist dafür erlaubt (per `cargo add --dev`).
4. Zusätzlich für jedes Fixture: `render(...)`-Dokument enthält
   `script-src 'none'` und ohne Erlaubnis `img-src data:;`.

## Wenn eine Lücke gefunden wird

Test mit dem **richtigen** Erwartungswert schreiben, mit
`#[ignore = "Lücke: …"]` markieren und im Ergebnis auflisten – mit kurzer
Einschätzung, ob die zweite Schicht (CSP) den Angriff trotzdem stoppt.
Den Filter repariert Claude.

## Fertig, wenn

Alle Prüfbefehle aus AGENTS.md grün (ignorierte Tests erlaubt) und die
Liste gefundener Lücken im Ergebnis steht.

## Ergebnis

- `scraper` (`0.29.0`) als `dev-dependency` zu `crates/halcyon-core/Cargo.toml` hinzugefügt.
- 48 bösartige HTML-Fixtures unter `crates/halcyon-core/tests/fixtures/html/*.html` angelegt (ausschließlich `*.example`-Domains): `<script>`-Varianten, Event-Handler, gefährliche URLs (`javascript:`, `vbscript:`, `data:text/html`, Entities, Tabs, Leerzeichen), verbotene Tags (`meta`, `base`, `link`, `iframe`, `frame`, `object`, `embed`, `applet`, `form`, `input`, `button formaction`, `a ping`), CSS-Angriffe, Mutations-XSS, Tracking-Pixel und Robustheitsfälle.
- `crates/halcyon-core/tests/html_safety.rs` prüft per `scraper::Html::parse_fragment` das bereinigte DOM, die CSS-Deobfuskierung sowie die CSP-Direktiven (`script-src 'none'`, `img-src data:;`) in `render(..., false)`. 42 von 48 Tests bestehen direkt.
- Gefundene Lücken (mit `#[ignore = "Lücke: …"]` markiert):
  1. `robust_deep_nesting_completes_fast`: 10 000 verschachtelte `<div>`-Tags benötigen ~17,1 s im Debug-Build (> 2 s Zielwert). **CSP hilft nicht**, da der DoS im Sanitizer passiert.
  2. `track_srcset_blocked`: `ammonia` entfernt `srcset` zuverlässig, aber `remote_blocked` bleibt `0` (die UI erkennt also nicht, dass ein Tracking-`srcset` blockiert wurde). **CSP** blockt ohnehin (`img-src data:;`).
  3. `css_url_escaped`: CSS-escapetes `\75\72\6c(...)` und `u/**/rl(...)` bleiben im Output und werden auch nicht in `remote_blocked` gezählt. **CSP** (`default-src 'none'; img-src data:;`) stoppt den Abruf bei `allow_remote = false`.
  4. `track_style_attr_url_blocked` & `track_style_block_url_blocked`: `sanitize_html` belässt `url(https://...)` absichtlich im CSS und zählt es nur (`blocked > 0`); bei `allow_remote = false` **stoppt die CSP** (`img-src data:;`) den Abruf.
  5. `css_import_variants`: `@import` wird durch `/* import removed */` ersetzt, sodass dahinter stehendes `url("https://...")` als ungültige CSS-Syntax im `<style>`-Block stehen bleibt. **CSP** (`default-src 'none'; style-src 'unsafe-inline'`) verhindert das Nachladen externer Stylesheets.
- Prüfungen: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` und `cargo test --workspace` grün (42 passed, 6 ignored in `html_safety`).

## Review (Claude)

Alle fünf Lücken behoben, alle 48 Tests laufen ohne `#[ignore]`:

1. Tiefe Verschachtelung / sehr große HTML-Teile: Vorprüfung ohne Parser
   (Tiefe > 256 oder > 4 MB) zeigt die Mail als bereinigten Text an.
   10 000 `<div>` jetzt in Millisekunden statt 17 s.
2. `srcset`/`poster` mit Fremdadressen werden gezählt.
3.–5. CSS in `style`-Attributen und `<style>`-Blöcken wird normalisiert
   (Kommentare und Escapes aufgelöst), `@import` entfernt, fremde `url()`
   ohne Erlaubnis durch `none` ersetzt und gezählt; mit Erlaubnis nur HTTPS.
