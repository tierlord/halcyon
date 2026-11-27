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

*Von der ausführenden Session auszufüllen.*

- **Rechner / Werkzeuge:** Android-Studio-, NDK-, JDK-, Rust-Version
- **Hat „Heute" mit Testdaten im Emulator funktioniert?**
- **Befunde je Hürde** (Fehlermeldung, Ursache, kleinste Abhilfe):
- **Geänderte Dateien** und warum:
- **Was auf dem schmalen Bildschirm nicht passt:**
- **Offene Fragen:**
