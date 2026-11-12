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

_(von Gemini auszufüllen)_
