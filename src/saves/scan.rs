//! Searching PC and Thor for folders that probably contain the saves of a game.
//!
//! A folder is "interesting" if its name contains a search token of the game, looks like a save
//! folder (`save`, `memcard`, ...) or matches a known emulator layout (Switch title IDs, 3DS,
//! Steam `userdata/.../remote`). The results are ranked and later shown to the user / the AI.
//!
//! Used by `main.rs` (`scan_game`) and `import.rs` (`thor_stats`, `thor_roots`).

use super::{cached_regex, save_re, tokens};
use crate::adb::{self, sh, sh_script, shell_quote, tcmd};
use crate::util::{self, when};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// How deep to search below each root folder.
const PC_MAX_DEPTH: usize = 14;
const THOR_MAX_DEPTH: usize = 12;

/// Stop counting files in a PC folder after this many (only needed as "has files" + rough size).
const PC_FILE_COUNT_LIMIT: u64 = 5000;

/// At most this many candidates are returned per side.
const MAX_CANDIDATES: usize = 300;

/// At most this many Thor folders are examined in detail (one shell call per folder).
const MAX_THOR_FOLDERS: usize = 800;

/// PC folder names that are never searched (caches, browsers, system folders, ourselves ...).
const PC_SKIP: &[&str] = &[
    "node_modules", ".git", "__pycache__", "Trash", "flatpak", "baloo", "icons", "shadercache",
    "shader_cache", "GPUCache", "Code Cache", "windows", "mozilla", "Code", "Code - OSS", "Claude",
    "claude", "discord", "Vencord", "spotify", "Element", "net.imput.helium", "obsidian", "keyrings",
    "pki", "vulkan", "crash_dumps", "dump", "load", "Logs", "logs", "thor-save", "thorBro",
    "Program Files", "Program Files (x86)", "Microsoft", "Packages", "Temp", "CrashDumps",
];

/// Thor folder names that are skipped (media folders, never saves).
const THOR_PRUNE: &[&str] = &[
    "DCIM", "Pictures", "Movies", "Music", "obb", ".thumbnails", "Ringtones", "Notifications",
    "Alarms", "Podcasts", "Audiobooks", "Recordings",
];

/// A folder (or file) that may hold the saves. Sent to the UI as JSON.
#[derive(Serialize, Clone)]
pub struct Cand {
    pub path: String,
    /// Newest modification time of the files inside (Unix seconds).
    pub mtime: f64,
    /// `mtime` formatted for display.
    pub when: String,
    pub files: u64,
    /// Extra hint, e.g. `"[Steam: Elden Ring]"` or `"[archive]"`.
    pub note: String,
    /// The path contains a token of the game name.
    /// `#[serde(rename)]` because `match` is a keyword in Rust but the UI expects that name.
    #[serde(rename = "match")]
    pub matched: bool,
}

/// Decides whether a folder path looks like it could contain saves of the game.
fn interesting(path: &str, game_tokens: &[String]) -> bool {
    static SWITCH_TITLE_ID_RE: OnceLock<Regex> = OnceLock::new();
    let lowercase = path.to_lowercase();
    let parts: Vec<&str> = lowercase.split('/').collect();
    let name = *parts.last().unwrap_or(&"");

    if save_re().is_match(name) || game_tokens.iter().any(|token| name.contains(token.as_str())) {
        return true;
    }
    // Switch emulators: .../save/.../<16 hex digit title ID>
    if cached_regex(&SWITCH_TITLE_ID_RE, r"^[0-9a-f]{16}$").is_match(name) && lowercase.contains("save") {
        return true;
    }
    // Steam cloud saves: userdata/<user>/<app>/remote
    if name == "remote" && parts.contains(&"userdata") {
        return true;
    }
    // 3DS: …/title/00040000/<title-low>/data/00000001
    let depth = parts.len();
    if name == "00000001" && depth >= 5 && parts[depth - 2] == "data" && parts[depth - 5] == "title" {
        return true;
    }
    // Up to 2 levels below a save folder (e.g. savedata/1/CUSA12345).
    let first_parent = depth.saturating_sub(3);
    let parents = &parts[first_parent..depth.saturating_sub(1)];
    parents.iter().any(|part| save_re().is_match(part))
}

/// PC folders where games usually keep their saves. Only existing folders, no duplicates.
fn pc_roots() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut roots = vec![
        dirs::data_dir(),
        dirs::config_dir(),
        dirs::data_local_dir(),
        dirs::document_dir(),
        Some(home.join("Games")),
        Some(home.join(".var/app")),
        Some(home.join("Dokumente")),
        Some(home.join("Saved Games")),
        Some(home.join("AppData/LocalLow")),
    ];
    if cfg!(windows) {
        roots.push(Some(PathBuf::from("C:/Program Files (x86)/Steam/userdata")));
    }
    let mut seen = BTreeSet::new();
    // `.flatten()` on an iterator of `Option`s drops the `None`s and unwraps the rest.
    // `seen.insert(..)` returns false for duplicates, so they are filtered out.
    roots.into_iter().flatten().filter(|root| root.is_dir() && seen.insert(root.clone())).collect()
}

/// Steam app ID -> game name, read from the `appmanifest_*.acf` files of all known Steam libraries.
fn steam_names() -> HashMap<String, String> {
    static APP_ID_RE: OnceLock<Regex> = OnceLock::new();
    static NAME_RE: OnceLock<Regex> = OnceLock::new();
    let home = dirs::home_dir().unwrap_or_default();
    let steam_libraries = [
        home.join(".local/share/Steam/steamapps"),
        home.join("Games/steam/steamapps"),
        PathBuf::from("C:/Program Files (x86)/Steam/steamapps"),
    ];
    let mut names = HashMap::new();
    for library in steam_libraries {
        let Ok(read_dir) = fs::read_dir(&library) else { continue };
        for entry in read_dir.flatten() {
            if !entry.file_name().to_string_lossy().starts_with("appmanifest_") {
                continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            let app_id = cached_regex(&APP_ID_RE, r#""appid"\s+"(\d+)""#).captures(&text);
            let name = cached_regex(&NAME_RE, r#""name"\s+"([^"]+)""#).captures(&text);
            if let (Some(app_id), Some(name)) = (app_id, name) {
                names.insert(app_id[1].to_string(), name[1].to_string());
            }
        }
    }
    names
}

/// `"[Steam: <game>]"` if the path is inside a Steam `compatdata/<id>` or `userdata/<user>/<id>`
/// folder of a known game, else empty.
fn annotate(path: &str, steam: &HashMap<String, String>) -> String {
    static STEAM_ID_RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&STEAM_ID_RE, r"/(?:compatdata|userdata/\d+)/(\d+)(?:/|$)")
        .captures(path)
        .and_then(|captures| steam.get(&captures[1]).map(|name| format!("[Steam: {name}]")))
        .unwrap_or_default()
}

/// Newest modification time and number of files below a PC folder (counting stops at `limit`).
fn pc_newest(dir: &Path, limit: u64) -> (f64, u64) {
    let (mut newest, mut count) = (0.0f64, 0u64);
    for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            newest = newest.max(util::mtime_of(entry.path()));
            count += 1;
            if count >= limit {
                break;
            }
        }
    }
    (newest, count)
}

/// Sorts candidates: name matches first, then newest first. Keeps at most `MAX_CANDIDATES`.
///
/// `mut candidates` takes the vector *by value* (ownership moves in, no copy) and allows
/// modifying it; it is moved back out as the return value.
fn rank(mut candidates: Vec<Cand>) -> Vec<Cand> {
    candidates.sort_by(|a, b| {
        let by_match = (!a.matched).cmp(&!b.matched);
        // `partial_cmp` because floats can be NaN; NaN counts as equal here.
        let by_time = b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal);
        by_match.then(by_time)
    });
    candidates.truncate(MAX_CANDIDATES);
    candidates
}

/// True if the walker should descend into this entry: folders only, minus `PC_SKIP` and the
/// Steam game install folder `steamapps/common` (huge and never contains saves).
fn should_enter_pc_dir(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    if PC_SKIP.contains(&name.as_ref()) {
        return false;
    }
    let is_steam_common = name == "common" && entry.path().parent().is_some_and(|parent| parent.ends_with("steamapps"));
    !is_steam_common
}

/// All interesting folders below the PC roots.
fn find_interesting_pc_dirs(game_tokens: &[String]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in pc_roots() {
        let walker = walkdir::WalkDir::new(&root).max_depth(PC_MAX_DEPTH).into_iter().filter_entry(should_enter_pc_dir);
        for entry in walker.filter_map(Result::ok) {
            if entry.depth() == 0 {
                continue; // the root itself
            }
            if interesting(&util::slashed(entry.path()), game_tokens) {
                found.push(entry.into_path());
            }
        }
    }
    found
}

/// Searches the PC for save folder candidates of `game`.
pub fn scan_pc(game: &str) -> Vec<Cand> {
    let game_tokens = tokens(game);
    let steam = steam_names();
    let mut candidates = Vec::new();
    for dir in find_interesting_pc_dirs(&game_tokens) {
        let (mtime, files) = pc_newest(&dir, PC_FILE_COUNT_LIMIT);
        if files == 0 {
            continue;
        }
        let slashed_path = util::slashed(&dir);
        let lowercase = slashed_path.to_lowercase();
        candidates.push(Cand {
            note: annotate(&slashed_path, &steam),
            matched: game_tokens.iter().any(|token| lowercase.contains(token.as_str())),
            when: when(mtime),
            path: dir.to_string_lossy().into_owned(),
            mtime,
            files,
        });
    }
    rank(candidates)
}

/// Root paths of all Thor storages (`/sdcard`, `/storage/XXXX-XXXX`, ...).
pub(super) fn thor_roots() -> Vec<String> {
    adb::storages().into_iter().map(|(_, path)| path).collect()
}

/// All folders inside the private data folders of debuggable apps (only there `run-as` works).
fn private_dirs() -> Vec<String> {
    let script = format!(
        "for p in $(pm list packages -3 | cut -d: -f2); do run-as \"$p\" find \"/data/data/$p\" -maxdepth {THOR_MAX_DEPTH} -type d 2>/dev/null; done\n"
    );
    sh_script(&script).stdout.lines().map(String::from).collect()
}

/// For each Thor path: `(path, number of files, newest modification time)`, all in one shell
/// session. Paths without files are left out.
///
/// With `files_too == true`, a path may also be a single file (counted as 1 file).
pub(super) fn thor_stats(paths: &[String], files_too: bool) -> Vec<(String, u64, f64)> {
    let mut script = String::new();
    for path in paths {
        script += &tcmd(path, &thor_stats_command(path, files_too));
        script.push('\n');
    }
    sh_script(&script)
        .stdout
        .lines()
        .filter_map(parse_thor_stats_line)
        .filter(|(_, file_count, _)| *file_count > 0)
        .collect()
}

/// Shell snippet that prints `"<file count>|<newest mtime>|<path>"` for one path.
fn thor_stats_command(path: &str, files_too: bool) -> String {
    if files_too {
        format!(
            "d={}; if [ -d \"$d\" ]; then n=$(find \"$d\" -type f 2>/dev/null | wc -l); \
             t=$(find \"$d\" -type f -exec stat -c %Y {{}} + 2>/dev/null | sort -n | tail -1); \
             else n=1; t=$(stat -c %Y \"$d\"); fi; echo \"$n|${{t:-0}}|$d\"",
            shell_quote(path)
        )
    } else {
        format!(
            "d={}; n=$(find \"$d\" -type f 2>/dev/null | wc -l); \
             t=$(find \"$d\" -type f -exec stat -c %Y {{}} + 2>/dev/null | sort -n | tail -1); echo \"$n|${{t:-0}}|$d\"",
            shell_quote(path)
        )
    }
}

/// Parses `"<count>|<mtime>|<path>"`. `None` for lines that don't fit.
fn parse_thor_stats_line(line: &str) -> Option<(String, u64, f64)> {
    let mut fields = line.splitn(3, '|');
    let file_count = fields.next()?.trim().parse::<u64>().ok()?;
    let mtime = fields.next()?.trim().parse::<f64>().unwrap_or(0.0);
    Some((fields.next()?.to_string(), file_count, mtime))
}

/// All interesting folders on the Thor: shared storage (one `find` over all roots) plus the
/// private folders of debuggable apps. Caches and shader caches are skipped.
fn find_interesting_thor_dirs(game_tokens: &[String]) -> Vec<String> {
    let prune_names: Vec<String> = THOR_PRUNE.iter().map(|name| format!("-name {}", shell_quote(name))).collect();
    let prune = prune_names.join(" -o ");
    let quoted_roots: Vec<String> = thor_roots().iter().map(|root| shell_quote(&format!("{root}/"))).collect();
    let roots = quoted_roots.join(" ");
    let find_output = sh(&format!(
        "find {roots} -maxdepth {THOR_MAX_DEPTH} \\( {prune} \\) -prune -o -type d -print 2>/dev/null"
    ))
    .stdout;

    let mut found: Vec<String> = Vec::new();
    for line in find_output.lines().map(String::from).chain(private_dirs()) {
        let path = line.trim_end_matches('/').replace("//", "/");
        let lowercase = path.to_lowercase();
        if path.is_empty() || lowercase.contains("/cache") || lowercase.contains("/shader") {
            continue;
        }
        if interesting(&path, game_tokens) {
            found.push(path);
        }
    }
    found
}

/// Searches the Thor for save folder candidates of `game`.
pub fn scan_thor(game: &str) -> Vec<Cand> {
    let game_tokens = tokens(game);
    let mut found = find_interesting_thor_dirs(&game_tokens);
    found.truncate(MAX_THOR_FOLDERS);
    if found.is_empty() {
        return vec![];
    }
    let candidates = thor_stats(&found, false)
        .into_iter()
        .map(|(path, files, mtime)| {
            let lowercase = path.to_lowercase();
            Cand {
                matched: game_tokens.iter().any(|token| lowercase.contains(token.as_str())),
                when: when(mtime),
                path,
                mtime,
                files,
                note: String::new(),
            }
        })
        .collect();
    rank(candidates)
}
