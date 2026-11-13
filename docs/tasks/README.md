# Aufgaben für Gemini

Claude arbeitet am Sync, am Speicher und an der Oberfläche. Diese Aufgaben
sind so geschnitten, dass sie **nur eigene, neue Dateien** berühren und
parallel dazu laufen können.

## Ablauf

1. Arbeitskopie anlegen (einmalig), damit sich niemand gegenseitig Dateien
   überschreibt:
   ```bash
   cd ~/Repositories/halcyon
   git worktree add ../halcyon-gemini -b gemini/<aufgabe> main
   cd ../halcyon-gemini
   ```
2. [AGENTS.md](../../AGENTS.md) lesen, dann die Aufgabendatei.
3. **Nur die Dateien anfassen, die in der Aufgabe unter „Dateien" stehen.**
   Wenn eine andere Datei geändert werden müsste: nicht ändern, sondern in
   der Übergabenotiz beschreiben.
4. Vor der Übergabe die Prüfbefehle aus AGENTS.md ausführen, alle grün.
5. Commit auf dem eigenen Branch (nicht pushen, nicht nach `main` mergen).
   Commit-Nachricht: `<bereich>: <was>` auf Englisch, z. B.
   `core: add modified UTF-7 codec`.
6. Unten in der Aufgabendatei den Abschnitt **Ergebnis** ausfüllen: was
   umgesetzt ist, was offen ist, Fragen. Claude prüft dann und übernimmt.

## Aufgaben

| Nr. | Aufgabe | Status |
|-----|---------|--------|
| 01 | [Modifiziertes UTF-7 für Ordnernamen](01-utf7-folder-names.md) | erledigt |
| 02 | [Testkorpus für Mail-Header](02-header-fixtures.md) | erledigt |
| 03 | [Einmalcodes und Sendungsnummern erkennen](03-extractors.md) | erledigt (mit Review-Fixes) |
| 04 | [Bösartige HTML-Mails als Testkorpus](04-hostile-html.md) | erledigt (5 Lücken gefunden und behoben) |
| 05 | [Antworten, Allen antworten, Weiterleiten](05-reply-logic.md) | offen |

Reihenfolge ist frei; jede Aufgabe ist für sich abgeschlossen.
