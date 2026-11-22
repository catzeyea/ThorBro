//! User settings: custom game library folders and systems picked by hand for folders ThorBro
//! can't identify (file manager), and the last used options per display (Stream tab).
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
    }
}
