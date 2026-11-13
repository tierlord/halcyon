# Wer ist am Zug? – Konzept

Stand: 14.11.2026 · Status: Konzept

Baut auf [vorgaenge.md](vorgaenge.md) und
[klassifizierung.md](klassifizierung.md) auf und ordnet beide ein.

## Nordstern

> **Halcyon zeigt nicht Mails, sondern was gerade passiert – und was als
> Nächstes kommt.**

Prüffrage bleibt: *Muss der Nutzer dafür weniger Mails lesen und weniger
nachdenken?*

## Ausgangslage

Eine Auswertung eines echten Postfachs (lokal, nur Zahlen) ergab:

- Ohne Spam rund 28 000 Mails. **91 % davon sind Wiederholungen**:
  gleicher Absender, gleiches Betreffmuster. Es sind nur rund 2 400 Muster,
  und 109 Muster decken 87 % aller Mails ab.
- Bestellungen, Reisen, Verträge, Rechnungen und Logins zusammen: rund 13 %.
- Echte Gespräche (Mails von Leuten, denen man selbst geschrieben hat):
  **unter 0,2 %**.

Dieses Postfach ist kein Normalfall. Wer täglich mit Menschen schreibt, hat
ein anderes Problem: nicht Wiederholung, sondern den Überblick, wer auf wen
wartet. Das Konzept muss beide Fälle tragen, ohne dass Halcyon sich für eine
Zielgruppe entscheidet.

Folgerung: Vorgänge allein (Bestellungen) decken nur einen kleinen Teil ab.
Halcyon braucht ein Grundmodell, das für Maschinen-Mails und für Menschen
gleich funktioniert.

## Grundmodell: Wer ist am Zug?

Jede Geschichte im Postfach hat zu jedem Zeitpunkt jemanden, der am Zug ist:

| Am Zug | Bedeutung | Ansicht |
|---|---|---|
| **Du** | du musst etwas tun | „Braucht dich“ |
| **Jemand anderes** | du wartest | „Wartet“ |
| **Niemand** | es passiert nur etwas, oder es ist abgeschlossen | „Nebenbei“ bzw. „Erledigt“ |

Dazu kommt ein Querschnitt: **„Anders als sonst“**. Eine Geschichte, bei der
niemand am Zug ist, wird sichtbar, sobald sie sich ungewöhnlich verhält.

## Drei Arten von Geschichten

### 1. Quelle – wiederkehrende Maschinen-Mails

Ein Muster von einem Absender: Server-Warnungen, ein Newsletter,
Login-Hinweise, Kontoauszüge, Versand-Updates eines Shops.

- **Schlüssel:** Absenderadresse bzw. `List-ID` plus normalisiertes
  Betreffmuster (Zahlen, Daten und Beträge durch Platzhalter ersetzt, Präfixe
  wie „Re:“ entfernt).
- **Darstellung:** eine Zeile statt vieler Mails, z. B. „Warnung von X ·
  340× seit August · zuletzt heute 9:12“. Aufklappen zeigt die Mails.
- **Normalverhalten:** typischer Abstand zwischen Mails, typische Tageszeit,
  typische Betreffvarianten, falls Text vorhanden typische Beträge.
- **Abweichung** („Anders als sonst“):
  - neues Betreffmuster von einem bekannten Absender
  - Häufung (viel öfter als üblich) oder Stille (bleibt deutlich länger aus)
  - anderer Betrag als sonst, z. B. bei Abbuchungen oder Rechnungen
  - Absender wie bisher, aber DMARC schlägt plötzlich fehl
- **Eine Entscheidung pro Quelle:** anzeigen · zusammenfassen · nur Stand
  zeigen · stumm. Bei rund 100 Quellen sind das 100 Entscheidungen statt
  zehntausender gelesener Mails. Halcyon schlägt die Einstellung vor; der
  Nutzer bestätigt oder ändert sie mit einem Klick.
- Am Zug ist normalerweise **niemand**. Nur eine Abweichung, die eine
  Handlung verlangt, macht daraus „Braucht dich“.

Funktioniert allein aus Kopfzeilen, also auch für alte Mails ohne
gespeicherten Text.

### 2. Vorgang – Geschichte mit Zustand

Wie in [vorgaenge.md](vorgaenge.md): Bestellung, Vertrag, Rechnung, Termin.
Mehrere Quellen und Mails gehören zu einem Ablauf mit Zustandsautomat.

- Am Zug ergibt sich aus dem Zustand: Paket unterwegs → **jemand anderes**;
  Rechnung fällig → **du**; zugestellt und bezahlt → **niemand**.
- Ein Vorgang ist eine Quelle mit Zustand. Der bestehende Verknüpfer
  (`linker.rs`) bleibt und hängt sich in dieses Modell ein.

### 3. Gespräch – Austausch mit Menschen

Threads, wie sie die Nachrichtenliste schon bildet, plus eine Bewertung, wer
am Zug ist.

- **Du bist am Zug**, wenn die letzte Mail von der anderen Seite kommt, du in
  „An“ stehst (nicht nur in „CC“) und sie eine Frage oder Bitte enthält.
- **Jemand anderes ist am Zug**, wenn deine Mail die letzte ist und eine Frage
  enthält. Nach einigen Tagen ohne Antwort schlägt Halcyon vor nachzuhaken.
- **Niemand ist am Zug**, wenn das Gespräch abgeschlossen wirkt, z. B. bei
  einer Bestätigung oder einem Dank ohne Frage. Beispiel: eine bestätigte
  Tischreservierung.
- **Mitlesen:** Mails von Menschen, in denen du nur in „CC“ stehst oder die
  über einen Verteiler kommen. Menschlich, aber selten an dich gerichtet.
- **Person als Einstieg:** „Was ist gerade offen mit Anna?“ zeigt alle
  laufenden Gespräche und Vorgänge mit ihr, egal unter welchem Betreff.

Signale kommen zuerst aus den Kopfzeilen (wer zuletzt, An/CC, Antwort-Header,
Gesendet-Ordner). Ob eine Mail eine Frage enthält, prüft Version 1 mit
einfachen Regeln (Fragezeichen, „kannst du“, „bitte“, „could you“); später
optional das KI-Paket aus [klassifizierung.md](klassifizierung.md).

## Startseite „Heute“

Eine kurze Übersicht statt einer Liste, in dieser Reihenfolge:

1. **Braucht dich** – Gespräche, Vorgänge und Abweichungen, bei denen du am
   Zug bist
2. **Wartet** – worauf du wartest, mit Hinweis zum Nachhaken
3. **Anders als sonst** – Quellen mit Abweichung
4. **Nebenbei** – eine Zeile: „140 Mails aus 23 Quellen, nichts
   Ungewöhnliches“

Die Gewichtung ergibt sich aus den Daten. Bei einem Postfach voller
Maschinen-Mails ist Abschnitt 3 groß, bei einem Büro-Postfach sind es die
Abschnitte 1 und 2. Der Posteingang bleibt erreichbar, ist aber nicht mehr
der Mittelpunkt.

## Lernen

Jede Korrektur ist eine Regel, kein Modelltraining:

- Einstellung einer Quelle ändern → gilt für alle künftigen Mails der Quelle
- „Kein Vorgang“, „Erledigt“ → wie bisher
- „Ich bin nicht am Zug“ bei einem Gespräch → senkt die Bewertung für diese
  Art Mail dieses Absenders
- Jede automatische Entscheidung zeigt ihr „Warum?“, z. B. „du stehst in An,
  letzte Mail enthält eine Frage“

## Prüfen ohne fremde Postfächer

- **Eigenes Postfach** für Quellen und Vorgänge.
- **Enron-Korpus** (öffentlich, rund 500 000 Geschäftsmails mit vielen
  Gesprächen) für die Erkennung „Wer ist am Zug?“ bei Gesprächen. Liegt nur
  lokal, gehört nicht ins Repository.
- **Lokale Auswertung für Tester:** ein Befehl in Halcyon, der nur Zahlen
  ausgibt (Anteile je Art, Zahl der Quellen, Trefferquote der Bewertungen).
  Tester schicken nur diese Zahlen.

## Erfolgsmaß

- **Verdichtung:** Zeilen auf „Heute“ geteilt durch neue Mails am Tag.
  Ziel: unter 20 %.
- **„Braucht dich“ stimmt:** höchstens eine von zehn Einträgen falsch, kaum
  übersehene Fälle (geprüft am Enron-Korpus und eigenen Gesprächen).
- **Ruhe:** Anteil der Mails in stummen oder zusammengefassten Quellen, die
  der Nutzer nie öffnen musste.
- Korrekturen brauchen höchstens eine Aktion.

## Aufgaben

| # | Aufgabe | Abhängig von |
|---|---|---|
| Q1 | Quellen im Kern: Schlüssel, Normalisierung des Betreffs, Tabelle `sources`, Zuordnung jeder Mail, Nachverarbeitung aller Mails | – |
| Q2 | Normalverhalten und Abweichungen je Quelle | Q1 |
| Q3 | Einstellung pro Quelle (anzeigen, zusammenfassen, nur Stand, stumm) mit Vorschlag | Q1 |
| G1 | Gespräche bewerten: wer am Zug, Mitlesen, Nachhaken | Threads (vorhanden) |
| G2 | Prüfstand mit Enron-Korpus, nur lokal, nur Zahlen | G1 |
| V | Vorgänge in das Modell einhängen (Am-Zug aus dem Zustand) | Q1 |
| H1 | Startseite „Heute“ | Q2, G1, V |
| H2 | Personen-Ansicht | G1 |
| M | Lokale Auswertung für Tester | Q1, G1 |

**Meilensteine**

1. *Ruhe* – Q1, Q3: Quellen sichtbar und einstellbar. Sofort spürbar bei
   Postfächern voller Maschinen-Mails.
2. *Am Zug* – G1, G2, V: Gespräche und Vorgänge zeigen, wer am Zug ist.
3. *Heute* – Q2, H1: neue Startseite.
4. *Menschen* – H2, M: Personen-Ansicht und Auswertung mit Testern.

## Offene Fragen

- Wie fein ist ein Muster? Ein Shop schickt „Versendet: X“ und
  „Zugestellt: X“. Das sind zwei Muster, aber eine Quelle? Vorschlag: Quelle =
  Absender, Muster = Unterteilung darin.
- Wo endet „Anders als sonst“, bevor es selbst zur Störung wird? Schwellen
  pro Quelle, die sich an Korrekturen anpassen.
- Wie lange gilt ein Gespräch ohne Antwort als „Wartet“, bevor Halcyon es
  aufgibt?
