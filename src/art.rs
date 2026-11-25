//! Game covers from the art the iiSU launcher already keeps on the Thor. ThorBro ships no art.
//!
//! Layout found on the Thor (step 1 of Update 9):
//! `<storage>/Android/media/com.iisulauncher/iiSULauncher/assets/media/roms/consoles/<system>/<ROM file name without extension>/icon.(jpg|png)`
//! (square, 512–1024 px). For a game folder iiSU nests one more level: `<system>/<folder>/<ROM stem>/`.
//!
//! - `index` lists every `icon.*` there in one adb call (with mtime and size).
//! - `ArtIndex::find` maps a ROM to its icon: by relative path, by file stem, and for Switch by
//!   the base title ID in the folder name (for games the emulator brings without a ROM).
//! - `fetch` copies icons into the PC cache (`~/.cache/thorBro/art/<md5>.<ext>`), the UI loads
//!   them through the `thorart://` protocol (`serve`, registered in `main.rs`).

use crate::adb::{self, sh_script, shell_quote};
use crate::emulators::{ids, is_allowed_thor_path, normalize_thor_path};
use md5::{Digest, Md5};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::OnceLock;

/// iiSU's art folder below a storage root.
const IISU_CONSOLES: &str = "Android/media/com.iisulauncher/iiSULauncher/assets/media/roms/consoles";

/// Icons bigger than this are not copied (the biggest seen on the Thor was 1.3 MB).
const MAX_ICON_BYTES: usize = 8 * 1024 * 1024;

/// One icon file on the Thor.
#[derive(Clone, Debug, PartialEq)]
pub struct ArtFile {
    pub path: String,
    pub mtime: u64,
    pub size: u64,
}

/// A cover for the UI: the Thor path, the cache key and whether it is already cached.
#[derive(Serialize, Clone, Debug)]
pub struct ArtRef {
    pub path: String,
    pub key: String,
    pub cached: bool,
}

/// All iiSU icons, by `(system, relative folder)`, `(system, last folder name)` and Switch title ID.
#[derive(Default)]
pub struct ArtIndex {
    by_rel: HashMap<(String, String), ArtFile>,
    by_name: HashMap<(String, String), ArtFile>,
    by_title_id: HashMap<String, ArtFile>,
}

impl ArtIndex {
    /// Builds the index from `stat -c '%Y %s %n'` lines.
    fn from_listing(output: &str) -> ArtIndex {
        let mut index = ArtIndex::default();
        let marker = format!("/{IISU_CONSOLES}/");
        for line in output.lines() {
            let mut fields = line.splitn(3, ' ');
            let (Some(mtime), Some(size), Some(path)) = (fields.next(), fields.next(), fields.next()) else { continue };
            let (Ok(mtime), Ok(size)) = (mtime.parse::<u64>(), size.parse::<u64>()) else { continue };
            let path = normalize_thor_path(&path.replace("//", "/"));
            let Some(rel) = path.split_once(&marker).map(|(_, rel)| rel.to_string()) else { continue };
            let parts: Vec<&str> = rel.split('/').collect();
            if parts.len() < 3 || !is_icon_name(parts[parts.len() - 1]) {
                continue;
            }
            let system = parts[0].to_lowercase();
            let folder = parts[1..parts.len() - 1].join("/");
            let name = parts[parts.len() - 2].to_string();
            let file = ArtFile { path: path.clone(), mtime, size };
            index.by_rel.entry((system.clone(), folder.to_lowercase())).or_insert_with(|| file.clone());
            index.by_name.entry((system.clone(), name.to_lowercase())).or_insert_with(|| file.clone());
            if system == "switch" {
                if let Some((title_id, ids::SwitchKind::Base)) = ids::switch_title(&name) {
                    index.by_title_id.entry(title_id).or_insert(file);
                }
            }
        }
        index
    }

    /// The icon of a game: `systems` are folder names to try (iiSU uses ES-DE system ids),
    /// `rom_rel` the ROM's path inside the system folder (`Game.nsp`, `Folder/Game.3ds`).
    pub fn find(&self, systems: &[&str], rom_rel: Option<&str>, title_id: Option<&str>) -> Option<&ArtFile> {
        for system in systems {
            let system = system.to_lowercase();
            if let Some(rel) = rom_rel {
                let stem = strip_ext(rel);
                if let Some(file) = self.by_rel.get(&(system.clone(), stem.to_lowercase())) {
                    return Some(file);
                }
                let last = stem.rsplit('/').next().unwrap_or(&stem).to_lowercase();
                if let Some(file) = self.by_name.get(&(system.clone(), last)) {
                    return Some(file);
                }
                let folder = rel.split('/').next().unwrap_or(rel).to_lowercase();
                if rel.contains('/') {
                    if let Some(file) = self.by_name.get(&(system.clone(), folder)) {
                        return Some(file);
                    }
                }
            }
        }
        title_id.and_then(|title_id| self.by_title_id.get(title_id))
    }
}

/// `"Folder/Game.3ds"` -> `"Folder/Game"`; a folder name without extension stays as it is.
fn strip_ext(rel: &str) -> String {
    match rel.rsplit_once('.') {
        Some((stem, ext)) if !ext.contains('/') && (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) => stem.to_string(),
        _ => rel.to_string(),
    }
}

fn icon_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^icon\.(jpg|jpeg|png|webp)$").unwrap())
}

fn is_icon_name(name: &str) -> bool {
    icon_re().is_match(name)
}

fn key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[0-9a-f]{32}\.(jpg|jpeg|png|webp)$").unwrap())
}

/// Lists all iiSU icons on every storage (one adb call).
pub fn index() -> ArtIndex {
    let mut script = String::new();
    for (_, root) in adb::storages() {
        let dir = format!("{root}/{IISU_CONSOLES}");
        script += &format!(
            "[ -d {d} ] && find {d} -mindepth 3 -maxdepth 4 -type f -iname 'icon.*' -exec stat -c '%Y %s %n' {{}} + 2>/dev/null\n",
            d = shell_quote(&dir)
        );
    }
    ArtIndex::from_listing(&sh_script(&script).stdout)
}

/// Cache folder on the PC (created on first use).
pub fn cache_dir() -> PathBuf {
    let dir = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("thorBro").join("art");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Cache key: MD5 of path, mtime and size, plus the lowercase extension. A changed icon on the
/// Thor gets a new key, so stale covers are never shown.
pub fn key_for(file: &ArtFile) -> String {
    let ext = file.path.rsplit('.').next().unwrap_or("jpg").to_lowercase();
    let mut hasher = Md5::new();
    hasher.update(format!("{}|{}|{}", file.path, file.mtime, file.size).as_bytes());
    format!("{:x}.{ext}", hasher.finalize())
}

/// The cover reference for the UI.
pub fn art_ref(file: &ArtFile) -> ArtRef {
    let key = key_for(file);
    ArtRef { cached: cache_dir().join(&key).is_file(), path: file.path.clone(), key }
}

/// One icon the UI wants copied into the cache.
#[derive(Deserialize)]
pub struct FetchItem {
    pub path: String,
    pub key: String,
}

/// True if `path` is an iiSU icon on the Thor (nothing else may be fetched through here).
fn is_fetchable(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or("");
    is_allowed_thor_path(path) && path.contains(&format!("/{IISU_CONSOLES}/")) && is_icon_name(name)
}

/// Copies icons into the cache. One `true`/`false` per item (`true` = in the cache now).
pub fn fetch(items: &[FetchItem]) -> Vec<bool> {
    let dir = cache_dir();
    items
        .iter()
        .map(|item| {
            if !key_re().is_match(&item.key) || !is_fetchable(&item.path) {
                return false;
            }
            let target = dir.join(&item.key);
            target.is_file() || fetch_one(&item.path, &target).is_ok()
        })
        .collect()
}

fn fetch_one(path: &str, target: &std::path::Path) -> Result<(), String> {
    let mut child = adb::exec_out(&format!("cat {}", shell_quote(path))).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        stdout.take(MAX_ICON_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    }
    let _ = child.kill();
    let _ = child.wait();
    if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES || image_type(&bytes).is_none() {
        return Err("not an image".into());
    }
    let temp = target.with_extension("part");
    fs::write(&temp, &bytes).map_err(|e| e.to_string())?;
    fs::rename(&temp, target).map_err(|e| e.to_string())
}

/// MIME type from the first bytes (JPEG, PNG, WebP), `None` for anything else.
fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Answers a `thorart://localhost/<key>` request from the cache: `(status, mime type, body)`.
pub fn serve(uri_path: &str) -> (u16, &'static str, Vec<u8>) {
    let key = uri_path.trim_start_matches('/');
    if !key_re().is_match(key) {
        return (400, "text/plain", b"bad key".to_vec());
    }
    match fs::read(cache_dir().join(key)) {
        Ok(bytes) => match image_type(&bytes) {
            Some(mime) => (200, mime, bytes),
            None => (415, "text/plain", b"not an image".to_vec()),
        },
        Err(_) => (404, "text/plain", b"not cached".to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/storage/EEFF-BF6C/Android/media/com.iisulauncher/iiSULauncher/assets/media/roms/consoles";

    /// `stat` lines as the index script prints them on the Thor.
    fn listing() -> String {
        [
            format!("1762700580 88565 {ROOT}/switch/The Legend of Zelda Breath of the Wild [01007EF00011E800][BASE][v1.5.0]/icon.jpg"),
            format!("1762700580 88565 {ROOT}/switch/Nine Sols [0100D810170D0800][v327680]/icon.png"),
            format!("1762700580 1320202 {ROOT}/n3ds/Mario & Luigi - Dream Team (USA) (En,Fr,Es)/icon.jpg"),
            format!("1762700580 1000 {ROOT}/n3ds/Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11)/Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11)/icon.png"),
            format!("1762700580 1000 {ROOT}/switch/Dead Cells [0100646009FBE000][v0]/title.png"),
            "garbage line".to_string(),
        ]
        .join("\n")
    }

    #[test]
    fn icons_are_found_by_rom_name_folder_and_title_id() {
        let index = ArtIndex::from_listing(&listing());
        let botw = index.find(&["switch"], Some("The Legend of Zelda Breath of the Wild [01007EF00011E800][BASE][v1.5.0].xci"), None).unwrap();
        assert!(botw.path.ends_with("[v1.5.0]/icon.jpg"));
        // A game Peanuts brings without a ROM: matched by the base title ID in iiSU's folder name.
        assert_eq!(index.find(&["switch"], None, Some("01007EF00011E000")), Some(botw));
        // Updates in iiSU never count as a game's art.
        assert_eq!(index.find(&["switch"], None, Some("0100D810170D0000")), None);
        let ml = index.find(&["n3ds"], Some("Mario & Luigi - Dream Team (USA) (En,Fr,Es).3ds"), None).unwrap();
        assert_eq!(ml.size, 1320202);
        // A game folder: iiSU nests the ROM stem inside the folder name.
        let smash_dir = "Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11)";
        assert!(index.find(&["n3ds"], Some(&format!("{smash_dir}/{smash_dir}.3ds")), None).is_some());
        assert!(index.find(&["n3ds"], Some(&format!("{smash_dir}.3ds")), None).is_some(), "same name as a file");
        assert!(index.find(&["switch"], Some("Dead Cells [0100646009FBE000][v0].xci"), None).is_none(), "title.png isn't a cover");
        assert!(index.find(&["gba"], Some("Pokemon.gba"), None).is_none());
    }

    /// Real Thor (`cargo test device_art -- --ignored`): indexes iiSU's icons, copies BotW's
    /// into the PC cache and serves it like the `thorart://` protocol does.
    #[test]
    #[ignore]
    fn device_art_index_fetch_and_serve() {
        let index = index();
        let file = index.find(&["switch"], None, Some("01007EF00011E000")).expect("BotW icon on the Thor").clone();
        let key = key_for(&file);
        let _ = fs::remove_file(cache_dir().join(&key));
        assert_eq!(fetch(&[FetchItem { path: file.path.clone(), key: key.clone() }]), [true]);
        let (status, mime, body) = serve(&format!("/{key}"));
        assert_eq!((status, mime, body.len() as u64), (200, "image/jpeg", file.size));
        // Anything that isn't an iiSU icon is refused.
        // The path is checked before the cache, even for a key that is cached already.
        assert_eq!(fetch(&[FetchItem { path: "/sdcard/Android/data/dev.eden.eden_emulator/files/keys/prod.keys".into(), key: key.clone() }]), [false]);
        let other = "0123456789abcdef0123456789abcdef.jpg".to_string();
        assert_eq!(fetch(&[FetchItem { path: "/sdcard/Android/data/dev.eden.eden_emulator/files/keys/prod.keys".into(), key: other.clone() }]), [false]);
        assert!(!cache_dir().join(other).exists());
    }

    #[test]
    fn keys_change_with_the_file_and_are_checked() {
        let file = ArtFile { path: format!("{ROOT}/switch/X/icon.JPG"), mtime: 1, size: 2 };
        let key = key_for(&file);
        assert!(key_re().is_match(&key) && key.ends_with(".jpg"));
        assert_ne!(key, key_for(&ArtFile { mtime: 3, ..file.clone() }));
        assert_eq!(serve("/../../etc/passwd").0, 400);
        assert_eq!(serve("/0123456789abcdef0123456789abcdef.exe").0, 400);
        assert!(is_fetchable(&file.path));
        assert!(!is_fetchable("/sdcard/Android/data/dev.eden.eden_emulator/files/keys/prod.keys"));
        assert!(!is_fetchable(&format!("{ROOT}/switch/X/../../../../../../../../data/icon.png")));
        assert_eq!(image_type(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(image_type(b"#!/bin/sh"), None);
    }
}
