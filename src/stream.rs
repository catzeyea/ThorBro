//! Mirrors the Thor's two screens to the PC with scrcpy (port of the old `thor-stream` script).
//!
//! Each screen is a separate scrcpy process. `Streams` keeps track of the running processes so
//! the UI can show which screens are open and close them again.
//!
//! Used by `main.rs` (Tauri commands `stream_start` / `stream_stop` / `stream_status`, the CLI
//! command `thorbro stream`, and closing all windows when ThorBro exits). Uses `adb.rs` so scrcpy
//! talks to the same adb as ThorBro, and `util::which` to find scrcpy.

use std::process::Child;
use std::sync::Mutex;

/// scrcpy options for both screens: view only, 60 fps, high bitrate.
const COMMON_ARGS: &[&str] = &["--no-control", "--max-fps=60", "--video-bit-rate=16M"];

/// scrcpy options for the top screen (display 0, with audio).
const TOP_SCREEN_ARGS: [&str; 2] = ["--display-id=0", "--window-title=Thor – top"];

/// scrcpy options for the bottom screen (display 4 on the Thor, without audio so it isn't doubled).
const BOTTOM_SCREEN_ARGS: [&str; 3] = ["--display-id=4", "--no-audio", "--window-title=Thor – bottom"];

/// The running scrcpy processes, each tagged with its screen name (`"top"` / `"bottom"`).
///
/// The `Mutex` makes the list safe to use from several threads (Tauri commands may run in
/// parallel). Unlike `std::mutex` in C++, a Rust `Mutex` *owns* the data it protects: you can
/// only reach the `Vec` through `lock()`, so forgetting to lock is a compile error.
#[derive(Default)]
pub struct Streams {
    running: Mutex<Vec<(String, Child)>>,
}

/// Finds scrcpy: next to the ThorBro executable, in the bundled `tools/` folder (Windows
/// release), or in `PATH`.
fn scrcpy() -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent().unwrap();
        for dir in [exe_dir.to_path_buf(), exe_dir.join("tools")] {
            for name in ["scrcpy", "scrcpy.exe"] {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    crate::util::which("scrcpy")
}

impl Streams {
    /// Starts one scrcpy window for `screen` (`"top"`, anything else means bottom).
    ///
    /// `&self` is like a `const` member function's `this` in C++, but thanks to the `Mutex`
    /// we can still change the list inside.
    fn spawn(&self, screen: &str) -> Result<(), String> {
        // `.ok_or(...)` turns `None` into an error, then `?` returns it.
        let exe = scrcpy().ok_or("scrcpy not found – please install scrcpy (see README)")?;
        let mut args: Vec<&str> = COMMON_ARGS.to_vec();
        if screen == "top" {
            args.extend(TOP_SCREEN_ARGS);
        } else {
            args.extend(BOTTOM_SCREEN_ARGS);
        }
        let mut command = std::process::Command::new(exe);
        command.args(&args);
        // scrcpy should use the same adb as ThorBro (two different adb versions fight each other).
        command.env("ADB", crate::adb::adb_path());
        let child = command.spawn().map_err(|e| e.to_string())?;
        self.running.lock().unwrap().push((screen.to_string(), child));
        Ok(())
    }

    /// Starts the requested screen(s).
    ///
    /// `which`: `"both"`, `"top"` or `"bottom"` (the old German `"beide"` / `"oben"` / `"unten"`
    /// still work for the CLI). A screen that is already running isn't started twice.
    pub fn start(&self, which: &str) -> Result<(), String> {
        self.cleanup();
        let wanted: &[&str] = match which {
            "top" | "oben" => &["top"],
            "bottom" | "unten" => &["bottom"],
            _ => &["top", "bottom"],
        };
        for screen in wanted {
            let already_running = self.running.lock().unwrap().iter().any(|(name, _)| name == screen);
            if !already_running {
                self.spawn(screen)?;
            }
        }
        Ok(())
    }

    /// Closes all scrcpy windows.
    pub fn stop(&self) {
        // `drain(..)` removes all elements and hands them to us one by one (by value, so we
        // own each `Child` and may kill it).
        for (_, mut child) in self.running.lock().unwrap().drain(..) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Removes windows the user has already closed from the list.
    /// Returns the names of the screens that are still running.
    pub fn cleanup(&self) -> Vec<String> {
        let mut running = self.running.lock().unwrap();
        // `try_wait()` returns `Ok(None)` while the process is still alive.
        running.retain_mut(|(_, child)| matches!(child.try_wait(), Ok(None)));
        running.iter().map(|(name, _)| name.clone()).collect()
    }
}
