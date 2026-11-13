# Vorgänge – Plan

Stand: 13.11.2026 · Status: erste Version im Test

## Nordstern

> **Halcyon zeigt nicht Mails, sondern was gerade passiert – und was als
> Nächstes kommt.**
>
> Jede Mail landet in der Geschichte, zu der sie gehört. Auf einen Blick:
> worauf ich warte, was mich braucht, was erledigt ist und was nur nebenbei
> passiert. Halcyon erledigt das Sortieren selbst und lernt aus jeder
> Korrektur.

Prüffrage für jede Entscheidung: *Muss der Nutzer dafür weniger Mails lesen
und weniger nachdenken?*

## Begriffe

**Vorgang** (Karte): eine zusammenhängende Geschichte, z. B. „Bestellung
Rennrad-Schlauch bei Shop X".

- **Art**: Bestellung, Vertrag/Kündigung, Rechnung, Gespräch (z. B.
  Kleinanzeigen), Termin/Reise, Konto & Sicherheit, Support-Anfrage, Sonstiges
- **Gegenüber**: Firma oder Person (Domain bzw. Adresse)
- **Kennungen**: Bestell-, Sendungs-, Rechnungs-, Vertrags-, Kunden-,
  Ticketnummer, Gesprächs-Reply-To, Anzeigentitel
- **Zeitleiste**: Ereignisse, jede Mail ist eines (bestellt → versandt →
  zugestellt → Rechnung → bezahlt)
- **Nächster Schritt** mit erwartetem Datum („Zustellung bis Fr")
- **Status**, daraus die Ansicht:

| Bereich | Bedeutung | Beispiele |
|---|---|---|
| **Wartet** | offen, jemand anderes ist dran | Paket unterwegs, Kündigung eingereicht, noch nicht bestätigt |
| **Braucht dich** (Hervorhebung in „Wartet") | du musst etwas tun | Rechnung fällig, Rückfrage, Termin bestätigen |
| **Nebenbei** | passiert, aber niemand wartet | Newsletter-Gutschrift, Login-Hinweis, Versandinfo ohne Bestellung |
| **Erledigt** | abgeschlossen, archiviert | zugestellt und bezahlt, Kündigung bestätigt |

## Verarbeitung pro Mail

```
Mail ─► 1. Merkmale     (Kennungen, Beträge, Daten, Links, Absender, DMARC)
     ─► 2. Ereignis     (Art + Typ, z. B. order.shipped, contract.cancel_confirmed)
     ─► 3. Verknüpfen   (zu bestehendem Vorgang / neuer Vorgang / Vorschlag)
     ─► 4. Zustand      (Automat der Vorgangsart → Status, nächster Schritt, Frist)
     ─► 5. Lernen       (jede Korrektur wird Regel oder Beispiel)
```

1. **Merkmale** – reine Funktionen, offline, getestet (baut auf
   `extract/` auf).
2. **Ereignis** – Version 1 regelbasiert (Deutsch und Englisch: Betreff,
   Schlüsselwörter, Struktur), mit Konfidenz. Später zusätzlich Embeddings
   aus dem KI-Paket.
3. **Verknüpfen** – Punkte für gemeinsame Kennungen (stark), Thread-Header,
   Gesprächs-Reply-To, gleiches Gegenüber + zeitliche Nähe + gleiche
   Artikelnamen (schwach). Sicher → automatisch; unsicher → Vorschlag auf der
   Karte; nichts → neuer Vorgang oder „Nebenbei".
4. **Zustand** – je Vorgangsart ein kleiner Zustandsautomat als Tabelle
   (Daten, nicht Code). Zeitregeln: „zugestellt + 14 Tage ohne Rechnung →
   erledigt", „Nebenbei" nach 7 Tagen ins Archiv, Fristen aus Rechnungen.
5. **Lernen** – aus Zusammenführen, Aufteilen, Verschieben, Umbenennen,
   „Erledigt": Absender-Regeln (welche Kennung verbindet, welcher Betreff
   welches Ereignis ist) und Beispiele. Jede automatische Entscheidung zeigt
   ihr „Warum?".

## Grundsätze

- **Allgemein statt anbieterspezifisch.** Wissen über bekannte Absender
  (Betreff-Muster, Kennungsformate) liegt als **Daten** vor
  (`knowledge/*.toml`), nicht im Code, und ist erweiterbar.
- **Lokal.** Keine Cloud; Sendungsstatus nur aus Mails, kein Abfragen von
  Versanddiensten ohne Zustimmung.
- **Korrigierbar mit einem Klick** (Ziehen, Zusammenführen, Aufteilen,
  „Erledigt"), und jede Korrektur macht das System dauerhaft besser.
- **Vorsicht vor falscher Sicherheit:** lieber ein Vorschlag als eine falsche
  Verknüpfung.

## Erfolgsmaß

- ≥ 80 % der Bestell-, Versand- und Rechnungsmails automatisch im richtigen
  Vorgang, < 2 % falsch verknüpft
- „Wartet" stimmt: kein erledigter Vorgang darin älter als einen Tag
- Korrekturen brauchen höchstens eine Aktion
- Gemessen lokal am eigenen Postfach; ausgegeben werden nur Zahlen, keine
  Inhalte

## Aufgaben

Abgegrenzt nach eigenen Dateien, damit Agenten parallel arbeiten können.
Schnittstellen legt Claude in Aufgabe A fest, bevor B–E starten.

| # | Aufgabe | Wer | Abhängig von |
|---|---|---|---|
| A | Datenmodell und Speicher: `cases`, `case_events`, `case_keys`, `case_rules`, Migration, Store-API, Typen für Ereignisse und Zustände | Claude | – |
| B | Merkmale erweitern: Bestell-, Rechnungs-, Vertrags-, Kunden-, Ticketnummern; Betrag und Fälligkeit; Kündigungs- und Terminangaben; je mit Konfidenz; synthetischer Korpus | Gemini | A (Typen) |
| C | Ereignis-Erkennung v1 (regelbasiert): Mail → Art + Ereignistyp + Konfidenz; Korpus aus ≥ 150 synthetischen Mails | Gemini | A, B |
| D | Zustandsautomaten je Vorgangsart als Daten + Ableitung von Status, nächstem Schritt, Fristen, Zeitregeln | Gemini | A |
| E | Kartenansicht: Bereiche Wartet / Nebenbei / Erledigt, Karte mit Zeitleiste, nächstem Schritt, „Warum?", Aktionen; erst mit Testdaten | Gemini | A (API) |
| F | Verknüpfer: Bewertung, automatisch vs. Vorschlag, neuer Vorgang; Einbindung in den Sync | Claude | A–D |
| G | Lernen aus Korrekturen: Absender-Regeln, Beispiele, Erklärung | Claude | F, E |
| H | Nachverarbeitung bestehender Mails + lokale Auswertung (nur Zahlen) | Claude | F |
| I | Später: Embeddings aus dem KI-Paket für unscharfe Verknüpfung | offen | G |

**Meilensteine**

1. *Fundament* – A, danach B, D und E parallel
2. *Verstehen* – C, F: erste Vorgänge entstehen automatisch
3. *Sichtbar* – E mit echten Daten: Kartenansicht im Alltag nutzbar
4. *Lernend* – G, H: Korrekturen wirken, Auswertung am eigenen Postfach

**Stand 13.11.2026**

- A erledigt (`cases.rs`, Migration 10, Store-API). Beträge als Cent plus
  Währung, Ereignisse mit Typ (`order.shipped`).
- E als Prototyp von Gemini, an echte Daten angebunden. Testdaten nur noch im
  Browser-Modus.
- F in Version 1 (`linker.rs`):
  - Bestellungen über Bestell- oder Sendungsnummer bzw. Artikelname desselben
    Absenders.
  - Gespräche über Antwort-Header, nur wenn du selbst und jemand anderes
    geschrieben haben.
  - Versanddienste starten keinen Vorgang.
  - Betrachtet werden Mails der letzten 120 Tage. Offene Vorgänge ohne
    Neuigkeiten gelten nach 21 Tagen als erledigt.
- Korrekturen auf der Karte: „Erledigt“ und „Kein Vorgang“. Ein abgelehnter
  Vorgang entsteht nicht neu.
- Offen: B, C, D (Rechnungen, Verträge, Fristen), Nebenbei-Kategorien,
  Aufteilen und Zusammenführen, G, H.
