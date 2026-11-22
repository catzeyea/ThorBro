//! File manager for the Thor, the copy queue PC -> Thor, and dragging files out Thor -> PC.
//!
//! - File manager: list, create, rename, delete, pull (all via `adb shell` / `adb pull`).
//! - Copy queue: jobs from the UI are processed one by one on a background thread. Progress is
//!   sent to the UI as `transfer` events (see `Progress`).
//! - Drag out: a file is first fetched into a local cache, then `main.rs` starts a real OS drag.
//!
//! Called from the Tauri commands in `main.rs`. Uses `adb.rs` for all Thor access and
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

/// Name of the Tauri event that carries copy progress to the UI (`app.js` listens to it).
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
}

/// Lists a folder on the Thor: folders first, then files, each sorted case-insensitively.
///
/// Hidden files are included. If the folder can't be read, the list is simply empty.
pub fn listdir(path: &str) -> Result<Vec<Entry>, String> {
    // `stat` prints "<type>|<size>|<name>" for every entry, e.g. "directory|4096|Download".
    let output = sh(&format!("cd {} && stat -c '%F|%s|%n' -- * .[!.]* 2>/dev/null", shell_quote(path)));
    let mut entries: Vec<Entry> = output.stdout.lines().filter_map(parse_stat_line).collect();
    // `!a.dir` is `false` for folders, and `false < true`, so folders sort first.
    entries.sort_by_key(|entry| (!entry.dir, entry.name.to_lowercase()));
    Ok(entries)
}

/// Parses one `"<type>|<size>|<name>"` line from `listdir`. `None` for malformed lines.
fn parse_stat_line(line: &str) -> Option<Entry> {
    // `splitn(3, ..)` stops after the second `|`, so names containing `|` stay intact.
    let mut fields = line.splitn(3, '|');
    let (kind, size, name) = (fields.next()?, fields.next()?, fields.next()?);
    Some(Entry { name: name.to_string(), dir: kind.contains("directory"), size: size.parse().unwrap_or(0) })
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
pub fn rename(from: &str, to: &str) -> Result<(), String> {
    check(sh(&format!("mv -- {} {}", shell_quote(from), shell_quote(to))))
}

/// Deletes a file or folder on the Thor (recursively, no recycle bin).
/// Refuses paths directly below `/` as a safety net against deleting a whole storage.
pub fn delete(path: &str) -> Result<(), String> {
    if path.trim_end_matches('/').matches('/').count() < MIN_DELETE_DEPTH {
        return Err("I'd rather not delete that (too high up in the file system).".into());
    }
    check(sh(&format!("rm -rf -- {}", shell_quote(path))))
}

/// Local folder for files copied from the Thor: `~/Downloads/thor`.
pub fn pull_dir() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Downloads"))
        .join("thor")
}

/// Copies a file or folder from the Thor into `pull_dir()`. Returns that folder's path.
pub fn pull(src: &str) -> Result<String, String> {
    let dest = pull_dir();
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    check(adb::run(&["pull", src, &dest.to_string_lossy()]))?;
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
}

/// One copy job from the UI.
///
/// - `unpack`: extract the archive first and upload its contents.
/// - `flatten`: skip a single wrapper folder inside the archive (for games).
/// - `remote`: `src` is on the Thor (it is fetched, extracted on the PC and pushed back).
///
/// `Deserialize` generates the code that builds this struct from the JSON sent by `app.js`.
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
}

/// Waiting jobs plus a flag whether a worker thread is currently running.
#[derive(Default)]
pub struct Queue {
    jobs: VecDeque<Job>,
    busy: bool,
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

/// Sends a progress event to the UI. A failed emit (e.g. window closed) is ignored.
fn emit_progress(app: &AppHandle, progress: Progress) {
    let _ = app.emit(TRANSFER_EVENT, progress);
}

/// Sends a "still working" progress event.
fn report_busy(app: &AppHandle, text: String, detail: String, fraction: f64) {
    emit_progress(app, Progress { busy: true, text, detail, fraction, dests: vec![], error: None });
}

/// Sends the final "done" / "failed" event.
fn report_finished(app: &AppHandle, text: String, dests: Vec<String>, error: Option<String>) {
    emit_progress(app, Progress { busy: false, text, detail: String::new(), fraction: 1.0, dests, error });
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
/// `impl FnMut()` accepts any closure that may modify what it captured, like a template
/// parameter `F` with `operator()` in C++.
fn wait_with_polling(child: &mut Child, sleep: Duration, poll_interval: Duration, mut on_poll: impl FnMut()) {
    let mut last_poll = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => {}
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
fn push_one(app: &AppHandle, src: &Path, dest: &str, more: &str) -> Result<(), String> {
    let name = local_name(src);
    let target_name = unpack::safe_name(&name);
    let total = size_of(src).max(1);
    let dest_trimmed = dest.trim_end_matches('/');
    let target = format!("{dest_trimmed}/{target_name}");
    report_busy(app, format!("Copying {target_name} ({})", human(total)), format!("to {dest}{more}"), 0.0);
    let _ = sh(&format!("mkdir -p {}", shell_quote(dest)));

    // A folder is pushed *into* dest/, a file directly to its target path.
    let push_to = if src.is_dir() { format!("{dest_trimmed}/") } else { target.clone() };
    // `_temp_dir` keeps the temporary symlink folder alive until the end of this function.
    // (A name starting with `_` tells the compiler "unused on purpose", but unlike plain `_`
    // the value is *not* dropped immediately.)
    let (push_src, _temp_dir) = push_source(src, &name, &target_name)?;
    let mut child = spawn_adb(&["push", &push_src.to_string_lossy(), &push_to]).map_err(|e| format!("{name}: {e}"))?;

    // Progress: regularly check how much already arrived on the Thor.
    let started = Instant::now();
    wait_with_polling(&mut child, PUSH_SLEEP, PUSH_POLL_INTERVAL, || {
        let done = remote_size(&target).min(total);
        let (detail, fraction) = transfer_detail(done, total, started);
        report_busy(app, format!("Copying {target_name}"), format!("{detail}{more}"), fraction);
    });

    match child.wait_with_output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(transfer_error(&name, &output)),
        Err(e) => Err(format!("{name}: {e}")),
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
/// contents. Returns the number of uploaded entries.
fn unpack_job(app: &AppHandle, job: &Job, more: &str) -> Result<u32, String> {
    // `work` is a `TempDir`: when this function returns (also on error), it is dropped and
    // deletes the folder with everything in it.
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
    let mut uploaded = 0;
    for item in &items {
        push_one(app, item, &job.dest, more)?;
        uploaded += 1;
    }
    Ok(uploaded)
}

/// Takes the next job from the queue. Returns `(job, jobs still waiting)`.
/// If the queue is empty, the busy flag is cleared in the same lock, so a concurrent
/// `enqueue` either sees the worker still running or starts a new one, never neither.
fn next_job(queue: &SharedQueue) -> (Option<Job>, usize) {
    let mut locked_queue = queue.lock().unwrap();
    let job = locked_queue.jobs.pop_front();
    if job.is_none() {
        locked_queue.busy = false;
    }
    (job, locked_queue.jobs.len())
}

/// Background thread: processes jobs until the queue is empty, then sends a summary.
fn worker(app: AppHandle, queue: SharedQueue) {
    let (mut copied, mut failed) = (0u32, 0u32);
    let mut dests: BTreeSet<String> = BTreeSet::new();
    let mut last_error = None;
    loop {
        let (job, waiting) = next_job(&queue);
        let Some(job) = job else { break };
        dests.insert(job.dest.clone());
        let more = if waiting > 0 { format!(" · {waiting} more waiting") } else { String::new() };
        let result = if job.unpack {
            unpack_job(&app, &job, &more)
        } else {
            push_one(&app, Path::new(&job.src), &job.dest, &more).map(|_| 1)
        };
        match result {
            Ok(count) => copied += count,
            Err(e) => {
                failed += 1;
                last_error = Some(e);
            }
        }
    }
    trigger_media_scan(&dests);
    let text = summary_text(copied, failed, &dests);
    report_finished(&app, text, dests.into_iter().collect(), last_error);
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

/// `"Done: 3 copied to /sdcard/ROMs/snes, 1 failed"`.
fn summary_text(copied: u32, failed: u32, dests: &BTreeSet<String>) -> String {
    let destination = if dests.len() == 1 {
        dests.iter().next().cloned().unwrap_or_default()
    } else {
        format!("{} folders", dests.len())
    };
    let mut text = format!("Done: {copied} copied to {destination}");
    if failed > 0 {
        text += &format!(", {failed} failed");
    }
    text
}

// --- Dragging out Thor -> PC -------------------------------------------------------------------
//
// To drag a file out of the window, it must exist locally. It is fetched into the cache first
// (~/.cache/thorbro/ziehen/<hash>/<name>), then `main.rs` starts a real OS drag with that file.
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
    report_finished(app, format!("{name} is ready to drag out"), vec![], None);
    Ok(Fetched { local: slot.local.to_string_lossy().into_owned(), fetched: true })
}

/// `adb pull src part/` with progress events. On failure the partial download is removed and a
/// "couldn't be fetched" event is sent.
fn pull_with_progress(app: &AppHandle, src: &str, name: &str, part: &Path) -> Result<(), String> {
    let total = remote_size(src).max(1);
    report_busy(app, format!("Fetching {name} from the Thor ({})", human(total)), "to drag out".into(), 0.0);
    let mut child = spawn_adb(&["pull", src, &part.to_string_lossy()]).map_err(|e| format!("{name}: {e}"))?;

    let started = Instant::now();
    wait_with_polling(&mut child, FETCH_SLEEP, FETCH_POLL_INTERVAL, || {
        let done = size_of(part).min(total);
        let (detail, fraction) = transfer_detail(done, total, started);
        report_busy(app, format!("Fetching {name} from the Thor"), format!("{detail} · then drag it out"), fraction);
    });

    let output = child.wait_with_output().map_err(|e| format!("{name}: {e}"))?;
    if !output.status.success() {
        let error = transfer_error(name, &output);
        let _ = std::fs::remove_dir_all(part);
        report_finished(app, format!("{name} couldn't be fetched"), vec![], Some(error.clone()));
        return Err(error);
    }
    Ok(())
}
