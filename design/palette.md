# Halcyon – Farbpalette

Abgeleitet vom Eisvogel (Halcyon): Rücken aqua/petrol, Bauch orange.

| Token              | Wert      | Rolle |
|--------------------|-----------|-------|
| `--pitch-black`    | `#15110e` | Text (hell), Hintergrund (dunkel) |
| `--sky-aqua`       | `#1dc6ee` | Primärer Akzent: Auswahl, Fokus, aktive Elemente |
| `--tangerine-dream`| `#f79d65` | Sekundärer Akzent: Hinweise, ungelesen, Kategorie-Highlights |
| `--white`          | `#fdfdfe` | Hintergrund (hell), Text (dunkel) |
| `--stormy-teal`    | `#3c6b74` | Text-Akzent, Links, Buttons im hellen Modus |

## Kontrastregeln (WCAG)

| Vordergrund   | Hintergrund | Kontrast | Verwendung |
|---------------|-------------|----------|------------|
| pitch-black   | white       | 18.5 | Fließtext |
| stormy-teal   | white       | 5.8  | Links, Akzent-Text, Button-Fläche mit weißem Text |
| white         | stormy-teal | 5.8  | Text auf Primär-Buttons |
| sky-aqua      | pitch-black | 9.3  | Akzent-Text im dunklen Modus |
| tangerine     | pitch-black | 8.9  | Hinweis-Text im dunklen Modus |
| sky-aqua      | white       | 2.0  | **nur Flächen/Icons/Rahmen, kein Text** |
| tangerine     | white       | 2.1  | **nur Flächen/Icons/Rahmen, kein Text** |

Auf Aqua- oder Tangerine-Flächen steht Text immer in `--pitch-black`.
