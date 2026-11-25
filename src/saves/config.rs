//! Configuration of the save sync: which games exist and where their save folders are.
//!
//! The list is stored as JSON in `~/.config/thor-save/games.json` (same file as the old Python
//! script). This file also knows the data folder for backups and temporary work folders.
//!
//! Used by every other file in `saves/` and by `commands.rs` (adding/removing games, opening backups).

use super::cached_regex;
use crate::emulators::Profiles;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Path of `games.json`.
pub fn conf_file() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("thor-save").join("games.json")
}

/// Data folder for backups and temporary work folders (`~/.local/share/thor-save`).
/// Created on first use.
pub fn state_dir() -> PathBuf {
    let dir = dirs::data_dir().unwrap_or_default().join("thor-save");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Folder that holds one backup sub-folder per game.
pub fn backups_root() -> PathBuf {
    state_dir().join("backups")
}

/// File-system friendly version of a game name: `"Tears of the Kingdom!"` -> `"tears-of-the-kingdom"`.
/// Used as the backup folder name.
pub fn slug(name: &str) -> String {
    static NON_ALNUM_RE: OnceLock<Regex> = OnceLock::new();
    let lowercase = name.to_lowercase();
    let dashed = cached_regex(&NON_ALNUM_RE, r"[^a-z0-9]+").replace_all(&lowercase, "-");
    let trimmed = dashed.trim_matches('-').to_string();
    if trimmed.is_empty() { "game".into() } else { trimmed }
}

/// Save folders of one game. `None` = not set up on that side (the game only exists on one device).
///
/// `Option<String>` becomes `null` or a string in JSON thanks to serde. The fields after `thor`
/// came with Update 9; they are left out of the JSON when empty, so old entries stay unchanged.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct GameCfg {
    pub pc: Option<String>,
    pub thor: Option<String>,
    /// Id of the emulator profile the Thor folder belongs to (`emulators::Profile::id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emulator: Option<String>,
    /// Display title of the game (the entry's key is unique per game and emulator).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Title ID of the game, uppercase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    /// When the last "Continue on PC / Thor" succeeded (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<f64>,
}

impl GameCfg {
    /// The folder for `side` (`"pc"` or anything else = Thor), if set.
    ///
    /// Returns `Option<&String>`: a *borrowed* reference into `self`, no copy is made.
    pub fn side(&self, side: &str) -> Option<&String> {
        if side == "pc" { self.pc.as_ref() } else { self.thor.as_ref() }
    }
}

/// All configured games, sorted by name. `type` creates an alias, like `using` in C++.
pub type Games = BTreeMap<String, GameCfg>;

/// Reads `games.json`. A missing or broken file results in an empty list.
pub fn load_games() -> Games {
    load_games_from(&conf_file())
}

fn load_games_from(file: &Path) -> Games {
    let content = fs::read_to_string(file).ok();
    content.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Writes `games.json` atomically: first to a `.tmp` file, then renamed over the old one,
/// so a crash in the middle never leaves a half-written config behind.
pub fn save_games(games: &Games) -> Result<(), String> {
    save_games_to(&conf_file(), games)
}

fn save_games_to(file: &Path, games: &Games) -> Result<(), String> {
    fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp_file = file.with_extension("json.tmp");
    fs::write(&temp_file, serde_json::to_string_pretty(games).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(&temp_file, file).map_err(|e| e.to_string())
}

/// Name of the copy of games.json made before the first Update 9 migration (next to it).
pub const MIGRATION_BACKUP: &str = "games.json.before-update9";

/// Tags entries that have a Thor folder but no emulator yet: if exactly one profile claims the
/// folder (`Profiles::match_save_path`), `emulator` and `title_id` are set. Folders are never
/// changed. Returns how many entries were tagged.
pub fn tag_games(games: &mut Games, profiles: &Profiles) -> usize {
    let mut tagged = 0;
    for cfg in games.values_mut() {
        if cfg.emulator.is_some() {
            continue;
        }
        let Some(thor) = &cfg.thor else { continue };
        if let Some((emulator, title_id)) = profiles.match_save_path(thor) {
            cfg.emulator = Some(emulator);
            cfg.title_id = Some(title_id);
            tagged += 1;
        }
    }
    tagged
}

/// Runs `tag_games` on games.json and writes it back if something changed. Before the first
/// change, the old file is copied to `MIGRATION_BACKUP` (an existing backup is never replaced).
pub fn migrate_games(profiles: &Profiles) -> Result<usize, String> {
    migrate_games_at(&conf_file(), profiles)
}

fn migrate_games_at(file: &Path, profiles: &Profiles) -> Result<usize, String> {
    if !file.is_file() {
        return Ok(0);
    }
    let mut games = load_games_from(file);
    let tagged = tag_games(&mut games, profiles);
    if tagged == 0 {
        return Ok(0);
    }
    let backup = file.with_file_name(MIGRATION_BACKUP);
    if !backup.exists() {
        fs::copy(file, &backup).map_err(|e| format!("Couldn't back up games.json: {e}"))?;
    }
    save_games_to(file, &games)?;
    Ok(tagged)
}

/// Remembers that `name` was synced just now (shown as "last synced" on the card).
pub fn mark_synced(name: &str) -> Result<(), String> {
    let mut games = load_games();
    let Some(cfg) = games.get_mut(name) else { return Ok(()) };
    cfg.last_sync = Some(chrono::Local::now().timestamp() as f64);
    save_games(&games)
}

/// The config of one game, or an error text if it doesn't exist.
pub fn get_game(name: &str) -> Result<GameCfg, String> {
    load_games().get(name).cloned().ok_or_else(|| format!("Game “{name}” doesn't exist"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emulators::load_from;

    /// games.json as it was on the PC before Update 9 (step 1 of the investigation).
    const REAL_GAMES_JSON: &str = r#"{
  "Breath Of The Wild": {
    "pc": "/home/unterhose/.local/share/eden/nand/user/save/0000000000000000/57F9DD99C7E62A7AE71954A31BC3B628/01007EF00011E000",
    "thor": "/sdcard/Android/data/dev.igawa6.edenduo/files/nand/user/save/0000000000000000/5D1B4CDA089A581DAA9E48BC2B43C120/01007EF00011E000"
  },
  "Dead Cells": {
    "pc": null,
    "thor": "/sdcard/Android/data/dev.igawa6.edenduo/files/nand/user/save/0000000000000000/5D1B4CDA089A581DAA9E48BC2B43C120/0100646009FBE000"
  },
  "Super Smash Bros Ultimate": {
    "pc": "/home/unterhose/.local/share/eden/nand/user/save/0000000000000000/57F9DD99C7E62A7AE71954A31BC3B628/01006A800016E000",
    "thor": "/sdcard/Android/data/dev.igawa6.edenduo/files/nand/user/save/0000000000000000/5D1B4CDA089A581DAA9E48BC2B43C120/01006A800016E000"
  },
  "Super Smash Bros. for 3DS": {
    "pc": null,
    "thor": "/storage/EEFF-BF6C/ROMs/n3ds/sdmc/Nintendo 3DS/00000000000000000000000000000000/00000000000000000000000000000000/title/00040000/000ee000/data/00000001"
  },
  "Tears Of The Kingdom": {
    "pc": "/home/unterhose/.local/share/eden/nand/user/save/0000000000000000/57F9DD99C7E62A7AE71954A31BC3B628/0100F2C0115B6000",
    "thor": "/sdcard/Android/data/dev.igawa6.edenduo/files/nand/user/save/0000000000000000/5D1B4CDA089A581DAA9E48BC2B43C120/0100F2C0115B6000"
  }
}"#;

    #[test]
    fn migration_tags_the_real_entries_and_keeps_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("games.json");
        fs::write(&file, REAL_GAMES_JSON).unwrap();
        let profiles = load_from(None);

        assert_eq!(migrate_games_at(&file, &profiles).unwrap(), 5);
        let games = load_games_from(&file);
        let tags: Vec<(&str, &str, &str)> = games
            .iter()
            .map(|(name, cfg)| (name.as_str(), cfg.emulator.as_deref().unwrap(), cfg.title_id.as_deref().unwrap()))
            .collect();
        assert_eq!(
            tags,
            [
                ("Breath Of The Wild", "eden-duo", "01007EF00011E000"),
                ("Dead Cells", "eden-duo", "0100646009FBE000"),
                ("Super Smash Bros Ultimate", "eden-duo", "01006A800016E000"),
                ("Super Smash Bros. for 3DS", "azahar", "00040000000EE000"),
                ("Tears Of The Kingdom", "eden-duo", "0100F2C0115B6000"),
            ]
        );
        // Folders are untouched, the old file is kept byte for byte.
        let old: Games = serde_json::from_str(REAL_GAMES_JSON).unwrap();
        for (name, cfg) in &games {
            assert_eq!((&cfg.pc, &cfg.thor), (&old[name].pc, &old[name].thor));
        }
        assert_eq!(fs::read_to_string(dir.path().join(MIGRATION_BACKUP)).unwrap(), REAL_GAMES_JSON);

        // A second run changes nothing and doesn't touch the backup.
        fs::write(dir.path().join(MIGRATION_BACKUP), "keep me").unwrap();
        assert_eq!(migrate_games_at(&file, &profiles).unwrap(), 0);
        assert_eq!(fs::read_to_string(dir.path().join(MIGRATION_BACKUP)).unwrap(), "keep me");
    }

    #[test]
    fn migration_leaves_unknown_folders_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("games.json");
        fs::write(&file, r#"{"Elden Ring": {"pc": "/home/me/elden", "thor": "/sdcard/Saves/elden"}, "PC only": {"pc": "/x", "thor": null}}"#).unwrap();
        assert_eq!(migrate_games_at(&file, &load_from(None)).unwrap(), 0);
        assert!(!dir.path().join(MIGRATION_BACKUP).exists());
        // Old entries are written without the new fields.
        let json = serde_json::to_string(&load_games_from(&file)).unwrap();
        assert!(!json.contains("emulator") && !json.contains("last_sync"), "{json}");
        assert_eq!(migrate_games_at(&dir.path().join("missing.json"), &load_from(None)).unwrap(), 0);
    }
}
