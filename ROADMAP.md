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
- [ ] IDLE für sofortige Zustellung
- [ ] SMTP-Versand, Entwürfe

## Komfort-Features

- [ ] Vorgänge: zusammengehörige Mails gruppieren (Threads, Kleinanzeigen,
      Bestellung → Versand → Rechnung)
- [ ] Kontenübergreifende Kategorien: automatisch bei Eindeutigkeit, sonst
      Vorschläge mit Drag & Drop
- [ ] Temporäre Kategorien mit Ablaufzeit (Einmalcodes, Versandmails)
- [ ] Rechnungen erkennen und als PDF mit Name und Datum ablegen
- [ ] Newsletter erkennen und abbestellen (RFC 8058 One-Click)
- [ ] Einmalcodes mit einem Klick kopieren
- [ ] Snooze
- [ ] „Wartet auf Antwort"-Erinnerungen
- [ ] Lokale KI für die Einordnung (kein Cloud-Dienst)

## Plattformen

- [x] Linux
- [ ] Windows
- [ ] Android (später)
