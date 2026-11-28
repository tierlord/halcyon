# 06 – Android-Versuchsbau

Für eine Claude-Session auf einem Rechner mit Android Studio und Emulator.
Ziel ist **kein fertiges Android-Halcyon**, sondern herauszufinden, was beim
Bauen und Starten wirklich klemmt. Das Ergebnis ist eine Übergabenotiz mit
Befunden, kein großer Umbau.

## Ziel

1. Halcyon startet im Android-Emulator und zeigt „Heute" mit den
   eingebauten **Testdaten** (kein echtes Konto, keine Zugangsdaten).
2. Für jede Hürde unterwegs ist festgehalten: was passiert ist (Fehlermeldung,
   Logausschnitt), woran es liegt, und was die kleinste Abhilfe wäre.
3. Wenn die Zeit reicht: ein Versuch **ohne** Testdaten-Modus (nur starten,
   ohne Konto anzulegen), um zu sehen, ob der Rust-Teil auf Android hochkommt.

## Regeln

- [AGENTS.md](../../AGENTS.md) lesen und befolgen.
- Auf einem eigenen Branch arbeiten: `android/spike`. **Nicht nach `main`
  mergen.** Am Ende den Branch pushen, damit Claude auf dem Hauptrechner die
  Befunde übernehmen kann.
- **Keine echten Konten, Mails oder Passwörter** im Emulator oder im Repo.
- Änderungen am Code nur, wo sie zum Starten nötig sind, klein und mit
  Kommentar *warum*. Alles Größere nicht umbauen, sondern unten im Ergebnis
  beschreiben.
- Der von Tauri erzeugte Android-Ordner `apps/desktop/src-tauri/gen/android`
  darf committet werden (so empfiehlt es Tauri).
- Nichts installieren, was über die Android-Werkzeuge, Rust-Ziele und
  `pnpm`-Pakete des Projekts hinausgeht, ohne vorher nachzufragen.

## Schritte

### 1. Werkzeuge prüfen

```bash
java -version                 # JDK 17 oder neuer; Android Studio bringt eins mit (jbr)
echo $ANDROID_HOME            # z. B. ~/Android/Sdk
echo $NDK_HOME                # z. B. $ANDROID_HOME/ndk/<version>
ls $ANDROID_HOME/ndk          # mindestens ein NDK installiert?
rustup --version && cargo --version
node --version && pnpm --version
```

Fehlt etwas davon, in der Übergabenotiz festhalten und den Nutzer fragen.
Typische Belegung, falls die Variablen fehlen (Pfade an den Rechner anpassen):

```bash
export JAVA_HOME="$HOME/android-studio/jbr"
export ANDROID_HOME="$HOME/Android/Sdk"
export NDK_HOME="$ANDROID_HOME/ndk/$(ls $ANDROID_HOME/ndk | sort -V | tail -1)"
```

Rust-Ziele für Android:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
```

### 2. Repository holen

```bash
git clone https://github.com/tierlord/halcyon.git
cd halcyon
git switch -c android/spike
cd apps/desktop
pnpm install
```

### 3. Android-Projekt erzeugen

```bash
pnpm tauri android init
```

Erzeugt `src-tauri/gen/android`. Ausgabe und eventuelle Warnungen notieren.

### 4. Mit Testdaten im Emulator starten

Einen Emulator in Android Studio starten (x86_64-Abbild genügt), dann:

```bash
VITE_HALCYON_MOCK=1 pnpm tauri android dev
```

`VITE_HALCYON_MOCK=1` sorgt dafür, dass die Oberfläche die eingebauten
Testdaten zeigt statt die App-Befehle aufzurufen (oben erscheint dann
„Testdaten"). Der Rust-Teil startet trotzdem mit; stürzt die App beim Start
ab, liegt es fast sicher dort (siehe „Bekannte Verdächtige").

Logs dazu:

```bash
adb logcat | grep -iE "halcyon|rust|tauri|panic|AndroidRuntime"
```

Wenn „Heute" erscheint: einige Bildschirmfotos über den Emulator
(Seitenleisten-Kamerasymbol) ablegen und in der Notiz beschreiben, was auf
dem schmalen Bildschirm nicht passt (Seitenleiste, zwei Spalten, Ziehen,
Hinweise beim Überfahren). Die Oberfläche **nicht** umbauen.

### 5. Optional: ohne Testdaten-Modus starten

```bash
pnpm tauri android dev
```

Nur prüfen, ob die App startet und „Heute" leer, aber ohne Absturz zeigt.
Kein Konto anlegen.

## Bekannte Verdächtige

Was auf dem Hauptrechner schon absehbar ist – bitte gezielt prüfen und den
tatsächlichen Befund notieren:

1. **Datenbankpfad:** `default_db_path()` in `apps/desktop/src-tauri/src/lib.rs`
   nutzt `directories::ProjectDirs`. Auf Android liefert das vermutlich nichts,
   dann wird `halcyon.db` relativ geöffnet, und `Store::open(...).expect(...)`
   beendet die App. Kleinste Abhilfe für den Versuch: den Pfad über die
   Umgebungsvariable `HALCYON_DB` setzen oder auf `cfg(mobile)` das
   App-Datenverzeichnis von Tauri nutzen. Was davon funktioniert hat, notieren.
2. **Passwortspeicher:** `crates/halcyon-core/src/secrets.rs` nutzt `keyring`
   (unter Linux Secret Service). Baut das für Android überhaupt, und was
   passiert beim ersten Zugriff?
3. **Zertifikate:** Das Projekt nutzt `rustls-platform-verifier` bzw.
   `rustls-native-certs`. Ersteres braucht auf Android eine Initialisierung
   über JNI und eine Kotlin-Komponente. Tritt das schon beim Start auf oder
   erst bei der ersten Verbindung?
4. **`aws-lc-rs`** (über `lettre`): braucht beim Bauen einen C-Compiler aus
   dem NDK und gegebenenfalls `cmake`. Baut es durch?
5. **Hintergrund-Abruf:** `apps/desktop/src-tauri/src/watch.rs` hält dauerhafte
   IMAP-Verbindungen. Für den Versuch irrelevant (keine Konten), aber
   festhalten, ob beim Start Fehler daraus kommen.
6. **Dateien:** Export und Anhänge nutzen den Download-Ordner und „im Ordner
   zeigen" (`tauri_plugin_opener::reveal_item_in_dir`). Nicht umbauen, nur
   notieren, falls es beim Start schon stört.

## Ergebnis

Ausgeführt am 2026-11-28 auf dem Notebook (Debian 13, 9,5 GB RAM) durch Claude.

- **Rechner / Werkzeuge:** Android-SDK per Kommandozeile in `~/android-sdk`
  (Plattform 35–37, Build-Tools 34/37, Emulator 37.2, AVD `skyq-pixel`,
  Android 35 x86_64 mit Google APIs, 1080×2400), NDK 29.0.14206865
  (clang 21), JDK 21, Rust 1.100.1 (rustup, minimal + 4 Android-Ziele),
  Node 20, pnpm 12.23, Gradle 9.6.1 (vom Wrapper geladen).
  Emulator lief ohne Fenster (`-no-window -gpu swiftshader_indirect`).
- **Hat „Heute" mit Testdaten im Emulator funktioniert?** **Ja**, nach einer
  Korrektur (Datenbankpfad, siehe unten). Die App startet, „Heute" zeigt
  die Testdaten (Begrüßung, Rechnungskarte, Personen-Kreise).
  Bildschirmfoto: nur lokal beim Ausführenden, nicht im Repo.
- **Befunde je Hürde:**
  1. *Werkzeuge:* Rust und NDK fehlten, pnpm war nur ein veralteter
     Corepack-Shim (sucht `pnpm.cjs`, pnpm 12 liefert `pnpm.mjs`).
     Abhilfe: rustup, `sdkmanager "ndk;29.0.14206865"`, Wrapper-Skript
     `~/.local/bin/pnpm`. `ANDROID_HOME`/`NDK_HOME`/`PATH` müssen gesetzt sein
     (`NDK_HOME` zeigt auf die NDK-Version, nicht auf `ndk/`).
  2. *Emulator und KVM:* „This user doesn't have permissions to use KVM", weil
     eine ältere Shell die Gruppe `kvm` nicht kannte. Abhilfe: `sg kvm -c
     "emulator ..."` oder neu anmelden. Der Emulator liegt nicht im `PATH`
     von `env.sh` (nur `platform-tools`).
  3. *`.gitignore`:* `apps/desktop/src-tauri/gen/` war ignoriert, obwohl der
     Android-Ordner committet werden soll. Geändert auf `gen/*` mit Ausnahme
     `!gen/android`.
  4. *`tauri android init --ci`:* ohne Warnungen durchgelaufen.
  5. *Datenbankpfad (Verdächtiger 1) – bestätigt:* Absturz beim Start,
     `panicked at src-tauri/src/lib.rs:1916: could not open the Halcyon
     database: ... CannotOpen ... "unable to open database file: halcyon.db"`.
     Ursache: `ProjectDirs` liefert auf Android nichts, `halcyon.db` wird im
     schreibgeschützten Arbeitsverzeichnis geöffnet. Abhilfe im Versuch:
     auf Android `/data/data/<Paketname>/files/halcyon.db`, Paketname aus
     `/proc/self/cmdline`. Sauber wäre `app.path().app_data_dir()` in `setup`
     (Store dort statt vor dem Builder anlegen), das ist ein größerer Umbau.
  6. *`aws-lc-rs` (Verdächtiger 4):* baut mit dem NDK-Compiler durch, kein
     `cmake` nötig. Kompletter Rust-Build (x86_64, Debug, kalt): 6 min 53 s.
  7. *Passwortspeicher `keyring` (Verdächtiger 2):* baut für Android. Beim
     Start kein Zugriff und kein Fehler; das Verhalten beim ersten echten
     Zugriff ist **ungeprüft** (ohne Konto nicht auslösbar). Vermutung:
     ohne Android-Backend (Keystore) schlägt das Speichern fehl.
  8. *Zertifikate (Verdächtiger 3):* beim Start kein Fehler. Die
     JNI-Initialisierung von `rustls-platform-verifier` greift vermutlich
     erst bei der ersten Verbindung; **ungeprüft**.
  9. *`watch.rs` (Verdächtiger 5):* ohne Konten keine Fehler beim Start.
     Dauerhafte IMAP-Verbindungen im Hintergrund brauchen auf Android einen
     Vordergrunddienst, sonst beendet das System sie.
  10. *Dateien (Verdächtiger 6):* beim Start keine Auffälligkeit,
      `reveal_item_in_dir` und Download-Ordner nicht geprüft.
  11. *Berechtigungen:* in der Konsole `notification.is_permission_granted
      not allowed`. Für das Benachrichtigungs-Plugin fehlt unter
      `capabilities/` die Erlaubnis für Mobil (kleine Ergänzung nötig).
  12. *Gradle-Warnungen:* nur Veraltet-Hinweise (Java-Ziel 8, `onBackPressed`),
      kein Fehler.
  13. *Lauf ohne Testdaten (Schritt 5):* **nicht durchgeführt**, weil der
      Emulator überlastet war (siehe Offene Fragen).
- **Geänderte Dateien** und warum:
  `apps/desktop/src-tauri/src/lib.rs` (Datenbankpfad auf Android, mit
  Kommentar), `.gitignore` (Android-Ordner nicht ignorieren),
  `apps/desktop/src-tauri/gen/android/` (neu, von Tauri erzeugt),
  diese Notiz.
- **Was auf dem schmalen Bildschirm nicht passt** (aus einem einzigen
  Bildschirmfoto, 1080×2400):
  - Die Seitenleiste steht dauerhaft als etwa 44 % breite Spalte neben dem
    Inhalt, statt als ausfahrbare Leiste; Beschriftungen werden abgeschnitten
    („Vorgänge & Quel…").
  - Der Inhalt wird dadurch sehr schmal (Rechnungskarte bricht Zeile für Zeile
    um, Suchfeld läuft rechts aus dem Bild).
  - Statusleiste und Kopfzeile überlappen (Uhrzeit über dem Logo): es fehlen
    die Sicherheitsabstände (`env(safe-area-inset-*)`).
  - Ziehen und Hinweise beim Überfahren (Hover) nicht geprüft.
- **Offene Fragen:**
  - Emulator war mit 4 GB frei, Software-GPU und laufendem Gradle-Daemon
    überlastet („System UI isn't responding"). Weitere Bildschirmfotos und
    der Lauf ohne Testdaten stehen aus; besser mit mehr RAM oder
    Hardware-GPU wiederholen.
  - Soll der Datenbankpfad sauber über `app_data_dir()` gelöst werden?
  - Wie soll der Passwortspeicher auf Android aussehen (Keystore)?
