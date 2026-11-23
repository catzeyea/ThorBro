//! User settings: custom game library folders and systems picked by hand for folders ThorBro
//! can't identify (file manager), pinned Thor folders (file manager sidebar and Start tab) and
//! the last used options per display (Stream tab).
//!
//! Stored as JSON in `<config dir>/thorBro/settings.json` (next to `last_path`, see `files.rs`).
//! The config dir comes from `dirs::config_dir()`: `%APPDATA%` on Windows, `~/Library/Application
//! Support` on macOS, `$XDG_CONFIG_HOME` or `~/.config` on Linux.
//! Used by `library.rs` (`find_libraries`) and `commands.rs` (the commands that change it).

use crate::stream::StreamOptions;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// Contents of `settings.json`. `#[serde(default)]`: missing fields are left empty, so older or
/// hand-edited files still load.
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Settings {
    /// Library folders on the Thor chosen by the user, e.g. `/sdcard/lisu/Roms`.
    pub libraries: Vec<String>,
    /// Full Thor path of a system folder -> system id picked by the user,
    /// e.g. `/sdcard/lisu/Roms/Wii` -> `wii`.
    pub folder_systems: BTreeMap<String, String>,
    /// Display id -> options last used to mirror that display. JSON object keys are strings,
    /// so this is stored as e.g. `"stream": { "0": { "max_size": 0, ... } }`.
    pub stream: BTreeMap<u32, StreamOptions>,
    /// Pinned Thor folders in the order they were pinned (oldest first), at most `MAX_PINS`.
    pub pins: Vec<Pin>,
}

/// Most folders that can be pinned. More would no longer fit the sidebar card and the Start row.
pub const MAX_PINS: usize = 12;

/// A pinned Thor folder. `storage` is the root of the storage it is on (`/sdcard` for the
/// internal storage, `/storage/1A2B-3C4D` for an SD card), so pins can be told apart per
/// storage and a pin on a removed SD card can be shown as "not found".
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub storage: String,
    pub path: String,
}

impl Pin {
    /// Checks and normalizes a pin (no trailing slashes, `path` on `storage`).
    pub fn new(storage: &str, path: &str) -> Result<Pin, String> {
        let storage = clean_path(storage)?;
        let path = clean_path(path)?;
        if path != storage && !path.starts_with(&format!("{storage}/")) {
            return Err(format!("{path} is not on {storage}"));
        }
        Ok(Pin { storage, path })
    }
}

/// Pins a folder at the end of the list. Pinning a folder twice changes nothing.
/// Fails with a short message when `MAX_PINS` folders are pinned already.
pub fn add_pin(settings: &mut Settings, pin: Pin) -> Result<(), String> {
    if settings.pins.contains(&pin) {
        return Ok(());
    }
    if settings.pins.len() >= MAX_PINS {
        return Err(format!("You can pin up to {MAX_PINS} folders. Unpin one first."));
    }
    settings.pins.push(pin);
    Ok(())
}

/// Unpins a folder; the others keep their order. Unpinning a folder that isn't pinned changes nothing.
pub fn remove_pin(settings: &mut Settings, pin: &Pin) {
    settings.pins.retain(|existing| existing != pin);
}

/// Path of `settings.json`, or `None` if the OS has no config directory for this user.
/// (Falling back to a relative path would write the file into whatever the working dir is.)
fn settings_file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("thorBro").join("settings.json"))
}

/// Reads the settings. A missing or broken file results in empty settings.
pub fn load() -> Settings {
    let content = settings_file().and_then(|file| fs::read_to_string(file).ok());
    content
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes the settings atomically (temp file + rename, like `saves::save_games`).
pub fn save(settings: &Settings) -> Result<(), String> {
    let file =
        settings_file().ok_or("No config folder found for this user, can't save the settings")?;
    fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp_file = file.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(&temp_file, json).map_err(|e| e.to_string())?;
    fs::rename(&temp_file, &file).map_err(|e| e.to_string())
}

/// Normalizes a Thor folder path for storing: no trailing slash. Refuses relative paths and `/`.
pub fn clean_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim().trim_end_matches('/');
    if !trimmed.starts_with('/') {
        return Err(format!("Not a valid folder on the Thor: {path}"));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_path_trims_and_rejects() {
        assert_eq!(
            clean_path("/sdcard/lisu/Roms/").unwrap(),
            "/sdcard/lisu/Roms"
        );
        assert!(clean_path("/").is_err());
        assert!(clean_path("Roms").is_err());
    }

    #[test]
    fn stream_options_round_trip() {
        let mut settings = Settings::default();
        settings.stream.insert(4, StreamOptions { max_size: 1280, bitrate_mbps: 8, always_on_top: true });
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"4\""), "{json}");
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.stream, settings.stream);
    }

    #[test]
    fn old_settings_without_stream_still_load() {
        let loaded: Settings =
            serde_json::from_str(r#"{"libraries":["/sdcard/Roms"],"stream":{"0":{"bitrate_mbps":24}}}"#).unwrap();
        assert_eq!(loaded.libraries, ["/sdcard/Roms"]);
        // Missing option fields fall back to the defaults.
        assert_eq!(loaded.stream[&0], StreamOptions { bitrate_mbps: 24, ..Default::default() });
        let empty: Settings = serde_json::from_str("{}").unwrap();
        assert!(empty.stream.is_empty());
        assert!(empty.pins.is_empty(), "settings from before pins load without any");
    }

    fn pin(path: &str) -> Pin {
        Pin::new("/sdcard", path).unwrap()
    }

    #[test]
    fn pin_checks_path_and_storage() {
        assert_eq!(Pin::new("/sdcard/", "/sdcard/Pictures/").unwrap(), Pin { storage: "/sdcard".into(), path: "/sdcard/Pictures".into() });
        assert!(Pin::new("/sdcard", "/sdcard").is_ok(), "the storage root itself can be pinned");
        assert!(Pin::new("/sdcard", "/sdcardX/a").is_err(), "a prefix of the name is not inside");
        assert!(Pin::new("/storage/1A2B-3C4D", "/sdcard/a").is_err());
        assert!(Pin::new("/sdcard", "Pictures").is_err());
    }

    #[test]
    fn pins_keep_pin_order_and_unpin_keeps_the_rest() {
        let mut settings = Settings::default();
        for path in ["/sdcard/b", "/sdcard/a", "/sdcard/c"] {
            add_pin(&mut settings, pin(path)).unwrap();
        }
        add_pin(&mut settings, pin("/sdcard/b")).unwrap(); // twice: no change, no error
        let sd = Pin::new("/storage/1A2B-3C4D", "/storage/1A2B-3C4D/x").unwrap();
        add_pin(&mut settings, sd.clone()).unwrap();
        let paths = |s: &Settings| s.pins.iter().map(|p| p.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&settings), ["/sdcard/b", "/sdcard/a", "/sdcard/c", "/storage/1A2B-3C4D/x"]);
        remove_pin(&mut settings, &pin("/sdcard/a"));
        remove_pin(&mut settings, &pin("/sdcard/not-pinned"));
        assert_eq!(paths(&settings), ["/sdcard/b", "/sdcard/c", "/storage/1A2B-3C4D/x"]);
        assert_eq!(settings.pins[2].storage, "/storage/1A2B-3C4D");
    }

    #[test]
    fn pins_are_capped() {
        let mut settings = Settings::default();
        for i in 0..MAX_PINS {
            add_pin(&mut settings, pin(&format!("/sdcard/{i}"))).unwrap();
        }
        let err = add_pin(&mut settings, pin("/sdcard/one-more")).unwrap_err();
        assert!(err.contains(&MAX_PINS.to_string()), "{err}");
        assert_eq!(settings.pins.len(), MAX_PINS);
        add_pin(&mut settings, pin("/sdcard/0")).unwrap(); // re-pinning an existing one is still fine
        remove_pin(&mut settings, &pin("/sdcard/0"));
        add_pin(&mut settings, pin("/sdcard/one-more")).unwrap();
        assert_eq!(settings.pins.last().unwrap().path, "/sdcard/one-more");
    }

    #[test]
    fn pins_round_trip() {
        let mut settings = Settings::default();
        add_pin(&mut settings, pin("/sdcard/Pictures")).unwrap();
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains(r#""pins":[{"storage":"/sdcard","path":"/sdcard/Pictures"}]"#), "{json}");
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.pins, settings.pins);
    }
}
