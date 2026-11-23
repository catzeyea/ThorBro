# ThorBro – Architektur

Diese Seite erklärt, wie ThorBro aufgebaut ist und wie Oberfläche und Rust-Backend
zusammenspielen. Sie richtet sich an Leute, die C++ kennen, aber neu in Rust und Tauri sind.
(Eine englische Fassung steht in [ARCHITECTURE.md](ARCHITECTURE.md).)

## Überblick

ThorBro ist eine [Tauri 2](https://tauri.app)-App:

- Das **Frontend** ist normales HTML/CSS/JavaScript in `ui/`. Es läuft in einer Webview, also
  einem eingebetteten Browserfenster. Es gibt keinen Bundler und kein Framework.
- Das **Backend** ist ein Rust-Programm in `src/`. Es macht die eigentliche Arbeit: `adb` aufrufen,
  Dateien kopieren, Archive entpacken, Spielstände vergleichen.
- Beide reden über Tauris **IPC** miteinander. JavaScript ruft Rust-Funktionen per
  `invoke(...)` auf. Rust schickt Fortschritt per **Events** zurück.

Zum Thor kommt das Backend ausschließlich über das externe Programm `adb` (Android Debug Bridge).
Für die Bildschirmspiegelung startet es `scrcpy`.

```
┌──────────── Webview (ui/) ────────────┐        ┌──────────── Rust (src/) ─────────────┐        ┌── Thor ──┐
│ index.html + js/*.js                  │ invoke │ commands.rs  (#[tauri::command])     │  adb   │ Android  │
│   invoke("list_dir", {path})  ────────┼───────▶│   └▶ files.rs / library.rs / saves/ ─┼───────▶│  shell   │
│   T.event.listen("transfer") ◀────────┼────────│ app.emit("transfer", Progress)       │        │          │
└───────────────────────────────────────┘ events └──────────────────────────────────────┘        └──────────┘
```

## Dateien

### Backend (`src/`)

| Datei | Aufgabe |
|---|---|
| `main.rs` | Einstiegspunkt. Mit Argumenten startet die Kommandozeile, sonst das Fenster. Registriert State und alle Commands. |
| `commands.rs` | Alle `#[tauri::command]`-Funktionen, also das, was die UI aufrufen darf. Dünne Hüllen um die Module darunter. Enthält auch `AppState`. |
| `cli.rs` | Kommandozeile: `thorbro stream / displays / list / status / pc / thor`. |
| `adb.rs` | adb finden und aufrufen, Shell-Quoting (`shell_quote`), `run-as` für private App-Ordner, Gerätestatus, Speicherorte. |
| `files.rs` | Dateimanager (auflisten, umbenennen, löschen, Größen), Kopier-Warteschlange mit Fortschritt und Abbrechen: PC → Thor, innerhalb des Thor (geprüftes Verschieben), Thor → PC („Copy to PC…“ aus dem File Pool). |
| `library.rs` | ES-DE-`ROMs`-Ordner finden und gedroppte Dateien einem System zuordnen (Switch-Title-ID, Header-Bytes, Dateiendung). |
| `unpack.rs` | Archive entpacken (zip/7z/tar in Rust, sonst externes 7z/bsdtar/unrar), Dateinamen FAT-tauglich machen. |
| `stream.rs` | Displays erkennen (`scrcpy --list-displays`), pro Display ein scrcpy-Fenster starten und beenden, festhalten, warum ein Stream fehlgeschlagen ist. |
| `pcstream/` | „Stream PC games to your Thor“ (Unter-Tab von Stream): `sunshine.rs` (Dienststatus, `GET /serverinfo`), `network.rs` (LAN-Adressen, `tailscale`-CLI), `admin.rs` (Sunshine-Admin-API per HTTPS aus Rust, Login im Schlüsselbund des Systems, Pairing, App-Liste), `thor.rs` (Moonlight/Tailscale-App prüfen, „Play on Thor“ und „Send to Thor app“ per `am start`), `profile.rs` (Stream-Profil-JSON, siehe `STREAM-PROFILE.md`). Hintergrund: `PC-STREAMING-RESEARCH.md`. |
| `util.rs` | Kleinkram: Größen/Zeit formatieren, MD5, Ordner kopieren und spiegeln (Ersatz für rsync), `which`, Prozess mit Timeout. |
| `saves/mod.rs` | Überblick über das Spielstand-Modul, gemeinsame Helfer, Re-Exports. |
| `saves/config.rs` | `games.json` lesen und schreiben, Daten- und Backup-Ordner. |
| `saves/snapshot.rs` | „Fingerabdruck“ eines Save-Ordners (MD5 pro Datei) und Vergleich zweier Fingerabdrücke. |
| `saves/scan.rs` | PC und Thor nach möglichen Save-Ordnern eines Spiels durchsuchen. |
| `saves/ai.rs` | Die `claude`-CLI aus den Suchergebnissen die passenden Ordner wählen lassen (optional). |
| `saves/transfer.rs` | Ordner zwischen PC und Thor kopieren, Backups, laufende Emulatoren schließen. |
| `saves/sync.rs` | Status, Plan und der eigentliche Sync „Weiterspielen auf PC/Thor“. |
| `saves/import.rs` | Ablauf beim Importieren eines heruntergeladenen Spielstands. |
| `saves/import_package.rs` | Import-Paket holen, entpacken und von Readmes/ROMs/Bildern befreien. |
| `saves/import_root.rs` | Den Unterordner im Paket finden, der zum Save-Ordner des Spiels passt (ggf. Dateien umsortieren). |

### Frontend (`ui/`)

| Datei | Aufgabe |
|---|---|
| `index.html` | Gerüst der drei Tabs (Files, Saves, Stream) und lädt die Scripts in fester Reihenfolge. |
| `style.css` | Aussehen. |
| `js/core.js` | Tauri-Zugriff (`T`, `invoke`), Helfer (`$`, `esc`, `el`, `human`), globaler Zustand `S`, `toast`, `call`. |
| `js/dialogs.js` | Modale Dialoge: `modal`, `dialog`, `ask`, `busyModal`, `folderPicker`. |
| `js/device.js` | Tabs, Geräteerkennung (`pollDevice`) und der Verbindungsstatus im Header (`renderDeviceStatus`: Status-Pille, ThorBro wach / schlafend). |
| `js/files.js` | Dateimanager-Tab, Teil 1: Orte, Listing-Cache, virtualisierte Liste, Auswahl, Verlauf, Tastatur. |
| `js/files-actions.js` | Dateimanager-Tab, Teil 2: Ausschneiden/Kopieren/Einfügen, Umbenennen in der Zeile, Verschieben, Löschen, Kontextmenüs, Ziehen von Zeilen (in Ordner oder in den File Pool). |
| `js/transfer.js` | Senden an den Thor: Drag & Drop mit Drop-Karte, Ctrl+O / „Add files …“, Einsortier-Dialog, Kopier-Warteschlange, Fortschrittsleiste (zeigt die „transfer“-Events, auch „file X of Y“). |
| `js/pool.js` | Der File Pool in der Seitenleiste (Thor-Dateien sammeln, „Copy to PC…“), die Rückfrage bei großen Mengen für Drops und Einfügen. |
| `js/saves.js` | Spielkarten, Kartenmenü, Sync-Ablauf. |
| `js/saves-setup.js` | Spiel hinzufügen (Suche, KI-Vorschlag, Prüfen). |
| `js/saves-import.js` | Spielstand importieren. |
| `js/stream.js` | Stream-Tab, Unter-Tab „Mirror Thor screens“: drei Kacheln (beide / oben / unten, feste IDs 0 und 4), Stopp, Fehler; die scrcpy-Display-Erkennung für die Bildschirmauswahl. |
| `js/pcstream.js` | Stream-Tab, Unter-Tab „Stream PC games“ (Status-Chips, „Play on Thor“, „Send to Thor app“, Pairing) und die Help-Seite (Sunshine und Login, Adressen, Apps auf dem Thor, Bildschirmauswahl, Checkliste, Stream-Profil). |
| `js/pins.js` | Angeheftete Thor-Ordner: Pin-Knopf in der Pfadleiste (Strg+D), Karte „pinned“ in der Ortsleiste, Zeile „pinned“ im Start-Tab, Zustand „not found“ (`pins`, `add_pin`, `remove_pin`, `pins_found`; gespeichert in `settings.json`). |
| `js/start.js` | Start-Tab (Ablagefläche, Bibliotheks-Kacheln), die Ortsleiste des Dateimanagers, die Tastenkürzel-Liste. |
| `js/main.js` | Start: öffnet Start (bei jedem Start, der letzte Tab wird nicht gemerkt), erste Geräteabfrage und Timer. |

Die JS-Dateien sind klassische `<script>`-Dateien ohne `type="module"`. Sie teilen sich einen
globalen Gültigkeitsbereich, so als wären sie hintereinander in eine Datei kopiert. Deshalb
gilt: Code, der **beim Laden sofort** läuft (z. B. `$("#up").onclick = …`), darf nur Namen aus
derselben oder einer früheren Datei benutzen. Code **in Funktionen** läuft erst später und darf
alles benutzen.

## Wie Frontend und Backend reden

### 1. JavaScript ruft Rust auf: `invoke`

```js
// ui/js/files.js
const entries = await invoke("list_dir", { path });
```

`call` ist ein kleiner Wrapper aus `core.js`. Er ruft `T.core.invoke(cmd, args)` auf und zeigt
Fehler als Toast an. `invoke` gibt ein **Promise** zurück, das Ergebnis kommt also asynchron.

### 2. Die Rust-Seite: `#[tauri::command]`

```rust
// src/commands.rs
#[tauri::command]
pub async fn list_dir(path: String) -> CmdResult<Vec<files::Entry>> {
    run_blocking(move || files::listdir(&path)).await
}
```

- Das Attribut `#[tauri::command]` ist ein **Makro**. Es erzeugt beim Kompilieren den Code,
  der die JSON-Argumente aus JavaScript in Rust-Typen umwandelt. Der Name im JS-Objekt (`path`)
  muss zum Parameternamen passen. JS schreibt camelCase (`stopPkg`), Rust snake_case
  (`stop_pkg`). Tauri übersetzt automatisch.
- Der Rückgabewert wird mit **serde** nach JSON umgewandelt. Deshalb tragen alle Structs, die zur
  UI gehen, `#[derive(Serialize)]`. Das ist ungefähr so, als würde der Compiler eine
  `to_json()`-Methode für dich schreiben.
- `CmdResult<T>` ist `Result<T, String>`. `Ok(wert)` kommt in JS als aufgelöstes Promise an,
  `Err(text)` als Exception mit diesem Text.
- `async fn` plus `run_blocking(...)`: adb-Aufrufe blockieren. Damit das Fenster nicht einfriert,
  läuft die Arbeit in einem Thread-Pool (`spawn_blocking`), und der Command wartet mit `.await`
  darauf. Vergleichbar mit `std::async` und anschließendem Warten auf das Future, nur ohne
  dass ein Thread dabei blockiert.
- Parameter wie `state: State<'_, AppState>` oder `app: AppHandle` kommen nicht aus JavaScript.
  Tauri setzt sie selbst ein (Dependency Injection).

### 3. Registrieren: `generate_handler!`

```rust
// src/main.rs
tauri::Builder::default()
    .manage(AppState::default())
    .invoke_handler(tauri::generate_handler![commands::device, commands::list_dir, /* … */])
```

Nur Funktionen, die hier stehen, kann die UI aufrufen. `generate_handler!` ist ebenfalls ein
Makro. Es baut daraus eine Tabelle „Name → Funktion“. Der Name in JS ist der reine
Funktionsname (`"list_dir"`), ohne Modulpfad.

`.manage(AppState::default())` legt den globalen Zustand an, z. B. die gefundenen Game-Libraries
oder die Kopier-Warteschlange. Commands bekommen ihn als `State<'_, AppState>`. Weil mehrere
Commands gleichzeitig laufen können, steckt veränderlicher Zustand in einem `Mutex`.

### 4. Rust meldet sich bei JavaScript: Events

Für Dinge, die länger dauern, schickt Rust zwischendurch Events:

```rust
// src/files.rs
let _ = app.emit("transfer", progress);   // Progress ist #[derive(Serialize)]
```

```js
// ui/js/transfer.js
T.event.listen("transfer", async ({ payload: progress }) => { /* Balken aktualisieren */ });
```

| Event | Gesendet von | Empfangen in | Inhalt |
|---|---|---|---|
| `transfer` | `files.rs` | `transfer.js` | Fortschritt beim Kopieren/Entpacken/Holen (`Progress`) |
| `save-step` | `commands.rs` (`scan_game`, `do_import`) | `saves-setup.js`, `saves-import.js` | Statustext für den Spinner |

Dazu kommt ein Event, das Tauri selbst liefert: `onDragDropEvent` meldet, wenn Dateien aus dem
System auf das Fenster gezogen werden (`transfer.js`).

## Beispiel 1: Gerät erkennen

1. **Start.** `ui/js/main.js` ruft `pollDevice(true)` auf und danach alle 3 Sekunden
   `pollDevice(false)`.
2. **Anfrage.** `pollDevice` in `ui/js/device.js` ruft `invoke("device")` auf.
3. **Command.** `commands::device` in `src/commands.rs` führt `adb::device_status()` über
   `run_blocking` im Thread-Pool aus.
4. **adb.** `adb::device_status` in `src/adb.rs` startet `adb devices`. Das adb-Programm wurde
   beim ersten Aufruf von `adb_path()` gesucht und zwischengespeichert. Die Ausgabe wird
   ausgewertet:
   - adb nicht startbar → „adb not found …“
   - keine Zeile → „No Thor found …“
   - mehr als ein Gerät → „More than one Android device found …“
   - Status `unauthorized` → „Confirm the USB debugging prompt …“
   - Status `device` → verbunden. Der Modellname kommt aus `adb shell getprop ro.product.model`.
5. **Antwort.** Das Ergebnis `Device { connected, message }` wird zu JSON
   `{ "connected": true, "message": "AYN Thor" }`.
6. **Anzeige.** `pollDevice` ruft bei jeder Antwort `renderDeviceStatus()` auf: Die Pille im
   Header zeigt „Connected: <Modell>“ oder „Not connected“ (der Grund aus Schritt 4 steht im
   Tooltip), der ThorBro daneben ist wach (`thorbro-done.svg`) oder schläft (`thorbro-wait-off.svg`).
   Hat sich `connected` geändert, folgt:
   - verbunden: `loadPlaces(true)` → `invoke("places")` → `commands::places` → `adb::storages()`
     (interner Speicher und SD-Karten) und `library::find_libraries()` (ein Shell-Script pro
     Speicher liest alle `ROMs/*/systeminfo.txt`). Die Libraries landen in `AppState.libs` und
     in `S.libs`. Danach öffnet der Dateimanager den zuletzt benutzten Ordner.
   - getrennt: Statt der Dateiliste erscheint die Karte „No Thor connected“.
   - Im Saves-Tab werden zusätzlich die Spielkarten neu geprüft.

## Beispiel 2: Spiel per Drop einsortieren

1. **Drop.** Der Benutzer zieht z. B. `Zelda [0100F2C0115B6000].nsp` auf das Fenster. Tauri ruft
   den `onDragDropEvent`-Handler in `ui/js/transfer.js` mit `type: "drop"` und den lokalen Pfaden auf.
2. **Weiterleiten.** Ist der Import-Dialog offen, geht der Pfad dorthin
   (`S.importTarget`). Sonst wird `incoming(paths)` aufgerufen.
3. **Klassifizieren.** `incoming` ruft `invoke("classify", { paths, cwd, lib: 0, force: false })` auf.
4. **Command.** `commands::classify` nimmt die Library aus `AppState.libs` und baut für jeden Pfad mit
   `incoming_item` ein `Incoming`:
   - `library::classify` → `classify_inner` probiert nacheinander:
     1. `guess_switch`: Title-ID im Namen. Endet sie auf `000`, ist es ein Spiel. Auf `800` ist
        es ein Update, sonst DLC (beides → `switch/update`).
     2. `guess_by_header`: Magic Bytes am Dateianfang (Switch, 3DS, GameCube, Wii, PSP, PS1/PS2-ISO).
     3. `guess_by_extension`: Welche Systeme laut `systeminfo.txt` die Endung kennen, sortiert mit
        `rank` (Systeme mit Spielen zuerst, dann die `PREFERRED`-Liste).

     Bei Ordnern wird die größte Datei untersucht, bei zip/7z die größte Datei im Archiv.
   - `targets_for` baut die Liste der Zielordner: ★ vermutetes System, weitere Kandidaten, aktueller
     Ordner, Switch-Updates, alle Systeme A–Z.
   - Bei Archiven entscheidet `incoming_item`, ob „Erst entpacken“ vorgeschlagen wird.
   - Ist nichts Spieleartiges dabei, gibt der Command eine leere Liste zurück. Die UI kopiert dann
     direkt in den aktuellen Ordner.
5. **Dialog.** `sortDialog` in `transfer.js` zeigt pro Datei eine Zeile (`buildSortRow`) mit
   vorausgewähltem Ziel. Bei mehreren Libraries kann man oben wechseln. Das löst erneut `classify`
   aus, diesmal mit `force: true`.
6. **Kopieren.** Klick auf „Copy“ baut pro Zeile einen Job `{ src, dest, unpack, flatten, remote }`
   und ruft `invoke("enqueue", { jobs })` auf.
7. **Warteschlange.** `commands::enqueue` → `files::enqueue` hängt die Jobs an die `SharedQueue`
   (`Arc<Mutex<Queue>>`, vergleichbar mit `std::shared_ptr` plus Mutex). Läuft noch kein
   Worker-Thread, wird einer gestartet.
8. **Worker.** `files::worker` holt Job für Job (`next_job`):
   - Normal: `push_one` → `adb push`. Der Zielname wird mit `unpack::safe_name` FAT-tauglich
     gemacht. Währenddessen fragt `wait_with_polling` alle 0,7 s per `du` auf dem Thor ab, wie viel
     schon angekommen ist, und schickt `transfer`-Events.
   - Mit Entpacken: `unpack_job` entpackt in einen Cache-Ordner (`unpack::extract`) und schiebt den
     Inhalt hoch.
9. **Fertig.** Danach stößt der Worker den Android-Medienscanner an und schickt ein letztes
   `transfer`-Event mit `busy: false`. `transfer.js` zeigt einen Toast, holt mit `refresh_libs`
   die neuen Spielzahlen und lädt den aktuellen Ordner neu, falls er betroffen war.

## Beispiel 3: Spielstand synchronisieren

Ausgangslage: Das Spiel ist in `~/.config/thor-save/games.json` mit einem PC- und einem
Thor-Ordner eingetragen. Man hat zuletzt auf dem Thor gespielt.

1. **Status anzeigen.** Beim Öffnen des Saves-Tabs ruft `loadGames()` (`ui/js/saves.js`)
   `invoke("games")` auf und baut pro Spiel eine `GameCard`. `card.refresh()` ruft
   `invoke("game_status", { name, connected })` auf.
2. **Fingerabdrücke.** `commands::game_status` → `saves::status` (`saves/sync.rs`):
   - `pc_snapshot` (`saves/snapshot.rs`) geht den PC-Ordner durch und berechnet pro Datei MD5,
     Änderungszeit und Größe.
   - `thor_snapshot` macht dasselbe auf dem Thor mit **einem** Shell-Aufruf (`md5sum` + `stat`).
     Bei privaten App-Ordnern (`/data/data/<paket>`) wird das Kommando über `adb::tcmd` in
     `run-as <paket>` verpackt.
   - `same()` vergleicht die MD5-Werte. `newest()` liefert die jüngste Änderungszeit.
3. **Karte.** `showSyncStatus` zeigt „● Thor is newer“ und hebt den Knopf „💻 Continue on PC“ hervor.
4. **Planen.** Klick darauf → `startSync(card, "pc")` → `invoke("plan_sync", { name, target: "pc" })`
   → `saves::plan_sync`. Es wird noch nichts kopiert, nur ausgewertet: Ist die Quelle leer? Sind
   beide Seiten gleich? Ist das Ziel neuer (`dst_newer`)? Beim Ziel Thor kommt hinzu: Läuft der
   Emulator gerade (`thor_running_pkg` über `pidof`)?
5. **Bestätigen.** `syncConfirmText` baut daraus den Dialogtext. Ist das Ziel neuer, gibt es eine
   rote Warnung, und „Cancel“ ist vorausgewählt.
6. **Sync.** Nach „Let's go“ ruft `runSync` `invoke("run_sync", { name, target, stopPkg })` auf
   → `saves::run_sync`:
   1. ggf. Emulator beenden (`stop_thor_pkg` → `am force-stop`)
   2. neue Snapshots beider Seiten
   3. **Backup** des Ziels nach `~/.local/share/thor-save/backups/<spiel>/<zeitstempel>-pc`
      (`transfer::backup_side`). Danach werden alte Backups gelöscht, sodass 15 übrig bleiben
      (`prune_backups`).
   4. **Kopieren** (`copy_thor_to_pc`): `transfer::pull_dir` holt den Thor-Ordner in einen
      temporären Ordner (normal per `adb pull`, bei privaten Ordnern als tar-Stream über
      `adb exec-out` + `run-as`). Dann macht `util::mirror` den PC-Ordner exakt gleich, wie
      `rsync --delete`.
   5. **Prüfen:** neuer Snapshot des Ziels. Stimmt er nicht mit der Quelle überein, gibt es einen
      Fehler mit dem Pfad zum Backup.

   In die andere Richtung (`copy_pc_to_thor`) schiebt `transfer::push_dir` den Ordner erst in
   einen Staging-Ordner neben dem Ziel (`<ordner>.thor-save-neu`). Erst am Ende tauscht es ihn
   mit dem alten Ordner. So bleibt bei einem Abbruch nie ein halb kopierter Spielstand zurück.
7. **Ende.** Die UI zeigt „Done! Have fun on the PC 🎮“ und ruft `card.refresh()` auf. Die Karte
   zeigt jetzt „✓ Both in sync“.

Dasselbe gibt es auf der Kommandozeile: `thorbro pc <spiel>` in `src/cli.rs` (`cli_sync`) nutzt
genau dieselben Funktionen `plan_sync` und `run_sync`.
