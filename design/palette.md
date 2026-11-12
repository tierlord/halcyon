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
