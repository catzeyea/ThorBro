//! ThorBro – files, games and saves between your PC and the AYN Thor.
//!
//! Entry point. With command line arguments it runs the CLI (`cli.rs`), otherwise it starts the
//! Tauri window: registers the state (`commands::AppState`), all commands the UI may call
//! (`generate_handler!`) and closes the mirroring windows when the app exits (window destroyed or
//! `RunEvent::Exit`).
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
mod pcstream;
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
    files::remove_legacy_drag_cache();
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
            commands::pins,
            commands::add_pin,
            commands::remove_pin,
            commands::pins_found,
            commands::esde_preview,
            commands::esde_import,
            commands::list_dir,
            commands::remember_dir,
            commands::move_paths,
            commands::delete_paths,
            commands::make_dir,
            commands::rename,
            commands::delete,
            commands::remote_sizes,
            commands::paths_exist,
            commands::local_sizes,
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
            commands::stream_displays,
            commands::stream_start_displays,
            commands::stream_start,
            commands::stream_stop_display,
            commands::stream_stop,
            commands::stream_status,
            commands::pc_stream_detect,
            commands::pc_stream_open_link,
            commands::pc_stream_open_web_ui,
            commands::pc_stream_thor_apps,
            commands::pc_stream_login_status,
            commands::pc_stream_save_login,
            commands::pc_stream_forget_login,
            commands::pc_stream_create_login,
            commands::pc_stream_restart_sunshine,
            commands::pc_stream_pairings,
            commands::pc_stream_submit_pin,
            commands::pc_stream_apps,
            commands::pc_stream_play,
            commands::pc_stream_profile,
            commands::pc_stream_send_profile
        ])
        .on_window_event(|window, event| {
            // Close the scrcpy windows together with ThorBro.
            if let tauri::WindowEvent::Destroyed = event {
                use tauri::Manager;
                window.app_handle().state::<AppState>().streams.stop();
            }
        })
        .build(tauri::generate_context!())
        .expect("ThorBro failed to start")
        .run(|app, event| {
            // Also close the scrcpy windows if the app exits without destroying the window first.
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                app.state::<AppState>().streams.stop();
            }
        });
}
