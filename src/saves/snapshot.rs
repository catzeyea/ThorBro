//! Snapshots: a "fingerprint" of a save folder (MD5, modification time and size per file).
//!
//! Two snapshots are compared to decide whether PC and Thor are in sync, which side is newer,
//! and whether a copy really arrived intact. On the PC files are hashed in Rust, on the Thor
//! with `md5sum` / `stat` in one shell call.
//!
//! Used by `sync.rs`, `import.rs` and `main.rs` (`peek` shows what's inside a folder).

use super::cached_regex;
use crate::adb::{sh, shell_quote, tcmd};
use crate::util::{self, human, when};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::OnceLock;

/// How many file names `info` lists for the UI.
const PREVIEW_NAMES: usize = 12;

/// Separator line between the two parts of the Thor snapshot script output.
const THOR_SNAPSHOT_SEPARATOR: &str = "===\n";

/// Printed by the Thor snapshot script if the folder doesn't exist.
const THOR_MISSING_MARKER: &str = "NOPE";

/// One file in a snapshot: `(md5 hex, modification time in Unix seconds, size in bytes)`.
pub type FileFingerprint = (String, f64, u64);

/// Relative path -> fingerprint, sorted by path.
/// Functions return `Option<Snap>`, where `None` means "the folder doesn't exist".
pub type Snap = BTreeMap<String, FileFingerprint>;

/// Snapshot of a folder on the PC. `None` if `path` is `None` or not a folder.
pub fn pc_snapshot(path: Option<&str>) -> Option<Snap> {
    // `path?` on an `Option`: if it is `None`, return `None` from this function right away.
    let root = Path::new(path?);
    if !root.is_dir() {
        return None;
    }
    let mut snap = Snap::new();
    for entry in walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(hash) = util::md5_file(entry.path()) else { continue };
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        snap.insert(util::rel(root, entry.path()), (hash, util::mtime_of(entry.path()), size));
    }
    Some(snap)
}

/// Snapshot of a folder on the Thor. `None` if `path` is `None` or the folder doesn't exist.
///
/// The script prints `md5sum` lines, a separator, then `stat` lines (`mtime size name`).
/// Private app folders are handled transparently via `run-as` (see `adb::tcmd`).
///
/// Note: if adb itself fails, the output is empty and the result is an *empty* snapshot,
/// not `None`.
pub fn thor_snapshot(path: Option<&str>) -> Option<Snap> {
    let path = path?;
    let script = format!(
        "cd {} 2>/dev/null || {{ echo NOPE; exit 0; }}; find . -type f -exec md5sum {{}} + 2>/dev/null; \
         echo ===; find . -type f -exec stat -c '%Y %s %n' {{}} + 2>/dev/null",
        shell_quote(path)
    );
    let output = sh(&tcmd(path, &script)).stdout;
    if output.trim() == THOR_MISSING_MARKER {
        return None;
    }
    let (md5_lines, stat_lines) = output.split_once(THOR_SNAPSHOT_SEPARATOR).unwrap_or((&output, ""));
    let metadata = parse_thor_stat_lines(stat_lines);
    Some(parse_thor_md5_lines(md5_lines, &metadata))
}

/// `find` prints paths as `./a/b`; we store them as `a/b`.
fn strip_dot_slash(path: &str) -> String {
    path.strip_prefix("./").unwrap_or(path).to_string()
}

/// Parses `"<mtime> <size> <path>"` lines into `path -> (mtime, size)`.
fn parse_thor_stat_lines(stat_lines: &str) -> HashMap<String, (f64, u64)> {
    let mut metadata = HashMap::new();
    for line in stat_lines.lines() {
        let mut fields = line.splitn(3, ' ');
        if let (Some(mtime), Some(size), Some(name)) = (fields.next(), fields.next(), fields.next()) {
            if let (Ok(mtime), Ok(size)) = (mtime.parse::<f64>(), size.parse::<u64>()) {
                metadata.insert(strip_dot_slash(name), (mtime, size));
            }
        }
    }
    metadata
}

/// Parses `md5sum` lines (`"<32 hex>  <path>"`) and joins them with the stat data.
fn parse_thor_md5_lines(md5_lines: &str, metadata: &HashMap<String, (f64, u64)>) -> Snap {
    static MD5_LINE_RE: OnceLock<Regex> = OnceLock::new();
    let md5_line = cached_regex(&MD5_LINE_RE, r"^([0-9a-f]{32})\s+(.*)$");
    let mut snap = Snap::new();
    for line in md5_lines.lines() {
        if let Some(captures) = md5_line.captures(line) {
            let relative = strip_dot_slash(&captures[2]);
            // `.copied()` turns `Option<&(f64, u64)>` into `Option<(f64, u64)>` (copies the tuple).
            let (mtime, size) = metadata.get(&relative).copied().unwrap_or((0.0, 0));
            snap.insert(relative, (captures[1].to_string(), mtime, size));
        }
    }
    snap
}

/// Snapshot of `path` on `side` (`"pc"` or anything else = Thor).
pub fn snapshot(side: &str, path: Option<&str>) -> Option<Snap> {
    if side == "pc" { pc_snapshot(path) } else { thor_snapshot(path) }
}

/// Human-readable one-liner, e.g. `"3 files, 1.2 MB, last changed 2024-05-01 18:30"`.
///
/// `&Option<Snap>` is a borrowed reference: the function only reads the snapshot and the caller
/// keeps ownership (like passing `const std::optional<Snap>&` in C++).
pub fn summary(snap: &Option<Snap>) -> String {
    match snap {
        None => "doesn't exist (yet)".into(),
        Some(files) if files.is_empty() => "empty".into(),
        Some(files) => {
            let total_size: u64 = files.values().map(|(_, _, size)| size).sum();
            format!(
                "{} files, {}, last changed {}",
                files.len(),
                human(total_size),
                when(newest(&Some(files.clone())))
            )
        }
    }
}

/// Newest modification time in the snapshot (`0.0` if missing or empty).
pub fn newest(snap: &Option<Snap>) -> f64 {
    snap.as_ref()
        .map(|files| files.values().map(|(_, mtime, _)| *mtime).fold(0.0, f64::max))
        .unwrap_or(0.0)
}

/// True if both snapshots exist and contain the same files with the same MD5 hashes.
/// Modification times are ignored (they may differ after copying).
pub fn same(a: &Option<Snap>, b: &Option<Snap>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.len() == b.len()
                && a.iter().all(|(path, (hash, _, _))| b.get(path).is_some_and(|(other_hash, _, _)| other_hash == hash))
        }
        _ => false,
    }
}

/// Snapshot summary for the UI (JSON).
#[derive(Serialize)]
pub struct SnapInfo {
    pub exists: bool,
    pub files: usize,
    pub newest: f64,
    pub newest_text: String,
    pub summary: String,
    /// The first few file names, as a preview.
    pub names: Vec<String>,
}

/// Builds the UI summary of a snapshot.
pub fn info(snap: &Option<Snap>) -> SnapInfo {
    let preview_names = snap
        .as_ref()
        .map(|files| files.keys().take(PREVIEW_NAMES).cloned().collect())
        .unwrap_or_default();
    SnapInfo {
        exists: snap.is_some(),
        files: snap.as_ref().map_or(0, |files| files.len()),
        newest: newest(snap),
        newest_text: when(newest(snap)),
        summary: summary(snap),
        names: preview_names,
    }
}
