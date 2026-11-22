//! ThorBro – files, games and saves between your PC and the AYN Thor.
//!
//! Entry point. With command line arguments it runs the CLI (`cli.rs`), otherwise it starts the
//! Tauri window: registers the state (`commands::AppState`), all commands the UI may call
//! (`generate_handler!`) and closes the mirroring windows when the app exits.
//!
//! The UI lives in `ui/` (HTML/CSS/JS); see `docs/ARCHITECTURE.md` for the overall picture.

// On Windows release builds: don't open a console window next to the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Each `mod` line pulls in `src/<name>.rs` (or `src/<name>/mod.rs`) as a module.
mod adb;
mod cli;
mod commands;
mod esde_import;
mod files;
mod library;
mod saves;
mod settings;
mod stream;
mod systems;
mod unpack;
mod util;

use commands::AppState;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(cli::run(&args));
    }
    files::drag_cleanup();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // Makes `AppState` available to every command as `State<'_, AppState>`.
        .manage(AppState::default())
        // Every function the UI may call with `invoke(...)` must be listed here.
        .invoke_handler(tauri::generate_handler![
            commands::device,
            commands::places,
            commands::refresh_libs,
            commands::add_library,
            commands::remove_library,
            commands::known_systems,
            commands::set_folder_system,
            commands::esde_preview,
            commands::esde_import,
            commands::list_dir,
            commands::remember_dir,
            commands::move_paths,
            commands::delete_paths,
            commands::make_dir,
            commands::rename,
            commands::delete,
            commands::pull,
            commands::open_pull_dir,
            commands::drag_fetch,
            commands::drag_start,
            commands::classify,
            commands::enqueue,
            commands::cancel_transfer,
            commands::games,
            commands::game_status,
            commands::plan_sync,
            commands::run_sync,
            commands::scan_game,
            commands::peek,
            commands::save_game,
            commands::remove_game,
            commands::open_backups,
            commands::scan_thor_sources,
            commands::prepare_import,
            commands::check_root,
            commands::do_import,
            commands::cleanup_import,
            commands::is_dir,
            commands::log,
            commands::stream_start,
            commands::stream_stop,
            commands::stream_status
        ])
        .on_window_event(|window, event| {
            // Close the scrcpy windows together with ThorBro.
            if let tauri::WindowEvent::Destroyed = event {
                use tauri::Manager;
                window.app_handle().state::<AppState>().streams.stop();
            }
        })
        .run(tauri::generate_context!())
        .expect("ThorBro failed to start");
}
