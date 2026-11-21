//! Game-Libraries (ES-DE `ROMs`-Ordner) finden und gedroppte Dateien einem System zuordnen.
//!
//! Welche Endung zu welchem System gehört, steht in den `systeminfo.txt` von ES-DE
//! auf dem Thor. Dazu kommen Header-Checks und die Title-ID bei Switch-Dateien.

use crate::adb::{q, sh};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::OnceLock;

/// Bei mehrdeutigen Endungen gewinnen Systeme mit Games, danach diese Reihenfolge.
const PREFERRED: &[&str] = &[
    "switch", "n3ds", "nds", "ps2", "psp", "psx", "gc", "wii", "wiiu", "gba", "gbc", "gb", "n64",
    "snes", "nes", "megadrive", "genesis", "dreamcast", "saturn", "ps3", "psvita",
];
const ARCHIVES: &[&str] = &[".zip", ".7z", ".rar"];

#[derive(Serialize, Clone, Default)]
pub struct System {
    pub full: String,
    pub exts: BTreeSet<String>,
    pub count: u32,
}

#[derive(Serialize, Clone)]
pub struct Library {
    pub path: String,
    pub label: String,
    pub systems: BTreeMap<String, System>,
    pub total: u32,
}

impl Library {
    pub fn full(&self, name: &str) -> String {
        match self.systems.get(name) {
            Some(s) if !s.full.is_empty() => s.full.clone(),
            _ => name.to_string(),
        }
    }
}

pub fn find_libraries(stores: &[(String, String)]) -> Vec<Library> {
    let mut libs = Vec::new();
    for (label, root) in stores {
        let lib_path = format!("{root}/ROMs");
        let out = sh(&format!(
            "cd {} 2>/dev/null || exit 1; echo @@DIRS; ls -1p | grep '/$'; \
             echo @@INFO; grep -A1 -e '^Full system name' -e '^Supported file ext' */systeminfo.txt; \
             echo @@COUNTS; find . -mindepth 2 -maxdepth 2 ! -name systeminfo.txt | cut -d/ -f2 | sort | uniq -c",
            q(&lib_path)
        ))
        .stdout;
        if !out.contains("@@DIRS") {
            continue;
        }
        let mut systems: BTreeMap<String, System> = BTreeMap::new();
        let (mut section, mut key) = ("", String::new());
        for line in out.lines() {
            if let Some(s) = line.strip_prefix("@@") {
                section = match s { "DIRS" => "DIRS", "INFO" => "INFO", _ => "COUNTS" };
                continue;
            }
            match section {
                "DIRS" => {
                    if let Some(n) = line.strip_suffix('/') {
                        systems.entry(n.to_string()).or_default();
                    }
                }
                "INFO" => {
                    if let Some((name, rest)) = line.split_once("/systeminfo.txt") {
                        let sys = systems.entry(name.to_string()).or_default();
                        if let Some(k) = rest.strip_prefix(':') {
                            key = k.to_string();
                        } else if let Some(v) = rest.strip_prefix('-') {
                            let v = v.trim();
                            if key.starts_with("Full") {
                                sys.full = v.to_string();
                            } else if key.starts_with("Supported") {
                                sys.exts = v.split_whitespace().map(|e| e.to_lowercase()).collect();
                            }
                        }
                    }
                }
                _ => {
                    let t = line.trim();
                    if let Some((n, name)) = t.split_once(char::is_whitespace) {
                        if let (Ok(n), Some(s)) = (n.parse::<u32>(), systems.get_mut(name.trim())) {
                            s.count = n;
                        }
                    }
                }
            }
        }
        if !systems.is_empty() {
            let total = systems.values().map(|s| s.count).sum();
            libs.push(Library { path: lib_path, label: label.clone(), systems, total });
        }
    }
    libs.sort_by(|a, b| b.total.cmp(&a.total));
    libs
}

// --- Erkennung ---------------------------------------------------------------

fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn at(b: &[u8], off: usize, len: usize) -> Option<&[u8]> {
    b.get(off..off + len)
}

/// System am Datei-Header erkennen.
fn sniff(path: &Path) -> Option<(&'static str, &'static str)> {
    let mut f = File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let mut head = vec![0u8; 0x200];
    let n = f.read(&mut head).ok()?;
    head.truncate(n);
    let mut pvd = vec![0u8; 0x40];
    let pn = if size > 0x8040 && f.seek(SeekFrom::Start(0x8000)).is_ok() { f.read(&mut pvd).unwrap_or(0) } else { 0 };
    pvd.truncate(pn);

    if at(&head, 0, 4) == Some(b"PFS0") || at(&head, 0x100, 4) == Some(b"HEAD") {
        return Some(("switch", "Switch header"));
    }
    if at(&head, 0x100, 4) == Some(b"NCSD") {
        return Some(("n3ds", "3DS header"));
    }
    if matches!(at(&head, 0, 4), Some(b"WIA\x01") | Some(b"RVZ\x01")) {
        if let Some(d) = at(&head, 0x48, 4) {
            match u32::from_be_bytes([d[0], d[1], d[2], d[3]]) {
                1 => return Some(("gc", "GameCube header")),
                2 => return Some(("wii", "Wii header")),
                _ => {}
            }
        }
    }
    if at(&head, 0x1C, 4) == Some(&[0xC2, 0x33, 0x9F, 0x3D]) {
        return Some(("gc", "GameCube header"));
    }
    if at(&head, 0x18, 4) == Some(&[0x5D, 0x1C, 0x9E, 0xA3]) || at(&head, 0, 4) == Some(b"WBFS") {
        return Some(("wii", "Wii header"));
    }
    if at(&head, 0, 4) == Some(b"CISO") {
        return Some(("psp", "CSO image (usually PSP)"));
    }
    if at(&pvd, 1, 5) == Some(b"CD001") {
        let sysid = String::from_utf8_lossy(at(&pvd, 8, 32).unwrap_or_default()).trim().to_string();
        if sysid.starts_with("PSP GAME") {
            return Some(("psp", "PSP-ISO"));
        }
        if sysid.starts_with("PLAYSTATION") {
            return Some(if size > 800 * 1024 * 1024 { ("ps2", "PS2-ISO") } else { ("psx", "PS1 image") });
        }
    }
    None
}

/// Name der größten Datei in einem Archiv.
fn archive_inner(path: &Path) -> Option<String> {
    match ext_of(&file_name(path)).as_str() {
        ".zip" => {
            let mut z = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
            let mut best: Option<(u64, String)> = None;
            for i in 0..z.len() {
                let e = z.by_index_raw(i).ok()?;
                if e.is_dir() {
                    continue;
                }
                if best.as_ref().map_or(true, |(s, _)| e.size() > *s) {
                    best = Some((e.size(), e.name().to_string()));
                }
            }
            best.map(|b| b.1)
        }
        ".7z" => {
            let r = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty()).ok()?;
            r.archive()
                .files
                .iter()
                .filter(|f| f.has_stream())
                .max_by_key(|f| f.size())
                .map(|f| f.name().to_string())
        }
        _ => None,
    }
}

fn largest_file(dir: &Path) -> Option<std::path::PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .max_by_key(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .map(|e| e.into_path())
}

#[derive(Serialize, Clone, Default)]
pub struct Guess {
    pub system: Option<String>,
    pub sub: String,
    pub reason: String,
    pub candidates: Vec<String>,
    pub sure: bool,
    /// Archiv kann gepackt bleiben (System liest .zip/.7z selbst, z. B. Arcade)
    pub packed_ok: bool,
}

fn rank(lib: &Library, names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = names.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
    v.sort_by_key(|n| {
        let pref = PREFERRED.iter().position(|p| p == n).unwrap_or(999);
        (lib.systems.get(n).map_or(true, |s| s.count == 0), pref, n.clone())
    });
    v
}

fn switch_id(name: &str) -> Option<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"[\[(](0100[0-9A-Fa-f]{12})[\])]").unwrap());
    re.captures(name).map(|c| c[1].to_uppercase())
}

pub fn classify(path: &Path, lib: &Library) -> Guess {
    let mut g = classify_inner(path, lib);
    g.packed_ok = true;
    if let Some(sys) = &g.system {
        let ext = ext_of(&file_name(path));
        if path.is_file() && ARCHIVES.contains(&ext.as_str()) {
            if let Some(inner) = archive_inner(path) {
                if !ARCHIVES.contains(&ext_of(&inner).as_str()) {
                    g.packed_ok = lib.systems.get(sys).map_or(false, |s| s.exts.contains(&ext)) && g.sub.is_empty();
                }
            }
        }
    }
    g
}

fn classify_inner(path: &Path, lib: &Library) -> Guess {
    let name = file_name(path);
    let mut probe = path.to_path_buf();
    let mut via = String::new();
    if path.is_dir() {
        match largest_file(path) {
            Some(p) => {
                via = format!(" (contents: {})", file_name(&p));
                probe = p;
            }
            None => return Guess { reason: "Empty folder".into(), ..Default::default() },
        }
    }

    let mut ext = ext_of(&file_name(&probe));
    let mut probe_name = file_name(&probe);
    let mut archive_ext = String::new();
    if ARCHIVES.contains(&ext.as_str()) && probe.is_file() {
        if let Some(inner) = archive_inner(&probe) {
            let iext = ext_of(&inner);
            if !ARCHIVES.contains(&iext.as_str()) {
                let base = inner.rsplit('/').next().unwrap_or(&inner).to_string();
                via = format!(" (in archive: {base})");
                archive_ext = std::mem::replace(&mut ext, iext);
                probe_name = base;
            }
        }
    }

    let cands: Vec<String> = lib
        .systems
        .iter()
        .filter(|(_, s)| !ext.is_empty() && s.exts.contains(&ext))
        .map(|(n, _)| n.clone())
        .collect();

    // Switch: Title-ID im Namen, Updates/DLC enden nicht auf 000
    let sw_ext = [".nsp", ".xci", ".nsz", ".xcz"].contains(&ext.as_str());
    if let Some(tid) = switch_id(&name).or_else(|| switch_id(&probe_name)) {
        if lib.systems.contains_key("switch") && (sw_ext || cands.is_empty()) {
            let tail = &tid[tid.len() - 4..];
            if !tid.ends_with("000") {
                let kind = if tid.ends_with("800") { "Update" } else { "DLC/Update" };
                return Guess {
                    system: Some("switch".into()),
                    sub: "update".into(),
                    reason: format!("Switch {kind} (title ID …{tail})"),
                    candidates: vec!["switch".into()],
                    sure: true,
                    ..Default::default()
                };
            }
            return Guess {
                system: Some("switch".into()),
                reason: format!("Switch game (title ID …{tail})"),
                candidates: vec!["switch".into()],
                sure: true,
                ..Default::default()
            };
        }
    }

    if probe.is_file() && archive_ext.is_empty() {
        if let Some((sys, why)) = sniff(&probe) {
            if lib.systems.contains_key(sys) {
                let mut all = cands.clone();
                all.push(sys.to_string());
                return Guess {
                    system: Some(sys.into()),
                    reason: format!("{why}{via}"),
                    candidates: rank(lib, all),
                    sure: true,
                    ..Default::default()
                };
            }
        }
    }

    if cands.is_empty() {
        let what = if ext.is_empty() { "this file".to_string() } else { ext.clone() };
        return Guess { reason: format!("No system knows {what}{via}"), ..Default::default() };
    }
    let ranked = rank(lib, cands);
    let top = ranked[0].clone();
    let warn = if !archive_ext.is_empty() && !lib.systems[&top].exts.contains(&archive_ext) {
        format!(" – {archive_ext} may not be read there, consider extracting")
    } else {
        String::new()
    };
    if ranked.len() == 1 {
        return Guess {
            system: Some(top),
            reason: format!("Extension {ext}{via}{warn}"),
            candidates: ranked,
            sure: true,
            ..Default::default()
        };
    }
    if ranked.len() <= 5 || lib.systems[&top].count > 0 {
        let others = ranked[1..ranked.len().min(4)].join(", ") + if ranked.len() > 4 { " …" } else { "" };
        return Guess {
            system: Some(top),
            reason: format!("Unsure: {ext} also fits {others}{via}{warn}"),
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
