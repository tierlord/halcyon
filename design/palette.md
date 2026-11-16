# Halcyon – Farbpalette

Abgeleitet vom Logo (Eisvogel mit Brief, `images/halcyon-logo.svg`):
Rücken blaugrün, Kopf und Flügel schieferblau, Bauch ocker.

| Token           | Wert      | Rolle |
|-----------------|-----------|-------|
| `--pitch-black` | `#15110e` | Text (hell), Hintergrund (dunkel) |
| `--blue-green`  | `#1698b5` | Primärer Akzent: Auswahl, Fokus, Markierungen; Akzent-Text im dunklen Modus |
| `--ochre`       | `#df7618` | Sekundärer Akzent: ungelesen, Hinweise, Kategorie-Highlights |
| `--snow`        | `#f7f0f0` | Hintergrund (hell), Text (dunkel) |
| `--blue-slate`  | `#3b5966` | Akzent-Text, Links und Primär-Buttons im hellen Modus |

## Kontrastregeln (WCAG)

| Vordergrund | Hintergrund | Kontrast | Verwendung |
|-------------|-------------|----------|------------|
| pitch-black | snow        | 16.7 | Fließtext |
| blue-slate  | snow        | 6.7  | Links, Akzent-Text |
| snow        | blue-slate  | 6.7  | Text auf Primär-Buttons (hell) |
| blue-green  | pitch-black | 5.5  | Akzent-Text im dunklen Modus |
| pitch-black | blue-green  | 5.5  | Text auf Primär-Buttons (dunkel) |
| ochre       | pitch-black | 6.0  | Hinweis-Text im dunklen Modus |
| pitch-black | ochre       | 6.0  | Text auf Ocker-Flächen |
| blue-green  | snow        | 3.0  | **nur** Icons, Rahmen, Fokusring (≥ 3:1 für Bedienelemente), kein Fließtext |
| ochre       | snow        | 2.8  | **nur** Flächen und Punkte, immer mit zweitem Merkmal (z. B. fett) |

Mailinhalte werden auf reinem Weiß (`#ffffff`) angezeigt, da sie dafür gestaltet sind.

## Karten auf „Heute" und „Rechnungen"

Karten haben keinen Rahmen. Tiefe entsteht im hellen Design durch einen weichen
Schatten, im dunklen durch eine hellere Fläche mit feinem Glanzrand.

| Token | Rolle |
|-------|-------|
| `--card-surface` | Fläche einer Karte |
| `--card-radius`, `--card-radius-sm` | Eckenradius (22 px, 16 px für Listen und kleine Karten) |
| `--card-shadow`, `--card-highlight` | Schatten (hell) bzw. Glanzrand (dunkel) |
| `--brand-mix`, `--brand-mix-end` | Anteil der Markenfarbe am Anfang und Ende des Verlaufs (hell 14 % → 0 %, dunkel 42 % → 14 %) |
| `--text-soft` | Zweite Zeilen und Detailtext |
| `--tile-shadow` | Schatten bzw. Ring der Logo-Kachel |
| `--pill-mix` | Tönung weicher Schaltflächen |

- Jede Karte trägt die Farbe des Absenders als sanften Verlauf (`--brand`, pro
  Karte gesetzt). Die Farbe kommt aus dem Logo, sonst aus einem festen, aus dem
  Namen abgeleiteten Farbton (`apps/desktop/src/lib/brandColor.ts`).
- Text bleibt immer in der normalen Textfarbe. Erreicht er auf der getönten
  Fläche nicht 4,5:1, wird die Tönung abgeschwächt oder weggelassen.
- Ocker ist keine Kartenfarbe mehr; es bleibt Punkten und Hinweisen vorbehalten.
- Im dunklen Design ist die Karte deutlich farbiger: Verlauf von 42 % auf 14 %
  Markenfarbe, eine gesättigte 4-px-Kante in der Markenfarbe (`--edge-w`), ein
  weicher Schein unter der Karte (`--glow-mix`) und um die Logo-Kachel
  (`--halo-*`). Akzente (Fortschritt, Chips, Punkte, gefüllte Schaltfläche)
  nehmen eine hellere, gesättigtere Variante (`--brand-d`, Sättigung 0,6–0,88,
  Helligkeit ≥ 0,6, Label pitch-black ≥ 4,5:1). Die Tönungsquelle `--brand-dt`
  wird je Farbton so hell gewählt, wie Snow-Text es auf dem stärksten Punkt des
  Verlaufs noch mit ≥ 4,65:1 (Fließtext und `--text-soft`) erlaubt. Die Klasse
  `.brand-scope` (app.css) wählt je Theme zwischen den hellen (`--brand`,
  `--brand-k`) und dunklen Werten (`--brand-d`, `--brand-dt`, `--brand-kd`).
- Neutrale Karten („Läuft von allein“, „Wie immer“, leerer Zustand) sind im
  dunklen Design etwas reicher (`--card-surface` = Blue Slate 22 % über Snow 6 %
  über Pitch Black), bleiben aber ruhig.
- Die Werte in `brandColor.ts` spiegeln `--card-surface`, `--brand-mix` (hell
  14 %, dunkel 42 %) und müssen bei Änderungen mitgezogen werden.

## Bestellungen auf „Heute“

Bestellungen stehen als waagerechte Leiste unter der Begrüßung: Logo-Kachel mit
Fortschrittsring (vier Segmente in der Akzentfarbe der Marke), darunter Titel und
Status. Mehrere Bestellungen derselben Firma liegen als Stapel übereinander. Ein
ockerfarbener Punkt (nur Fläche, mit Beschriftung im `aria-label`) markiert
Bestellungen, die einen Blick brauchen.
