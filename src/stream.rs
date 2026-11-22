//! Mirrors the Thor's screens to the PC with scrcpy (port of the old `thor-stream` script).
//!
//! The Thor has two displays. `scrcpy --list-displays` reports which display ids scrcpy can see
//! (`list_displays`), and every mirrored display is its own scrcpy process (`--display-id=N`) with
//! its own window title. `Streams` keeps track of the running processes, remembers why a process
//! ended with an error, and kills all of them when ThorBro exits.
//!
//! Why `scrcpy --list-displays` and not adb (`adb shell cmd display get-displays` or
//! `dumpsys display`): scrcpy lists exactly the ids it accepts for `--display-id`, in a format
//! scrcpy controls. The adb variants print Android-internal debug text that changes between
//! Android versions, and a display that adb lists but scrcpy can't capture is of no use here.
//!
//! Used by `commands.rs` (the `stream_*` Tauri commands), `cli.rs` (`thorbro stream`,
//! `thorbro displays`) and `main.rs` (closing all windows when ThorBro exits). Uses `adb.rs` so
//! scrcpy talks to the same adb as ThorBro, and `util::which` to find scrcpy.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Display id of the Thor's top (main) screen. Only this stream gets audio, so it isn't doubled.
pub const TOP_DISPLAY: u32 = 0;

/// Display id of the Thor's bottom screen on the firmware we know (reported by
/// `scrcpy --list-displays` as `--display-id=4 (1240x1080)`). Used by the quick-start buttons
/// and `thorbro stream bottom`, which don't run the detection first.
pub const BOTTOM_DISPLAY: u32 = 4;

/// scrcpy options for every stream: view only, 60 fps.
const COMMON_ARGS: &[&str] = &["--no-control", "--max-fps=60"];

/// Allowed video bitrates in Mbit/s (scrcpy itself accepts more, this guards against typos).
const BITRATE_RANGE_MBPS: RangeInclusive<u32> = 1..=200;

/// Largest accepted `--max-size` (0 = no limit). The Thor's largest screen is 1920 px wide.
const MAX_SIZE_LIMIT: u32 = 8192;

/// How long `scrcpy --list-displays` may take. It pushes the scrcpy server to the Thor first,
/// which normally takes about a second.
const LIST_TIMEOUT: Duration = Duration::from_secs(20);

/// How often a waiting loop checks whether a process has exited.
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// After a process exited: how long to wait for its output pipes to be read to the end.
/// scrcpy passes its pipes on to the adb processes it starts, so a pipe can stay open after
/// scrcpy itself is gone; we don't wait for that forever.
const PIPE_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

/// Output lines kept per running stream, used to explain why a stream stopped.
const STREAM_LOG_LINES: usize = 40;

/// Output lines kept from `scrcpy --list-displays`.
const LIST_LOG_LINES: usize = 500;

/// How many lines of scrcpy output go into an error message at most.
const ERROR_LINES: usize = 3;

/// Error text when scrcpy can't be found.
const SCRCPY_MISSING: &str = "scrcpy not found – please install scrcpy (see README)";

/// Options the user can pick per display. Stored per display id in `settings.json`
/// (`settings::Settings::stream`), so the next stream of that display starts the same way.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct StreamOptions {
    /// `--max-size`: longest side of the video in pixels, 0 = original size.
    pub max_size: u32,
    /// `--video-bit-rate` in Mbit/s.
    pub bitrate_mbps: u32,
    /// `--always-on-top`.
    pub always_on_top: bool,
}

impl Default for StreamOptions {
    /// The values the stream used before options existed: original size, 16 Mbit/s.
    fn default() -> Self {
        StreamOptions { max_size: 0, bitrate_mbps: 16, always_on_top: false }
    }
}

impl StreamOptions {
    /// Checks the values before they are passed to scrcpy or saved.
    pub fn validate(self) -> Result<Self, String> {
        if !BITRATE_RANGE_MBPS.contains(&self.bitrate_mbps) {
            return Err(format!(
                "Bitrate must be between {} and {} Mbit/s",
                BITRATE_RANGE_MBPS.start(),
                BITRATE_RANGE_MBPS.end()
            ));
        }
        if self.max_size > MAX_SIZE_LIMIT {
            return Err(format!("Size must be at most {MAX_SIZE_LIMIT} pixels (0 = original)"));
        }
        Ok(self)
    }
}

/// One display reported by `scrcpy --list-displays`.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct Display {
    pub id: u32,
    /// Current size as scrcpy reports it, e.g. 1920 x 1080 (missing if scrcpy didn't print one).
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Parses the output of `scrcpy --list-displays`. scrcpy 2.x – 5.x print:
///
/// ```text
/// [server] INFO: List of displays:
///     --display-id=0    (1920x1080)
///     --display-id=4    (1240x1080)
/// ```
///
/// Lines without `--display-id=<number>` are ignored, so log lines around the list and CRLF line
/// ends (Windows) don't matter. Each id is returned once, in the order scrcpy printed them.
pub fn parse_display_list(output: &str) -> Vec<Display> {
    let mut displays: Vec<Display> = Vec::new();
    for line in output.lines() {
        let Some(start) = line.find("--display-id=") else { continue };
        let rest = &line[start + "--display-id=".len()..];
        let digits_len = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let Ok(id) = rest[..digits_len].parse::<u32>() else { continue };
        let size = parse_size(&rest[digits_len..]);
        if !displays.iter().any(|display| display.id == id) {
            displays.push(Display { id, width: size.map(|(w, _)| w), height: size.map(|(_, h)| h) });
        }
    }
    displays
}

/// Reads `(1920x1080)` at the start of `text` (leading spaces allowed).
fn parse_size(text: &str) -> Option<(u32, u32)> {
    let inner = text.trim_start().strip_prefix('(')?.split_once(')')?.0;
    let (width, height) = inner.split_once('x')?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

/// Short user-facing name of a display. `ids` are all detected ids (may be empty if the
/// detection hasn't run): with exactly two displays the one that isn't 0 is the bottom screen,
/// even if its id isn't the usual 4.
pub fn display_label(id: u32, ids: &[u32]) -> String {
    if id == TOP_DISPLAY {
        "Top screen".into()
    } else if id == BOTTOM_DISPLAY || (ids.len() == 2 && ids.contains(&TOP_DISPLAY) && ids.contains(&id)) {
        "Bottom screen".into()
    } else {
        format!("Display {id}")
    }
}

/// Window title of a stream. Always contains the display id, so two windows never share a title.
fn window_title(id: u32, ids: &[u32]) -> String {
    match display_label(id, ids).as_str() {
        "Top screen" => format!("Thor – top (display {id})"),
        "Bottom screen" => format!("Thor – bottom (display {id})"),
        _ => format!("Thor – display {id}"),
    }
}

/// The scrcpy arguments for one stream (no shell involved: each element is one argument).
fn stream_args(id: u32, options: &StreamOptions, title: &str) -> Vec<String> {
    let mut args: Vec<String> = COMMON_ARGS.iter().map(|arg| arg.to_string()).collect();
    args.push(format!("--video-bit-rate={}M", options.bitrate_mbps));
    args.push(format!("--display-id={id}"));
    args.push(format!("--window-title={title}"));
    if options.max_size > 0 {
        args.push(format!("--max-size={}", options.max_size));
    }
    if options.always_on_top {
        args.push("--always-on-top".into());
    }
    // Android plays the same audio no matter which display is mirrored; only the top stream
    // takes it, otherwise you'd hear everything twice.
    if id != TOP_DISPLAY {
        args.push("--no-audio".into());
    }
    args
}

/// Finds scrcpy: next to the ThorBro executable, in the bundled `tools/` folder (Windows
/// release), or in `PATH`.
fn scrcpy() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for dir in [exe_dir.to_path_buf(), exe_dir.join("tools")] {
                for name in ["scrcpy", "scrcpy.exe"] {
                    let candidate = dir.join(name);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    crate::util::which("scrcpy")
}

/// A scrcpy `Command` with both output pipes captured and no console window on Windows.
/// The program is started directly, without a shell in between.
fn scrcpy_command(args: &[String]) -> Result<Command, String> {
    let exe = scrcpy().ok_or(SCRCPY_MISSING)?;
    let mut command = crate::adb::cmd(exe);
    command
        .args(args)
        // scrcpy should use the same adb as ThorBro (two different adb versions fight each other).
        .env("ADB", crate::adb::adb_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

/// The last lines a process printed (stdout and stderr mixed, in arrival order).
///
/// `Arc` is a thread-safe reference-counted pointer (like `std::shared_ptr`): the reader threads
/// and `Streams` share one buffer.
type LogTail = Arc<Mutex<VecDeque<String>>>;

/// Starts a thread that reads `pipe` line by line into `tail` (keeping at most `limit` lines)
/// and reports on `done` when the pipe is closed.
///
/// Reading is required even if nobody looks at the text: a full pipe would block scrcpy.
/// Bytes are read raw and converted lossily, so invalid UTF-8 can't stop the reading.
fn collect_lines(pipe: impl Read + Send + 'static, tail: LogTail, limit: usize, done: mpsc::Sender<()>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buffer).trim_end().to_string();
                    let mut lines = tail.lock().unwrap();
                    lines.push_back(line);
                    while lines.len() > limit {
                        lines.pop_front();
                    }
                }
            }
        }
        // The receiver may already be gone; that's fine.
        let _ = done.send(());
    });
}

/// Hooks `collect_lines` to stdout and stderr of `child`. The returned receiver gets one
/// message per pipe that was read to the end.
fn capture_output(child: &mut Child, limit: usize) -> (LogTail, mpsc::Receiver<()>) {
    let tail: LogTail = Arc::default();
    let (done_sender, done_receiver) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        collect_lines(stdout, tail.clone(), limit, done_sender.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        collect_lines(stderr, tail.clone(), limit, done_sender);
    }
    (tail, done_receiver)
}

/// Waits (at most `PIPE_DRAIN_TIMEOUT` in total) until both output pipes have been read to the end.
fn wait_for_pipes(done: &mpsc::Receiver<()>) {
    let deadline = Instant::now() + PIPE_DRAIN_TIMEOUT;
    for _ in 0..2 {
        let left = deadline.saturating_duration_since(Instant::now());
        if done.recv_timeout(left).is_err() {
            return;
        }
    }
}

/// The interesting part of scrcpy's output for an error message: its `ERROR:` lines, or the
/// last few lines if there are none.
fn error_summary(lines: &[String]) -> String {
    let errors: Vec<&str> = lines
        .iter()
        .filter_map(|line| line.split_once("ERROR:").map(|(_, message)| message.trim()))
        .filter(|message| !message.is_empty())
        .collect();
    let chosen: Vec<&str> = if errors.is_empty() {
        let non_empty: Vec<&str> = lines.iter().map(|line| line.trim()).filter(|line| !line.is_empty()).collect();
        non_empty[non_empty.len().saturating_sub(ERROR_LINES)..].to_vec()
    } else {
        errors[errors.len().saturating_sub(ERROR_LINES)..].to_vec()
    };
    chosen.join(" · ")
}

/// Runs `scrcpy --list-displays` and returns the displays scrcpy can mirror.
///
/// An empty list is returned as an error together with scrcpy's output, because it means scrcpy
/// couldn't talk to the Thor (no device, unauthorized, several devices, ...).
pub fn list_displays() -> Result<Vec<Display>, String> {
    let mut child = scrcpy_command(&["--list-displays".to_string()])?
        .spawn()
        .map_err(|e| format!("couldn't start scrcpy: {e}"))?;
    let (tail, done) = capture_output(&mut child, LIST_LOG_LINES);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > LIST_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("scrcpy --list-displays didn't finish within {} s", LIST_TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(PROCESS_POLL_INTERVAL),
            Err(e) => return Err(format!("couldn't wait for scrcpy: {e}")),
        }
    };
    wait_for_pipes(&done);
    let lines: Vec<String> = tail.lock().unwrap().iter().cloned().collect();
    let displays = parse_display_list(&lines.join("\n"));
    if displays.is_empty() {
        let details = error_summary(&lines);
        let reason = if status.success() { "scrcpy listed no displays" } else { "scrcpy couldn't list the displays" };
        return Err(if details.is_empty() { reason.to_string() } else { format!("{reason}: {details}") });
    }
    Ok(displays)
}

/// One running scrcpy process.
struct RunningStream {
    display: u32,
    child: Child,
    /// Its recent output, to explain an unexpected exit.
    log: LogTail,
}

/// A stream that couldn't start or ended with an error. Shown in the UI until that display is
/// started again or stopped.
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
pub struct StreamFailure {
    pub display: u32,
    /// Increases with every new failure, so the UI can tell new ones from ones it already showed.
    pub seq: u64,
    pub message: String,
}

/// What the UI needs to show the stream state.
#[derive(Serialize, Clone, PartialEq, Eq, Debug, Default)]
pub struct StreamStatus {
    /// Display ids that are being mirrored right now, sorted.
    pub running: Vec<u32>,
    /// Streams that failed, sorted by display id.
    pub failures: Vec<StreamFailure>,
}

/// Everything `Streams` protects with its one lock.
#[derive(Default)]
struct StreamsInner {
    running: Vec<RunningStream>,
    failures: BTreeMap<u32, StreamFailure>,
    next_seq: u64,
    /// Ids from the last successful detection, used for labels and window titles.
    detected: Vec<u32>,
}

impl StreamsInner {
    fn record_failure(&mut self, display: u32, message: String) {
        self.next_seq += 1;
        self.failures.insert(display, StreamFailure { display, seq: self.next_seq, message });
    }

    /// Drops processes that have exited. A non-zero exit (scrcpy couldn't start, the device was
    /// unplugged, ...) is recorded as a failure; closing the window normally exits with 0.
    fn reap(&mut self) {
        let mut finished: Vec<(u32, ExitStatus, LogTail)> = Vec::new();
        // `retain_mut` keeps the elements for which the closure returns true.
        self.running.retain_mut(|stream| match stream.child.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                finished.push((stream.display, status, stream.log.clone()));
                false
            }
            Err(_) => false,
        });
        for (display, status, log) in finished {
            if status.success() {
                continue;
            }
            let lines: Vec<String> = log.lock().unwrap().iter().cloned().collect();
            let why = match status.code() {
                Some(code) => format!("scrcpy for display {display} stopped (exit code {code})"),
                None => format!("scrcpy for display {display} was terminated"),
            };
            let details = error_summary(&lines);
            self.record_failure(display, if details.is_empty() { why } else { format!("{why}: {details}") });
        }
    }

    fn status(&self) -> StreamStatus {
        let mut running: Vec<u32> = self.running.iter().map(|stream| stream.display).collect();
        running.sort_unstable();
        StreamStatus { running, failures: self.failures.values().cloned().collect() }
    }
}

/// The running scrcpy processes.
///
/// The `Mutex` makes the state safe to use from several threads (Tauri commands may run in
/// parallel). Unlike `std::mutex` in C++, a Rust `Mutex` *owns* the data it protects: you can
/// only reach it through `lock()`, so forgetting to lock is a compile error.
#[derive(Default)]
pub struct Streams {
    inner: Mutex<StreamsInner>,
}

impl Streams {
    /// Remembers the ids of the last detection (for labels and window titles).
    pub fn set_detected(&self, ids: Vec<u32>) {
        self.inner.lock().unwrap().detected = ids;
    }

    /// Starts one scrcpy window for `display`. Does nothing if that display is already mirrored.
    /// A start error is recorded as a failure for that display and also returned.
    ///
    /// `&self` is like a `const` member function's `this` in C++, but thanks to the `Mutex`
    /// we can still change the state inside.
    pub fn start_display(&self, display: u32, options: &StreamOptions) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        inner.reap();
        if inner.running.iter().any(|stream| stream.display == display) {
            return Ok(());
        }
        inner.failures.remove(&display);
        let args = stream_args(display, options, &window_title(display, &inner.detected));
        // `and_then` chains a second fallible step: only spawn if the command could be built.
        let spawned = scrcpy_command(&args)
            .and_then(|mut command| command.spawn().map_err(|e| format!("couldn't start scrcpy for display {display}: {e}")));
        match spawned {
            Ok(mut child) => {
                let (log, _done) = capture_output(&mut child, STREAM_LOG_LINES);
                inner.running.push(RunningStream { display, child, log });
                Ok(())
            }
            Err(message) => {
                inner.record_failure(display, message.clone());
                Err(message)
            }
        }
    }

    /// Starts several displays. One that fails doesn't keep the others from starting; its error
    /// ends up in the returned status. Only "scrcpy not found" is returned as an error, because
    /// then nothing can start at all.
    pub fn start_many(&self, requests: &[(u32, StreamOptions)]) -> Result<StreamStatus, String> {
        if scrcpy().is_none() {
            return Err(SCRCPY_MISSING.into());
        }
        for (display, options) in requests {
            // The error is already recorded in the failures.
            let _ = self.start_display(*display, options);
        }
        Ok(self.status())
    }

    /// Starts the screens for the quick-start buttons and the CLI, with default options and the
    /// fixed ids `TOP_DISPLAY` / `BOTTOM_DISPLAY`.
    ///
    /// `which`: `"both"`, `"top"` or `"bottom"` (the old German `"beide"` / `"oben"` / `"unten"`
    /// still work for the CLI). If one screen fails to start, the other keeps running and the
    /// error is returned.
    pub fn start(&self, which: &str) -> Result<(), String> {
        let wanted: &[u32] = match which {
            "top" | "oben" => &[TOP_DISPLAY],
            "bottom" | "unten" => &[BOTTOM_DISPLAY],
            _ => &[TOP_DISPLAY, BOTTOM_DISPLAY],
        };
        let errors: Vec<String> = wanted
            .iter()
            .filter_map(|display| self.start_display(*display, &StreamOptions::default()).err())
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }

    /// Closes the window of one display and forgets its failure.
    pub fn stop_display(&self, display: u32) {
        let mut inner = self.inner.lock().unwrap();
        inner.failures.remove(&display);
        let (stopped, kept): (Vec<RunningStream>, Vec<RunningStream>) =
            inner.running.drain(..).partition(|stream| stream.display == display);
        inner.running = kept;
        for mut stream in stopped {
            let _ = stream.child.kill();
            let _ = stream.child.wait();
        }
    }

    /// Closes all scrcpy windows and forgets all failures.
    pub fn stop(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.failures.clear();
        // `drain(..)` removes all elements and hands them to us one by one (by value, so we
        // own each `Child` and may kill it).
        for mut stream in inner.running.drain(..) {
            let _ = stream.child.kill();
            let _ = stream.child.wait();
        }
    }

    /// Forgets processes that have exited (windows the user closed, crashed streams) and returns
    /// what is running and what failed.
    pub fn status(&self) -> StreamStatus {
        let mut inner = self.inner.lock().unwrap();
        inner.reap();
        inner.status()
    }
}

impl Drop for Streams {
    /// Kills the remaining windows when the `Streams` goes away (end of `thorbro stream`, or an
    /// early return there). The app additionally calls `stop()` on exit, see `main.rs`.
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real output of scrcpy 5.0.1 with an AYN Thor (Android 13), stdout and stderr combined.
    const THOR_OUTPUT: &str = "scrcpy 5.0.1 <https://github.com/Genymobile/scrcpy>\n\
        INFO: ADB device found:\n\
        INFO:     -->   (usb)  8527ad10                        device  AYN_Thor\n\
        /usr/share/scrcpy/scrcpy-server: 1 file pushed, 0 skipped. 321.3 MB/s (733930 bytes in 0.002s)\n\
        [server] INFO: Device: [AYN] qti AYN Thor (Android 13)\n\
        [server] INFO: List of displays:\n    \
        --display-id=0    (1920x1080)\n    \
        --display-id=4    (1240x1080)\n";

    fn display(id: u32, width: u32, height: u32) -> Display {
        Display { id, width: Some(width), height: Some(height) }
    }

    #[test]
    fn parses_the_thor_display_list() {
        assert_eq!(parse_display_list(THOR_OUTPUT), vec![display(0, 1920, 1080), display(4, 1240, 1080)]);
    }

    #[test]
    fn parses_crlf_and_single_display() {
        let output = "[server] INFO: List of displays:\r\n    --display-id=0    (1920x1080)\r\n";
        assert_eq!(parse_display_list(output), vec![display(0, 1920, 1080)]);
    }

    #[test]
    fn parses_ids_without_size_and_skips_duplicates_and_junk() {
        let output = "    --display-id=7\n    --display-id=7    (100x200)\n    --display-id=id\n    --display-id=3 (bad)\n";
        assert_eq!(
            parse_display_list(output),
            vec![Display { id: 7, width: None, height: None }, Display { id: 3, width: None, height: None }]
        );
    }

    #[test]
    fn error_output_has_no_displays() {
        let output = "scrcpy 5.0.1 <https://github.com/Genymobile/scrcpy>\nERROR: Could not find any ADB device\n";
        assert!(parse_display_list(output).is_empty());
        assert_eq!(error_summary(&output.lines().map(String::from).collect::<Vec<_>>()), "Could not find any ADB device");
    }

    #[test]
    fn error_summary_falls_back_to_last_lines() {
        let lines: Vec<String> = ["a", "", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(error_summary(&lines), "b · c · d");
        assert_eq!(error_summary(&[]), "");
    }

    #[test]
    fn labels_and_titles() {
        assert_eq!(display_label(0, &[]), "Top screen");
        assert_eq!(display_label(4, &[]), "Bottom screen");
        // Two displays, but the second one has an unexpected id: still the bottom screen.
        assert_eq!(display_label(2, &[0, 2]), "Bottom screen");
        assert_eq!(display_label(2, &[0, 2, 5]), "Display 2");
        let ids = [0, 4, 9];
        let titles: Vec<String> = ids.iter().map(|id| window_title(*id, &ids)).collect();
        assert_eq!(titles, ["Thor – top (display 0)", "Thor – bottom (display 4)", "Thor – display 9"]);
    }

    #[test]
    fn default_args_match_the_old_stream() {
        let mut top = stream_args(0, &StreamOptions::default(), "T");
        top.sort();
        assert_eq!(top, ["--display-id=0", "--max-fps=60", "--no-control", "--video-bit-rate=16M", "--window-title=T"]);
        let bottom = stream_args(4, &StreamOptions::default(), "B");
        assert!(bottom.contains(&"--no-audio".to_string()));
        assert!(bottom.contains(&"--display-id=4".to_string()));
    }

    #[test]
    fn options_become_args() {
        let options = StreamOptions { max_size: 1280, bitrate_mbps: 8, always_on_top: true };
        let args = stream_args(0, &options, "T");
        for expected in ["--max-size=1280", "--video-bit-rate=8M", "--always-on-top"] {
            assert!(args.contains(&expected.to_string()), "missing {expected}");
        }
        assert!(!args.contains(&"--no-audio".to_string()));
    }

    #[test]
    fn options_are_validated() {
        assert!(StreamOptions::default().validate().is_ok());
        assert!(StreamOptions { bitrate_mbps: 0, ..Default::default() }.validate().is_err());
        assert!(StreamOptions { bitrate_mbps: 500, ..Default::default() }.validate().is_err());
        assert!(StreamOptions { max_size: 100_000, ..Default::default() }.validate().is_err());
    }

    /// Opens real scrcpy windows. Needs a connected Thor:
    /// `cargo test device_ -- --ignored --test-threads=1`
    #[test]
    #[ignore]
    fn device_streams_every_detected_display_and_stops_them() {
        let displays = list_displays().expect("detection failed");
        let ids: Vec<u32> = displays.iter().map(|display| display.id).collect();
        let streams = Streams::default();
        streams.set_detected(ids.clone());
        let requests: Vec<(u32, StreamOptions)> = ids
            .iter()
            .map(|id| (*id, StreamOptions { max_size: 720, bitrate_mbps: 4, always_on_top: false }))
            .collect();
        streams.start_many(&requests).unwrap();
        // Long enough for scrcpy to fail if it is going to (no display, encoder error, ...).
        std::thread::sleep(Duration::from_secs(5));
        let status = streams.status();
        assert_eq!(status.running, ids, "failures: {:?}", status.failures);
        assert!(status.failures.is_empty(), "{:?}", status.failures);
        streams.stop_display(ids[0]);
        assert_eq!(streams.status().running, ids[1..]);
        streams.stop();
        assert!(streams.status().running.is_empty());
    }

    /// A display id the Thor doesn't have: scrcpy exits with an error that ends up in `failures`.
    #[test]
    #[ignore]
    fn device_unknown_display_is_reported_as_failure() {
        let streams = Streams::default();
        streams.start_display(99, &StreamOptions::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !streams.status().running.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        let status = streams.status();
        assert!(status.running.is_empty());
        assert_eq!(status.failures.len(), 1);
        eprintln!("failure message: {}", status.failures[0].message);
        assert!(status.failures[0].message.contains("display 99"), "{}", status.failures[0].message);
    }

    #[test]
    fn failures_are_numbered_and_status_is_sorted() {
        let mut inner = StreamsInner::default();
        inner.record_failure(4, "b".into());
        inner.record_failure(0, "a".into());
        inner.record_failure(4, "c".into());
        let status = inner.status();
        assert!(status.running.is_empty());
        let summary: Vec<(u32, u64, &str)> =
            status.failures.iter().map(|f| (f.display, f.seq, f.message.as_str())).collect();
        assert_eq!(summary, [(0, 2, "a"), (4, 3, "c")]);
    }
}
