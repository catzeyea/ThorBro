// ThorBro – Dateien, Games und Spielstände zwischen PC und AYN Thor.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod adb;
mod files;
mod library;
mod saves;
mod stream;
mod unpack;
mod util;

use library::Library;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
struct AppState {
    libs: Mutex<Vec<Library>>,
    queue: files::SharedQueue,
    streams: stream::Streams,
}

type R<T> = Result<T, String>;

/// Blockierende Arbeit aus dem UI-Thread raushalten.
async fn bg<T: Send + 'static>(f: impl FnOnce() -> R<T> + Send + 'static) -> R<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

// --- Gerät & Orte ---------------------------------------------------------------------

#[tauri::command]
async fn device() -> R<adb::Device> {
    bg(|| Ok(adb::device_status())).await
}

#[derive(Serialize)]
struct Place {
    label: String,
    path: String,
    games: bool,
}

#[derive(Serialize)]
struct Places {
    places: Vec<Place>,
    libs: Vec<Library>,
    start: String,
}

#[tauri::command]
async fn places(state: State<'_, AppState>, restore: bool) -> R<Places> {
    let (stores, libs) = bg(|| {
        let s = adb::storages();
        let l = library::find_libraries(&s);
        Ok((s, l))
    })
    .await?;
    *state.libs.lock().unwrap() = libs.clone();
    let mut places: Vec<Place> =
        libs.iter().map(|l| Place { label: format!("Games – {}", l.label), path: l.path.clone(), games: true }).collect();
    places.extend(stores.into_iter().map(|(label, path)| Place { label, path, games: false }));
    let default = libs.first().map(|l| l.path.clone()).unwrap_or_else(|| "/sdcard".into());
    let start = if restore { files::load_last().unwrap_or(default) } else { default };
    Ok(Places { places, libs, start })
}

#[tauri::command]
async fn refresh_libs(state: State<'_, AppState>) -> R<Vec<Library>> {
    let libs = bg(|| Ok(library::find_libraries(&adb::storages()))).await?;
    *state.libs.lock().unwrap() = libs.clone();
    Ok(libs)
}

// --- Dateimanager -------------------------------------------------------------------------

#[tauri::command]
async fn list_dir(path: String) -> R<Vec<files::Entry>> {
    bg(move || {
        files::save_last(&path);
        files::listdir(&path)
    })
    .await
}

#[tauri::command]
async fn make_dir(path: String) -> R<()> {
    bg(move || files::mkdir(&path)).await
}

#[tauri::command]
async fn rename(from: String, to: String) -> R<()> {
    bg(move || files::rename(&from, &to)).await
}

#[tauri::command]
async fn delete(path: String) -> R<()> {
    bg(move || files::delete(&path)).await
}

#[tauri::command]
async fn pull(path: String) -> R<String> {
    bg(move || files::pull(&path)).await
}

#[tauri::command]
async fn drag_fetch(app: AppHandle, path: String) -> R<files::Fetched> {
    bg(move || files::drag_fetch(&app, &path)).await
}

/// Linux: der drag-Crate baut die file://-URI ohne Escaping, Leerzeichen & Co. also selbst kodieren.
#[cfg(target_os = "linux")]
fn drag_path(p: &str) -> PathBuf {
    let mut s = String::new();
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    PathBuf::from(s)
}
#[cfg(not(target_os = "linux"))]
fn drag_path(p: &str) -> PathBuf {
    PathBuf::from(p)
}

/// Echten System-Drag mit lokalen Dateien starten (Maustaste muss noch gedrückt sein).
#[tauri::command]
fn drag_start(app: AppHandle, window: tauri::WebviewWindow, paths: Vec<String>) -> R<()> {
    let items: Vec<PathBuf> = paths.iter().map(|p| drag_path(p)).collect();
    let w = window.clone();
    window
        .run_on_main_thread(move || {
            #[cfg(target_os = "linux")]
            let handle = match w.gtk_window() {
                Ok(h) => h,
                Err(e) => return eprintln!("drag: {e}"),
            };
            #[cfg(not(target_os = "linux"))]
            let handle = w.clone();
            let icon = drag::Image::Raw(include_bytes!("../icons/32x32.png").to_vec());
            let done = move |r: drag::DragResult, _| {
                let _ = app.emit("drag-done", matches!(r, drag::DragResult::Dropped));
            };
            let opts = drag::Options { mode: drag::DragMode::Copy, ..Default::default() };
            if let Err(e) = drag::start_drag(&handle, drag::DragItem::Files(items), icon, done, opts) {
                eprintln!("drag: {e}");
            }
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_pull_dir() -> R<()> {
    let d = files::pull_dir();
    let _ = std::fs::create_dir_all(&d);
    util::open_in_os(&d)
}

// --- Drop & Einsortieren ----------------------------------------------------------------------

#[derive(Serialize)]
struct Target {
    label: String,
    path: String,
}

#[derive(Serialize)]
struct Incoming {
    path: String,
    name: String,
    dir: bool,
    size: String,
    guess: library::Guess,
    targets: Vec<Target>,
    selected: usize,
    /// Ist ein Archiv (zip, 7z, rar …)
    archive: bool,
    /// Vorschlag: vor dem Kopieren entpacken
    unpack: bool,
}

fn targets_for(lib: &Library, g: &library::Guess, cwd: &str) -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();
    let mut add = |label: String, path: String| {
        if !out.iter().any(|t| t.path == path) {
            out.push(Target { label, path });
        }
    };
    if let Some(sys) = &g.system {
        let (extra, p) = if g.sub.is_empty() {
            (String::new(), format!("{}/{sys}", lib.path))
        } else {
            (format!(" / {}", g.sub), format!("{}/{sys}/{}", lib.path, g.sub))
        };
        add(format!("★ {}{extra}  ({sys}{extra})", lib.full(sys)), p);
    }
    for c in &g.candidates {
        add(format!("{}  ({c})", lib.full(c)), format!("{}/{c}", lib.path));
    }
    add(format!("Current folder  ({cwd})"), cwd.to_string());
    if lib.systems.contains_key("switch") {
        add("Nintendo Switch / update  (Updates & DLC)".into(), format!("{}/switch/update", lib.path));
    }
    let mut all: Vec<&String> = lib.systems.keys().collect();
    all.sort_by_key(|n| lib.full(n).to_lowercase());
    for s in all {
        add(format!("{}  ({s})", lib.full(s)), format!("{}/{s}", lib.path));
    }
    out
}

/// Gedroppte Dateien erkennen. Leere Liste = kein Game dabei, direkt in den aktuellen Ordner.
#[tauri::command]
async fn classify(state: State<'_, AppState>, paths: Vec<String>, cwd: String, lib: usize, force: bool) -> R<Vec<Incoming>> {
    let libs = state.libs.lock().unwrap().clone();
    bg(move || {
        let Some(lib) = libs.get(lib).or(libs.first()) else { return Ok(vec![]) };
        let items: Vec<Incoming> = paths
            .iter()
            .map(|p| {
                let pb = PathBuf::from(p);
                let guess = library::classify(&pb, lib);
                let targets = targets_for(lib, &guess, &cwd);
                let archive = unpack::is_archive(&pb);
                // Entpacken nur vorschlagen, wenn ein einzelnes Game drinsteckt, das der Emulator
                // gepackt nicht liest. Viele Dateien (z. B. Switch-Firmware voller .nca) = kein Game.
                let few = unpack::file_count(&pb).map_or(true, |n| n <= 4);
                let unpack = archive && guess.system.is_some() && !guess.packed_ok && few;
                let selected = if guess.system.is_some() { 0 } else { targets.iter().position(|t| t.path == cwd).unwrap_or(0) };
                Incoming {
                    name: pb.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or(p.clone()),
                    dir: pb.is_dir(),
                    size: util::human(files::size_of(&pb)),
                    path: p.clone(),
                    guess,
                    targets,
                    selected,
                    archive,
                    unpack,
                }
            })
            .collect();
        if !force && items.iter().all(|i| i.guess.system.is_none() && !i.archive) {
            return Ok(vec![]);
        }
        Ok(items)
    })
    .await
}

#[tauri::command]
fn enqueue(app: AppHandle, state: State<'_, AppState>, jobs: Vec<files::Job>) {
    files::enqueue(app, state.queue.clone(), jobs);
}

// --- Saves ---------------------------------------------------------------------------------------

#[tauri::command]
fn games() -> saves::Games {
    saves::load_games()
}

#[tauri::command]
async fn game_status(name: String, connected: bool) -> R<saves::Status> {
    bg(move || Ok(saves::status(&saves::get_game(&name)?, connected))).await
}

#[tauri::command]
async fn plan_sync(name: String, target: String) -> R<saves::Plan> {
    bg(move || Ok(saves::plan_sync(&saves::get_game(&name)?, &target))).await
}

#[tauri::command]
async fn run_sync(name: String, target: String, stop_pkg: Option<String>) -> R<Option<String>> {
    bg(move || saves::run_sync(&name, &saves::get_game(&name)?, &target, stop_pkg.as_deref())).await
}

#[derive(Serialize)]
struct Found {
    pc: Vec<saves::Cand>,
    thor: Vec<saves::Cand>,
    ai: saves::AiPick,
}

#[tauri::command]
async fn scan_game(app: AppHandle, name: String) -> R<Found> {
    bg(move || {
        let step = |t: &str| {
            let _ = app.emit("save-step", t.to_string());
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

#[tauri::command]
async fn peek(side: String, path: String) -> R<saves::SnapInfo> {
    bg(move || Ok(saves::info(&saves::snapshot(&side, Some(&path))))).await
}

#[tauri::command]
fn save_game(name: String, pc: Option<String>, thor: Option<String>) -> R<()> {
    let mut g = saves::load_games();
    g.insert(name, saves::GameCfg { pc, thor });
    saves::save_games(&g)
}

#[tauri::command]
fn remove_game(name: String) -> R<()> {
    let mut g = saves::load_games();
    g.remove(&name);
    saves::save_games(&g)
}

#[tauri::command]
fn open_backups(name: String) -> R<bool> {
    let p = saves::backups_root().join(saves::slug(&name));
    if !p.is_dir() {
        return Ok(false);
    }
    util::open_in_os(&p).map(|_| true)
}

#[tauri::command]
async fn scan_thor_sources(name: String) -> R<Vec<saves::Cand>> {
    bg(move || Ok(saves::scan_thor_sources(&name))).await
}

#[tauri::command]
async fn prepare_import(name: String, source: String, from_thor: bool) -> R<saves::Prepared> {
    bg(move || saves::prepare_import(&name, &source, from_thor)).await
}

#[tauri::command]
async fn check_root(name: String, content: String, root: String) -> R<saves::RootCheck> {
    bg(move || saves::check_root(&name, &content, &root)).await
}

#[tauri::command]
async fn do_import(app: AppHandle, name: String, root: String, targets: Vec<String>, merge: bool) -> R<()> {
    bg(move || {
        let step = |t: &str| {
            let _ = app.emit("save-step", t.to_string());
        };
        saves::import_save(&name, Path::new(&root), &targets, merge, &step)
    })
    .await
}

#[tauri::command]
fn cleanup_import(work: String) {
    let p = PathBuf::from(&work);
    // Nur Arbeitsordner von uns löschen
    if p.starts_with(saves::state_dir()) && p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("import-")) {
        let _ = std::fs::remove_dir_all(p);
    }
}

#[tauri::command]
fn is_dir(path: String) -> bool {
    Path::new(&path).is_dir()
}

#[tauri::command]
fn log(msg: String) {
    eprintln!("[ui] {msg}");
}

// --- Stream ----------------------------------------------------------------------------------------

#[tauri::command]
fn stream_start(state: State<'_, AppState>, which: String) -> R<Vec<String>> {
    state.streams.start(&which)?;
    Ok(state.streams.cleanup())
}

#[tauri::command]
fn stream_stop(state: State<'_, AppState>) {
    state.streams.stop();
}

#[tauri::command]
fn stream_status(state: State<'_, AppState>) -> Vec<String> {
    state.streams.cleanup()
}

// --- CLI ---------------------------------------------------------------------------------------------

fn ask(prompt: &str, default: bool) -> bool {
    use std::io::Write;
    print!("{prompt} {} ", if default { "[Y/n]" } else { "[y/N]" });
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    if std::io::stdin().read_line(&mut s).is_err() {
        return false;
    }
    let a = s.trim().to_lowercase();
    if a.is_empty() { default } else { ["j", "ja", "y", "yes"].contains(&a.as_str()) }
}

fn pick_game(query: &str) -> Option<String> {
    let g = saves::load_games();
    let q = query.to_lowercase();
    let hits: Vec<&String> = g
        .keys()
        .filter(|n| {
            let n = n.to_lowercase();
            let initials: String = n.split(|c: char| !c.is_alphanumeric()).filter_map(|w| w.chars().next()).collect();
            n.contains(&q) || n.replace(' ', "").contains(&q.replace(' ', "")) || initials == q
        })
        .collect();
    match hits.as_slice() {
        [one] => Some((*one).clone()),
        [] => {
            eprintln!("No game matches “{query}”.");
            None
        }
        many => {
            eprintln!("Multiple matches: {}", many.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
            None
        }
    }
}

fn cli(args: &[String]) -> i32 {
    let rest = args[1..].join(" ");
    match args[0].as_str() {
        "stream" => {
            let s = stream::Streams::default();
            if let Err(e) = s.start(args.get(1).map(|s| s.as_str()).unwrap_or("both")) {
                eprintln!("✗ {e}");
                return 1;
            }
            while !s.cleanup().is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            0
        }
        "list" | "ls" => {
            for (n, c) in saves::load_games() {
                println!("{n}\n  PC:   {}\n  Thor: {}", c.pc.as_deref().unwrap_or("—"), c.thor.as_deref().unwrap_or("—"));
            }
            0
        }
        "status" => {
            let Some(name) = pick_game(&rest) else { return 1 };
            let st = saves::status(&saves::get_game(&name).unwrap(), adb::device_status().connected);
            println!("{name}\n  PC:   {}\n  Thor: {}", st.pc.summary, st.thor.summary);
            if st.same {
                println!("✓ In sync.");
            } else if st.pc.newest > st.thor.newest {
                println!("→ PC is newer.");
            } else if st.thor.newest > st.pc.newest {
                println!("→ Thor is newer.");
            }
            0
        }
        t @ ("pc" | "thor") => {
            let Some(name) = pick_game(&rest) else { return 1 };
            let dev = adb::device_status();
            if !dev.connected {
                eprintln!("✗ {}", dev.message);
                return 1;
            }
            let cfg = saves::get_game(&name).unwrap();
            println!("Reading both sides …");
            let p = saves::plan_sync(&cfg, t);
            println!("{name}: {} → {}\n  Source: {}\n  Target: {}", p.src_name, p.dst_name, p.src.summary, p.dst.summary);
            if p.empty {
                eprintln!("✗ There's no save on the {}.", p.src_name);
                return 1;
            }
            if p.same {
                println!("✓ Both sides are already the same.");
                return 0;
            }
            let go = if p.dst_newer {
                println!("⚠ The save on the {} is NEWER! It goes into the backup if you continue.", p.dst_name);
                ask("Really overwrite?", false)
            } else {
                ask(&format!("Replace the {} save with the one from the {}?", p.dst_name, p.src_name), true)
            };
            if !go {
                println!("Cancelled.");
                return 0;
            }
            if let Some(pkg) = &p.running_pkg {
                if !ask(&format!("{pkg} is running on the Thor. Close it?"), true) {
                    return 0;
                }
            }
            match saves::run_sync(&name, &cfg, t, p.running_pkg.as_deref()) {
                Ok(b) => {
                    if let Some(b) = b {
                        println!("  Backup: {b}");
                    }
                    println!("✓ Done, you can continue playing on the {}.", p.dst_name);
                    0
                }
                Err(e) => {
                    eprintln!("✗ {e}");
                    1
                }
            }
        }
        _ => {
            println!(
                "ThorBro {}\n\nWithout arguments the app starts. Otherwise:\n  \
                 thorbro stream [top|bottom]   mirror screens (default: both)\n  \
                 thorbro list                  list configured games\n  \
                 thorbro status <game>         compare saves\n  \
                 thorbro pc <game>             pull the Thor's save to the PC\n  \
                 thorbro thor <game>           push the PC's save to the Thor\n\n\
                 Setting up and importing games is done in the app (Saves tab).",
                env!("CARGO_PKG_VERSION")
            );
            0
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(cli(&args));
    }
    files::drag_cleanup();
    let state = AppState { queue: Arc::new(Mutex::new(Default::default())), ..Default::default() };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            device, places, refresh_libs, list_dir, make_dir, rename, delete, pull, open_pull_dir, drag_fetch, drag_start,
            classify, enqueue, games, game_status, plan_sync, run_sync, scan_game, peek, save_game,
            remove_game, open_backups, scan_thor_sources, prepare_import, check_root, do_import,
            cleanup_import, is_dir, log, stream_start, stream_stop, stream_status
        ])
        .on_window_event(|w, e| {
            if let tauri::WindowEvent::Destroyed = e {
                use tauri::Manager;
                w.app_handle().state::<AppState>().streams.stop();
            }
        })
        .run(tauri::generate_context!())
        .expect("ThorBro failed to start");
}
