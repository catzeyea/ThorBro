# ThorBro – Architecture

This page explains how ThorBro is put together and how the UI and the Rust backend work
together. It is written for people who know C++ but are new to Rust and Tauri.
(A German version of this page is in [ARCHITEKTUR.md](ARCHITEKTUR.md).)

## Overview

ThorBro is a [Tauri 2](https://tauri.app) app:

- The **frontend** is plain HTML/CSS/JavaScript in `ui/`. It runs in a webview, an embedded
  browser window. There is no bundler and no framework.
- The **backend** is a Rust program in `src/`. It does the actual work: calling `adb`, copying
  files, extracting archives, comparing saves.
- The two talk over Tauri's **IPC**. JavaScript calls Rust functions with `invoke(...)`. Rust
  sends progress back as **events**.

The backend reaches the Thor only through the external program `adb` (Android Debug Bridge).
For screen mirroring it starts `scrcpy`.

```
┌──────────── Webview (ui/) ────────────┐        ┌──────────── Rust (src/) ─────────────┐        ┌── Thor ──┐
│ index.html + js/*.js                  │ invoke │ commands.rs  (#[tauri::command])     │  adb   │ Android  │
│   invoke("list_dir", {path})  ────────┼───────▶│   └▶ files.rs / library.rs / saves/ ─┼───────▶│  shell   │
│   T.event.listen("transfer") ◀────────┼────────│ app.emit("transfer", Progress)       │        │          │
└───────────────────────────────────────┘ events └──────────────────────────────────────┘        └──────────┘
```

## Files

### Backend (`src/`)

| File | Purpose |
|---|---|
| `main.rs` | Entry point. With arguments it runs the command line, otherwise the window. Registers the state and all commands. |
| `commands.rs` | All `#[tauri::command]` functions, i.e. everything the UI is allowed to call. Thin wrappers around the modules below. Also holds `AppState`. |
| `cli.rs` | Command line: `thorbro stream / list / status / pc / thor`. |
| `adb.rs` | Finding and calling adb, shell quoting (`shell_quote`), `run-as` for private app folders, device status, storage locations. |
| `files.rs` | File manager (list, rename, delete, fetch), copy queue PC → Thor with progress, dragging Thor → PC through a cache. |
| `library.rs` | Finding ES-DE `ROMs` folders and assigning dropped files to a system (Switch title ID, header bytes, file extension). |
| `unpack.rs` | Extracting archives (zip/7z/tar in Rust, otherwise external 7z/bsdtar/unrar), making file names FAT-safe. |
| `stream.rs` | Starting and stopping the scrcpy windows for the top and bottom screen. |
| `util.rs` | Small helpers: formatting sizes/times, MD5, copying and mirroring folders (replacement for rsync), `which`, running a process with a timeout. |
| `saves/mod.rs` | Overview of the save module, shared helpers, re-exports. |
| `saves/config.rs` | Reading and writing `games.json`, data and backup folders. |
| `saves/snapshot.rs` | "Fingerprint" of a save folder (MD5 per file) and comparing two fingerprints. |
| `saves/scan.rs` | Searching the PC and the Thor for possible save folders of a game. |
| `saves/ai.rs` | Letting the `claude` CLI pick the matching folders from the search results (optional). |
| `saves/transfer.rs` | Copying folders between PC and Thor, backups, closing running emulators. |
| `saves/sync.rs` | Status, plan and the actual sync "Continue on PC/Thor". |
| `saves/import.rs` | The flow for importing a downloaded save. |
| `saves/import_package.rs` | Fetching an import package, extracting it and stripping readmes/ROMs/images. |
| `saves/import_root.rs` | Finding the sub-folder in the package that matches the game's save folder (rearranging files if needed). |

### Frontend (`ui/`)

| File | Purpose |
|---|---|
| `index.html` | Skeleton of the three tabs (Files, Saves, Stream); loads the scripts in a fixed order. |
| `style.css` | Look and feel. |
| `js/core.js` | Tauri access (`T`, `invoke`), helpers (`$`, `esc`, `el`, `human`), global state `S`, `toast`, `call`. |
| `js/dialogs.js` | Modal dialogs: `modal`, `dialog`, `ask`, `busyModal`, `folderPicker`. |
| `js/device.js` | Tabs and device detection (`pollDevice`). |
| `js/files.js` | File manager tab and dragging out to the PC. |
| `js/transfer.js` | Sending to the Thor: drag & drop, sort dialog, copy queue, progress bar. |
| `js/saves.js` | Game cards, card menu, sync flow. |
| `js/saves-setup.js` | Adding a game (search, AI suggestion, check). |
| `js/saves-import.js` | Importing a save. |
| `js/stream.js` | Stream tab. |
| `js/main.js` | Startup: first device poll and timers. |

The JS files are classic `<script>` files without `type="module"`. They share one global scope,
as if they had been pasted into one file one after another. That means: code that runs
**immediately on load** (e.g. `$("#up").onclick = …`) may only use names from the same or an
earlier file. Code **inside functions** runs later and may use anything.

## How frontend and backend talk

### 1. JavaScript calls Rust: `invoke`

```js
// ui/js/files.js
S.entries = await call("list_dir", { path: S.cwd });
```

`call` is a small wrapper from `core.js`. It calls `T.core.invoke(cmd, args)` and shows errors
as a toast. `invoke` returns a **Promise**, so the result arrives asynchronously.

### 2. The Rust side: `#[tauri::command]`

```rust
// src/commands.rs
#[tauri::command]
pub async fn list_dir(path: String) -> CmdResult<Vec<files::Entry>> {
    run_blocking(move || {
        files::save_last(&path);
        files::listdir(&path)
    })
    .await
}
```

- The attribute `#[tauri::command]` is a **macro**. At compile time it generates the code that
  converts the JSON arguments from JavaScript into Rust types. The key in the JS object (`path`)
  must match the parameter name. JS uses camelCase (`stopPkg`), Rust uses snake_case
  (`stop_pkg`); Tauri translates automatically.
- The return value is converted to JSON with **serde**. That's why every struct that goes to the
  UI carries `#[derive(Serialize)]`. It's roughly as if the compiler wrote a `to_json()` method
  for you.
- `CmdResult<T>` is `Result<T, String>`. `Ok(value)` arrives in JS as a resolved Promise,
  `Err(text)` as an exception with that text.
- `async fn` plus `run_blocking(...)`: adb calls block. So that the window doesn't freeze, the
  work runs in a thread pool (`spawn_blocking`) and the command waits for it with `.await`.
  Comparable to `std::async` followed by waiting on the future, except that no thread is blocked
  while waiting.
- Parameters like `state: State<'_, AppState>` or `app: AppHandle` don't come from JavaScript.
  Tauri fills them in itself (dependency injection).

### 3. Registering: `generate_handler!`

```rust
// src/main.rs
tauri::Builder::default()
    .manage(AppState::default())
    .invoke_handler(tauri::generate_handler![commands::device, commands::list_dir, /* … */])
```

Only functions listed here can be called by the UI. `generate_handler!` is a macro too. It builds
a "name → function" table from the list. The name in JS is the bare function name
(`"list_dir"`), without the module path.

`.manage(AppState::default())` creates the global state, e.g. the detected game libraries or the
copy queue. Commands receive it as `State<'_, AppState>`. Because several commands can run at
the same time, mutable state sits behind a `Mutex`.

### 4. Rust reports back to JavaScript: events

For things that take a while, Rust sends events along the way:

```rust
// src/files.rs
let _ = app.emit("transfer", progress);   // Progress is #[derive(Serialize)]
```

```js
// ui/js/transfer.js
T.event.listen("transfer", async ({ payload: progress }) => { /* update the bar */ });
```

| Event | Sent by | Received in | Payload |
|---|---|---|---|
| `transfer` | `files.rs` | `transfer.js` | Progress while copying/extracting/fetching (`Progress`) |
| `save-step` | `commands.rs` (`scan_game`, `do_import`) | `saves-setup.js`, `saves-import.js` | Status text for the spinner |
| `drag-done` | `commands.rs` (`start_os_drag`) | `files.js` | `true` if the drag-out ended with a drop |

On top of that there is one event that Tauri provides itself: `onDragDropEvent` reports files
being dragged from the system onto the window (`transfer.js`).

## Example 1: Detecting the device

1. **Startup.** `ui/js/main.js` calls `pollDevice(true)`, then `pollDevice(false)` every
   3 seconds.
2. **Request.** `pollDevice` in `ui/js/device.js` calls `invoke("device")`.
3. **Command.** `commands::device` in `src/commands.rs` runs `adb::device_status()` in the thread
   pool via `run_blocking`.
4. **adb.** `adb::device_status` in `src/adb.rs` runs `adb devices`. The adb program was looked
   up on the first call of `adb_path()` and cached. The output is evaluated:
   - adb can't be started → "adb not found …"
   - no device line → "No Thor found …"
   - more than one device → "More than one Android device found …"
   - status `unauthorized` → "Confirm the USB debugging prompt …"
   - status `device` → connected. The model name comes from `adb shell getprop ro.product.model`.
5. **Response.** The result `Device { connected, message }` becomes the JSON
   `{ "connected": true, "message": "AYN Thor" }`.
6. **Display.** `pollDevice` colors the dot in the top right and sets the text. If `connected`
   changed, this follows:
   - connected: `loadPlaces(true)` → `invoke("places")` → `commands::places` → `adb::storages()`
     (internal storage and SD cards) and `library::find_libraries()` (one shell script per
     storage reads all `ROMs/*/systeminfo.txt`). The libraries end up in `AppState.libs` and in
     `S.libs`. Then the file manager opens the folder used last.
   - disconnected: the file list shows "No Thor connected".
   - In the Saves tab the game cards are re-checked as well.

## Example 2: Sorting a game in by drop

1. **Drop.** The user drags e.g. `Zelda [0100F2C0115B6000].nsp` onto the window. Tauri calls the
   `onDragDropEvent` handler in `ui/js/transfer.js` with `type: "drop"` and the local paths.
2. **Filter.** Paths from our own drag cache are ignored (that would be our own drag coming back
   into the window). If the import dialog is open, the path goes there (`S.importTarget`).
   Otherwise `incoming(paths)` is called.
3. **Classify.** `incoming` calls `invoke("classify", { paths, cwd, lib: 0, force: false })`.
4. **Command.** `commands::classify` takes the library from `AppState.libs` and builds an
   `Incoming` for each path with `incoming_item`:
   - `library::classify` → `classify_inner` tries, in order:
     1. `guess_switch`: title ID in the name. If it ends in `000`, it's a game. `800` means an
        update, anything else DLC (both → `switch/update`).
     2. `guess_by_header`: magic bytes at the start of the file (Switch, 3DS, GameCube, Wii, PSP,
        PS1/PS2 ISO).
     3. `guess_by_extension`: which systems know the extension according to `systeminfo.txt`,
        sorted by `rank` (systems that have games first, then the `PREFERRED` list).

     For folders the biggest file is examined; for zip/7z the biggest file inside the archive.
   - `targets_for` builds the list of target folders: ★ guessed system, other candidates, current
     folder, Switch updates, all systems A–Z.
   - For archives, `incoming_item` decides whether "Extract first" is suggested.
   - If nothing game-like is among the paths, the command returns an empty list. The UI then
     copies straight into the current folder.
5. **Dialog.** `sortDialog` in `transfer.js` shows one row per file (`buildSortRow`) with the
   target preselected. With several libraries you can switch at the top. That triggers
   `classify` again, this time with `force: true`.
6. **Copy.** Clicking "Copy" builds a job `{ src, dest, unpack, flatten, remote }` per row and
   calls `invoke("enqueue", { jobs })`.
7. **Queue.** `commands::enqueue` → `files::enqueue` appends the jobs to the `SharedQueue`
   (`Arc<Mutex<Queue>>`, comparable to `std::shared_ptr` plus a mutex). If no worker thread is
   running yet, one is started.
8. **Worker.** `files::worker` takes job after job (`next_job`):
   - Normal: `push_one` → `adb push`. The target name is made FAT-safe with `unpack::safe_name`.
     Meanwhile `wait_with_polling` asks the Thor every 0.7 s via `du` how much has arrived and
     sends `transfer` events.
   - With extraction: `unpack_job` extracts into a cache folder (`unpack::extract`) and pushes the
     contents.
9. **Done.** Afterwards the worker triggers the Android media scanner and sends a final
   `transfer` event with `busy: false`. `transfer.js` shows a toast, fetches the new game counts
   with `refresh_libs` and reloads the current folder if it was affected.

## Example 3: Syncing a save

Starting point: the game is registered in `~/.config/thor-save/games.json` with a PC folder and a
Thor folder. You last played on the Thor.

1. **Show status.** When the Saves tab opens, `loadGames()` (`ui/js/saves.js`) calls
   `invoke("games")` and builds a `GameCard` per game. `card.refresh()` calls
   `invoke("game_status", { name, connected })`.
2. **Fingerprints.** `commands::game_status` → `saves::status` (`saves/sync.rs`):
   - `pc_snapshot` (`saves/snapshot.rs`) walks the PC folder and computes MD5, modification time
     and size per file.
   - `thor_snapshot` does the same on the Thor with **one** shell call (`md5sum` + `stat`). For
     private app folders (`/data/data/<package>`) the command is wrapped in `run-as <package>`
     via `adb::tcmd`.
   - `same()` compares the MD5 values. `newest()` returns the most recent modification time.
3. **Card.** `showSyncStatus` shows "● Thor is newer" and highlights the "💻 Continue on PC"
   button.
4. **Plan.** Clicking it → `startSync(card, "pc")` → `invoke("plan_sync", { name, target: "pc" })`
   → `saves::plan_sync`. Nothing is copied yet, it only evaluates: Is the source empty? Are both
   sides equal? Is the target newer (`dst_newer`)? If the target is the Thor, also: Is the
   emulator running right now (`thor_running_pkg` via `pidof`)?
5. **Confirm.** `syncConfirmText` turns that into the dialog text. If the target is newer, there
   is a red warning and "Cancel" is preselected.
6. **Sync.** After "Let's go", `runSync` calls `invoke("run_sync", { name, target, stopPkg })`
   → `saves::run_sync`:
   1. stop the emulator if needed (`stop_thor_pkg` → `am force-stop`)
   2. fresh snapshots of both sides
   3. **backup** of the target to `~/.local/share/thor-save/backups/<game>/<timestamp>-pc`
      (`transfer::backup_side`). Old backups are then deleted so that 15 remain
      (`prune_backups`).
   4. **copy** (`copy_thor_to_pc`): `transfer::pull_dir` fetches the Thor folder into a temporary
      folder (normally with `adb pull`, for private folders as a tar stream over
      `adb exec-out` + `run-as`). Then `util::mirror` makes the PC folder exactly the same, like
      `rsync --delete`.
   5. **verify:** a new snapshot of the target. If it doesn't match the source, there is an error
      with the path to the backup.

   In the other direction (`copy_pc_to_thor`), `transfer::push_dir` first pushes the folder into
   a staging folder next to the target (`<folder>.thor-save-neu`). Only at the very end does it
   swap it with the old folder. That way an interruption never leaves a half-copied save behind.
7. **End.** The UI shows "Done! Have fun on the PC 🎮" and calls `card.refresh()`. The card now
   shows "✓ Both in sync".

The same exists on the command line: `thorbro pc <game>` in `src/cli.rs` (`cli_sync`) uses
exactly the same functions `plan_sync` and `run_sync`.
