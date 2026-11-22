//! Command line interface: `thorbro <command>` without starting the window.
//!
//! Supports screen mirroring and the save sync (`list`, `status`, `pc`, `thor`). Setting up and
//! importing games is only possible in the app.
//!
//! Called from `main()` in `main.rs` when there are arguments. Uses `saves/`, `stream.rs`, `adb.rs`.

use crate::{adb, saves, stream};
use std::io::Write;

/// While mirroring from the CLI: how often to check whether the windows are still open.
const STREAM_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Answers accepted as "yes" (English and German).
const YES_ANSWERS: [&str; 4] = ["j", "ja", "y", "yes"];

/// Runs a CLI command. `args` are the arguments without the program name (never empty).
/// Returns the process exit code (0 = success).
pub fn run(args: &[String]) -> i32 {
    // Everything after the command word, e.g. the game name in `thorbro status zelda totk`.
    let rest = args[1..].join(" ");
    match args[0].as_str() {
        "stream" => cli_stream(args.get(1).map(|s| s.as_str()).unwrap_or("both")),
        "list" | "ls" => cli_list(),
        "status" => cli_status(&rest),
        // `target @ (...)` binds the matched value to `target`, so we know which one it was.
        target @ ("pc" | "thor") => cli_sync(target, &rest),
        _ => {
            print_help();
            0
        }
    }
}

/// Asks a yes/no question on the terminal. Empty input = `default`.
fn ask(prompt: &str, default: bool) -> bool {
    print!("{prompt} {} ", if default { "[Y/n]" } else { "[y/N]" });
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    let answer = input.trim().to_lowercase();
    if answer.is_empty() { default } else { YES_ANSWERS.contains(&answer.as_str()) }
}

/// True if `query` (already lowercase) matches the game name: as substring, ignoring spaces, or as initials
/// (e.g. `"totk"` for "Tears Of The Kingdom").
fn matches_game(name: &str, query: &str) -> bool {
    let name = name.to_lowercase();
    let initials: String = name
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|word| word.chars().next())
        .collect();
    name.contains(query) || name.replace(' ', "").contains(&query.replace(' ', "")) || initials == query
}

/// Finds exactly one configured game for `query`. Prints an error and returns `None` if there
/// is no match or more than one.
fn pick_game(query: &str) -> Option<String> {
    let games = saves::load_games();
    let lowercase_query = query.to_lowercase();
    let hits: Vec<&String> = games.keys().filter(|name| matches_game(name, &lowercase_query)).collect();
    // Slice patterns: match on the *shape* of the list (exactly one, empty, anything else).
    match hits.as_slice() {
        [one] => Some((*one).clone()),
        [] => {
            eprintln!("No game matches “{query}”.");
            None
        }
        many => {
            let names: Vec<&str> = many.iter().map(|name| name.as_str()).collect();
            eprintln!("Multiple matches: {}", names.join(", "));
            None
        }
    }
}

/// `thorbro stream [top|bottom]`: mirrors and waits until all windows are closed.
fn cli_stream(which: &str) -> i32 {
    let streams = stream::Streams::default();
    if let Err(e) = streams.start(which) {
        eprintln!("✗ {e}");
        return 1;
    }
    while !streams.cleanup().is_empty() {
        std::thread::sleep(STREAM_POLL_INTERVAL);
    }
    0
}

/// `thorbro list`: prints all games with their folders.
fn cli_list() -> i32 {
    for (name, cfg) in saves::load_games() {
        println!("{name}\n  PC:   {}\n  Thor: {}", cfg.pc.as_deref().unwrap_or("—"), cfg.thor.as_deref().unwrap_or("—"));
    }
    0
}

/// `thorbro status <game>`: compares both sides.
fn cli_status(query: &str) -> i32 {
    let Some(name) = pick_game(query) else { return 1 };
    // `unwrap()` is fine here: `pick_game` only returns names that exist in the config.
    let status = saves::status(&saves::get_game(&name).unwrap(), adb::device_status().connected);
    println!("{name}\n  PC:   {}\n  Thor: {}", status.pc.summary, status.thor.summary);
    if status.same {
        println!("✓ In sync.");
    } else if status.pc.newest > status.thor.newest {
        println!("→ PC is newer.");
    } else if status.thor.newest > status.pc.newest {
        println!("→ Thor is newer.");
    }
    0
}

/// `thorbro pc <game>` / `thorbro thor <game>`: syncs towards `target` after asking.
fn cli_sync(target: &str, query: &str) -> i32 {
    let Some(name) = pick_game(query) else { return 1 };
    let device = adb::device_status();
    if !device.connected {
        eprintln!("✗ {}", device.message);
        return 1;
    }
    let cfg = saves::get_game(&name).unwrap();
    println!("Reading both sides …");
    let plan = saves::plan_sync(&cfg, target);
    println!(
        "{name}: {} → {}\n  Source: {}\n  Target: {}",
        plan.src_name, plan.dst_name, plan.src.summary, plan.dst.summary
    );
    if plan.empty {
        eprintln!("✗ There's no save on the {}.", plan.src_name);
        return 1;
    }
    if plan.same {
        println!("✓ Both sides are already the same.");
        return 0;
    }
    if !confirm_sync(&plan) {
        println!("Cancelled.");
        return 0;
    }
    if let Some(pkg) = &plan.running_pkg {
        if !ask(&format!("{pkg} is running on the Thor. Close it?"), true) {
            return 0;
        }
    }
    match saves::run_sync(&name, &cfg, target, plan.running_pkg.as_deref()) {
        Ok(backup) => {
            if let Some(backup) = backup {
                println!("  Backup: {backup}");
            }
            println!("✓ Done, you can continue playing on the {}.", plan.dst_name);
            0
        }
        Err(e) => {
            eprintln!("✗ {e}");
            1
        }
    }
}

/// Asks whether to overwrite the target. If the target is newer, the default is "no".
fn confirm_sync(plan: &saves::Plan) -> bool {
    if plan.dst_newer {
        println!("⚠ The save on the {} is NEWER! It goes into the backup if you continue.", plan.dst_name);
        ask("Really overwrite?", false)
    } else {
        ask(&format!("Replace the {} save with the one from the {}?", plan.dst_name, plan.src_name), true)
    }
}

/// Usage text for unknown commands.
fn print_help() {
    println!(
        "ThorBro {}\n\nWithout arguments the app starts. Otherwise:\n  \
         thorbro stream [top|bottom]   mirror screens (default: both)\n  \
         thorbro list                  list configured games\n  \
         thorbro status <game>         compare saves\n  \
         thorbro pc <game>             pull the Thor's save to the PC\n  \
         thorbro thor <game>           push the PC's save to the Thor\n\n\
         Setting up and importing games is done in the app (Saves tab).",
        // `env!` reads the version from Cargo.toml at compile time.
        env!("CARGO_PKG_VERSION")
    );
}
