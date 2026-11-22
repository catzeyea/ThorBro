//! User settings of the file manager: custom game library folders and systems picked by hand
//! for folders ThorBro can't identify.
//!
//! Stored as JSON in `<config dir>/thorBro/settings.json` (next to `last_path`, see `files.rs`).
//! The config dir comes from `dirs::config_dir()`: `%APPDATA%` on Windows, `~/Library/Application
//! Support` on macOS, `$XDG_CONFIG_HOME` or `~/.config` on Linux.
//! Used by `library.rs` (`find_libraries`) and `commands.rs` (the commands that change it).

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
}
