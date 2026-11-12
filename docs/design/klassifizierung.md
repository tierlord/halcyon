# Kategorien und Vorgänge erkennen

Stand: 12.11.2026 · Status: Konzept

## Grundsätze

1. **Halcyon funktioniert vollständig ohne KI.** Der Download bleibt schlank.
   Regeln, gelernte Absender-Regeln, Vorgänge und Drag & Drop gibt es immer.
2. **KI ist ein optionales Paket**, das der Nutzer in der App nachlädt und
   jederzeit wieder entfernen kann.
3. **Standardmäßig bleibt alles lokal** – auf dem Gerät oder auf einem
   eigenen Rechner im Heimnetz. Eine Cloud-KI ist eine **ausdrückliche Wahl**
   des Nutzers mit eigenem API-Schlüssel, pro Konto, mit Schutzregeln (siehe
   „Cloud-KI mit eigenem Schlüssel").
4. **Lernen im Betrieb heißt Beispiele speichern**, nicht Modelle neu
   trainieren. Jede Zuordnung wirkt sofort.
5. **Jede automatische Entscheidung ist erklärbar** („Warum?").

## Kategorien: vier Stufen

| Stufe | Verfahren | Lernt mit? | Braucht KI-Paket? |
|---|---|---|---|
| 1 | Feste Signale: `List-Unsubscribe` → Newsletter, Einmalcode erkannt → „Einmalcodes" (mit Ablaufzeit), Sendungsnummer → Versand, PDF + Rechnungsmerkmale → Rechnung | nein | nein |
| 2 | Gelernte Absender-Regeln: wiederholt gleiche Zuordnung von Absender / `List-ID` → Regel | ja, sofort | nein |
| 3 | Ähnlichkeit über Embeddings, Nachbarschaftssuche über gespeicherte Beispiele | ja, sofort | ja (Embedding-Modell, ca. 100 MB) |
| 4 | LLM: Kaltstart neuer Kategorien, Rechnungsdetails (Betrag, Fälligkeit, Nummer), unsichere Fälle | nein | lokales LLM, Rechner im Heimnetz **oder** Cloud-KI |

Die Stufen laufen der Reihe nach; die erste sichere Entscheidung gewinnt.

### Entscheidung bei Stufe 3

- Eingabe: Absender, Betreff, erste ca. 2 000 Zeichen des Textteils.
- Für jede Kategorie: positive Beispiele (zugeordnet) und **negative
  Beispiele** (vom Nutzer herausgenommen oder Vorschlag abgelehnt).
- Ergebnis je nach Abstand der besten zur zweitbesten Kategorie:
  - **eindeutig** → automatisch zuordnen
  - **unsicher** → 2–3 Vorschläge zum Draufziehen
  - **unklar** → nichts tun
- Die Schwelle für „eindeutig" stellt sich pro Kategorie anhand der
  Korrekturen selbst ein (Ziel: ca. 95 % Trefferquote bei automatischen
  Zuordnungen).
- Beispiele pro Absender und Kategorie werden begrenzt, damit kein
  Newsletter eine Kategorie „auswendig lernt".
- Ab ca. 20 Beispielen kann eine kleine lineare Schicht auf den Vektoren
  dazukommen (Training in Millisekunden).
- Suche: einfacher Durchlauf über alle Vektoren (int8 oder f16); bei
  100 000 Mails wenige Millisekunden. Keine Vektordatenbank nötig.

### Kaltstart

Eingebaute Kategorien (Rechnungen, Bestellungen, Logindaten, Verträge,
Newsletter, Einmalcodes) bringen eine Beschreibung und synthetische
Startbeispiele mit. Diese zählen schwächer als echte Zuordnungen.

## Vorgänge: Lernen per Beispiel

Vorgänge entstehen über **Schlüssel**, nicht über ein Modell:

1. Thread-Header (`In-Reply-To`, `References`)
2. Gesprächsspezifische Antwortadressen (z. B. Kleinanzeigen)
3. Gemeinsame Kennungen aus `halcyon_core::extract`: Bestell-, Sendungs-,
   Rechnungsnummer, Anzeigentitel

Zieht der Nutzer eine Mail in einen Vorgang, ermittelt Halcyon, was die Mails
gemeinsam haben (Antwortadresse, Kennung, Betreff ohne Präfixe), und legt
daraus eine **Gruppierungsregel für diesen Absender** an. Trennt der Nutzer
einen Vorgang, wird die Regel abgeschwächt oder entfernt.

Embeddings (falls installiert) liefern nur Vorschläge („gehört vermutlich zu
…") aus Ähnlichkeit, gleicher Gegenseite und zeitlicher Nähe.

## Das KI-Paket

### Inhalt und Download

- Wird in den Einstellungen angeboten, mit Größe, Lizenz und Zweck.
- Enthält das Modell **und** die Laufzeitbibliothek: ONNX Runtime wird über
  `ort` mit `load-dynamic` erst zur Laufzeit geladen. Die App selbst enthält
  nur den kleinen Code, der das Paket anspricht.
- Manifest mit Version und **SHA-256 jeder Datei, fest in der App
  hinterlegt**. Nur HTTPS. Stimmt eine Prüfsumme nicht, wird nichts geladen.
- Nur Modelle mit freier Lizenz (z. B. MIT / Apache-2.0).

### Pakete

| Paket | Größe (ca.) | Läuft gut auf |
|---|---|---|
| Ähnlichkeit (Stufe 3): kleines mehrsprachiges Embedding-Modell | 100–150 MB | jedem Notebook, CPU |
| Assistent (Stufe 4): LLM 3–4 B Parameter, quantisiert | 2–3 GB | Notebook mit 16 GB, langsam, nur im Hintergrund |
| Assistent groß: LLM 7–8 B, quantisiert | 4–6 GB | Rechner mit GPU ab 8 GB VRAM |

### Rechner im Heimnetz

Für Stufe 4 kann statt eines lokalen Modells ein **eigener Rechner im
Heimnetz** genutzt werden (z. B. Desktop mit GPU, darauf ein
OpenAI-kompatibler Server wie llama.cpp `server` oder Ollama).

- Nur nach ausdrücklicher Zustimmung, mit Hinweis, dass Mailinhalte das
  Gerät verlassen (aber nicht das Heimnetz).
- Adresse und Zugangsschlüssel in den Einstellungen; Schlüssel im
  Schlüsselbund. Verbindung nur mit TLS oder zu privaten Adressen.
- Ist der Rechner nicht erreichbar, werden Aufträge gesammelt und später
  abgearbeitet; die App bleibt voll benutzbar.

### Cloud-KI mit eigenem Schlüssel

Für Nutzer ohne leistungsfähige Hardware kann Stufe 4 über die API eines
Anbieters laufen (Claude, Gemini, ChatGPT, …).

**Anbieter**

- Eine gemeinsame Schnittstelle `LlmProvider` im Kern mit drei Varianten:
  - Anthropic Messages API
  - Google Gemini API
  - OpenAI-kompatibel (ChatGPT und ebenso Ollama, llama.cpp, LM Studio –
    damit nutzt der Rechner im Heimnetz dieselbe Schnittstelle)
- Schlanke HTTP-Aufrufe aus dem Rust-Kern, keine großen SDKs.
- **Eigener API-Schlüssel des Nutzers** (im Schlüsselbund). Es gibt keinen
  Halcyon-Server dazwischen.
- Modell frei wählbar; für die Einordnung reicht ein kleines, schnelles
  Modell des Anbieters.

**Was die Cloud bekommt – und was nie**

- Nur **Stufe 4**. Stufe 1–3 laufen immer lokal; Stufe 3 (Embeddings) ist
  mit ca. 100–150 MB auch auf schwachen Geräten möglich. Ohne installiertes
  Stufe-3-Paket gehen nur Mails an die Cloud, die Stufe 1–2 nicht sicher
  zuordnen.
- Gesendet werden höchstens: Absenderdomain, Betreff, gekürzter Textanfang
  (ca. 2 000 Zeichen), bei Rechnungen der extrahierte PDF-Text.
- **Vor dem Senden maskiert**: Mailadressen, Namen aus dem Adressbuch,
  Telefonnummern, IBAN/Kontonummern, Postanschriften, URLs mit Tokens.
- **Nie gesendet**: Mails mit erkanntem Einmalcode oder Passwort, Mails in
  Kategorien mit dem Merkmal „vertraulich" (standardmäßig „Logindaten",
  „Verträge"), Anhänge außer dem Rechnungstext.
- Gelernte Beispiele bleiben lokal. Optional (eigener Schalter) gehen einige
  **maskierte** Beispiele als Kontext mit, um die Genauigkeit zu erhöhen.

**Zustimmung und Kontrolle**

- Standardmäßig **aus**; Einschalten **pro Konto** mit klarem Hinweis, dass
  ausgewählte Mailinhalte an den Anbieter gehen und dessen Bedingungen gelten
  (Training mit Eingaben ist bei manchen kostenlosen Tarifen erlaubt – der
  Hinweis empfiehlt einen bezahlten API-Zugang und verlinkt die
  Datenschutzbedingungen des gewählten Anbieters).
- Eigener Hinweis bei geschäftlichen Konten: Weitergabe an externe KI kann
  durch den Arbeitgeber untersagt sein.
- **Protokoll** jeder Anfrage (Zeit, Anbieter, Modell, gesendeter Inhalt nach
  Maskierung, Token, Kosten), einsehbar und löschbar.
- **Monatsbudget** mit Kostenanzeige; bei Erreichen stoppt die Cloud-Nutzung,
  die App arbeitet lokal weiter.

**Effizienz**

- **Strukturierte Antworten** nach festem JSON-Schema
  (`{category, confidence, amount, due_date, …}`); alle drei
  Anbieterfamilien unterstützen das. Nicht passende Antworten werden
  verworfen, nie geraten.
- Gleichbleibender Teil der Anfrage (Anweisungen, Kategoriebeschreibungen)
  steht vorne, damit Anbieter ihn zwischenspeichern können
  (Prompt-Caching).
- Mehrere Mails pro Anfrage; nicht eilige Arbeit (erster Durchlauf über alte
  Mails) als Sammelauftrag, wo der Anbieter das günstiger anbietet.
- Ergebnisse werden gespeichert; jede Mail wird höchstens einmal gesendet.

**Größenordnung der Kosten** (Stand 11/2026): Mit einem kleinen Modell
(z. B. Claude Haiku 5.5) kostet die Einordnung einer Mail grob ein
Hundertstel Cent, 1 000 Mails also etwa 10 Cent. Die App zeigt die echten
Kosten aus den Antworten der Anbieter an.

### Einrichtung: Auswahl für den Nutzer

1. **Ohne KI** – Regeln und Lernen aus eigenen Zuordnungen
2. **Lokal** – ca. 150 MB, Mails bleiben auf dem Gerät
3. **Eigener Rechner im Heimnetz**
4. **Cloud-KI mit eigenem Schlüssel** – ausgewählte Mails gehen an den
   Anbieter

Kombinationen sind möglich und üblich, z. B. „Lokal" für die Einordnung plus
Cloud nur für Rechnungsdetails.

### Updates und Entfernen

- Jeder gespeicherte Vektor kennt seine Modellversion. Nach einem
  Modell-Update werden Vektoren im Hintergrund neu berechnet; gelernte
  Beispiele bleiben erhalten, weil sie an der Mail hängen.
- „KI-Funktionen entfernen" löscht Modell, Laufzeit und alle Vektoren. Regeln
  und manuelle Zuordnungen bleiben.

## Datenmodell (Entwurf)

```text
categories        (id, name, description, ttl_seconds, builtin, auto_threshold)
message_category  (message_id, category_id, source[signal|rule|model|user], confidence, expires_at)
category_examples (message_id, category_id, label[+1|-1], created_at)
sender_rules      (id, match[from|domain|list_id], value, category_id, hits, misses)
cases             (id, title, created_at, updated_at)
case_members      (case_id, message_id, source[header|key|rule|user])
case_rules        (id, sender_match, key_kind, key_pattern, hits, misses)
embeddings        (message_id, model_version, vector BLOB)
```

Alle Tabellen liegen in der verschlüsselten Datenbank (siehe ROADMAP:
SQLCipher). Vektoren lassen Rückschlüsse auf Inhalte zu.

## Reihenfolge

1. Mailtext laden (erster Textteil, gekürzt) – Voraussetzung für alles
   Weitere
2. Datenmodell, Drag & Drop, Stufe 1 und 2, Vorgänge über Schlüssel – **ohne
   KI**
3. KI-Paket-Verwaltung (Download, Prüfsummen, Entfernen) und Stufe 3
4. Stufe 4: `LlmProvider` mit OpenAI-kompatibel (lokal und Heimnetz), dann
   Anthropic und Gemini; Maskierung, Protokoll und Budget **vor** dem ersten
   Cloud-Aufruf

## Offene Fragen

- Maskierung testen: synthetischer Satz mit allen sensiblen Mustern, Ziel
  „nichts rutscht durch" (eigene Testaufgabe für Gemini geeignet).

- Welches Embedding-Modell konkret? Kandidaten mit Deutsch-Englisch-Qualität
  und freier Lizenz vergleichen; dafür einen kleinen synthetischen
  Bewertungssatz anlegen.
- Hardware-Beschleunigung auf Intel-Grafik (OpenVINO / Vulkan) – erst
  messen, ob die CPU nicht reicht.
