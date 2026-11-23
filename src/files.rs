//! File manager for the Thor, the copy queue PC -> Thor, and dragging files out Thor -> PC.
//!
//! - File manager: list, create, rename, move, delete, pull (all via `adb shell` / `adb pull`).
//!   Moving and deleting several entries is one adb call per batch (`move_paths`, `delete_paths`).
//! - Copy queue: jobs from the UI are processed one by one on a background thread. Besides
//!   PC -> Thor uploads it also copies (or moves) inside the Thor, see `thor_copy_one`. Progress is
//!   sent to the UI as `transfer` events (see `Progress`). The running job can be cancelled
//!   (see `cancel`).
//! - Drag out: a file is first fetched into a local cache, then `commands.rs` starts a real OS drag.
//!
//! Called from the Tauri commands in `commands.rs`. Uses `adb.rs` for all Thor access and
//! `unpack.rs` to extract archives before uploading them.

use crate::adb::{self, sh, shell_quote};
use crate::unpack;
use crate::util::human;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Child, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Name of the Tauri event that carries copy progress to the UI (`ui/js/transfer.js` listens to it).
const TRANSFER_EVENT: &str = "transfer";

/// Paths with fewer slashes than this (e.g. `/sdcard`) are refused by `delete`.
const MIN_DELETE_DEPTH: usize = 2;

/// While pushing: how long to sleep between checks, and how often to measure progress.
const PUSH_SLEEP: Duration = Duration::from_millis(150);
const PUSH_POLL_INTERVAL: Duration = Duration::from_millis(700);

/// While fetching for drag-out: same as above, a bit faster because local size is cheap to read.
const FETCH_SLEEP: Duration = Duration::from_millis(100);
const FETCH_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Minimum elapsed time used for speed calculations (avoids dividing by ~0 right at the start).
const MIN_ELAPSED_SECS: f64 = 0.1;

/// Cached drag-out files older than this are deleted on start-up.
const DRAG_CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// The media scanner is triggered for at most this many new files per destination folder.
const MEDIA_SCAN_LIMIT: u32 = 2000;

/// One entry of a folder listing on the Thor. Sent to the UI as JSON.
#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    /// Last modification time in seconds since 1970 (0 if unknown).
    pub mtime: i64,
}

/// Lists a folder on the Thor: folders first, then files, each sorted case-insensitively.
///
/// Hidden files are included. If the folder can't be opened (missing, no permission, adb
/// error), an error with the shell's message is returned instead of an empty list.
pub fn listdir(path: &str) -> Result<Vec<Entry>, String> {
    // `stat` prints "<type>|<size>|<mtime>|<name>" for every entry, e.g.
    // "directory|4096|1700000000|Download".
    // The globs: `*` normal names, `.[!.]*` hidden names, `..?*` names starting with "..".
    // A glob that matches nothing stays literal and makes stat fail, hence `2>/dev/null` and
    // `exit 0`: only a failed `cd` (or adb itself) makes the command fail.
    let output = sh(&format!(
        "cd {} || exit 1; stat -c '%F|%s|%Y|%n' -- * .[!.]* ..?* 2>/dev/null; exit 0",
        shell_quote(path)
    ));
    if !output.ok {
        return Err(output.err_text());
    }
    let mut entries: Vec<Entry> = output.stdout.lines().filter_map(parse_stat_line).collect();
    // `!a.dir` is `false` for folders, and `false < true`, so folders sort first.
    entries.sort_by_key(|entry| (!entry.dir, entry.name.to_lowercase()));
    Ok(entries)
}

/// Parses one `"<type>|<size>|<mtime>|<name>"` line from `listdir`. `None` for malformed lines.
fn parse_stat_line(line: &str) -> Option<Entry> {
    // `splitn(4, ..)` stops after the third `|`, so names containing `|` stay intact.
    let mut fields = line.splitn(4, '|');
    let (kind, size, mtime, name) = (fields.next()?, fields.next()?, fields.next()?, fields.next()?);
    if name.is_empty() {
        return None;
    }
    Some(Entry {
        name: name.to_string(),
        dir: kind.contains("directory"),
        size: size.parse().unwrap_or(0),
        mtime: mtime.parse().unwrap_or(0),
    })
}

/// Turns a failed adb call into an error with the most useful text.
fn check(output: adb::Out) -> Result<(), String> {
    if output.ok { Ok(()) } else { Err(output.err_text()) }
}

/// Creates a folder (and missing parents) on the Thor.
pub fn mkdir(path: &str) -> Result<(), String> {
    check(sh(&format!("mkdir -p {}", shell_quote(path))))
}

/// Renames / moves a file or folder on the Thor.
///
/// A rename that only changes upper/lower case (`Wii` -> `wii`) is done in two steps via a
/// temporary name. Android's shared storage (`/sdcard`, SD cards) is case-insensitive, so for
/// `mv Wii wii` the target "already exists" and is the source itself; mv then tries to move the
/// folder into itself and fails with `mv: bad '…/Wii': Invalid argument`. (This is the most
/// likely explanation of a colleague's report; it is not verified on a device yet.)
pub fn rename(from: &str, to: &str) -> Result<(), String> {
    if !differs_only_by_case(from, to) {
        return move_path(from, to);
    }
    let temp = format!("{}{CASE_RENAME_SUFFIX}", from.trim_end_matches('/'));
    move_path(from, &temp)?;
    if let Err(e) = move_path(&temp, to) {
        // Roll back, so the entry doesn't stay behind under the temporary name.
        return match move_path(&temp, from) {
            Ok(()) => Err(e),
            Err(rollback) => Err(format!("{e} (and moving it back failed, it is now called {temp}: {rollback})")),
        };
    }
    Ok(())
}

/// Suffix for the temporary name of a case-only rename (see `rename`).
const CASE_RENAME_SUFFIX: &str = ".thorbro-rename";

/// One `mv` on the Thor.
fn move_path(from: &str, to: &str) -> Result<(), String> {
    check(sh(&format!("mv -- {} {}", shell_quote(from), shell_quote(to))))
}

/// True if the two paths are different, but equal when upper/lower case is ignored.
fn differs_only_by_case(from: &str, to: &str) -> bool {
    from != to && from.to_lowercase() == to.to_lowercase()
}

/// Deletes a file or folder on the Thor (recursively, no recycle bin).
/// Refuses paths directly below `/` as a safety net against deleting a whole storage.
pub fn delete(path: &str) -> Result<(), String> {
    if path.trim_end_matches('/').matches('/').count() < MIN_DELETE_DEPTH {
        return Err("I'd rather not delete that (too high up in the file system).".into());
    }
    check(sh(&format!("rm -rf -- {}", shell_quote(path))))
}

// --- Batch move / delete -----------------------------------------------------------------------
//
// Many entries in one adb call: each entry becomes a small shell snippet that prints
// `<index>|ok`, `<index>|exists` or `<index>|err|<message>`. A failing entry doesn't stop the
// others, and the UI gets one result per entry (so it can roll back exactly the failed ones).

/// Outcome of one entry of `move_paths` / `delete_paths`. `error: None` means it worked.
#[derive(Serialize, Debug, PartialEq)]
pub struct ItemResult {
    pub path: String,
    pub error: Option<String>,
}

/// What to do for one entry of a batch.
#[derive(Debug, PartialEq)]
enum Step {
    /// Run `command` on the Thor; with `target`, fail instead if `target` already exists.
    Run { command: String, target: Option<String> },
    /// Nothing to do on the Thor; `None` = counts as success, `Some` = this error.
    Done(Option<String>),
}

/// Parent folder without trailing slash; the root stays `/`.
fn clean_dir(dir: &str) -> &str {
    let trimmed = dir.trim_end_matches('/');
    if trimmed.is_empty() { "/" } else { trimmed }
}

/// `dir` + `/` + `name`, without a double slash for the root.
fn join_remote(dir: &str, name: &str) -> String {
    let dir = clean_dir(dir);
    if dir == "/" { format!("/{name}") } else { format!("{dir}/{name}") }
}

/// True if `dir` is `src` itself or lies inside it (a folder can't be moved/copied into itself).
fn is_inside(dir: &str, src: &str) -> bool {
    dir == src || dir.starts_with(&format!("{src}/"))
}

/// How `src` is moved into the folder `dest_dir`.
fn plan_move(src: &str, dest_dir: &str) -> Step {
    let src = src.trim_end_matches('/');
    let dest_dir = clean_dir(dest_dir);
    let name = adb::posix_basename(src);
    if src.is_empty() || name.is_empty() {
        return Step::Done(Some("Invalid path".into()));
    }
    if adb::posix_dirname(src) == dest_dir {
        return Step::Done(None); // already there
    }
    if is_inside(dest_dir, src) {
        return Step::Done(Some(format!("{name}: can't move a folder into itself")));
    }
    let target = join_remote(dest_dir, name);
    Step::Run { command: format!("mv -- {} {}", shell_quote(src), shell_quote(&target)), target: Some(target) }
}

/// How `path` is deleted (with the same safety net as `delete`).
fn plan_delete(path: &str) -> Step {
    if path.trim_end_matches('/').matches('/').count() < MIN_DELETE_DEPTH {
        return Step::Done(Some("I'd rather not delete that (too high up in the file system).".into()));
    }
    Step::Run { command: format!("rm -rf -- {}", shell_quote(path)), target: None }
}

/// The shell snippet for step `index` (see the section comment for its output).
fn step_script(index: usize, command: &str, target: Option<&str>) -> String {
    let run = format!("if o=$({command} 2>&1); then echo '{index}|ok'; else echo \"{index}|err|$o\" | head -n 1; fi");
    match target {
        Some(target) => format!("if [ -e {} ]; then echo '{index}|exists'; else {run}; fi", shell_quote(target)),
        None => run,
    }
}

/// Reads the answers of a batch script: one `Option<Result>` per step (`None` = no answer).
fn parse_batch_output(stdout: &str, count: usize) -> Vec<Option<Result<(), String>>> {
    let mut answers = vec![None; count];
    for line in stdout.lines() {
        let mut fields = line.splitn(3, '|');
        let (Some(index), Some(status)) = (fields.next(), fields.next()) else { continue };
        let Some(slot) = index.trim().parse::<usize>().ok().and_then(|i| answers.get_mut(i)) else { continue };
        *slot = Some(match status {
            "ok" => Ok(()),
            "exists" => Err("already exists there".to_string()),
            _ => Err(fields.next().unwrap_or("").trim().to_string()),
        });
    }
    answers
}

/// Runs the planned steps in one adb call and returns one result per path.
fn run_batch(paths: &[String], steps: Vec<Step>) -> Vec<ItemResult> {
    let script: String = steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| match step {
            Step::Run { command, target } => Some(step_script(index, command, target.as_deref()) + "\n"),
            Step::Done(_) => None,
        })
        .collect();
    let output = if script.is_empty() { None } else { Some(adb::sh_script(&script)) };
    let answers = output.as_ref().map(|out| parse_batch_output(&out.stdout, steps.len())).unwrap_or_default();
    paths
        .iter()
        .zip(steps)
        .enumerate()
        .map(|(index, (path, step))| {
            let name = adb::posix_basename(path);
            let error = match step {
                Step::Done(error) => error,
                Step::Run { .. } => match answers.get(index).cloned().flatten() {
                    Some(Ok(())) => None,
                    Some(Err(e)) if e.is_empty() => Some(format!("{name}: failed")),
                    Some(Err(e)) => Some(format!("{name}: {e}")),
                    None => {
                        let why = output.as_ref().map(|out| out.err_text()).filter(|t| !t.is_empty());
                        Some(format!("{name}: {}", why.unwrap_or_else(|| "no answer from the Thor".into())))
                    }
                },
            };
            ItemResult { path: path.clone(), error }
        })
        .collect()
}

/// Moves entries into the folder `dest_dir` on the Thor (one `mv` each, no overwriting).
/// Inside one storage this is instant; the UI only uses it there (across storages it queues a
/// copy job with progress instead, see `thor_copy_one`).
pub fn move_paths(sources: &[String], dest_dir: &str) -> Vec<ItemResult> {
    let steps = sources.iter().map(|src| plan_move(src, dest_dir)).collect();
    run_batch(sources, steps)
}

/// Deletes entries on the Thor (recursively, no recycle bin).
pub fn delete_paths(paths: &[String]) -> Vec<ItemResult> {
    let steps = paths.iter().map(|path| plan_delete(path)).collect();
    run_batch(paths, steps)
}

/// Local folder for files copied from the Thor: `~/Downloads/thor`.
pub fn pull_dir() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Downloads"))
        .join("thor")
}

/// Copies a file or folder from the Thor into `pull_dir()` ("Copy to PC"). Returns that folder's
/// path. Sends "transfer" progress events while copying (not cancellable, no final event: the
/// caller gets the result directly).
pub fn pull(app: &AppHandle, src: &str) -> Result<String, String> {
    let dest = pull_dir();
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let src = src.trim_end_matches('/');
    let name = adb::posix_basename(src).to_string();
    let total = remote_size(src).max(1);
    let report = |text: String, detail: String, fraction: f64| emit_progress(app, busy_progress(text, detail, fraction, false));
    report(format!("Copying {name} to the PC ({})", human(total)), format!("to {}", dest.display()), 0.0);
    let mut child = spawn_adb(&["pull", src, &dest.to_string_lossy()]).map_err(|e| format!("{name}: {e}"))?;

    let target = dest.join(&name);
    let started = Instant::now();
    wait_with_polling(&mut child, FETCH_SLEEP, FETCH_POLL_INTERVAL, || false, || {
        let done = size_of(&target).min(total);
        let (detail, fraction) = transfer_detail(done, total, started);
        report(format!("Copying {name} to the PC"), detail, fraction);
    });

    let output = child.wait_with_output().map_err(|e| format!("{name}: {e}"))?;
    if !output.status.success() {
        return Err(transfer_error(&name, &output));
    }
    Ok(dest.to_string_lossy().into_owned())
}

// --- Last opened folder ------------------------------------------------------------------------

/// File that remembers the last opened Thor folder between app starts.
fn last_path_file() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("thorBro").join("last_path")
}

/// The last opened folder, if one was saved.
pub fn load_last() -> Option<String> {
    let content = std::fs::read_to_string(last_path_file()).ok()?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

/// Remembers `path` as the last opened folder. Errors are ignored (it's only a convenience).
pub fn save_last(path: &str) {
    let file = last_path_file();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, path);
}

// --- Pinned folders ----------------------------------------------------------------------------

/// Shell script that prints one line per path: `1` if it is a folder on the Thor, else `0`.
fn dirs_exist_script(paths: &[String]) -> String {
    paths.iter().map(|path| format!("[ -d {} ] && echo 1 || echo 0\n", shell_quote(path))).collect()
}

/// Reads the answer of `dirs_exist_script`. `None` if it doesn't have one line per path
/// (adb failed halfway, Thor unplugged).
fn parse_dirs_exist(output: &str, count: usize) -> Option<Vec<bool>> {
    let found: Vec<bool> = output.lines().map(str::trim).filter(|line| !line.is_empty()).map(|line| line == "1").collect();
    (found.len() == count).then_some(found)
}

/// Which of `paths` are existing folders on the Thor (for pinned folders, see `settings::Pin`).
/// One adb call for all of them. Fails if the Thor can't be asked.
pub fn dirs_exist(paths: &[String]) -> Result<Vec<bool>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let output = adb::sh_script(&dirs_exist_script(paths));
    if !output.ok {
        return Err(output.err_text());
    }
    parse_dirs_exist(&output.stdout, paths.len()).ok_or_else(|| "Couldn't check the pinned folders on the Thor".to_string())
}

// --- Copying PC -> Thor ------------------------------------------------------------------------

/// Size of a local file, or the sum of all files in a local folder.
pub fn size_of(path: &Path) -> u64 {
    if path.is_file() {
        return path.metadata().map(|meta| meta.len()).unwrap_or(0);
    }
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.metadata().map(|meta| meta.len()).unwrap_or(0))
        .sum()
}

/// Progress update for the UI (payload of the `transfer` event).
#[derive(Serialize, Clone)]
pub struct Progress {
    /// `true` while working, `false` for the final "done" message.
    pub busy: bool,
    pub text: String,
    pub detail: String,
    /// 0.0 ..= 1.0; 0 while busy means "unknown" (the UI shows an animated bar).
    pub fraction: f64,
    /// Destination folders of the finished batch (so the UI can reload them).
    pub dests: Vec<String>,
    pub error: Option<String>,
    /// The UI may offer "Cancel" (only for copy queue jobs, not for drag-out fetches).
    pub cancellable: bool,
    /// A job was cancelled (on the event right after the cancel, and on the final summary).
    pub cancelled: bool,
    /// Copy queue only: position of the running job in this batch (1-based) and the size of the
    /// batch so far (it grows when more jobs are queued meanwhile). Both 0 for other transfers.
    /// The UI shows them as "file X of Y".
    pub index: u32,
    pub count: u32,
}

/// One copy job from the UI.
///
/// - `unpack`: extract the archive first and upload its contents.
/// - `flatten`: skip a single wrapper folder inside the archive (for games).
/// - `remote`: `src` is on the Thor (it is fetched, extracted on the PC and pushed back).
/// - `thor_copy`: `src` and `dest` are both on the Thor; copied with `cp -r` on the device.
/// - `move_after`: with `thor_copy`, delete `src` after a successful copy (a move across storages).
///
/// `Deserialize` generates the code that builds this struct from the JSON sent by `ui/js/transfer.js`.
/// `#[serde(default)]` means: if the field is missing in the JSON, use `false`.
#[derive(Deserialize, Clone)]
pub struct Job {
    pub src: String,
    pub dest: String,
    #[serde(default)]
    pub unpack: bool,
    #[serde(default)]
    pub flatten: bool,
    #[serde(default)]
    pub remote: bool,
    #[serde(default)]
    pub thor_copy: bool,
    #[serde(default)]
    pub move_after: bool,
}

/// Waiting jobs plus a flag whether a worker thread is currently running.
#[derive(Default)]
pub struct Queue {
    jobs: VecDeque<Job>,
    busy: bool,
    /// Set by `cancel`, read by the worker while it pushes. Reset when the next job starts.
    cancel: bool,
}

/// Why a copy job didn't finish.
enum JobError {
    Failed(String),
    /// Cancelled by the user. The text says what happened to the partial copy (may be empty).
    Cancelled(String),
}

// Lets `?` turn a plain `String` error into `JobError::Failed`.
impl From<String> for JobError {
    fn from(error: String) -> Self {
        JobError::Failed(error)
    }
}

/// The queue as shared between the Tauri state and the worker thread.
///
/// `Arc` = atomically reference-counted pointer (like `std::shared_ptr`), `Mutex` = lock that
/// owns the data. Together: "shared, thread-safe, mutable".
pub type SharedQueue = Arc<Mutex<Queue>>;

/// Adds jobs to the queue and starts a worker thread if none is running yet.
pub fn enqueue(app: AppHandle, queue: SharedQueue, jobs: Vec<Job>) {
    // The block limits how long the lock is held: the guard is dropped (= unlocked) at `}`.
    let start_worker = {
        let mut locked_queue = queue.lock().unwrap();
        locked_queue.jobs.extend(jobs);
        if locked_queue.busy {
            false
        } else {
            locked_queue.busy = true;
            true
        }
    };
    if start_worker {
        // `move` hands ownership of `app` and `queue` to the new thread.
        std::thread::spawn(move || worker(app, queue));
    }
}

/// Asks the worker to stop the running job. `clear_queue` also drops all waiting jobs.
/// Returns `false` if no worker is running (nothing to cancel).
pub fn cancel(queue: &SharedQueue, clear_queue: bool) -> bool {
    let mut locked_queue = queue.lock().unwrap();
    if !locked_queue.busy {
        return false;
    }
    locked_queue.cancel = true;
    if clear_queue {
        locked_queue.jobs.clear();
    }
    true
}

fn is_cancelled(queue: &SharedQueue) -> bool {
    queue.lock().unwrap().cancel
}

/// Position of the running copy queue job in its batch and the batch size so far (see
/// `Progress::index`). Only the worker thread writes it, and there is at most one worker.
static QUEUE_POSITION: Mutex<(u32, u32)> = Mutex::new((0, 0));

/// Sends a progress event to the UI. A failed emit (e.g. window closed) is ignored.
fn emit_progress(app: &AppHandle, progress: Progress) {
    let _ = app.emit(TRANSFER_EVENT, progress);
}

/// Sends a "still working" progress event for a copy queue job (can be cancelled).
fn report_busy(app: &AppHandle, text: String, detail: String, fraction: f64) {
    let (index, count) = *QUEUE_POSITION.lock().unwrap();
    emit_progress(app, Progress { index, count, ..busy_progress(text, detail, fraction, true) });
}

fn busy_progress(text: String, detail: String, fraction: f64, cancellable: bool) -> Progress {
    Progress { busy: true, text, detail, fraction, dests: vec![], error: None, cancellable, cancelled: false, index: 0, count: 0 }
}

/// Sends the "this job was cancelled" event. The worker may continue with the next job.
fn report_cancelled(app: &AppHandle, text: String, detail: String) {
    emit_progress(app, Progress { cancelled: true, ..busy_progress(text, detail, 0.0, false) });
}

/// Sends the final "done" / "failed" / "cancelled" event.
fn report_finished(app: &AppHandle, text: String, dests: Vec<String>, error: Option<String>, cancelled: bool) {
    let detail = String::new();
    emit_progress(app, Progress { busy: false, text, detail, fraction: 1.0, dests, error, cancellable: false, cancelled, index: 0, count: 0 });
}

/// Does a file/folder exist on the Thor?
fn remote_exists(path: &str) -> bool {
    sh(&format!("test -e {}", shell_quote(path))).ok
}

/// Size of a file/folder on the Thor in bytes (via `du`, so rounded to KiB). `0` if unknown.
fn remote_size(path: &str) -> u64 {
    let output = sh(&format!("du -sk {} 2>/dev/null | cut -f1", shell_quote(path)));
    output.stdout.trim().parse::<u64>().unwrap_or(0) * 1024
}

/// File name of a local path as `String` (empty if there is none).
fn local_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Starts adb with piped output (so we can read errors later) without waiting for it.
fn spawn_adb(args: &[&str]) -> std::io::Result<Child> {
    adb::cmd(adb::adb_path())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// Waits for `child` to exit. Every `poll_interval` it calls `on_poll` (to measure and report
/// progress), sleeping `sleep` between exit checks.
///
/// If `is_cancelled` returns true, the child is killed and reaped and `true` is returned. The
/// progress polling (`du` on the Thor) runs on this same thread, so once this function returns
/// no further poll can happen.
///
/// `impl FnMut()` accepts any closure that may modify what it captured, like a template
/// parameter `F` with `operator()` in C++.
fn wait_with_polling(
    child: &mut Child,
    sleep: Duration,
    poll_interval: Duration,
    is_cancelled: impl Fn() -> bool,
    mut on_poll: impl FnMut(),
) -> bool {
    let mut last_poll = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return false,
            Ok(None) => {}
        }
        if is_cancelled() {
            // adb is started directly (no shell or cmd.exe in between), so this ends adb itself:
            // TerminateProcess on Windows, SIGKILL on Linux/macOS. `wait` reaps it and, on
            // Windows, guarantees adb no longer holds the source files open before the temp
            // copy from `push_source` is deleted.
            let _ = child.kill();
            let _ = child.wait();
            return true;
        }
        std::thread::sleep(sleep);
        if last_poll.elapsed() >= poll_interval {
            last_poll = Instant::now();
            on_poll();
        }
    }
}

/// Builds the progress detail line `"<done> of <total> · <speed>/s · about m:ss min left"`
/// and the progress fraction.
fn transfer_detail(done: u64, total: u64, started: Instant) -> (String, f64) {
    let elapsed_secs = started.elapsed().as_secs_f64().max(MIN_ELAPSED_SECS);
    let speed = done as f64 / elapsed_secs;
    let eta_secs = if speed > 0.0 { ((total - done) as f64 / speed) as u64 } else { 0 };
    let detail = format!(
        "{} of {} · {}/s · about {}:{:02} min left",
        human(done),
        human(total),
        human(speed as u64),
        eta_secs / 60,
        eta_secs % 60
    );
    (detail, done as f64 / total as f64)
}

/// Error text for a failed adb transfer: `"<name>: <last line of its output>"`.
fn transfer_error(name: &str, output: &Output) -> String {
    let all_output = String::from_utf8_lossy(&output.stderr).to_string() + &String::from_utf8_lossy(&output.stdout);
    format!("{name}: {}", all_output.trim().lines().last().unwrap_or("?"))
}

/// What to hand to `adb push`. Usually just `src`.
///
/// adb can only upload a *folder* under a new name by naming the target folder, so if the
/// folder name isn't FAT-safe we create a temporary symlink (a copy on Windows) with the safe
/// name and push that instead. The returned `TempDir` must be kept alive until the push is done;
/// it deletes itself when dropped.
fn push_source(src: &Path, name: &str, target_name: &str) -> Result<(PathBuf, Option<tempfile::TempDir>), String> {
    if !(src.is_dir() && target_name != name) {
        return Ok((src.to_path_buf(), None));
    }
    let temp = unpack::work_dir()?;
    let link = temp.path().join(target_name);
    #[cfg(unix)]
    std::os::unix::fs::symlink(src, &link).map_err(|e| e.to_string())?;
    #[cfg(not(unix))]
    copy_dir(src, &link)?;
    Ok((link, Some(temp)))
}

/// Pushes one file/folder via `adb push` to `<dest>/<name>` and reports progress.
///
/// The target name is made FAT-safe (no `:` etc.), otherwise the SD card refuses it.
/// `more` is appended to the detail line (e.g. `" · 3 more waiting"`).
/// On cancel the partial copy is removed, but only if the target didn't exist before.
fn push_one(app: &AppHandle, queue: &SharedQueue, src: &Path, dest: &str, more: &str) -> Result<(), JobError> {
    if is_cancelled(queue) {
        return Err(JobError::Cancelled(String::new()));
    }
    let name = local_name(src);
    let target_name = unpack::safe_name(&name);
    let total = size_of(src).max(1);
    let dest_trimmed = dest.trim_end_matches('/');
    let target = format!("{dest_trimmed}/{target_name}");
    report_busy(app, format!("Copying {target_name} ({})", human(total)), format!("to {dest}{more}"), 0.0);
    let _ = sh(&format!("mkdir -p {}", shell_quote(dest)));
    let existed_before = remote_exists(&target);

    // A folder is pushed *into* dest/, a file directly to its target path.
    let push_to = if src.is_dir() { format!("{dest_trimmed}/") } else { target.clone() };
    // `_temp_dir` keeps the temporary symlink folder alive until the end of this function.
    // (A name starting with `_` tells the compiler "unused on purpose", but unlike plain `_`
    // the value is *not* dropped immediately.)
    let (push_src, _temp_dir) = push_source(src, &name, &target_name)?;
    let mut child = spawn_adb(&["push", &push_src.to_string_lossy(), &push_to]).map_err(|e| format!("{name}: {e}"))?;

    // Progress: regularly check how much already arrived on the Thor.
    let started = Instant::now();
    let cancelled = wait_with_polling(&mut child, PUSH_SLEEP, PUSH_POLL_INTERVAL, || is_cancelled(queue), || {
        let done = remote_size(&target).min(total);
        let (detail, fraction) = transfer_detail(done, total, started);
        report_busy(app, format!("Copying {target_name}"), format!("{detail}{more}"), fraction);
    });
    if cancelled {
        return Err(JobError::Cancelled(clean_up_partial(&target, &target_name, existed_before)));
    }

    match child.wait_with_output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(JobError::Failed(transfer_error(&name, &output))),
        Err(e) => Err(JobError::Failed(format!("{name}: {e}"))),
    }
}

/// Removes a half-pushed file/folder after a cancel and says what happened.
///
/// If the target existed before the push, it is left alone: adb merges a folder into an existing
/// one (and overwrites files in place), so deleting it would also delete what was there before.
fn clean_up_partial(target: &str, target_name: &str, existed_before: bool) -> String {
    if existed_before {
        return format!("{target_name} existed before and was left as it is – check it, it may be incomplete");
    }
    match delete(target) {
        Ok(()) => "Partial copy removed".into(),
        Err(e) => format!("Couldn't remove the partial copy {target}: {e}"),
    }
}

/// Recursive folder copy, only needed on Windows (instead of a symlink in `push_source`).
#[cfg(not(unix))]
fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    for entry in walkdir::WalkDir::new(src).into_iter().filter_map(Result::ok) {
        let relative = entry.path().strip_prefix(src).unwrap();
        let target = dst.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// For `remote` jobs: fetches the archive from the Thor into `work_dir` first.
/// For local jobs: returns the local path unchanged.
fn local_archive(app: &AppHandle, job: &Job, work_dir: &Path, more: &str) -> Result<PathBuf, String> {
    if !job.remote {
        return Ok(PathBuf::from(&job.src));
    }
    let name = job.src.rsplit('/').next().unwrap_or("archiv").to_string();
    report_busy(app, format!("Fetching {name} from the Thor"), format!("to extract it{more}"), 0.0);
    let output = adb::run(&["pull", &job.src, &work_dir.to_string_lossy()]);
    if !output.ok {
        return Err(format!("{name}: {}", output.err_text()));
    }
    Ok(work_dir.join(&name))
}

/// Extracts `archive` into `out` and reports progress as `"Extracting <name>"`.
fn extract_with_progress(app: &AppHandle, archive: &Path, out: &Path, name: &str, more: &str) -> Result<(), String> {
    report_busy(app, format!("Extracting {name}"), format!("on the PC{more}"), 0.0);
    let started = Instant::now();
    unpack::extract(archive, out, &mut |done, total| {
        let speed = done as f64 / started.elapsed().as_secs_f64().max(MIN_ELAPSED_SECS);
        let (detail, fraction) = if total > 0 {
            let detail = format!("{} of {} · {}/s{more}", human(done), human(total), human(speed as u64));
            (detail, done as f64 / total as f64)
        } else {
            (format!("{} · {}/s{more}", human(done), human(speed as u64)), 0.0)
        };
        report_busy(app, format!("Extracting {name}"), detail, fraction);
    })
    .map_err(|e| format!("{name}: extraction failed: {e}"))
}

/// Extracts an archive (fetching it from the Thor first if it's remote) and uploads its
/// contents.
///
/// Returns how many entries were uploaded *and* the result. They are separate because an upload
/// can fail halfway: the entries pushed before the error still count as copied.
///
/// A cancel during fetching or extracting takes effect when the first upload would start
/// (those steps can't be interrupted halfway).
fn unpack_job(app: &AppHandle, queue: &SharedQueue, job: &Job, more: &str) -> (u32, Result<(), JobError>) {
    // `work` is a `TempDir`: it deletes the folder with everything in it when it is dropped at
    // the end of this function, so it must stay alive until all uploads are done.
    let (work, items) = match extract_job(app, job, more) {
        Ok(extracted) => extracted,
        Err(e) => return (0, Err(JobError::Failed(e))),
    };
    let mut uploaded = 0;
    for item in &items {
        if let Err(e) = push_one(app, queue, item, &job.dest, more) {
            return (uploaded, Err(e));
        }
        uploaded += 1;
    }
    drop(work);
    (uploaded, Ok(()))
}

/// First half of `unpack_job`: fetch (if remote) and extract. Returns the work folder and the
/// entries to upload.
fn extract_job(app: &AppHandle, job: &Job, more: &str) -> Result<(tempfile::TempDir, Vec<PathBuf>), String> {
    let work = unpack::work_dir()?;
    let archive = local_archive(app, job, work.path(), more)?;
    let name = local_name(&archive);
    let extracted = work.path().join("inhalt");
    extract_with_progress(app, &archive, &extracted, &name, more)?;
    if job.remote {
        let _ = std::fs::remove_file(&archive); // free disk space early
    }
    let items = unpack::top_entries(&extracted, job.flatten);
    if items.is_empty() {
        return Err(format!("{name}: archive is empty"));
    }
    Ok((work, items))
}

// --- Copying inside the Thor ------------------------------------------------------------------

/// PID file of the running Thor-side `cp`, so a cancel can stop it on the device.
const THOR_COPY_PID_FILE: &str = "/data/local/tmp/thorbro-copy.pid";

/// Most "name (copy N)" variants tried before giving up.
const MAX_COPY_NAMES: u32 = 99;

/// Name for the n-th duplicate of `name` in the same folder:
/// `a.txt` -> `a (copy).txt`, `a (copy 2).txt`, …; names without extension (or dot files) get the
/// suffix at the end.
fn copy_name(name: &str, n: u32) -> String {
    let suffix = if n <= 1 { " (copy)".to_string() } else { format!(" (copy {n})") };
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{}{suffix}{}", &name[..dot], &name[dot..]),
        _ => format!("{name}{suffix}"),
    }
}

/// First `copy_name` that doesn't exist in `dir` yet.
fn free_copy_name(dir: &str, name: &str) -> Result<String, String> {
    (1..=MAX_COPY_NAMES)
        .map(|n| copy_name(name, n))
        .find(|candidate| !remote_exists(&join_remote(dir, candidate)))
        .ok_or_else(|| format!("{name}: no free name for the copy"))
}

/// Stops the `cp` started by `thor_copy_one` on the Thor and waits (up to 3 s) until it is gone,
/// so the partial copy can be removed afterwards. Killing the local adb process alone doesn't
/// reliably stop the command on the device.
fn kill_thor_copy() {
    let _ = sh(&format!(
        "p=$(cat {f} 2>/dev/null); rm -f {f}; if [ -n \"$p\" ]; then kill $p 2>/dev/null; i=0; \
         while kill -0 $p 2>/dev/null && [ $i -lt 30 ]; do sleep 0.1; i=$((i+1)); done; fi",
        f = THOR_COPY_PID_FILE
    ));
}

/// Shell command that copies `src` to `target` on the Thor and leaves its PID in
/// `THOR_COPY_PID_FILE`. `$$` is the PID of the shell on the Thor; `exec` makes `cp` keep it.
fn thor_copy_command(src: &str, target: &str) -> String {
    format!("echo $$ > {THOR_COPY_PID_FILE}; exec cp -r -- {} {}", shell_quote(src), shell_quote(target))
}

/// Copies (or, with `move_after`, moves) `job.src` into the folder `job.dest`, both on the Thor,
/// with progress and cancel like an upload.
///
/// Pasting into the folder the source is in makes a "name (copy)". Otherwise an existing target
/// is never overwritten. On cancel the partial copy is removed; the source is only deleted after
/// the copy finished without error.
fn thor_copy_one(app: &AppHandle, queue: &SharedQueue, job: &Job, more: &str) -> Result<(), JobError> {
    if is_cancelled(queue) {
        return Err(JobError::Cancelled(String::new()));
    }
    let src = job.src.trim_end_matches('/');
    let dest = clean_dir(&job.dest);
    let name = adb::posix_basename(src).to_string();
    if is_inside(dest, src) {
        return Err(JobError::Failed(format!("{name}: can't copy a folder into itself")));
    }
    if !remote_exists(src) {
        return Err(JobError::Failed(format!("{name} doesn't exist on the Thor (anymore).")));
    }
    let same_folder = adb::posix_dirname(src) == dest;
    let target_name = if same_folder && !job.move_after { free_copy_name(dest, &name)? } else { name.clone() };
    let target = join_remote(dest, &target_name);
    if remote_exists(&target) {
        return Err(JobError::Failed(format!("{target_name} already exists in {dest}")));
    }

    let verb = if job.move_after { "Moving" } else { "Copying" };
    let total = remote_size(src).max(1);
    report_busy(app, format!("{verb} {target_name} ({})", human(total)), format!("to {dest}{more}"), 0.0);
    let _ = sh(&format!("rm -f {THOR_COPY_PID_FILE}"));
    let mut child = spawn_adb(&["shell", &thor_copy_command(src, &target)]).map_err(|e| format!("{name}: {e}"))?;

    let started = Instant::now();
    let cancelled = wait_with_polling(&mut child, PUSH_SLEEP, PUSH_POLL_INTERVAL, || is_cancelled(queue), || {
        let done = remote_size(&target).min(total);
        let (detail, fraction) = transfer_detail(done, total, started);
        report_busy(app, format!("{verb} {target_name}"), format!("{detail}{more}"), fraction);
    });
    if cancelled {
        kill_thor_copy();
        return Err(JobError::Cancelled(clean_up_partial(&target, &target_name, false)));
    }
    match child.wait_with_output() {
        Ok(output) if output.status.success() => {}
        Ok(output) => return Err(JobError::Failed(transfer_error(&name, &output))),
        Err(e) => return Err(JobError::Failed(format!("{name}: {e}"))),
    }
    if job.move_after {
        delete(src).map_err(|e| format!("{name} was copied to {dest}, but the original couldn't be removed: {e}"))?;
    }
    Ok(())
}

/// Takes the next job from the queue. Returns `(job, jobs still waiting)`.
/// If the queue is empty, the busy flag is cleared in the same lock, so a concurrent
/// `enqueue` either sees the worker still running or starts a new one, never neither.
fn next_job(queue: &SharedQueue) -> (Option<Job>, usize) {
    let mut locked_queue = queue.lock().unwrap();
    // A cancel only applies to the job that was running when it was requested.
    locked_queue.cancel = false;
    let job = locked_queue.jobs.pop_front();
    if job.is_none() {
        locked_queue.busy = false;
    }
    (job, locked_queue.jobs.len())
}

/// Background thread: processes jobs until the queue is empty, then sends a summary.
fn worker(app: AppHandle, queue: SharedQueue) {
    let (mut copied, mut failed, mut cancelled) = (0u32, 0u32, 0u32);
    let mut dests: BTreeSet<String> = BTreeSet::new();
    let mut last_error = None;
    let mut started = 0u32;
    loop {
        let (job, waiting) = next_job(&queue);
        let Some(job) = job else { break };
        started += 1;
        *QUEUE_POSITION.lock().unwrap() = (started, started + waiting as u32);
        dests.insert(job.dest.clone());
        let more = if waiting > 0 { format!(" · {waiting} more waiting") } else { String::new() };
        let (count, result) = if job.thor_copy {
            match thor_copy_one(&app, &queue, &job, &more) {
                Ok(()) => (1, Ok(())),
                Err(e) => (0, Err(e)),
            }
        } else if job.unpack {
            unpack_job(&app, &queue, &job, &more)
        } else {
            match push_one(&app, &queue, Path::new(&job.src), &job.dest, &more) {
                Ok(()) => (1, Ok(())),
                Err(e) => (0, Err(e)),
            }
        };
        copied += count;
        match result {
            Ok(()) => {}
            Err(JobError::Failed(e)) => {
                failed += 1;
                last_error = Some(e);
            }
            Err(JobError::Cancelled(note)) => {
                cancelled += 1;
                // `src` is a Thor path for remote jobs, a local (maybe Windows) path otherwise.
                let name = if job.remote || job.thor_copy { adb::posix_basename(&job.src).to_string() } else { local_name(Path::new(&job.src)) };
                report_cancelled(&app, format!("Cancelled {name}"), note);
            }
        }
    }
    *QUEUE_POSITION.lock().unwrap() = (0, 0);
    trigger_media_scan(&dests);
    let text = summary_text(copied, failed, cancelled, &dests);
    report_finished(&app, text, dests.into_iter().collect(), last_error, cancelled > 0);
}

/// Asks Android's media scanner to index new files, so pictures/music show up in apps right away.
/// Only files newer than this shell's start (`-newer /proc/self`) are announced.
fn trigger_media_scan(dests: &BTreeSet<String>) {
    for dest in dests {
        let _ = sh(&format!(
            "find {} -type f -newer /proc/self 2>/dev/null | head -n {MEDIA_SCAN_LIMIT} | while read -r f; do \
             am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d \"file://$f\" >/dev/null; done",
            shell_quote(dest)
        ));
    }
}

/// `"Done: 3 copied to /sdcard/ROMs/snes, 1 failed, 1 cancelled"`.
fn summary_text(copied: u32, failed: u32, cancelled: u32, dests: &BTreeSet<String>) -> String {
    let destination = if dests.len() == 1 {
        dests.iter().next().cloned().unwrap_or_default()
    } else {
        format!("{} folders", dests.len())
    };
    let mut text = format!("Done: {copied} copied to {destination}");
    if failed > 0 {
        text += &format!(", {failed} failed");
    }
    if cancelled > 0 {
        text += &format!(", {cancelled} cancelled");
    }
    text
}

// --- Dragging out Thor -> PC -------------------------------------------------------------------
//
// To drag a file out of the window, it must exist locally. It is fetched into the cache first
// (~/.cache/thorbro/ziehen/<hash>/<name>), then `commands.rs` starts a real OS drag with that file.
// Already fetched files are reused as long as they haven't changed on the Thor.

/// Cache folder for drag-out files.
pub fn drag_root() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("thorbro").join("ziehen")
}

/// True if the entry was last modified more than `max_age` ago (or its age is unknown).
fn is_older_than(entry: &std::fs::DirEntry, max_age: Duration) -> bool {
    let age = entry.metadata().and_then(|meta| meta.modified()).ok().and_then(|time| time.elapsed().ok());
    age.is_none_or(|age| age > max_age)
}

/// Deletes old entries (older than a day) from the drag cache. Called once at start-up.
pub fn drag_cleanup() {
    let Ok(read_dir) = std::fs::read_dir(drag_root()) else { return };
    for entry in read_dir.filter_map(Result::ok) {
        if is_older_than(&entry, DRAG_CACHE_MAX_AGE) {
            let path = entry.path();
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

/// Fingerprint of a file/folder on the Thor: modification time + size, e.g. `"1700000000|512"`.
/// Empty if the path doesn't exist.
fn remote_stamp(path: &str) -> String {
    let output = sh(&format!(
        "stat -c '%Y' -- {p} 2>/dev/null; du -sk -- {p} 2>/dev/null | cut -f1",
        p = shell_quote(path)
    ));
    output.stdout.split_whitespace().collect::<Vec<_>>().join("|")
}

/// Result of `drag_fetch`, sent to the UI.
#[derive(Serialize)]
pub struct Fetched {
    pub local: String,
    /// `true` = had to be fetched from the Thor (took a while), `false` = was already cached.
    pub fetched: bool,
}

/// Where one Thor path lives in the drag cache.
struct CacheSlot {
    /// `<root>/<hash>`: the finished download.
    dir: PathBuf,
    /// `<root>/<hash>.part`: download in progress (renamed to `dir` when complete).
    part: PathBuf,
    /// `<root>/<hash>.stamp`: the `remote_stamp` of the cached version.
    stamp_file: PathBuf,
    /// `<dir>/<name>`: the local file/folder handed to the OS drag.
    local: PathBuf,
}

/// Cache paths for a Thor path. The hash keeps different Thor folders with equal names apart.
fn cache_slot(src: &str, name: &str) -> CacheSlot {
    use md5::{Digest, Md5};
    let key = format!("{:x}", Md5::digest(src.as_bytes()));
    let root = drag_root();
    let dir = root.join(&key);
    CacheSlot {
        local: dir.join(name),
        part: root.join(format!("{key}.part")),
        stamp_file: root.join(format!("{key}.stamp")),
        dir,
    }
}

/// Fetches a file/folder from the Thor into the drag cache (with progress) and returns the
/// local path. A cached copy is reused if the Thor version still has the same fingerprint.
pub fn drag_fetch(app: &AppHandle, src: &str) -> Result<Fetched, String> {
    let src = src.trim_end_matches('/');
    let name = src.rsplit('/').next().filter(|n| !n.is_empty()).ok_or("Invalid path")?.to_string();
    let slot = cache_slot(src, &name);
    let stamp = remote_stamp(src);
    if stamp.is_empty() {
        return Err(format!("{name} doesn't exist on the Thor (anymore)."));
    }
    let cached_stamp = std::fs::read_to_string(&slot.stamp_file).ok();
    if slot.local.exists() && cached_stamp.as_deref() == Some(stamp.as_str()) {
        // Refresh the timestamp so `drag_cleanup` doesn't throw it away right away.
        let _ = filetime::set_file_mtime(&slot.dir, filetime::FileTime::now());
        return Ok(Fetched { local: slot.local.to_string_lossy().into_owned(), fetched: false });
    }

    let _ = std::fs::remove_dir_all(&slot.part);
    let _ = std::fs::remove_dir_all(&slot.dir);
    std::fs::create_dir_all(&slot.part).map_err(|e| e.to_string())?;
    pull_with_progress(app, src, &name, &slot.part)?;

    std::fs::rename(&slot.part, &slot.dir).map_err(|e| e.to_string())?;
    let _ = std::fs::write(&slot.stamp_file, &stamp);
    report_finished(app, format!("{name} is ready to drag out"), vec![], None, false);
    Ok(Fetched { local: slot.local.to_string_lossy().into_owned(), fetched: true })
}

/// `adb pull src part/` with progress events. On failure the partial download is removed and a
/// "couldn't be fetched" event is sent.
fn pull_with_progress(app: &AppHandle, src: &str, name: &str, part: &Path) -> Result<(), String> {
    let total = remote_size(src).max(1);
    let report = |text: String, detail: String, fraction: f64| emit_progress(app, busy_progress(text, detail, fraction, false));
    report(format!("Fetching {name} from the Thor ({})", human(total)), "to drag out".into(), 0.0);
    let mut child = spawn_adb(&["pull", src, &part.to_string_lossy()]).map_err(|e| format!("{name}: {e}"))?;

    let started = Instant::now();
    wait_with_polling(&mut child, FETCH_SLEEP, FETCH_POLL_INTERVAL, || false, || {
        let done = size_of(part).min(total);
        let (detail, fraction) = transfer_detail(done, total, started);
        report(format!("Fetching {name} from the Thor"), format!("{detail} · then drag it out"), fraction);
    });

    let output = child.wait_with_output().map_err(|e| format!("{name}: {e}"))?;
    if !output.status.success() {
        let error = transfer_error(name, &output);
        let _ = std::fs::remove_dir_all(part);
        report_finished(app, format!("{name} couldn't be fetched"), vec![], Some(error.clone()), false);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_folder_check_script_and_answer() {
        let paths = vec!["/sdcard/Pictures".to_string(), "/sdcard/it's here".to_string()];
        assert_eq!(dirs_exist_script(&paths), "[ -d /sdcard/Pictures ] && echo 1 || echo 0\n[ -d '/sdcard/it'\\''s here' ] && echo 1 || echo 0\n");
        assert_eq!(parse_dirs_exist("1\r\n0\r\n", 2), Some(vec![true, false]));
        assert_eq!(parse_dirs_exist("1\n", 2), None, "a short answer is not taken as 'not found'");
        assert_eq!(dirs_exist(&[]), Ok(vec![]), "no pins: no adb call");
    }

    #[test]
    fn case_only_rename_is_detected() {
        assert!(differs_only_by_case("/sdcard/lisu/Roms/Wii", "/sdcard/lisu/Roms/wii"));
        assert!(differs_only_by_case("/sdcard/a/GBA", "/sdcard/a/gba"));
        assert!(!differs_only_by_case("/sdcard/a/Wii", "/sdcard/a/Wii"));
        assert!(!differs_only_by_case("/sdcard/a/Wii", "/sdcard/a/WiiU"));
        assert!(!differs_only_by_case("/sdcard/a/Wii", "/sdcard/b/wii2"));
    }

    #[test]
    fn stat_lines_are_parsed() {
        let entry = parse_stat_line("directory|4096|1700000000|Download").unwrap();
        assert_eq!((entry.name.as_str(), entry.dir, entry.size, entry.mtime), ("Download", true, 4096, 1700000000));
        let entry = parse_stat_line("regular file|12|5|a|b.txt").unwrap();
        assert_eq!((entry.name.as_str(), entry.dir, entry.mtime), ("a|b.txt", false, 5));
        assert!(parse_stat_line("regular file|12|5").is_none());
        assert!(parse_stat_line("regular file|12|5|").is_none());
    }

    #[test]
    fn moves_are_planned() {
        assert_eq!(
            plan_move("/sdcard/a/x.iso", "/sdcard/b/"),
            Step::Run { command: "mv -- /sdcard/a/x.iso /sdcard/b/x.iso".into(), target: Some("/sdcard/b/x.iso".into()) }
        );
        assert_eq!(plan_move("/sdcard/a/x.iso", "/sdcard/a"), Step::Done(None));
        assert!(matches!(plan_move("/sdcard/a", "/sdcard/a/sub"), Step::Done(Some(_))));
        assert!(matches!(plan_move("/sdcard/a", "/sdcard/a"), Step::Done(Some(_))));
        // "/sdcard/ab" is not inside "/sdcard/a".
        assert!(matches!(plan_move("/sdcard/a", "/sdcard/ab"), Step::Run { .. }));
        assert!(matches!(plan_delete("/sdcard"), Step::Done(Some(_))));
        assert!(matches!(plan_delete("/sdcard/a"), Step::Run { .. }));
    }

    #[test]
    fn batch_output_is_parsed() {
        let answers = parse_batch_output("0|ok\n2|err|mv: bad: Permission denied\n3|exists\njunk\n9|ok\n", 4);
        assert_eq!(answers[0], Some(Ok(())));
        assert_eq!(answers[1], None);
        assert_eq!(answers[2], Some(Err("mv: bad: Permission denied".into())));
        assert_eq!(answers[3], Some(Err("already exists there".into())));
    }

    #[test]
    fn steps_without_shell_work_skip_adb() {
        let paths = vec!["/sdcard/a/x".to_string(), "/x".to_string()];
        let results = run_batch(&paths, vec![Step::Done(None), Step::Done(Some("nope".into()))]);
        assert_eq!(results[0].error, None);
        assert_eq!(results[1].error.as_deref(), Some("nope"));
    }

    #[test]
    fn copy_names() {
        assert_eq!(copy_name("game.iso", 1), "game (copy).iso");
        assert_eq!(copy_name("game.iso", 3), "game (copy 3).iso");
        assert_eq!(copy_name("Roms", 1), "Roms (copy)");
        assert_eq!(copy_name(".nomedia", 2), ".nomedia (copy 2)");
    }

    #[test]
    fn remote_paths_are_joined() {
        assert_eq!(join_remote("/", "a"), "/a");
        assert_eq!(join_remote("/sdcard/", "a"), "/sdcard/a");
        assert!(is_inside("/sdcard/a/b", "/sdcard/a"));
        assert!(!is_inside("/sdcard/ab", "/sdcard/a"));
    }
}

/// Checks the shell side against a real device. Run with a Thor attached:
/// `cargo test device_ -- --ignored --test-threads=1`
///
/// Everything happens inside fresh `thorbro-selftest-<time>` folders on internal storage and (if
/// present) the first SD card, which are deleted at the end. Nothing else is touched.
#[cfg(test)]
mod device_tests {
    use super::*;

    /// A fresh sandbox folder under `base`, deleted when dropped.
    struct Sandbox(String);
    impl Sandbox {
        fn new(base: &str) -> Self {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
            let path = format!("{base}/thorbro-selftest-{stamp}");
            mkdir(&path).expect("create sandbox");
            Sandbox(path)
        }
        fn path(&self, name: &str) -> String {
            join_remote(&self.0, name)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            assert!(self.0.contains("/thorbro-selftest-"));
            let _ = delete(&self.0);
        }
    }

    fn touch(path: &str, bytes: u32) {
        let out = sh(&format!("head -c {bytes} /dev/zero > {}", shell_quote(path)));
        assert!(out.ok, "touch {path}: {}", out.err_text());
    }

    fn names(dir: &str) -> Vec<String> {
        listdir(dir).unwrap().into_iter().map(|entry| entry.name).collect()
    }

    fn sd_card() -> Option<String> {
        adb::storages().into_iter().map(|(_, path)| path).find(|path| path.starts_with("/storage/"))
    }

    #[test]
    #[ignore = "needs a Thor attached via adb"]
    fn device_listing_has_mtime_and_odd_names() {
        let sandbox = Sandbox::new("/sdcard");
        // ("|" can't be used: Android's shared storage refuses it. Its parsing is unit-tested.)
        touch(&sandbox.path("a b's [c].txt"), 3);
        touch(&sandbox.path(".hidden"), 1);
        touch(&sandbox.path("..dots"), 1);
        mkdir(&sandbox.path("Sub")).unwrap();
        let entries = listdir(&sandbox.0).unwrap();
        let get = |name: &str| entries.iter().find(|entry| entry.name == name).unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(get("a b's [c].txt").size, 3);
        assert!(get("Sub").dir);
        assert!(get(".hidden").mtime > 1_600_000_000, "mtime {}", get(".hidden").mtime);
        assert!(entries.iter().any(|entry| entry.name == "..dots"));
        assert!(listdir(&sandbox.path("Sub")).unwrap().is_empty());
        assert!(listdir(&sandbox.path("missing")).is_err());
    }

    #[test]
    #[ignore = "needs a Thor attached via adb"]
    fn device_batch_move_delete_and_case_rename() {
        let sandbox = Sandbox::new("/sdcard");
        let dest = sandbox.path("dest");
        mkdir(&dest).unwrap();
        touch(&sandbox.path("one.bin"), 1);
        touch(&sandbox.path("it's two.bin"), 1);
        touch(&sandbox.path("Clash.txt"), 1);
        touch(&join_remote(&dest, "clash.txt"), 1);
        let sources = vec![sandbox.path("one.bin"), sandbox.path("it's two.bin"), sandbox.path("Clash.txt"), dest.clone()];
        let results = move_paths(&sources, &dest);
        assert_eq!(results[0].error, None);
        assert_eq!(results[1].error, None);
        // Shared storage ignores case: "Clash.txt" collides with "clash.txt" and must not overwrite it.
        assert!(results[2].error.as_deref().unwrap_or("").contains("already exists"), "{:?}", results[2]);
        assert!(results[3].error.as_deref().unwrap_or("").contains("into itself"), "{:?}", results[3]);
        let mut moved = names(&dest);
        moved.sort();
        assert_eq!(moved, ["clash.txt", "it's two.bin", "one.bin"]);

        rename(&dest, &sandbox.path("DEST")).expect("case-only rename");
        assert!(names(&sandbox.0).contains(&"DEST".to_string()));

        // (The "too high up" safety net is covered by `moves_are_planned` without touching a device.)
        let doomed = vec![sandbox.path("DEST/one.bin"), sandbox.path("Clash.txt")];
        let results = delete_paths(&doomed);
        assert_eq!(results[0].error, None);
        assert_eq!(results[1].error, None);
        assert_eq!(names(&sandbox.path("DEST")).len(), 2);
    }

    #[test]
    #[ignore = "needs a Thor attached via adb"]
    fn device_thor_copy_runs_and_can_be_killed() {
        let src_box = Sandbox::new("/sdcard");
        let dest_base = sd_card().unwrap_or_else(|| "/sdcard".into());
        let dest_box = Sandbox::new(&dest_base);
        // Many small files: a big single file lands in the page cache and is "copied" in
        // milliseconds, while per-file overhead on FUSE keeps this copy busy for seconds.
        let src = src_box.path("many");
        mkdir(&src).unwrap();
        let out = sh(&format!("cd {} && i=0; while [ $i -lt 3000 ]; do echo $i > f$i; i=$((i+1)); done", shell_quote(&src)));
        assert!(out.ok, "{}", out.err_text());

        // Finished copy.
        let small = src_box.path("small.bin");
        touch(&small, 4096);
        let target = dest_box.path("small.bin");
        let out = adb::run(&["shell", &thor_copy_command(&small, &target)]);
        assert!(out.ok, "{}", out.err_text());
        assert_eq!(listdir(&dest_box.0).unwrap()[0].size, 4096);

        // Cancelled copy: kill the local adb like wait_with_polling does, then kill_thor_copy.
        let target = dest_box.path("many");
        let _ = sh(&format!("rm -f {THOR_COPY_PID_FILE}"));
        let mut child = spawn_adb(&["shell", &thor_copy_command(&src, &target)]).unwrap();
        std::thread::sleep(Duration::from_millis(700));
        assert!(child.try_wait().unwrap().is_none(), "copy finished too fast to test the cancel");
        let pid = sh(&format!("cat {THOR_COPY_PID_FILE}")).stdout.trim().to_string();
        assert!(!pid.is_empty(), "no PID file");
        let _ = child.kill();
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(300));
        eprintln!("cp still running after killing only the local adb: {}", sh(&format!("kill -0 {pid}")).ok);
        kill_thor_copy();
        assert!(!sh(&format!("kill -0 {pid}")).ok, "cp {pid} still running after kill_thor_copy");
        let size_now = remote_size(&target);
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(remote_size(&target), size_now, "partial copy still growing");
        let copied = sh(&format!("ls {} | wc -l", shell_quote(&target))).stdout.trim().parse::<u32>().unwrap_or(0);
        eprintln!("files copied before the cancel: {copied} of 3000");
        assert!(copied < 3000, "copy was not interrupted");
        clean_up_partial(&target, "many", false);
        assert!(!remote_exists(&target));
        let _ = sh(&format!("rm -f {THOR_COPY_PID_FILE}"));
    }
}
