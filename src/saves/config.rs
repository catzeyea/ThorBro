//! Configuration of the save sync: which games exist and where their save folders are.
//!
//! The list is stored as JSON in `~/.config/thor-save/games.json` (same file as the old Python
//! script). This file also knows the data folder for backups and temporary work folders.
//!
//! Used by every other file in `saves/` and by `commands.rs` (adding/removing games, opening backups).

use super::cached_regex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
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
/// `Option<String>` becomes `null` or a string in JSON thanks to serde.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct GameCfg {
    pub pc: Option<String>,
    pub thor: Option<String>,
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
    let content = fs::read_to_string(conf_file()).ok();
    content.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Writes `games.json` atomically: first to a `.tmp` file, then renamed over the old one,
/// so a crash in the middle never leaves a half-written config behind.
pub fn save_games(games: &Games) -> Result<(), String> {
    let file = conf_file();
    fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp_file = file.with_extension("json.tmp");
    fs::write(&temp_file, serde_json::to_string_pretty(games).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(&temp_file, &file).map_err(|e| e.to_string())
}

/// The config of one game, or an error text if it doesn't exist.
pub fn get_game(name: &str) -> Result<GameCfg, String> {
    load_games().get(name).cloned().ok_or_else(|| format!("Game “{name}” doesn't exist"))
}
