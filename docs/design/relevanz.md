# Relevanz lernen – Konzept

Stand: 25.11.2026 · Status: Konzept

Baut auf [Wer ist am Zug?](am-zug.md) und
[Kategorien und Vorgänge erkennen](klassifizierung.md) auf. Dieses Konzept
präzisiert dort Stufe 3 (Ähnlichkeit) und ergänzt, wie Halcyon von dir lernt.

## Nordstern

> **Halcyon lernt, was dich interessiert – und fragt nur, solange es sich
> nicht sicher ist.**

Prüffrage bleibt: *Muss der Nutzer dafür weniger Mails lesen und weniger
nachdenken?* Eine Frage von Halcyon lohnt sich nur, wenn sie später viele
Entscheidungen erspart.

## Ausgangslage

- Regeln entscheiden heute, was unter „Für dich" steht: Kopfzeilen,
  wiederkehrende Quellen, Gesprächspartner, Vorgänge, Rechnungen.
- Seit Migration 19/20 zählt Halcyon pro Absender, was du mit seinen Mails
  machst (geöffnet und behalten, beantwortet, gelöscht, 14 Tage liegen
  gelassen), mit einer Halbwertszeit von 120 Tagen.
- Grenzen der Regeln: Sie kennen nur Absender, keine Inhalte. Dass dich
  Termine immer interessieren und Gewinnspiele nie, egal von wem, können sie
  nicht lernen.

## Aufbau: drei Bausteine

| Baustein | Größe (geschätzt) | Aufgabe | Lernt von dir? |
|---|---|---|---|
| Text-Embedding-Modell, mehrsprachig | 0,3–0,6 GB | wandelt jede Mail einmal in einen Vektor, der ihren Inhalt beschreibt | nein, fest |
| Persönliches Netz | wenige KB | schätzt aus Vektor, Kopfzeilen, Absender-Statistik und deinen Zuordnungen, wohin eine Mail gehört | ja, nach jeder Rückmeldung, in Millisekunden |
| Kleines Sprachmodell (optional) | 1–3 GB | formuliert die Sätze auf den Karten, erkennt Fragen, Bitten und Fristen | nein |

- Speicherbudget für alles zusammen: **1–4 GB**, als optionales Paket wie in
  [klassifizierung.md](klassifizierung.md) beschrieben (Download in der App,
  Prüfsummen, jederzeit entfernbar). Halcyon bleibt ohne Paket voll nutzbar.
- Alles läuft **lokal auf der CPU**. Das Embedding-Modell rechnet neue Mails
  im Hintergrund; der Altbestand wird einmalig nachgerechnet.
- Das Sprachmodell sieht nur die wenigen Mails unter „Für dich". **Beträge,
  Daten und Nummern kommen immer aus den Extraktoren**, nie aus dem
  Sprachmodell.
- Kein Feintuning großer Modelle auf dem Gerät: zu langsam, braucht eine
  Grafikkarte und ist schwer zu erklären. Gelernt wird nur im kleinen Netz.

## Wohin eine Mail gehören kann

Vorbild ist die Benachrichtigungs-Sortierung von Android (seit Android 16
auf Pixel-Geräten): Unwichtiges landet nach **Inhalt** in benannten,
leisen, zusammengeklappten Bündeln – nicht stört, aber griffbereit ist.

Halcyon stellt deshalb nur eine Frage: **Wohin gehört das?** Die Antwort ist
entweder **Für dich** oder **ein Bündel**. Bündel heißen nach dem, was drin
ist, nicht nach einer Entscheidung; das versteht man ohne Erklärung.

| Ziel | Was hinein gehört | Wie es auf „Heute" erscheint |
|---|---|---|
| **Für dich** | will etwas von dir oder interessiert dich jetzt | Karte unter „Für dich", mit Benachrichtigung |
| **Übersichten** | regelmäßige, persönliche Auszüge und Berichte: Kontoauszug, Depot- oder Verbrauchsübersicht, Zusammenfassung deiner Anzeigen | sichtbares, ruhiges Fach: je Absender nur der neueste Stand als kleine Kachel („Kontoauszug November liegt bereit"), ältere dahinter |
| **Konto & Hinweise** | Ereignisse rund um deine Konten: Anmeldung, Passwort geändert, neue Bedingungen, Bestätigungen | zusammengeklappt |
| **Newsletter & Aktuelles** | Newsletter, Blogs, Neuigkeiten von Diensten | zusammengeklappt |
| **Werbung** | Angebote, Rabatte, Gewinnspiele | zusammengeklappt |
| **Soziales** *(später, abschaltbar)* | Likes, Kommentare, Kontaktanfragen | zusammengeklappt |
| **Weitere Mails** | was noch keinem Bündel zugeordnet ist | zusammengeklappt, ganz unten |

- **Bündel sind immer leise**: keine Benachrichtigung, kein Zähler, kein
  Punkt. Aufgeklappt zeigen sie ihre Mails als Karten oder Zeilen.
- **Übersichten liegen bereit, statt zu stören.** Sie sind das einzige
  Bündel, das nicht zusammengeklappt ist, sondern als schmales Fach mit
  Kacheln sichtbar bleibt – wie die Bestell-Leiste. Halcyon kennt das schon
  als Quell-Einstellung „nur Stand zeigen".
- **Einstellungen gehören zum Bündel, nicht zur einzelnen Mail.** Zum
  Beispiel: *Werbung* wird gleich als gelesen markiert, *Übersichten*
  nicht. Was früher „Nebenbei" und „Brauche ich nicht" unterschied, steckt
  jetzt in diesen Einstellungen.
- **Anders als sonst** (siehe [am-zug.md](am-zug.md)) holt eine Mail aus
  ihrem Bündel nach „Für dich": ein Kontoauszug mit ungewöhnlichem Betrag,
  ein Anmelde-Hinweis von einem neuen Gerät.
- **Bündel sind erweiterbar**: Wer mag, legt ein eigenes an (z. B. „Verein").
  Die eingebauten Bündel bringen eine Beschreibung und Startbeispiele mit,
  damit die Einordnung ab dem ersten Tag funktioniert.
- **Vorsichtige Voreinstellung**: Anfangs sortiert Halcyon nur, wo es sicher
  ist (Abmelde-Link, bekannte Quellen, Werbewörter, Konto-Hinweise); der
  Rest bleibt in „Weitere Mails", bis du ihn zuordnest oder das Netz sicher
  ist.

Ob eine Mail eine **Antwort** von dir braucht, entscheidet weiter die
Bewertung „wer ist am Zug". Das Netz entscheidet, **wohin** sie gehört.

Auf „Heute" ersetzen die Bündel die bisherige Zeile „Wie immer".

## Rückmeldungen

### Ohne Zutun (implizit)

| Signal | Wertung |
|---|---|
| beantwortet (auch von anderen Geräten) | stark positiv |
| geöffnet und behalten | positiv |
| aus „Für dich" geöffnet | positiv |
| ungelesen gelöscht | negativ |
| 14 Tage ungelesen liegen gelassen | schwach negativ |
| Quelle auf „stumm" gestellt | negativ für die ganze Quelle |
| Karte als erledigt entfernt | **neutral**: „Thema durch", nicht „unwichtig" |

Implizite Signale sind verrauscht. Ob du eine Mail am Handy gelesen hast,
weiß Halcyon nur über das Gelesen-Flag des Servers.

### Fragen statt raten (explizit)

Am Anfang fehlen Rückmeldungen. Deshalb fragt Halcyon aktiv – aber so, dass
es sich nicht nach Arbeit anfühlt:

- **Eine einfache Frage**: *Wohin gehört das?* Antworten sind „Für dich"
  und die Bündel, immer dieselben, mit ihren Namen.
- **Ziehen und Ablegen**: Mails und Karten lassen sich auf „Für dich" oder
  ein Bündel ziehen; beim Ziehen werden die Bündel als Ablagestapel
  sichtbar. Klicken geht immer auch (Menü, Tastatur, Barrierefreiheit).
- **Direkt an Ort und Stelle**: Eine Mail aus einem Bündel nach oben ziehen
  heißt „für dich", eine Karte aus „Für dich" in ein Bündel ziehen ordnet sie
  dort ein. Jede Karte und Zeile hat dafür auch ein kleines Menü.
- **Eine Antwort, viele Mails – mit Vorschau**: Nach der Zuordnung zeigt
  Halcyon, **welche Mails dieselbe Zuordnung bekämen**, damit „alle dieser
  Art" greifbar wird (siehe unten).
- **Eine kleine Fragerunde auf „Heute"**: höchstens drei Fragen am Tag, in
  einer ruhigen Karte („Hilf Halcyon, dich kennenzulernen"), jederzeit
  überspringbar und abschaltbar. Nie Pflicht, nie ein roter Hinweis.

### Vorschau: was „alle dieser Art" heißt

Ein Absender schickt selten nur eine Art Mail. Beispiel: Von einem
Marktplatz kommen fast nur Angebote, die man sofort wegwischt – dazwischen
aber Versandbestätigungen mit Sendungsnummer, die wichtig sind. Eine Regel
„alles von diesem Absender" wäre falsch.

Deshalb gilt eine Zuordnung nie pauschal für einen Absender, sondern für
eine **Art von Mail**, und Halcyon zeigt vorher, was sie trifft.

**Was „gleiche Art" heißt**

- **Mit KI-Paket: nach Inhalt.** Das Embedding-Modell beschreibt, worum es
  in einer Mail geht. Halcyon gruppiert die Mails des Absenders nach
  Ähnlichkeit des Inhalts, nicht nach Betreff: Angebote landen bei
  Angeboten, Versandbestätigungen bei Versandbestätigungen, auch wenn die
  Betreffe fast gleich klingen. Das persönliche Netz sagt zusätzlich je
  Gruppe, ob sie dir vermutlich wichtig ist.
- **Ohne KI-Paket: nach Betreffmuster** wie bei den Quellen (Zahlen, Daten,
  Beträge durch Platzhalter ersetzt), ergänzt um Merkmale, die die
  Extraktoren schon kennen: Sendungs- oder Rechnungsnummer, Betrag,
  Anhang, Einmalcode.
- Je Gruppe zeigt Halcyon einen Namen in Worten („Angebote", „Versand"),
  die Anzahl, zwei, drei Beispiele und den Zeitraum.

**Was vorausgewählt ist**

- **Nur die Gruppe der zugeordneten Mail.** Die anderen Gruppen desselben
  Absenders erscheinen daneben, nicht ausgewählt, mit der Frage „Gilt das
  auch hierfür?".
- **Ausnahmen, die Halcyon selbst erkennt**, sind markiert und nie
  vorausgewählt: Mails, die zu einem Vorgang gehören (Bestellung, Sendung,
  Rechnung), eine Sendungs- oder Rechnungsnummer enthalten, einen
  Einmalcode tragen oder die du beantwortet hast. Ein Satz sagt warum
  („Versandbestätigungen bleiben sichtbar, weil sie zu Bestellungen
  gehören"). Mit KI-Paket kommen Mails dazu, die das Netz innerhalb der
  Gruppe für wichtig hält.
- Einzelne Mails lassen sich aus einer Gruppe herausnehmen. Halcyon merkt
  sich das als Ausnahme und als Gegenbeispiel fürs Lernen.
- Neue Mails, die zu keiner Gruppe passen, fragt Halcyon wieder nach,
  statt sie still mitzunehmen.
- Mit dem Netz gibt es zusätzlich eine Gruppe „Ähnlich, von anderen
  Absendern", nicht ausgewählt.

**Wie es aussieht**

Keine Liste mit Häkchen, sondern eine Szene direkt an der Mail (Grundsätze
in [design/interaktion.md](../../design/interaktion.md)):

1. Du ziehst eine Mail in ein Bündel (z. B. *Werbung*) oder wählst es im
   Menü.
2. Der Rest der Ansicht tritt leicht zurück (abgedunkelt, unscharf). Die
   Mail bleibt hervorgehoben stehen.
3. Aus ihr heraus gleiten die **Gruppen als kleine Stapel** in einer Reihe
   darunter auf, leicht versetzt nacheinander, über allem anderen liegend.
   Jeder Stapel zeigt oben eine Beispielmail, darunter angedeutet die
   weiteren, dazu Name und Anzahl.
4. Die Gruppe der Mail ist **farbig hervorgehoben** (ausgewählt), die
   anderen sind blass. Ein Klick oder Tipp wechselt das.
5. Ein Stapel lässt sich **auffächern**: Die Mails breiten sich zu
   Kärtchen aus. Eine Ausnahme schiebt man mit einer kurzen Bewegung aus
   dem Fächer heraus (oder per ×); sie fällt sichtbar beiseite.
6. Von Halcyon erkannte Ausnahmen liegen schon beiseite, mit ihrem Satz,
   warum.
7. **Übernehmen**: Die ausgewählten Stapel ziehen sich zusammen und gleiten
   in ihr Bündel, die Ansicht
   kehrt zurück. Ein Hinweis bietet „Rückgängig" an.
8. Escape oder ein Klick daneben bricht ab; alles gleitet an seinen Platz
   zurück.

**Welche Mails Halcyon fragt**: die, bei denen eine Antwort am meisten
bringt – Absender und Muster, die häufig vorkommen und bei denen das Netz am
unsichersten ist. Nach rund 100 Mustern ist der Großteil eines Postfachs
abgedeckt (siehe [am-zug.md](am-zug.md): 109 Muster decken 87 % ab).

**Wann Halcyon aufhört zu fragen**: pro Absender und pro Muster, sobald
Halcyon sich sicher ist, zum Beispiel wenn

- mehrere deiner Zuordnungen übereinstimmen und keine Korrektur folgte, und
- das Netz deine letzten Zuordnungen dieser Art im Rückblick richtig
  vorhergesagt hätte (Ziel: ca. 95 %).

Ab dann handelt Halcyon direkt und zeigt nur noch sein „Warum?". Jede
Korrektur senkt die Sicherheit für diesen Absender oder dieses Muster
wieder; nach zwei Korrekturen fragt Halcyon dort wieder nach.

## Wie Halcyon handelt

1. **Leitplanken zuerst**: Einmalcodes, fehlgeschlagene Absenderprüfung,
   offene Rechnungen, Gespräche, bei denen du am Zug bist, und Vorgänge, die
   dich brauchen, entscheiden weiter die Regeln. Das Netz kann sie nicht
   verstecken.
2. **Deine Zuordnung** für Absender oder Muster gilt vor dem Netz.
3. **Das Netz** entscheidet den Rest, aber nur, wenn es sicher ist. Sonst
   bleibt die Mail, wo die Regeln sie hinlegen.
4. **Sanfter Übergang**: Am Anfang entscheiden die Regeln. Das Netz übernimmt
   schrittweise, sobald es im Rückblick besser trifft als die Regeln.

## Erklärbarkeit

Jede Entscheidung zeigt ihr „Warum?", aus einer dieser Quellen:

- deine Zuordnung („Du hast Mails dieser Art in *Werbung* gelegt")
- ähnliche Mails („ähnlich wie 6 Mails, die du ungelesen gelöscht hast")
- die stärksten Merkmale („du öffnest Mails mit Terminen fast immer")

## Risiken

- **Blind lernen**: Was das Netz versteckt, kann nie positiv bewertet
  werden. Gegenmittel: unsichere Fälle bleiben in „Weitere Mails" sichtbar,
  und gelegentlich zeigt Halcyon eine versteckte Mail als Frage.
- **Zu viele Fragen**: Tageslimit, Stopp bei Sicherheit, abschaltbar.
- **Veränderte Interessen**: Rückmeldungen klingen ab (Halbwertszeit wie bei
  den Zählungen); neue Rückmeldungen wiegen mehr als alte.
- **Datenschutz**: Vektoren erlauben Rückschlüsse auf Inhalte. Sie liegen in
  der verschlüsselten Datenbank (siehe ROADMAP: SQLCipher) und verlassen nie
  das Gerät.

## Prüfen ohne fremde Postfächer

- **Rückblick-Test im CLI**: Ein Befehl trainiert das Netz auf älteren
  Mails des eigenen Postfachs, testet es auf neueren und gibt nur Zahlen aus.
  Verglichen wird mit den Regeln allein.
- **Startdaten aus dem Bestand**: Bis genug echte Rückmeldungen da sind,
  dienen vorhandene Spuren als grobe Startbeispiele: gelesen oder nicht,
  beantwortet, in Ordner verschoben, gelöscht.
- **Erfolgsmaß**:
  - „Für dich" stimmt: höchstens eine von zehn Karten falsch
  - Fragen: unter drei am Tag nach der ersten Woche, gegen null nach einem
    Monat
  - Korrekturen: weniger als eine pro Tag

## Aufgaben

| # | Aufgabe | Abhängig von |
|---|---|---|
| R1 | Zuordnung „Für dich" oder Bündel pro Mail und Art von Mail speichern, Bündel mit Einstellungen; Leitplanken und Vorrang wie oben | – |
| R2 | Oberfläche: Bündel auf „Heute" (statt „Wie immer"), Fach für Übersichten, Menü an Karten und Zeilen, Ziehen und Ablegen, Vorschau der betroffenen Mails mit Ausnahmen | R1 |
| R3 | Fragerunde auf „Heute" mit Auswahl nach Nutzen, Tageslimit und Stopp bei Sicherheit | R1, R2 |
| R4 | KI-Paket-Verwaltung (Download, Prüfsummen, Entfernen), Embedding-Modell anbinden, Vektoren berechnen | – |
| R5 | Persönliches Netz: Training aus Rückmeldungen, Sicherheit pro Absender und Muster, „Warum?" | R1, R4 |
| R6 | Rückblick-Test im CLI, nur Zahlen | R5 |
| R7 | Netz in die Entscheidung einhängen, sobald R6 zeigt, dass es besser trifft | R5, R6 |
| R8 | Optional: Sprachmodell für Kartensätze, Fragen, Bitten, Fristen | R4 |

**Meilensteine**

1. *Fragen* – R1, R2, R3: Zuordnen funktioniert schon ohne KI-Paket und
   wirkt sofort über Absender und Muster.
2. *Verstehen* – R4, R5, R6: das Netz lernt aus denselben Zuordnungen,
   messbar im Rückblick.
3. *Handeln* – R7: das Netz entscheidet mit.
4. *Sprechen* – R8.

## Offene Fragen

- Welches Embedding-Modell und welches Sprachmodell? Kandidaten mit guter
  deutscher Qualität und freier Lizenz vergleichen; Größe und Rechenzeit auf
  einem typischen Notebook messen.
- Werden die Bündelnamen sofort verstanden? Mit drei, vier Leuten ohne
  Erklärung testen: Was erwarten sie, wenn sie eine Mail dorthin ziehen?
- Braucht es „Soziales" und eigene Bündel gleich zu Beginn, oder erst später?
- Wie gut trennt das Embedding-Modell Arten von Mails desselben Absenders
  (Angebote, Versand, Konto)? Vor dem Bau mit dem eigenen Postfach messen;
  ohne KI-Paket bleibt das Betreffmuster plus Extraktor-Merkmale der
  Rückfall.
- Wie viele Fragen am Tag sind angenehm? Erst mit drei starten, dann mit
  Testern nachjustieren.
