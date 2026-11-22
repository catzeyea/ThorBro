//! Everything around adb (Android Debug Bridge): finding the device, running shell commands
//! on the Thor and moving files.
//!
//! adb is called as an external program. It is searched next to the ThorBro executable, in
//! `PATH` and in the usual Android SDK folders, so it also works on Windows without setup.
//!
//! Used by `files.rs` (file manager, copy queue), `library.rs` (finding game folders),
//! `saves/` (save sync), `stream.rs` (scrcpy gets the same adb) and `main.rs` (device status).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// File name of the adb program on this OS. `cfg!(windows)` is evaluated at compile time,
/// similar to `#ifdef _WIN32` in C++, but it stays type-checked on every platform.
const ADB_EXE: &str = if cfg!(windows) { "adb.exe" } else { "adb" };

/// Windows flag for `CreateProcess` that prevents a console window from popping up.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Characters that never need quoting in a POSIX shell.
const SHELL_SAFE_CHARS: &str = "/._-+=:,@";

/// Path prefix of the "shared" per-app data folders on Android.
const ANDROID_DATA_DIR: &str = "/Android/data/";

/// Every place adb might be installed, in the order we prefer them.
fn adb_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    // `if let Ok(x) = ...` only runs the block if the call succeeded (pattern matching on `Result`).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(ADB_EXE));
            candidates.push(dir.join("tools").join(ADB_EXE)); // shipped with the Windows release
            candidates.push(dir.join("platform-tools").join(ADB_EXE));
        }
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            candidates.push(dir.join(ADB_EXE));
        }
    }
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(sdk) = std::env::var_os(var) {
            candidates.push(PathBuf::from(sdk).join("platform-tools").join(ADB_EXE));
        }
    }
    if let Some(local) = dirs::data_local_dir() {
        candidates.push(local.join("Android").join("Sdk").join("platform-tools").join(ADB_EXE));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Library/Android/sdk/platform-tools").join(ADB_EXE));
        candidates.push(home.join("Android/Sdk/platform-tools").join(ADB_EXE));
    }
    candidates
}

/// First adb candidate that exists. Falls back to the bare name `adb`, so the OS can still
/// try to find it (and report a clear "not found" error if it can't).
fn find_adb() -> PathBuf {
    adb_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(ADB_EXE))
}

/// Path to adb. Searched only once, then cached for the lifetime of the program.
///
/// `&'static PathBuf` is a reference that is valid for the whole program run (`'static` is a
/// *lifetime*: it tells the compiler how long a reference stays valid). `OnceLock` is a
/// thread-safe "initialize on first use" cell, like a function-local `static` in C++.
pub fn adb_path() -> &'static PathBuf {
    static ADB_PATH: OnceLock<PathBuf> = OnceLock::new();
    ADB_PATH.get_or_init(find_adb)
}

/// Creates a `Command` for `program` that doesn't open a console window on Windows.
///
/// `impl AsRef<OsStr>` means "any type that can be viewed as an OS string" (`&str`, `String`,
/// `PathBuf`, ...). It works like a constrained template parameter in C++20.
pub fn cmd(program: impl AsRef<std::ffi::OsStr>) -> Command {
    // `mut` is only needed on Windows; the attribute silences the warning on other systems.
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// A `Command` that runs adb.
fn adb_cmd() -> Command {
    cmd(adb_path())
}

/// Result of an adb call: success flag plus captured output as text.
pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

// `impl Out { ... }` adds methods to the struct, like member functions in a C++ class.
// They are kept separate from the data definition above.
impl Out {
    /// The most useful error text: stderr if there is any, otherwise stdout (some adb
    /// commands print their errors to stdout).
    pub fn err_text(&self) -> String {
        let text = if self.stderr.trim().is_empty() { &self.stdout } else { &self.stderr };
        text.trim().to_string()
    }
}

/// Converts the raw result of a finished process into an `Out`.
/// If the process couldn't even be started, that becomes a failed `Out` with an explanation.
fn to_out(result: std::io::Result<Output>) -> Out {
    match result {
        Ok(output) => Out {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Err(e) => Out { ok: false, stdout: String::new(), stderr: format!("couldn't start adb: {e}") },
    }
}

/// Runs adb with the given arguments and waits for it to finish.
///
/// `<S: AsRef<OsStr>>` is a generic parameter (a template): it accepts `&["a", "b"]` as well as
/// a slice of `String`s.
pub fn run<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Out {
    to_out(adb_cmd().args(args).stdin(Stdio::null()).output())
}

/// Runs one command in the Thor's shell (`adb shell <command>`).
pub fn sh(command: &str) -> Out {
    run(&["shell", command])
}

/// Sends a longer shell script via stdin to `adb shell sh`.
/// This avoids the length limit of a single `adb shell` command line.
pub fn sh_script(script: &str) -> Out {
    let spawned = adb_cmd()
        .args(["shell", "sh"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => return Out { ok: false, stdout: String::new(), stderr: e.to_string() },
    };
    // The stdin handle is dropped (= closed) at the end of this block, so `sh` sees EOF.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }
    to_out(child.wait_with_output())
}

/// Starts `adb exec-out <command>` and returns the running process.
/// Its stdout is a raw byte stream (e.g. a tar archive), unlike `adb shell` which may mangle bytes.
pub fn exec_out(command: &str) -> std::io::Result<Child> {
    adb_cmd()
        .args(["exec-out", command])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// Quotes `text` for the Thor's POSIX shell (`sh`).
///
/// Harmless strings are returned unchanged so commands stay readable in logs. Everything else is
/// wrapped in single quotes; a single quote inside becomes `'\''` (close, escaped quote, reopen).
pub fn shell_quote(text: &str) -> String {
    let is_safe = !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || SHELL_SAFE_CHARS.contains(c));
    if is_safe {
        return text.to_string();
    }
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Connection state of the Thor, sent to the UI as JSON.
///
/// `#[derive(...)]` asks the compiler to generate code for this type: `Serialize` (from the
/// serde library) generates the JSON conversion, `Clone` a deep-copy method `.clone()`.
/// In C++ you would write those by hand or rely on the implicit copy constructor.
#[derive(serde::Serialize, Clone)]
pub struct Device {
    pub connected: bool,
    pub message: String,
}

/// Asks adb whether a Thor is connected and returns a user-facing status text.
///
/// When connected, `message` is the device model name (`ro.product.model`).
/// If several devices are attached, only the first one is looked at.
pub fn device_status() -> Device {
    let output = run(&["devices"]);
    if !output.ok && output.stdout.is_empty() {
        return Device { connected: false, message: adb_missing_message().into() };
    }
    // Output looks like: "List of devices attached\n<serial>\t<state>\n..."
    let devices: Vec<(String, String)> = output
        .stdout
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once('\t'))
        .map(|(serial, state)| (serial.trim().to_string(), state.trim().to_string()))
        .collect();
    // `let ... else` : if the pattern doesn't match (no first device), run the else block,
    // which must leave the function. It's an early return that also unpacks the value.
    let Some((_, state)) = devices.first() else {
        return Device { connected: false, message: "No Thor found – is the cable plugged in and USB debugging on?".into() };
    };
    match state.as_str() {
        "device" => {
            let model = sh("getprop ro.product.model").stdout.trim().to_string();
            Device { connected: true, message: if model.is_empty() { "Thor".into() } else { model } }
        }
        "unauthorized" => Device { connected: false, message: "Confirm the USB debugging prompt on the Thor".into() },
        other => Device { connected: false, message: format!("Thor status: {other}") },
    }
}

/// Hint shown when adb can't be started at all. Windows users get the bundled `tools` folder hint.
fn adb_missing_message() -> &'static str {
    if cfg!(windows) {
        "adb not found – keep the tools folder next to ThorBro.exe or install the Android Platform-Tools"
    } else {
        "adb not found – please install adb / android-tools (see README)"
    }
}

/// Storage locations on the Thor as `(display name, path)`:
/// internal storage plus every SD card / USB stick under `/storage`.
pub fn storages() -> Vec<(String, String)> {
    let mut found = vec![("Internal storage".to_string(), "/sdcard".to_string())];
    for name in sh("ls /storage").stdout.split_whitespace() {
        // "emulated" and "self" are aliases of the internal storage.
        if name != "emulated" && name != "self" {
            found.push((format!("SD card ({name})"), format!("/storage/{name}")));
        }
    }
    found
}

/// Package name if `path` is inside an app's private data folder (`/data/data/<pkg>` or
/// `/data/user/<n>/<pkg>`). Those folders are only reachable via `run-as <pkg>`.
pub fn private_pkg(path: &str) -> Option<String> {
    static PRIVATE_DIR_RE: OnceLock<regex::Regex> = OnceLock::new();
    let regex = PRIVATE_DIR_RE.get_or_init(|| regex::Regex::new(r"^/data/(?:data|user/\d+)/([^/]+)(?:/|$)").unwrap());
    regex.captures(path).map(|captures| captures[1].to_string())
}

/// Wraps a shell command so it works for `path`: inside a private app folder it runs via
/// `run-as <pkg> sh -c '<command>'`, everywhere else it is returned unchanged.
pub fn tcmd(path: &str, command: &str) -> String {
    match private_pkg(path) {
        Some(pkg) => format!("run-as {} sh -c {}", shell_quote(&pkg), shell_quote(command)),
        None => command.to_string(),
    }
}

/// The app package a Thor path belongs to: either `.../Android/data/<pkg>/...` or a private
/// folder (see `private_pkg`). Used to find out which emulator must be closed before a sync.
pub fn thor_pkg(path: &str) -> Option<String> {
    let with_slash = format!("{path}/");
    if let Some(index) = with_slash.find(ANDROID_DATA_DIR) {
        let rest = &with_slash[index + ANDROID_DATA_DIR.len()..];
        return rest.split('/').next().filter(|name| !name.is_empty()).map(String::from);
    }
    private_pkg(path)
}

/// Last component of a Thor (POSIX) path, ignoring trailing slashes. `"/a/b/"` -> `"b"`.
///
/// The returned `&str` points into `path` (no copy). The compiler checks that it isn't used
/// after `path` is gone; this is what Rust's *borrowing* rules guarantee.
pub fn posix_basename(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// Parent folder of a Thor (POSIX) path. `"/a/b"` -> `"/a"`, `"/a"` -> `"/"`, `"a"` -> `"."`.
pub fn posix_dirname(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/",
        Some(index) => &trimmed[..index],
        None => ".",
    }
}
