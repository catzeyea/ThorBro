//! Small helpers that don't belong to a specific feature.
//!
//! - Formatting for the UI (`human` sizes, `when` timestamps).
//! - File helpers: MD5 hashes, relative paths, copying and mirroring folders
//!   (`copy_tree` / `mirror` replace the `rsync` calls of the old Python script).
//! - Process helpers: `which` (find a program), `run_with_timeout`, `open_in_os`.
//!
//! Used by almost every other module, most heavily by `saves/` and `files.rs`.

use md5::{Digest, Md5};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Buffer size used when hashing files (1 MiB).
const HASH_BUFFER_SIZE: usize = 1 << 20;

/// How often `run_with_timeout` checks whether the child process has finished.
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Placeholder shown in the UI when a timestamp is unknown.
const NO_TIME: &str = "—";

/// Formats a byte count for humans, e.g. `1536` -> `"1.5 KB"`.
///
/// Plain bytes are shown without decimals (`"512 B"`), everything else with one.
pub fn human(bytes: u64) -> String {
    let mut value = bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if value < 1024.0 {
            return if unit == "B" { format!("{bytes} B") } else { format!("{value:.1} {unit}") };
        }
        value /= 1024.0;
    }
    format!("{value:.1} TB")
}

/// Formats a Unix timestamp (seconds) as local time `"YYYY-MM-DD HH:MM"`.
///
/// Returns `"—"` for `0` / negative values (= "no timestamp") and for values chrono can't represent.
pub fn when(timestamp: f64) -> String {
    if timestamp <= 0.0 {
        return NO_TIME.into();
    }
    // `Option` is Rust's "maybe a value" type, similar to C++ `std::optional`.
    // `.map(...)` transforms the value if there is one, `.unwrap_or_else(...)` supplies a fallback.
    // The `|d| ...` syntax is a closure (a lambda, like `[](auto d) { ... }` in C++).
    chrono::DateTime::from_timestamp(timestamp as i64, 0)
        .map(|date| date.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| NO_TIME.into())
}

/// Last modification time of `path` as Unix seconds, or `0.0` if it can't be read.
///
/// Uses `symlink_metadata`, so for a symlink the link itself is inspected, not its target.
pub fn mtime_of(path: &Path) -> f64 {
    // Each step returns a `Result` (value or error, a bit like `std::expected` in C++23).
    // `.ok()` turns it into an `Option` and throws the error away, because we only care
    // whether we got a value or not.
    let modified = fs::symlink_metadata(path).and_then(|meta| meta.modified()).ok();
    let since_epoch = modified.and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
    since_epoch.map(|duration| duration.as_secs_f64()).unwrap_or(0.0)
}

/// MD5 hash of a file as a lowercase hex string. Reads in 1 MiB chunks so big files don't fill RAM.
///
/// The `?` after a call means: "if this returned an error, return that error from *this*
/// function right now". It replaces the `if (!ok) return err;` boilerplate (or a `throw`) in C++.
pub fn md5_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Md5::new();
    let mut buffer = vec![0u8; HASH_BUFFER_SIZE];
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Path of `path` relative to `root`, always with `/` as separator (same on Windows and the Thor).
///
/// If `path` isn't below `root`, the whole path is used.
pub fn rel(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.join("/")
}

/// Path as a string with `/` separators (for pattern matching, also on Windows).
pub fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// All files below `root` (recursively) as relative `/`-separated paths.
pub fn files_under(root: &Path) -> Vec<String> {
    // `filter_map(Result::ok)` silently skips entries we aren't allowed to read.
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| rel(root, entry.path()))
        .collect()
}

/// Copies the modification time of `src` onto `dst`. Errors are ignored on purpose:
/// a wrong timestamp is not worth failing a copy for.
fn copy_mtime(src_meta: &fs::Metadata, dst: &Path) {
    let mtime = filetime::FileTime::from_last_modification_time(src_meta);
    let _ = filetime::set_file_mtime(dst, mtime);
}

/// Copies one file, creating parent folders as needed, and keeps the modification time.
fn copy_file_keep_time(src: &Path, dst: &Path) -> std::io::Result<()> {
    // `if let Some(x) = ...` runs the block only if the `Option` holds a value
    // and binds it to `x` (pattern matching, roughly `if (auto x = ...; x)` in C++).
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    if let Ok(meta) = fs::metadata(src) {
        copy_mtime(&meta, dst);
    }
    Ok(())
}

/// Copies a whole folder (like Python's `shutil.copytree`) and keeps timestamps.
///
/// - `merge == false`: `dst` must not exist yet, otherwise an error is returned.
/// - `merge == true`: copy into an existing folder, overwriting files with the same name.
///
/// Errors are returned as `String` because they end up as text in the UI anyway.
pub fn copy_tree(src: &Path, dst: &Path, merge: bool) -> Result<(), String> {
    if dst.exists() && !merge {
        return Err(format!("Already exists: {}", dst.display()));
    }
    // `.map_err(...)` converts the error type (here: io::Error -> String) so `?` can return it.
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in walkdir::WalkDir::new(src).min_depth(1).into_iter().filter_map(Result::ok) {
        let target = dst.join(entry.path().strip_prefix(src).unwrap());
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else if entry.file_type().is_file() {
            copy_file_keep_time(entry.path(), &target)
                .map_err(|err| format!("{}: {err}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// True if `a` and `b` have the same size and the same MD5 hash.
fn same_file_content(a: &Path, a_size: Option<u64>, b: &Path) -> bool {
    let b_size = fs::metadata(b).map(|m| m.len()).ok();
    b.is_file() && b_size == a_size && md5_file(b).ok() == md5_file(a).ok()
}

/// Makes `dst` an exact copy of `src` (like `rsync -a --checksum --delete`).
///
/// Files whose content is already identical are not copied again (only their timestamp is
/// fixed). Everything in `dst` that doesn't exist in `src` is deleted afterwards.
pub fn mirror(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let kept_paths = mirror_copy_pass(src, dst)?;
    mirror_delete_extra(dst, &kept_paths);
    Ok(())
}

/// First half of `mirror`: copies everything from `src` to `dst`.
/// Returns the set of relative paths that exist in `src` (these must be kept in `dst`).
fn mirror_copy_pass(src: &Path, dst: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut kept_paths: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in walkdir::WalkDir::new(src).min_depth(1).into_iter().filter_map(Result::ok) {
        let relative = entry.path().strip_prefix(src).unwrap().to_path_buf();
        let target = dst.join(&relative);
        kept_paths.insert(relative);

        if entry.file_type().is_dir() {
            // A file is in the way where a folder should be: replace it.
            if target.is_file() {
                fs::remove_file(&target).map_err(|e| e.to_string())?;
            }
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if !entry.file_type().is_file() {
            continue; // symlinks and other special files are skipped
        }
        // A folder is in the way where a file should be: replace it.
        if target.is_dir() {
            fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
        }
        let src_size = entry.metadata().map(|m| m.len()).ok();
        if same_file_content(entry.path(), src_size, &target) {
            if let Ok(meta) = entry.metadata() {
                copy_mtime(&meta, &target);
            }
        } else {
            copy_file_keep_time(entry.path(), &target)
                .map_err(|err| format!("{}: {err}", entry.path().display()))?;
        }
    }
    Ok(kept_paths)
}

/// Second half of `mirror`: deletes everything in `dst` whose relative path is not in
/// `kept_paths`. The deepest entries go first so folders are empty when we reach them.
fn mirror_delete_extra(dst: &Path, kept_paths: &BTreeSet<PathBuf>) {
    let mut extra: Vec<PathBuf> = walkdir::WalkDir::new(dst)
        .min_depth(1)
        .contents_first(true)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| !kept_paths.contains(path.strip_prefix(dst).unwrap()))
        .collect();
    extra.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in extra {
        if path.is_dir() {
            let _ = fs::remove_dir_all(&path);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Finds a program like the shell `which` command does.
///
/// Searches every folder in `PATH` plus `~/.local/bin` (where e.g. the `claude` CLI often lives).
/// On Windows it also tries `.exe`, `.cmd` and `.bat`.
pub fn which(name: &str) -> Option<PathBuf> {
    let extensions: Vec<String> = if cfg!(windows) {
        vec![".exe".into(), ".cmd".into(), ".bat".into(), String::new()]
    } else {
        vec![String::new()]
    };
    let mut search_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path_var| std::env::split_paths(&path_var).collect())
        .unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        search_dirs.push(home.join(".local/bin"));
    }
    for dir in search_dirs {
        for extension in &extensions {
            let candidate = dir.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Runs `command`, writes `input` to its stdin and returns its stdout.
///
/// Returns `None` if the program can't be started, fails to report its status, or runs longer
/// than `timeout` (then it gets killed). stderr is discarded.
///
/// stdout is read on a separate thread; otherwise a chatty program could fill the pipe buffer
/// and block forever while we wait for it to exit.
pub fn run_with_timeout(mut command: Command, input: &str, timeout: Duration) -> Option<String> {
    use std::io::Write;
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    // `take()` moves the pipe out of `child` so we own it. When `stdin` goes out of scope at the
    // end of this block it is closed automatically (RAII, like a destructor in C++), which tells
    // the child that the input is complete.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let mut stdout = child.stdout.take()?;
    // `move` makes the closure take ownership of `stdout`, so the thread can use it safely.
    let reader = std::thread::spawn(move || {
        let mut output = String::new();
        let _ = stdout.read_to_string(&mut output);
        output
    });
    let start = Instant::now();
    loop {
        // `match` is like a `switch` that can also look inside values (pattern matching).
        // `Ok(Some(_))` = exited, `Ok(None)` = still running, `Err(_)` = couldn't ask.
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(PROCESS_POLL_INTERVAL),
            Err(_) => return None,
        }
    }
    reader.join().ok()
}

/// Opens a folder or file in the system's file manager (Explorer, Finder, xdg-open).
pub fn open_in_os(path: &Path) -> Result<(), String> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program).arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}
