# Roadmap

## Muss vor der Veröffentlichung

- [ ] **Datenbank verschlüsseln** (SQLCipher über `rusqlite` mit
      `bundled-sqlcipher`). Schlüssel zufällig erzeugen und im
      System-Schlüsselbund ablegen; optional Passphrase des Nutzers.
      Migration bestehender unverschlüsselter Datenbanken (`sqlcipher_export`).
      Gilt auch für später gespeicherte Mailtexte und Anhänge im Dateisystem.
- [ ] OAuth2 für Gmail und Microsoft (keine gespeicherten Passwörter mehr)
- [ ] Flag-Änderungen und Löschungen synchronisieren (CONDSTORE/QRESYNC,
      Fallback: Flag-Scan)
- [ ] Ordnernamen in modifiziertem UTF-7 korrekt dekodieren (RFC 3501 5.1.3)
- [ ] HTML-Mails sicher anzeigen (abgeschottetes iframe, externe Inhalte
      standardmäßig blockiert)
- [ ] Namensprüfung „Halcyon" (GitHub, crates.io, Paketquellen, Domain)
- [ ] Lizenz final festlegen (derzeit GPL-3.0-or-later)

## Kern

- [ ] Mailtexte und Anhänge laden, Volltextsuche über Inhalte
- [x] IDLE für sofortige Zustellung
- [x] SMTP-Versand mit „Senden rückgängig"
- [ ] Entwürfe, Anhänge senden, Signaturen, Formatierung

## Einstellungen

- [ ] Richtiges Einstellungsmenü, darin unter anderem:
  - [ ] Absender-Logos abschaltbar (jede Abfrage verrät der Absender-Domain
        die IP-Adresse)
  - [ ] Ordner „Als Posteingang behandeln" (z. B. web.de „Unbekannt")
  - [ ] Dauer für „Senden rückgängig"

## Komfort-Features

- [ ] **Vorgänge** (Plan: [docs/design/vorgaenge.md](docs/design/vorgaenge.md)): zusammengehörige Mails gruppieren (Threads, Kleinanzeigen,
      Bestellung → Versand → Rechnung)
- [ ] Kontenübergreifende Kategorien: automatisch bei Eindeutigkeit, sonst
      Vorschläge mit Drag & Drop
- [ ] Temporäre Kategorien mit Ablaufzeit (Einmalcodes, Versandmails)
- [ ] Rechnungen erkennen und als PDF mit Name und Datum ablegen
- [ ] **Spam-Erkennung**, möglichst sauber und ohne Cloud:
      1. Urteil des Servers übernehmen (Junk-Ordner, Schlagwörter `$Junk` /
         `$NotJunk`, `X-Spam-Flag`/`X-Spam-Status`/`X-Spam-Score`)
      2. Absender-Echtheit aus `Authentication-Results` (SPF, DKIM, DMARC),
         nicht passende Anzeigenamen („PayPal" von fremder Domain),
         gefälschte Links (Linktext ≠ Ziel)
      3. Lokal lernender Filter (naiver Bayes über Header und Text), trainiert
         durch „Spam"/„Kein Spam" des Nutzers; Korrekturen auch per
         `$Junk`/`$NotJunk` an den Server zurückmelden
      4. Optional Embeddings aus dem KI-Paket als zusätzliches Merkmal
      Voraussetzung: die Spam- und Authentifizierungs-Header beim Sync
      mitladen. Echte Postfächer dienen nur lokal als Testmaterial, nie als
      Fixtures im Repo.
- [ ] Newsletter erkennen und abbestellen (RFC 8058 One-Click)
- [ ] Einmalcodes mit einem Klick kopieren
- [ ] Snooze
- [ ] „Wartet auf Antwort"-Erinnerungen
- [ ] Optionales KI-Paket (in der App nachladbar, App läuft ohne KI):
      Ähnlichkeitssuche, später lokales LLM, Rechner im Heimnetz oder
      Cloud-KI mit eigenem API-Schlüssel (opt-in, pro Konto, maskiert) –
      siehe [docs/design/klassifizierung.md](docs/design/klassifizierung.md)
- [ ] **Relevanz lernen** (Konzept: [docs/design/relevanz.md](docs/design/relevanz.md)):
      Halcyon fragt anfangs mit drei einfachen Antworten, lernt daraus und aus
      deinem Verhalten und handelt selbst, sobald es sicher ist

## Plattformen

- [x] Linux
- [ ] Windows
- [ ] Android (später)
