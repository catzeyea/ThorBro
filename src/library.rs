//! Finds game libraries (ES-DE `ROMs` folders) on the Thor and guesses which system a dropped
//! file belongs to.
//!
//! Which file extension belongs to which system is read from ES-DE's `systeminfo.txt` files on
//! the Thor. On top of that come header checks (magic bytes) and the title ID in Switch file names.
//!
//! Used by `commands.rs`: `places` / `refresh_libs` call `find_libraries`, `classify` calls
//! `classify` for every dropped file. Talks to the Thor through `adb.rs`.

use crate::adb::{sh, shell_quote};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// For ambiguous extensions, systems that already contain games win, then this order.
const PREFERRED: &[&str] = &[
    "switch", "n3ds", "nds", "ps2", "psp", "psx", "gc", "wii", "wiiu", "gba", "gbc", "gb", "n64",
    "snes", "nes", "megadrive", "genesis", "dreamcast", "saturn", "ps3", "psvita",
];

/// Rank for systems that are not in `PREFERRED` (sorted after all preferred ones).
const NOT_PREFERRED: usize = 999;

/// Archive types we can look into to find out what game is inside.
const ARCHIVES: &[&str] = &[".zip", ".7z", ".rar"];

/// Extensions of Switch game/update files.
const SWITCH_EXTS: [&str; 4] = [".nsp", ".xci", ".nsz", ".xcz"];

/// Number of bytes read from the start of a file for header detection.
const HEADER_SIZE: usize = 0x200;

/// Offset and size of the ISO 9660 "primary volume descriptor" (identifies CD/DVD images).
const ISO_PVD_OFFSET: u64 = 0x8000;
const ISO_PVD_SIZE: usize = 0x40;

/// PlayStation ISOs bigger than this are PS2 games (DVD), smaller ones PS1 (CD).
const PS2_MIN_ISO_SIZE: u64 = 800 * 1024 * 1024;

/// If more systems than this accept an extension and none has games yet, we don't guess.
const MAX_UNSURE_CANDIDATES: usize = 5;

/// Marker lines in the output of the shell script in `list_library`.
const MARKER_PREFIX: &str = "@@";

/// One system folder inside a library (e.g. `ROMs/snes`), as described by its `systeminfo.txt`.
///
/// `#[derive(...)]` lets the compiler generate code: `Serialize` = JSON conversion for the UI,
/// `Clone` = `.clone()` deep copy, `Default` = an "all empty / zero" constructor.
#[derive(Serialize, Clone, Default)]
pub struct System {
    /// Display name, e.g. "Nintendo SNES (Super Nintendo)". May be empty.
    pub full: String,
    /// Supported extensions, lowercase with dot.
    pub exts: BTreeSet<String>,
    /// Number of entries (games) in the folder.
    pub count: u32,
}

/// An ES-DE `ROMs` folder on one storage (internal or SD card).
#[derive(Serialize, Clone)]
pub struct Library {
    pub path: String,
    pub label: String,
    /// System folder name (e.g. "snes") -> info. `BTreeMap` is a sorted map like `std::map`.
    pub systems: BTreeMap<String, System>,
    /// Total number of games over all systems.
    pub total: u32,
}

impl Library {
    /// Display name of a system, or its folder name if it has none.
    pub fn full(&self, name: &str) -> String {
        match self.systems.get(name) {
            // `Some(s) if ...` = match only when the guard condition is true as well.
            Some(system) if !system.full.is_empty() => system.full.clone(),
            _ => name.to_string(),
        }
    }
}

/// Looks for a `ROMs` folder on every storage and reads its systems.
///
/// `stores` are `(label, root path)` pairs from `adb::storages()`. The result is sorted by number
/// of games (largest first), so the "main" library is at index 0.
pub fn find_libraries(stores: &[(String, String)]) -> Vec<Library> {
    let mut libraries = Vec::new();
    for (label, root) in stores {
        let library_path = format!("{root}/ROMs");
        if let Some(systems) = list_library(&library_path) {
            if !systems.is_empty() {
                let total = systems.values().map(|system| system.count).sum();
                libraries.push(Library { path: library_path, label: label.clone(), systems, total });
            }
        }
    }
    libraries.sort_by_key(|library| std::cmp::Reverse(library.total));
    libraries
}

/// Reads all systems of one library with a single adb call.
/// Returns `None` if the folder doesn't exist.
///
/// The shell script prints three sections, separated by marker lines:
/// - `@@DIRS`   – the system folders (`ls -1p`, folders end with `/`)
/// - `@@INFO`   – name and extensions from every `systeminfo.txt` (via `grep -A1`)
/// - `@@COUNTS` – number of entries per system folder (`uniq -c`)
fn list_library(library_path: &str) -> Option<BTreeMap<String, System>> {
    let script = format!(
        "cd {} 2>/dev/null || exit 1; echo @@DIRS; ls -1p | grep '/$'; \
         echo @@INFO; grep -A1 -e '^Full system name' -e '^Supported file ext' */systeminfo.txt; \
         echo @@COUNTS; find . -mindepth 2 -maxdepth 2 ! -name systeminfo.txt | cut -d/ -f2 | sort | uniq -c",
        shell_quote(library_path)
    );
    let output = sh(&script).stdout;
    if !output.contains("@@DIRS") {
        return None;
    }
    Some(parse_library_listing(&output))
}

/// Which part of the `list_library` output we are currently reading.
///
/// A Rust `enum` like this one is the same as a C++ `enum class`. (Rust enums can also carry
/// data per variant, see `Option` / `Result`, but this one doesn't.)
#[derive(Clone, Copy, PartialEq)]
enum Section {
    Dirs,
    Info,
    Counts,
}

/// Parses the three-section output of the `list_library` script into systems.
fn parse_library_listing(output: &str) -> BTreeMap<String, System> {
    let mut systems: BTreeMap<String, System> = BTreeMap::new();
    // Lines before the first marker (normally there are none) are parsed like counts.
    let mut section = Section::Counts;
    // In the INFO section the key ("Full system name:" / "Supported file ext...") comes on one
    // line and its value on the next, so we remember the last key.
    let mut info_key = String::new();
    for line in output.lines() {
        if let Some(marker) = line.strip_prefix(MARKER_PREFIX) {
            section = match marker {
                "DIRS" => Section::Dirs,
                "INFO" => Section::Info,
                _ => Section::Counts,
            };
            continue;
        }
        match section {
            Section::Dirs => parse_dir_line(line, &mut systems),
            Section::Info => parse_info_line(line, &mut info_key, &mut systems),
            Section::Counts => parse_count_line(line, &mut systems),
        }
    }
    systems
}

/// `snes/` -> registers system "snes".
fn parse_dir_line(line: &str, systems: &mut BTreeMap<String, System>) {
    if let Some(name) = line.strip_suffix('/') {
        // `entry(..).or_default()` = "get the value, insert a default one if missing",
        // like `map[key]` in C++.
        systems.entry(name.to_string()).or_default();
    }
}

/// Parses one line of `grep -A1` output. Key lines look like `snes/systeminfo.txt:Full system name:`
/// and value lines like `snes/systeminfo.txt-Nintendo SNES`.
fn parse_info_line(line: &str, info_key: &mut String, systems: &mut BTreeMap<String, System>) {
    let Some((name, rest)) = line.split_once("/systeminfo.txt") else { return };
    let system = systems.entry(name.to_string()).or_default();
    if let Some(key) = rest.strip_prefix(':') {
        *info_key = key.to_string();
    } else if let Some(value) = rest.strip_prefix('-') {
        let value = value.trim();
        if info_key.starts_with("Full") {
            system.full = value.to_string();
        } else if info_key.starts_with("Supported") {
            system.exts = value.split_whitespace().map(|ext| ext.to_lowercase()).collect();
        }
    }
}

/// `   12 snes` -> system "snes" has 12 entries.
fn parse_count_line(line: &str, systems: &mut BTreeMap<String, System>) {
    let Some((count, name)) = line.trim().split_once(char::is_whitespace) else { return };
    if let (Ok(count), Some(system)) = (count.parse::<u32>(), systems.get_mut(name.trim())) {
        system.count = count;
    }
}

// --- Detection -------------------------------------------------------------------------------

/// Lowercase extension of a file name including the dot (`"Game.NSP"` -> `".nsp"`), or `""`.
fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

/// File name of a path as `String` (empty if there is none).
fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `len` bytes of `bytes` starting at `offset`, or `None` if the buffer is too short.
/// (Safe slicing: a plain `&bytes[a..b]` would panic when out of range.)
fn bytes_at(bytes: &[u8], offset: usize, len: usize) -> Option<&[u8]> {
    bytes.get(offset..offset + len)
}

/// Reads the beginning of a file and, for big enough files, the ISO 9660 volume descriptor.
/// Returns `(header, iso_descriptor, file_size)`; the descriptor is empty for small files.
fn read_headers(path: &Path) -> Option<(Vec<u8>, Vec<u8>, u64)> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();

    let mut header = vec![0u8; HEADER_SIZE];
    let header_len = file.read(&mut header).ok()?;
    header.truncate(header_len);

    let mut iso_descriptor = vec![0u8; ISO_PVD_SIZE];
    let has_room_for_descriptor = size > ISO_PVD_OFFSET + ISO_PVD_SIZE as u64;
    let descriptor_len = if has_room_for_descriptor && file.seek(SeekFrom::Start(ISO_PVD_OFFSET)).is_ok() {
        file.read(&mut iso_descriptor).unwrap_or(0)
    } else {
        0
    };
    iso_descriptor.truncate(descriptor_len);
    Some((header, iso_descriptor, size))
}

/// Recognizes the system from the file header (magic bytes).
/// Returns `(system folder name, human-readable reason)` or `None`.
fn sniff(path: &Path) -> Option<(&'static str, &'static str)> {
    let (header, iso_descriptor, size) = read_headers(path)?;
    // `.or_else(..)` only runs the second check if the first one returned `None`.
    sniff_console_header(&header).or_else(|| sniff_iso(&iso_descriptor, size))
}

/// Magic-byte checks for cartridge / disc image formats that have their own header.
fn sniff_console_header(header: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes_at(header, 0, 4) == Some(b"PFS0") || bytes_at(header, 0x100, 4) == Some(b"HEAD") {
        return Some(("switch", "Switch header"));
    }
    if bytes_at(header, 0x100, 4) == Some(b"NCSD") {
        return Some(("n3ds", "3DS header"));
    }
    // WIA / RVZ (Dolphin's compressed formats) store the disc type at offset 0x48.
    if matches!(bytes_at(header, 0, 4), Some(b"WIA\x01") | Some(b"RVZ\x01")) {
        if let Some(disc_type) = bytes_at(header, 0x48, 4) {
            match u32::from_be_bytes([disc_type[0], disc_type[1], disc_type[2], disc_type[3]]) {
                1 => return Some(("gc", "GameCube header")),
                2 => return Some(("wii", "Wii header")),
                _ => {}
            }
        }
    }
    if bytes_at(header, 0x1C, 4) == Some(&[0xC2, 0x33, 0x9F, 0x3D]) {
        return Some(("gc", "GameCube header"));
    }
    if bytes_at(header, 0x18, 4) == Some(&[0x5D, 0x1C, 0x9E, 0xA3]) || bytes_at(header, 0, 4) == Some(b"WBFS") {
        return Some(("wii", "Wii header"));
    }
    if bytes_at(header, 0, 4) == Some(b"CISO") {
        return Some(("psp", "CSO image (usually PSP)"));
    }
    None
}

/// Checks the ISO 9660 volume descriptor for a PSP / PlayStation system ID.
fn sniff_iso(iso_descriptor: &[u8], size: u64) -> Option<(&'static str, &'static str)> {
    if bytes_at(iso_descriptor, 1, 5) != Some(b"CD001") {
        return None;
    }
    let system_id = String::from_utf8_lossy(bytes_at(iso_descriptor, 8, 32).unwrap_or_default()).trim().to_string();
    if system_id.starts_with("PSP GAME") {
        return Some(("psp", "PSP-ISO"));
    }
    if system_id.starts_with("PLAYSTATION") {
        return Some(if size > PS2_MIN_ISO_SIZE { ("ps2", "PS2-ISO") } else { ("psx", "PS1 image") });
    }
    None
}

/// Name of the largest file inside an archive (zip and 7z only).
/// That file is most likely the game, so its extension tells us the system.
fn archive_inner(path: &Path) -> Option<String> {
    match ext_of(&file_name(path)).as_str() {
        ".zip" => {
            let mut zip = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
            let mut largest: Option<(u64, String)> = None;
            for index in 0..zip.len() {
                let entry = zip.by_index_raw(index).ok()?;
                if entry.is_dir() {
                    continue;
                }
                let is_larger = largest.as_ref().is_none_or(|(largest_size, _)| entry.size() > *largest_size);
                if is_larger {
                    largest = Some((entry.size(), entry.name().to_string()));
                }
            }
            largest.map(|(_, name)| name)
        }
        ".7z" => {
            let reader = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty()).ok()?;
            reader
                .archive()
                .files
                .iter()
                .filter(|file| file.has_stream())
                .max_by_key(|file| file.size())
                .map(|file| file.name().to_string())
        }
        _ => None,
    }
}

/// Largest file anywhere below `dir` (a dropped folder is judged by its biggest file).
fn largest_file(dir: &Path) -> Option<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .max_by_key(|entry| entry.metadata().map(|meta| meta.len()).unwrap_or(0))
        .map(|entry| entry.into_path())
}

/// The result of guessing where a dropped file belongs. Sent to the UI as JSON.
#[derive(Serialize, Clone, Default)]
pub struct Guess {
    /// Best system folder, or `None` if we can't tell.
    pub system: Option<String>,
    /// Sub folder inside the system folder (`"update"` for Switch updates/DLC), else empty.
    pub sub: String,
    /// Explanation shown to the user.
    pub reason: String,
    /// All systems that could fit, best first.
    pub candidates: Vec<String>,
    /// `true` = confident guess, `false` = "unsure", shown differently in the UI.
    pub sure: bool,
    /// The archive can stay packed (the system reads .zip/.7z itself, e.g. arcade).
    pub packed_ok: bool,
}

/// Sorts system names: systems with games first, then by `PREFERRED`, then alphabetically.
/// Duplicates are removed.
fn rank(lib: &Library, names: impl IntoIterator<Item = String>) -> Vec<String> {
    // Collecting into a `BTreeSet` removes duplicates.
    let unique: BTreeSet<String> = names.into_iter().collect();
    let mut ranked: Vec<String> = unique.into_iter().collect();
    ranked.sort_by_key(|name| {
        let preference = PREFERRED.iter().position(|preferred| preferred == name).unwrap_or(NOT_PREFERRED);
        let has_no_games = lib.systems.get(name).is_none_or(|system| system.count == 0);
        // Tuples compare element by element, like `std::tuple` in C++. `false < true`, so
        // systems *with* games come first.
        (has_no_games, preference, name.clone())
    });
    ranked
}

/// Switch title ID (`0100` + 12 hex digits in `[...]` or `(...)`) from a file name, uppercase.
fn switch_id(name: &str) -> Option<String> {
    static TITLE_ID_RE: OnceLock<regex::Regex> = OnceLock::new();
    let regex = TITLE_ID_RE.get_or_init(|| regex::Regex::new(r"[\[(](0100[0-9A-Fa-f]{12})[\])]").unwrap());
    regex.captures(name).map(|captures| captures[1].to_uppercase())
}

/// Guesses the target system for a dropped file or folder.
///
/// Also decides `packed_ok`: an archive may stay packed if the chosen system lists the archive
/// extension itself (e.g. arcade ROM sets) and it isn't a Switch update.
pub fn classify(path: &Path, lib: &Library) -> Guess {
    let mut guess = classify_inner(path, lib);
    guess.packed_ok = true;
    if let Some(system) = &guess.system {
        let ext = ext_of(&file_name(path));
        if path.is_file() && ARCHIVES.contains(&ext.as_str()) {
            if let Some(inner) = archive_inner(path) {
                if !ARCHIVES.contains(&ext_of(&inner).as_str()) {
                    let system_reads_archive = lib.systems.get(system).is_some_and(|s| s.exts.contains(&ext));
                    guess.packed_ok = system_reads_archive && guess.sub.is_empty();
                }
            }
        }
    }
    guess
}

/// The file we actually look at to decide the system. For a folder that is its largest file;
/// for an archive it is the largest file inside (described by name and extension only).
struct Probe {
    /// File on disk that is examined (the archive itself for archives).
    path: PathBuf,
    /// Name of the examined file (for archives: the inner file's name).
    name: String,
    /// Extension used for matching (for archives: the inner file's extension).
    ext: String,
    /// Extension of the archive if we looked inside one, else empty.
    archive_ext: String,
    /// Text appended to reasons, e.g. `" (in archive: game.iso)"`.
    via: String,
}

/// Builds the `Probe` for `path`. Returns `None` for an empty folder.
fn probe_for(path: &Path) -> Option<Probe> {
    let mut probe_path = path.to_path_buf();
    let mut via = String::new();
    if path.is_dir() {
        let largest = largest_file(path)?;
        via = format!(" (contents: {})", file_name(&largest));
        probe_path = largest;
    }

    let mut ext = ext_of(&file_name(&probe_path));
    let mut name = file_name(&probe_path);
    let mut archive_ext = String::new();
    if ARCHIVES.contains(&ext.as_str()) && probe_path.is_file() {
        if let Some(inner) = archive_inner(&probe_path) {
            let inner_ext = ext_of(&inner);
            // An archive inside an archive tells us nothing, keep looking at the outer one.
            if !ARCHIVES.contains(&inner_ext.as_str()) {
                let inner_base = inner.rsplit('/').next().unwrap_or(&inner).to_string();
                via = format!(" (in archive: {inner_base})");
                // `mem::replace` puts the new value in and hands back the old one
                // (like `std::exchange` in C++).
                archive_ext = std::mem::replace(&mut ext, inner_ext);
                name = inner_base;
            }
        }
    }
    Some(Probe { path: probe_path, name, ext, archive_ext, via })
}

/// The actual guessing, in order of confidence:
/// 1. Switch title ID in the name, 2. file header, 3. file extension.
fn classify_inner(path: &Path, lib: &Library) -> Guess {
    let Some(probe) = probe_for(path) else {
        return Guess { reason: "Empty folder".into(), ..Default::default() };
    };

    // All systems that list this extension in their systeminfo.txt.
    let candidates: Vec<String> = lib
        .systems
        .iter()
        .filter(|(_, system)| !probe.ext.is_empty() && system.exts.contains(&probe.ext))
        .map(|(name, _)| name.clone())
        .collect();

    if let Some(guess) = guess_switch(lib, &file_name(path), &probe, &candidates) {
        return guess;
    }
    if let Some(guess) = guess_by_header(lib, &probe, &candidates) {
        return guess;
    }
    guess_by_extension(lib, &probe, candidates)
}

/// Switch detection via the title ID in the file name.
/// Base games end in `000`, updates in `800`, everything else is DLC.
fn guess_switch(lib: &Library, dropped_name: &str, probe: &Probe, candidates: &[String]) -> Option<Guess> {
    let title_id = switch_id(dropped_name).or_else(|| switch_id(&probe.name))?;
    let is_switch_ext = SWITCH_EXTS.contains(&probe.ext.as_str());
    if !lib.systems.contains_key("switch") || !(is_switch_ext || candidates.is_empty()) {
        return None;
    }
    let tail = &title_id[title_id.len() - 4..];
    if !title_id.ends_with("000") {
        let kind = if title_id.ends_with("800") { "Update" } else { "DLC/Update" };
        return Some(Guess {
            system: Some("switch".into()),
            sub: "update".into(),
            reason: format!("Switch {kind} (title ID …{tail})"),
            candidates: vec!["switch".into()],
            sure: true,
            // `..Default::default()` fills all remaining fields with their default values.
            ..Default::default()
        });
    }
    Some(Guess {
        system: Some("switch".into()),
        reason: format!("Switch game (title ID …{tail})"),
        candidates: vec!["switch".into()],
        sure: true,
        ..Default::default()
    })
}

/// Detection via magic bytes. Only for real files, not for files inside an archive.
fn guess_by_header(lib: &Library, probe: &Probe, candidates: &[String]) -> Option<Guess> {
    if !probe.path.is_file() || !probe.archive_ext.is_empty() {
        return None;
    }
    let (system, why) = sniff(&probe.path)?;
    if !lib.systems.contains_key(system) {
        return None;
    }
    let mut all = candidates.to_vec();
    all.push(system.to_string());
    Some(Guess {
        system: Some(system.into()),
        reason: format!("{why}{}", probe.via),
        candidates: rank(lib, all),
        sure: true,
        ..Default::default()
    })
}

/// Fallback: decide by file extension alone.
///
/// - one matching system: sure
/// - a few systems, or the best one already has games: best one, marked unsure
/// - many systems and none has games: no guess, the user must choose
fn guess_by_extension(lib: &Library, probe: &Probe, candidates: Vec<String>) -> Guess {
    let ext = &probe.ext;
    let via = &probe.via;
    if candidates.is_empty() {
        let what = if ext.is_empty() { "this file".to_string() } else { ext.clone() };
        return Guess { reason: format!("No system knows {what}{via}"), ..Default::default() };
    }
    let ranked = rank(lib, candidates);
    let top = ranked[0].clone();
    let archive_ext = &probe.archive_ext;
    let warning = if !archive_ext.is_empty() && !lib.systems[&top].exts.contains(archive_ext) {
        format!(" – {archive_ext} may not be read there, consider extracting")
    } else {
        String::new()
    };
    if ranked.len() == 1 {
        return Guess {
            system: Some(top),
            reason: format!("Extension {ext}{via}{warning}"),
            candidates: ranked,
            sure: true,
            ..Default::default()
        };
    }
    if ranked.len() <= MAX_UNSURE_CANDIDATES || lib.systems[&top].count > 0 {
        // Show up to three alternatives, "…" if there are more.
        let ellipsis = if ranked.len() > 4 { " …" } else { "" };
        let others = ranked[1..ranked.len().min(4)].join(", ") + ellipsis;
        return Guess {
            system: Some(top),
            reason: format!("Unsure: {ext} also fits {others}{via}{warning}"),
            candidates: ranked,
            ..Default::default()
        };
    }
    Guess {
        reason: format!("{ext} fits {} systems – please choose{via}", ranked.len()),
        candidates: ranked,
        ..Default::default()
    }
}
