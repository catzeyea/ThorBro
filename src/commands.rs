//! All Tauri commands: the functions the UI (`ui/*.js`) calls with `invoke("name", {...})`.
//!
//! Each `#[tauri::command]` function is registered in `main.rs` with `generate_handler!`.
//! Tauri converts the JSON arguments from JavaScript into Rust parameters (camelCase in JS,
//! snake_case here, e.g. `stopPkg` -> `stop_pkg`) and the return value back into JSON.
//! An `Err(String)` arrives in JavaScript as a rejected promise with that text.
//!
//! The commands are thin wrappers: the real work lives in `adb.rs`, `files.rs`, `library.rs`,
//! `saves/`, `stream.rs`, `pcstream/` and `unpack.rs`.

use crate::library::{self, Library};
use crate::{adb, esde_import, files, pcstream, saves, settings, stream, systems, unpack, util};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

/// Event name for progress texts during save scans / imports (`ui/js/saves-*.js` show them in a dialog).
const SAVE_STEP_EVENT: &str = "save-step";

/// Event sent when a drag-out started by `drag_start` has ended (payload: dropped yes/no).
const DRAG_DONE_EVENT: &str = "drag-done";

/// Archives with more files than this are not suggested for extraction (probably not a game,
/// e.g. Switch firmware full of `.nca` files).
const MAX_FILES_TO_SUGGEST_UNPACK: usize = 4;

/// Where to start browsing if there is no game library.
const DEFAULT_START_PATH: &str = "/sdcard";

/// Global app state that Tauri hands to commands via `State<'_, AppState>`.
///
/// Created once in `main()` and registered with `.manage(state)`.
#[derive(Default)]
pub struct AppState {
    /// Game libraries found on the Thor (refreshed by `places` / `refresh_libs`).
    pub libs: Mutex<Vec<Library>>,
    /// Copy queue PC -> Thor.
    pub queue: files::SharedQueue,
    /// Running scrcpy windows.
    pub streams: stream::Streams,
}

/// Result type of all commands. Errors are plain text for the UI.
type CmdResult<T> = Result<T, String>;

/// Runs blocking work (adb calls, file copies) on a background thread pool and waits for it
/// without blocking the UI thread.
///
/// This is an `async fn`: calling it returns a *future* (like `std::future` / a coroutine) that
/// Tauri's async runtime drives; `.await` suspends until the result is ready.
///
/// The generic bounds mean: `T` must be sendable to another thread (`Send`) and must not borrow
/// anything temporary (`'static`); `work` is a closure called exactly once (`FnOnce`).
async fn run_blocking<T: Send + 'static>(work: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    // The outer `?` handles "the thread panicked", the inner `CmdResult` is returned as-is.
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

// --- Device & places ---------------------------------------------------------------------------

/// Is a Thor connected? Polled by the UI every few seconds.
#[tauri::command]
pub async fn device() -> CmdResult<adb::Device> {
    run_blocking(|| Ok(adb::device_status())).await
}

/// One entry of the location drop-down in the file manager.
#[derive(Serialize)]
pub struct Place {
    label: String,
    path: String,
    games: bool,
}

/// Answer of `places`.
#[derive(Serialize)]
pub struct Places {
    places: Vec<Place>,
    libs: Vec<Library>,
    /// Folder to open first.
    start: String,
}

/// Lists storages and game libraries for the location drop-down.
///
/// `restore`: start in the last opened folder instead of the main game library.
/// `State<'_, AppState>`: Tauri injects the shared state; `'_` lets the compiler pick the lifetime.
#[tauri::command]
pub async fn places(state: State<'_, AppState>, restore: bool) -> CmdResult<Places> {
    let (stores, libs) = run_blocking(|| {
        let stores = adb::storages();
        let libs = library::find_libraries(&stores, &settings::load());
        Ok((stores, libs))
    })
    .await?;
    *state.libs.lock().unwrap() = libs.clone();

    let library_places = libs.iter().map(|lib| Place { label: format!("Games – {}", lib.label), path: lib.path.clone(), games: true });
    let storage_places = stores.into_iter().map(|(label, path)| Place { label, path, games: false });
    let places: Vec<Place> = library_places.chain(storage_places).collect();

    let default_start = libs.first().map(|lib| lib.path.clone()).unwrap_or_else(|| DEFAULT_START_PATH.into());
    let start = if restore { files::load_last().unwrap_or(default_start) } else { default_start };
    Ok(Places { places, libs, start })
}

/// Re-reads the game libraries (after a copy, game counts change).
#[tauri::command]
pub async fn refresh_libs(state: State<'_, AppState>) -> CmdResult<Vec<Library>> {
    let libs = run_blocking(|| Ok(library::find_libraries(&adb::storages(), &settings::load()))).await?;
    *state.libs.lock().unwrap() = libs.clone();
    Ok(libs)
}

// --- Library settings --------------------------------------------------------------------------

/// Adds a Thor folder as game library ("Set games folder"). The UI reloads the places afterwards.
#[tauri::command]
pub fn add_library(path: String) -> CmdResult<()> {
    let path = settings::clean_path(&path)?;
    let mut config = settings::load();
    if !config.libraries.contains(&path) {
        config.libraries.push(path);
    }
    settings::save(&config)
}

/// Removes a game library that the user added (the folder itself stays untouched).
#[tauri::command]
pub fn remove_library(path: String) -> CmdResult<()> {
    let path = settings::clean_path(&path)?;
    let mut config = settings::load();
    config.libraries.retain(|library| *library != path);
    settings::save(&config)
}

/// "Import ES-DE info", step 1: what would be written into the games folder `path` (writes nothing).
#[tauri::command]
pub async fn esde_preview(path: String) -> CmdResult<Vec<esde_import::PlanItem>> {
    run_blocking(move || esde_import::preview(&path)).await
}

/// "Import ES-DE info", step 2: writes the confirmed systeminfo.txt files / renames folders.
/// The UI reloads the places afterwards, so renamed folders and new systems show up.
#[tauri::command]
pub async fn esde_import(request: esde_import::ImportRequest) -> CmdResult<Vec<esde_import::ImportResult>> {
    run_blocking(move || esde_import::apply(&request)).await
}

/// All systems the user can pick for a folder ThorBro doesn't recognize.
#[tauri::command]
pub fn known_systems() -> Vec<systems::KnownSystem> {
    systems::KNOWN.to_vec()
}

/// Sets the system of a folder by hand (`system` = None: back to automatic detection).
#[tauri::command]
pub fn set_folder_system(path: String, system: Option<String>) -> CmdResult<()> {
    let path = settings::clean_path(&path)?;
    let mut config = settings::load();
    match system {
        Some(id) if systems::by_id(&id).is_some() => {
            config.folder_systems.insert(path, id);
        }
        Some(id) => return Err(format!("Unknown system: {id}")),
        None => {
            config.folder_systems.remove(&path);
        }
    }
    settings::save(&config)
}

// --- Pinned folders ----------------------------------------------------------------------------

/// The pinned Thor folders, in pin order (stored in settings.json, no adb needed).
#[tauri::command]
pub fn pins() -> Vec<settings::Pin> {
    settings::load().pins
}

/// Pins the Thor folder `path` on the storage `storage` (root path, e.g. `/sdcard`).
/// Returns the new list. Fails with a short message when the cap (`settings::MAX_PINS`) is reached.
#[tauri::command]
pub fn add_pin(storage: String, path: String) -> CmdResult<Vec<settings::Pin>> {
    let pin = settings::Pin::new(&storage, &path)?;
    let mut config = settings::load();
    settings::add_pin(&mut config, pin)?;
    settings::save(&config)?;
    Ok(config.pins)
}

/// Unpins a folder (the folder itself stays untouched). Returns the new list.
#[tauri::command]
pub fn remove_pin(storage: String, path: String) -> CmdResult<Vec<settings::Pin>> {
    let pin = settings::Pin::new(&storage, &path)?;
    let mut config = settings::load();
    settings::remove_pin(&mut config, &pin);
    settings::save(&config)?;
    Ok(config.pins)
}

/// Which of the pinned folders `paths` still exist on the Thor (one `true`/`false` per path).
#[tauri::command]
pub async fn pins_found(paths: Vec<String>) -> CmdResult<Vec<bool>> {
    run_blocking(move || files::dirs_exist(&paths)).await
}

// --- File manager ------------------------------------------------------------------------------

/// Lists a Thor folder.
///
/// `move` in `move || ...` makes the closure take ownership of `path`, which is required
/// because it runs on another thread.
#[tauri::command]
pub async fn list_dir(path: String) -> CmdResult<Vec<files::Entry>> {
    run_blocking(move || files::listdir(&path)).await
}

/// Remembers the folder the file manager shows, for the next start. Separate from `list_dir`,
/// because the UI also lists folders in the background (cache refresh) and only it knows which
/// folder is the current one.
#[tauri::command]
pub fn remember_dir(path: String) {
    files::save_last(&path);
}

/// Moves entries into `dest` (one storage, plain `mv`). One result per entry.
#[tauri::command]
pub async fn move_paths(sources: Vec<String>, dest: String) -> CmdResult<Vec<files::ItemResult>> {
    run_blocking(move || Ok(files::move_paths(&sources, &dest))).await
}

/// Deletes entries. One result per entry.
#[tauri::command]
pub async fn delete_paths(paths: Vec<String>) -> CmdResult<Vec<files::ItemResult>> {
    run_blocking(move || Ok(files::delete_paths(&paths))).await
}

#[tauri::command]
pub async fn make_dir(path: String) -> CmdResult<()> {
    run_blocking(move || files::mkdir(&path)).await
}

#[tauri::command]
pub async fn rename(from: String, to: String) -> CmdResult<()> {
    run_blocking(move || files::rename(&from, &to)).await
}

#[tauri::command]
pub async fn delete(path: String) -> CmdResult<()> {
    run_blocking(move || files::delete(&path)).await
}

/// Copies a Thor entry to `~/Downloads/thor` ("transfer" progress events while it runs). Returns that folder.
#[tauri::command]
pub async fn pull(app: AppHandle, path: String) -> CmdResult<String> {
    run_blocking(move || files::pull(&app, &path)).await
}

/// Fetches a Thor entry into the drag cache (first half of dragging a file out).
#[tauri::command]
pub async fn drag_fetch(app: AppHandle, path: String) -> CmdResult<files::Fetched> {
    run_blocking(move || files::drag_fetch(&app, &path)).await
}

/// Linux: the `drag` crate builds the file:// URI without escaping, so spaces etc. are
/// percent-encoded here.
///
/// `#[cfg(target_os = "linux")]` compiles this version only on Linux (like `#ifdef __linux__`);
/// the version below is used everywhere else.
#[cfg(target_os = "linux")]
fn drag_path(path: &str) -> PathBuf {
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    PathBuf::from(encoded)
}
#[cfg(not(target_os = "linux"))]
fn drag_path(path: &str) -> PathBuf {
    PathBuf::from(path)
}

/// Starts a real OS drag with local files (second half of dragging out; the mouse button
/// must still be pressed). Emits `drag-done` when the drag ends.
///
/// The drag has to be started on the main (GUI) thread, hence `run_on_main_thread`.
#[tauri::command]
pub fn drag_start(app: AppHandle, window: tauri::WebviewWindow, paths: Vec<String>) -> CmdResult<()> {
    let items: Vec<PathBuf> = paths.iter().map(|path| drag_path(path)).collect();
    let window_for_drag = window.clone();
    window
        .run_on_main_thread(move || start_os_drag(app, window_for_drag, items))
        .map_err(|e| e.to_string())
}

/// Runs on the main thread: hands the files to the `drag` crate. Errors are only logged.
fn start_os_drag(app: AppHandle, window: tauri::WebviewWindow, items: Vec<PathBuf>) {
    // On Linux the drag crate needs the GTK window, elsewhere the Tauri window itself.
    #[cfg(target_os = "linux")]
    let handle = match window.gtk_window() {
        Ok(handle) => handle,
        Err(e) => return eprintln!("drag: {e}"),
    };
    #[cfg(not(target_os = "linux"))]
    let handle = window.clone();

    // `include_bytes!` embeds the file into the executable at compile time.
    let icon = drag::Image::Raw(include_bytes!("../icons/32x32.png").to_vec());
    let on_done = move |result: drag::DragResult, _| {
        let _ = app.emit(DRAG_DONE_EVENT, matches!(result, drag::DragResult::Dropped));
    };
    let options = drag::Options { mode: drag::DragMode::Copy, ..Default::default() };
    if let Err(e) = drag::start_drag(&handle, drag::DragItem::Files(items), icon, on_done, options) {
        eprintln!("drag: {e}");
    }
}

/// Opens `~/Downloads/thor` in the system file manager.
#[tauri::command]
pub fn open_pull_dir() -> CmdResult<()> {
    let dir = files::pull_dir();
    let _ = std::fs::create_dir_all(&dir);
    util::open_in_os(&dir)
}

// --- Drop & sorting ----------------------------------------------------------------------------

/// One possible destination folder in the "Where should it go?" dialog.
#[derive(Serialize)]
pub struct Target {
    label: String,
    path: String,
}

/// A dropped file with the guessed system and all possible targets.
#[derive(Serialize)]
pub struct Incoming {
    path: String,
    name: String,
    dir: bool,
    size: String,
    guess: library::Guess,
    targets: Vec<Target>,
    /// Index into `targets` that is preselected.
    selected: usize,
    /// Is an archive (zip, 7z, rar …).
    archive: bool,
    /// Suggestion: extract before copying.
    unpack: bool,
}

/// All destination folders for a dropped file, best first, without duplicates:
/// guessed system (★), other candidate systems, current folder, Switch updates, all systems A–Z.
fn targets_for(lib: &Library, guess: &library::Guess, cwd: &str) -> Vec<Target> {
    let mut targets: Vec<Target> = Vec::new();
    // A closure that captures `targets` mutably. While it exists, `targets` can only be changed
    // through it (Rust's borrowing rules prevent two simultaneous writers).
    let mut add = |label: String, path: String| {
        if !targets.iter().any(|target| target.path == path) {
            targets.push(Target { label, path });
        }
    };
    if let Some(system) = &guess.system {
        let (extra, path) = if guess.sub.is_empty() {
            (String::new(), format!("{}/{system}", lib.path))
        } else {
            (format!(" / {}", guess.sub), format!("{}/{system}/{}", lib.path, guess.sub))
        };
        add(format!("★ {}{extra}  ({system}{extra})", lib.full(system)), path);
    }
    for candidate in &guess.candidates {
        add(format!("{}  ({candidate})", lib.full(candidate)), format!("{}/{candidate}", lib.path));
    }
    add(format!("Current folder  ({cwd})"), cwd.to_string());
    if let Some(switch) = lib.folder_for("switch") {
        add(format!("Nintendo Switch / update  ({switch}/update, Updates & DLC)"), format!("{}/{switch}/update", lib.path));
    }
    let mut all_systems: Vec<&String> = lib.systems.keys().collect();
    all_systems.sort_by_key(|name| lib.full(name).to_lowercase());
    for system in all_systems {
        add(format!("{}  ({system})", lib.full(system)), format!("{}/{system}", lib.path));
    }
    targets
}

/// Examines one dropped path: guess, targets, preselection and extraction suggestion.
fn incoming_item(path: &str, lib: &Library, cwd: &str) -> Incoming {
    let path_buf = PathBuf::from(path);
    let guess = library::classify(&path_buf, lib);
    let targets = targets_for(lib, &guess, cwd);
    let archive = unpack::is_archive(&path_buf);
    // Only suggest extracting if there's a single game inside that the emulator can't read
    // packed. Many files (e.g. Switch firmware full of .nca) = not a game.
    let few_files = unpack::file_count(&path_buf).is_none_or(|count| count <= MAX_FILES_TO_SUGGEST_UNPACK);
    let unpack = archive && guess.system.is_some() && !guess.packed_ok && few_files;
    // Recognized game: the ★ target (index 0). Otherwise: the current folder.
    let selected = if guess.system.is_some() {
        0
    } else {
        targets.iter().position(|target| target.path == cwd).unwrap_or(0)
    };
    Incoming {
        name: path_buf.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or(path.to_string()),
        dir: path_buf.is_dir(),
        size: util::human(files::size_of(&path_buf)),
        path: path.to_string(),
        guess,
        targets,
        selected,
        archive,
        unpack,
    }
}

/// Examines dropped files. An empty list means "no game among them", so the UI copies
/// straight into the current folder. `force` returns the list anyway (library switch in the dialog).
/// Without any library the list is empty too; the UI checks that case itself and asks first.
///
/// `lib` is an index into the libraries; out of range falls back to the first one.
#[tauri::command]
pub async fn classify(state: State<'_, AppState>, paths: Vec<String>, cwd: String, lib: usize, force: bool) -> CmdResult<Vec<Incoming>> {
    let libs = state.libs.lock().unwrap().clone();
    run_blocking(move || {
        let Some(lib) = libs.get(lib).or(libs.first()) else { return Ok(vec![]) };
        let items: Vec<Incoming> = paths.iter().map(|path| incoming_item(path, lib, &cwd)).collect();
        let nothing_game_like = items.iter().all(|item| item.guess.system.is_none() && !item.archive);
        if !force && nothing_game_like {
            return Ok(vec![]);
        }
        Ok(items)
    })
    .await
}

/// Adds copy jobs to the queue. Progress arrives as `transfer` events.
#[tauri::command]
pub fn enqueue(app: AppHandle, state: State<'_, AppState>, jobs: Vec<files::Job>) {
    files::enqueue(app, state.queue.clone(), jobs);
}

/// Cancels the running copy job; `clear_queue` also drops all waiting jobs.
/// Returns `false` if nothing was running.
#[tauri::command]
pub fn cancel_transfer(state: State<'_, AppState>, clear_queue: bool) -> bool {
    files::cancel(&state.queue, clear_queue)
}

// --- Saves -------------------------------------------------------------------------------------

/// All configured games.
#[tauri::command]
pub fn games() -> saves::Games {
    saves::load_games()
}

/// Save state of both sides for one game card.
#[tauri::command]
pub async fn game_status(name: String, connected: bool) -> CmdResult<saves::Status> {
    run_blocking(move || Ok(saves::status(&saves::get_game(&name)?, connected))).await
}

/// What a sync towards `target` (`"pc"` / `"thor"`) would do.
#[tauri::command]
pub async fn plan_sync(name: String, target: String) -> CmdResult<saves::Plan> {
    run_blocking(move || Ok(saves::plan_sync(&saves::get_game(&name)?, &target))).await
}

/// Performs the sync. Returns the backup folder, if a backup was made.
#[tauri::command]
pub async fn run_sync(name: String, target: String, stop_pkg: Option<String>) -> CmdResult<Option<String>> {
    run_blocking(move || saves::run_sync(&name, &saves::get_game(&name)?, &target, stop_pkg.as_deref())).await
}

/// Answer of `scan_game`: candidates on both sides plus the AI suggestion.
#[derive(Serialize)]
pub struct Found {
    pc: Vec<saves::Cand>,
    thor: Vec<saves::Cand>,
    ai: saves::AiPick,
}

/// Searches PC and Thor for save folders of a new game and asks the AI to pick.
/// Progress texts are sent as `save-step` events.
#[tauri::command]
pub async fn scan_game(app: AppHandle, name: String) -> CmdResult<Found> {
    run_blocking(move || {
        let step = |text: &str| {
            let _ = app.emit(SAVE_STEP_EVENT, text.to_string());
        };
        step("Searching the PC …");
        let pc = saves::scan_pc(&name);
        step("Searching the Thor …");
        let thor = saves::scan_thor(&name);
        step(if util::which("claude").is_some() { "The AI is picking the matching folders …" } else { "Almost done …" });
        let ai = saves::ai_pick(&name, &pc, &thor);
        Ok(Found { pc, thor, ai })
    })
    .await
}

/// Quick look into a folder (`side` = `"pc"` / `"thor"`) for the review dialog.
#[tauri::command]
pub async fn peek(side: String, path: String) -> CmdResult<saves::SnapInfo> {
    run_blocking(move || Ok(saves::info(&saves::snapshot(&side, Some(&path))))).await
}

/// Adds or updates a game in games.json.
#[tauri::command]
pub fn save_game(name: String, pc: Option<String>, thor: Option<String>) -> CmdResult<()> {
    let mut games = saves::load_games();
    games.insert(name, saves::GameCfg { pc, thor });
    saves::save_games(&games)
}

/// Removes a game from games.json (the saves themselves stay untouched).
#[tauri::command]
pub fn remove_game(name: String) -> CmdResult<()> {
    let mut games = saves::load_games();
    games.remove(&name);
    saves::save_games(&games)
}

/// Opens the backup folder of a game. `false` if there are no backups yet.
#[tauri::command]
pub fn open_backups(name: String) -> CmdResult<bool> {
    let dir = saves::backups_root().join(saves::slug(&name));
    if !dir.is_dir() {
        return Ok(false);
    }
    util::open_in_os(&dir).map(|_| true)
}

/// Searches the Thor for save packages to import.
#[tauri::command]
pub async fn scan_thor_sources(name: String) -> CmdResult<Vec<saves::Cand>> {
    run_blocking(move || Ok(saves::scan_thor_sources(&name))).await
}

/// Step 1 of importing: fetch, extract, clean, suggest the save folder.
#[tauri::command]
pub async fn prepare_import(name: String, source: String, from_thor: bool) -> CmdResult<saves::Prepared> {
    run_blocking(move || saves::prepare_import(&name, &source, from_thor)).await
}

/// Rates a folder of the package that the user picked.
#[tauri::command]
pub async fn check_root(name: String, content: String, root: String) -> CmdResult<saves::RootCheck> {
    run_blocking(move || saves::check_root(&name, &content, &root)).await
}

/// Step 2 of importing: write the save to the chosen sides. Progress as `save-step` events.
#[tauri::command]
pub async fn do_import(app: AppHandle, name: String, root: String, targets: Vec<String>, merge: bool) -> CmdResult<()> {
    run_blocking(move || {
        let step = |text: &str| {
            let _ = app.emit(SAVE_STEP_EVENT, text.to_string());
        };
        saves::import_save(&name, Path::new(&root), &targets, merge, &step)
    })
    .await
}

/// Deletes an import work folder. Only folders we created (`<state dir>/import-*`) are touched,
/// so a manipulated path from the UI can't delete anything else.
#[tauri::command]
pub fn cleanup_import(work: String) {
    let path = PathBuf::from(&work);
    let is_our_import_dir = path.starts_with(saves::state_dir())
        && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("import-"));
    if is_our_import_dir {
        let _ = std::fs::remove_dir_all(path);
    }
}

#[tauri::command]
pub fn is_dir(path: String) -> bool {
    Path::new(&path).is_dir()
}

/// Writes a message from the UI to stderr (JavaScript errors end up here).
#[tauri::command]
pub fn log(msg: String) {
    eprintln!("[ui] {msg}");
}

// --- Stream ------------------------------------------------------------------------------------

/// One display in the Stream tab: what scrcpy reported, a label and the last used options.
#[derive(Serialize)]
pub struct StreamDisplay {
    #[serde(flatten)]
    display: stream::Display,
    label: String,
    options: stream::StreamOptions,
}

/// Asks scrcpy which displays it can mirror (`scrcpy --list-displays`, takes about a second).
#[tauri::command]
pub async fn stream_displays(state: State<'_, AppState>) -> CmdResult<Vec<StreamDisplay>> {
    let displays = run_blocking(stream::list_displays).await?;
    let ids: Vec<u32> = displays.iter().map(|display| display.id).collect();
    state.streams.set_detected(ids.clone());
    let saved = settings::load().stream;
    Ok(displays
        .into_iter()
        .map(|display| StreamDisplay {
            label: stream::display_label(display.id, &ids),
            options: saved.get(&display.id).copied().unwrap_or_default(),
            display,
        })
        .collect())
}

/// One entry of `stream_start_displays`: a display id and the options to start it with.
#[derive(serde::Deserialize)]
pub struct StreamRequest {
    id: u32,
    options: stream::StreamOptions,
}

/// Starts one scrcpy window per requested display and remembers the options per display in
/// `settings.json`. A display that fails to start shows up in `failures` of the result, the
/// others keep running.
#[tauri::command]
pub fn stream_start_displays(state: State<'_, AppState>, displays: Vec<StreamRequest>) -> CmdResult<stream::StreamStatus> {
    let mut requests = Vec::with_capacity(displays.len());
    for request in displays {
        requests.push((request.id, request.options.validate()?));
    }
    let mut config = settings::load();
    for (id, options) in &requests {
        config.stream.insert(*id, *options);
    }
    // Not being able to remember the options is no reason to refuse mirroring.
    if let Err(e) = settings::save(&config) {
        eprintln!("couldn't save the stream options: {e}");
    }
    state.streams.start_many(&requests)
}

/// Quick start with the fixed display ids (`which` = `"both"`, `"top"`, `"bottom"`) and default
/// options. If a screen fails, it is listed in `failures` of the result.
#[tauri::command]
pub fn stream_start(state: State<'_, AppState>, which: String) -> CmdResult<stream::StreamStatus> {
    // The error text is part of the status (failures), so it isn't returned twice.
    let _ = state.streams.start(&which);
    Ok(state.streams.status())
}

/// Closes the window of one display.
#[tauri::command]
pub fn stream_stop_display(state: State<'_, AppState>, display: u32) -> stream::StreamStatus {
    state.streams.stop_display(display);
    state.streams.status()
}

/// Closes all mirroring windows.
#[tauri::command]
pub fn stream_stop(state: State<'_, AppState>) {
    state.streams.stop();
}

/// Which displays are mirrored right now and which streams failed.
#[tauri::command]
pub fn stream_status(state: State<'_, AppState>) -> stream::StreamStatus {
    state.streams.status()
}

// --- Stream PC games (Sunshine on the PC, Moonlight on the Thor) -------------------------------

/// Sunshine status (service, `/serverinfo`) and the PC's LAN / Tailscale addresses. Read-only.
#[tauri::command]
pub async fn pc_stream_detect() -> CmdResult<pcstream::PcStatus> {
    run_blocking(|| Ok(pcstream::detect())).await
}

/// Opens one of the official pages from `pcstream::LINKS` (by id) in the browser.
#[tauri::command]
pub fn pc_stream_open_link(id: String) -> CmdResult<()> {
    let url = pcstream::link_url(&id).ok_or_else(|| format!("Unknown link: {id}"))?;
    pcstream::open_url(url)
}

/// Opens Sunshine's own web UI (https://localhost:<admin port>) in the browser.
#[tauri::command]
pub fn pc_stream_open_web_ui() -> CmdResult<()> {
    let port = pcstream::sunshine::admin_port(pcstream::sunshine::configured_port());
    pcstream::open_url(&format!("https://localhost:{port}"))
}

/// Is the Thor connected, and are Moonlight and Tailscale installed on it? Read-only (adb).
#[tauri::command]
pub async fn pc_stream_thor_apps() -> CmdResult<pcstream::thor::ThorApps> {
    run_blocking(|| Ok(pcstream::thor::thor_apps())).await
}

/// Admin API port of the local Sunshine (from sunshine.conf).
fn sunshine_admin_port() -> u16 {
    pcstream::sunshine::admin_port(pcstream::sunshine::configured_port())
}

/// Is a Sunshine login stored, and does it work? Never returns the password.
#[tauri::command]
pub async fn pc_stream_login_status() -> CmdResult<pcstream::admin::LoginStatus> {
    run_blocking(|| Ok(pcstream::admin::login_status(sunshine_admin_port()))).await
}

/// Tests an existing Sunshine login and stores it in the OS keychain if it works.
#[tauri::command]
pub async fn pc_stream_save_login(username: String, password: String) -> CmdResult<pcstream::admin::LoginStatus> {
    run_blocking(move || pcstream::admin::save_login(sunshine_admin_port(), &pcstream::admin::Login { username, password })).await
}

/// Removes the stored Sunshine login from the keychain.
#[tauri::command]
pub async fn pc_stream_forget_login() -> CmdResult<()> {
    run_blocking(pcstream::admin::forget_login).await
}

/// Creates a new Sunshine web UI login with `sunshine --creds` (replaces an existing one) and
/// stores it in the keychain. Only called after the user confirmed it in the UI.
#[tauri::command]
pub async fn pc_stream_create_login(username: String, password: String) -> CmdResult<String> {
    run_blocking(move || pcstream::admin::create_login(&pcstream::admin::Login { username, password })).await
}

/// Restarts the Sunshine user service (Linux only). Ends a running stream.
#[tauri::command]
pub async fn pc_stream_restart_sunshine() -> CmdResult<()> {
    run_blocking(pcstream::admin::restart_service).await
}

/// Moonlight clients waiting for a PIN (`GET /api/pin`); `supported: false` on old Sunshine.
#[tauri::command]
pub async fn pc_stream_pairings() -> CmdResult<pcstream::admin::Pairings> {
    run_blocking(|| pcstream::admin::pending_pairings(sunshine_admin_port())).await
}

/// Sends the PIN Moonlight shows on the Thor (`POST /api/pin`). Waits until Moonlight finished
/// the handshake; returns whether pairing worked. `pairing_id` is None for old Sunshine versions.
#[tauri::command]
pub async fn pc_stream_submit_pin(pairing_id: Option<String>, pin: String, name: String) -> CmdResult<bool> {
    run_blocking(move || pcstream::admin::submit_pin(sunshine_admin_port(), pairing_id.as_deref(), &pin, &name)).await
}

/// Sunshine's app names (`GET /api/apps`).
#[tauri::command]
pub async fn pc_stream_apps() -> CmdResult<Vec<String>> {
    run_blocking(|| pcstream::admin::app_names(sunshine_admin_port())).await
}

/// "Play on Thor": starts the Sunshine app `app_name` in Moonlight on the Thor (adb, Moonlight's
/// `ShortcutTrampoline`). `display`: experimental, a display id from the screen detection.
#[tauri::command]
pub async fn pc_stream_play(app_name: String, display: Option<u32>) -> CmdResult<pcstream::thor::PlayResult> {
    run_blocking(move || pcstream::thor::play(&app_name, display)).await
}

/// The stream profile JSON for the ThorBro app on the Thor (`docs/STREAM-PROFILE.md`). No credentials.
#[tauri::command]
pub async fn pc_stream_profile() -> CmdResult<pcstream::ProfileAnswer> {
    run_blocking(pcstream::stream_profile).await
}

/// "Send to Thor app": builds the stream profile and hands it to the ThorBro app on the Thor with
/// `am start ... --es profile_b64 <base64>` (adb). Only says that Android got it; whether the app
/// accepted it is shown in the app.
#[tauri::command]
pub async fn pc_stream_send_profile() -> CmdResult<pcstream::thor::SendProfileResult> {
    run_blocking(pcstream::thor::send_profile).await
}
