# Halcyon – Interaktion und Bewegung

Ergänzt [palette.md](palette.md). Gilt für neue Oberflächen und beim Umbau
bestehender.

## Leitbild

Halcyon fühlt sich nicht an wie „hier ist eine Liste, hier sind deine
Buttons", sondern wie ein lockerer, greifbarer Raum: Dinge erscheinen dort,
wo es gerade um sie geht, bewegen sich nachvollziehbar und lassen sich
direkt anfassen.

## Grundsätze

1. **Am Ort statt woanders.** Was zu einer Mail, Karte oder Kachel gehört,
   erscheint direkt an ihr: aufklappen, auffächern, darüber schweben. Kein
   Wechsel in eine andere Ansicht, kein Dialog in der Bildschirmmitte, wenn
   es sich vermeiden lässt.
2. **Herkunft zeigen.** Neue Elemente gleiten aus dem Element heraus, das sie
   ausgelöst hat, und kehren beim Schließen dorthin zurück. Man sieht, woher
   etwas kommt und wohin es geht.
3. **Ebenen statt Seiten.** Eine Szene, die gerade wichtig ist, liegt
   sichtbar über allem anderen (höhere Ebene, weicher Schatten); der Rest
   tritt zurück (abgedunkelt, leicht unscharf), bleibt aber sichtbar. Ein
   Klick daneben oder Escape kehrt zurück.
4. **Anfassen.** Ziehen, wegschieben, auffächern, stapeln sind die
   Hauptwege. Jede Geste hat einen gleichwertigen Weg per Klick und Tastatur
   (Menü, Pfeiltasten, Enter, Escape).
5. **Dinge statt Zeilen.** Mails, Gruppen, Bestellungen treten als Karten,
   Kacheln und Stapel auf, mit Logo und Farbe ihres Absenders. Listen nur
   dort, wo man wirklich viel durchsieht (Posteingang, Suche).
6. **Bedienelemente erscheinen bei Bedarf.** Aktionen, die selten gebraucht
   werden, zeigen sich beim Überfahren, beim Fokus oder in einem kleinen
   Menü. Wichtige Aktionen bleiben immer sichtbar (auch auf Touch).
7. **Jede Bewegung hat einen Grund.** Sie erklärt eine Veränderung
   (erscheinen, verschwinden, umsortieren, zuordnen). Keine Dauerschleifen,
   kein Wackeln um Aufmerksamkeit.
8. **Rückgängig statt Nachfragen.** Lieber sofort handeln und kurz
   „Rückgängig" anbieten als vorher bestätigen lassen. Ausnahme: endgültiges
   Löschen.

## Bewegung

- **Dauer:** Rückmeldung auf Hover und Druck 120–160 ms; Erscheinen und
  Verschwinden 200–280 ms; Szenen mit mehreren Elementen insgesamt unter
  500 ms. Mehrere Elemente nacheinander mit 30–50 ms Versatz.
- **Verlauf:** Hinein schnell und weich auslaufend (ease-out), hinaus etwas
  schneller als hinein. Für Bewegungen, die etwas „werfen" oder „fallen
  lassen", eine leichte Feder ohne sichtbares Nachschwingen.
- **Weniger Bewegung:** Bei `prefers-reduced-motion` entfallen Wege und
  Skalierungen; Elemente erscheinen und verschwinden ohne Animation oder mit
  kurzem Überblenden, Ebenen und Abdunkeln bleiben.

## Technische Grenze: WebKitGTK

Die Linux-Version läuft in WebKitGTK. Auf „Heute" zeigte sich am Ende von
Hover-Übergängen auf `transform` und `opacity` für ein Bild der
Ausgangszustand (gemessen in einer Bildschirmaufnahme, 16.11.2026). Betroffen
waren die Bestell-Kacheln (in einer waagerecht scrollenden Leiste mit
`mask-image`) und die großen Karten (Verlauf, Schatten mit `color-mix`).

**Bewegungstest vom 24.11.2026** (Strg+Umschalt+B im Entwicklungsmodus,
`apps/desktop/src/lib/MotionLab.svelte`): Ein Kästchen gleitet auf schwarzem
Grund herein und hinaus, in acht Techniken. Ergebnis der Messung, Bild für
Bild bei 30 Bildern pro Sekunde: **keine einzige Technik flackert**, auch
nicht der schlichte CSS-Übergang auf `opacity` und `transform`.

**Zweite Runde** (gleiche Seite, „Runde 2"): derselbe schlichte Übergang in
den Umgebungen von damals – Maske, waagerecht scrollende Leiste, beides
zusammen, farbiger Schatten, Verlauf mit Schatten, Übergang nach einer
Einblend-Animation. Im Tauri-Fenster sah alles sauber aus (Sichtprüfung,
nicht Bild für Bild gemessen).

Folgerungen:

- WebKitGTK animiert `transform`, `opacity`, Masken, Schatten und scrollende
  Leisten sauber. **Die Technik ist nicht das Problem.**
- Wahrscheinliche Ursache des früheren Stotterns: Das Element, über dem die
  Maus stand, hat sich beim Überfahren selbst bewegt (angehoben,
  vergrößert). Steht der Zeiger nahe am Rand, rutscht das Element unter ihm
  weg, der Hover endet, das Element kehrt zurück, der Hover beginnt neu.
  Dazu kamen mehrere übereinander liegende Übergänge mit eigener Dauer.
- Regel daraus: **Auslöser bleiben stehen.** Die Fläche, die auf Maus oder
  Berührung reagiert, bewegt sich durch die Reaktion nicht. Was sich
  bewegen soll, ist ein Kind darin (Inhalt, Bild, Schatten) oder eine
  unsichtbare Trefferfläche hält den ursprünglichen Platz.
- Mehrere Elemente pro Auslöser sind ausdrücklich erwünscht, zum Beispiel
  Kandidaten, die nacheinander hereinfliegen. Sie sind selbst keine
  Auslöser, solange sie sich bewegen; anklickbar werden sie erst, wenn sie
  an ihrem Platz sind.
- Bei neuen Szenen im Zweifel kurz im Tauri-Fenster prüfen; der
  Bewegungstest (Strg+Umschalt+B) lässt sich dafür erweitern.
