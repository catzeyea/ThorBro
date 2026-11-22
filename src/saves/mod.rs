//! Save game sync between PC and Thor (port of the old Python script `thor-save`).
//!
//! Config and backups live in the same places as with the old script, so existing setups keep
//! working: `~/.config/thor-save/games.json` and `~/.local/share/thor-save/backups`.
//!
//! The module is split into files by topic:
//! - `config`         – games.json, data folders, slugs
//! - `snapshot`       – "fingerprint" of a save folder (MD5 per file) and comparing two of them
//! - `scan`           – searching PC and Thor for folders that may contain saves
//! - `ai`             – letting the `claude` CLI pick the right folders from the scan results
//! - `transfer`       – copying folders between PC and Thor, backups, closing emulators
//! - `sync`           – status, plan and the actual "continue on PC / Thor" sync
//! - `import`         – importing a downloaded save (overall flow)
//! - `import_package` – unpacking and cleaning an import package
//! - `import_root`    – finding the folder inside the package that matches the game's save
//!
//! `commands.rs` and `cli.rs` only use the items re-exported below (`saves::load_games`, `saves::run_sync`, ...).

// `mod x;` includes the file `x.rs` from this folder as a sub-module (a bit like a C++
// namespace that lives in its own file). Without `pub` the sub-modules are private to `saves`;
// only what is re-exported with `pub use` is visible to the rest of the crate.
mod ai;
mod config;
mod import;
mod import_package;
mod import_root;
mod scan;
mod snapshot;
mod sync;
mod transfer;

pub use ai::{ai_pick, AiPick};
pub use config::{backups_root, get_game, load_games, save_games, slug, state_dir, GameCfg, Games};
pub use import::{check_root, import_save, prepare_import, scan_thor_sources, Prepared, RootCheck};
pub use scan::{scan_pc, scan_thor, Cand};
pub use snapshot::{info, snapshot, SnapInfo};
pub use sync::{plan_sync, run_sync, status, Plan, Status};

use regex::Regex;
use std::sync::OnceLock;

/// Words ignored when turning a game name into search tokens.
const STOPWORDS: &[&str] = &["the", "of", "and", "der", "die", "das", "und", "a", "an", "von", "to"];

/// Search tokens shorter than this are ignored (too many false hits).
const MIN_TOKEN_LEN: usize = 3;

/// The name without spaces/punctuation is added as an extra token if it is at least this long.
const MIN_COMPACT_LEN: usize = 4;

/// Compiles `pattern` once and caches it in `cell`.
///
/// Every call site has its own `static` cell, so each regex is compiled only on first use
/// (compiling a regex is expensive, using it is cheap).
fn cached_regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).unwrap())
}

/// Regex for folder names that typically hold saves ("save", "memcard", "sram", German "spielst...").
fn save_re() -> &'static Regex {
    static SAVE_RE: OnceLock<Regex> = OnceLock::new();
    cached_regex(&SAVE_RE, r"(?i)save|memcard|sram|spielst")
}

/// Turns a game name into lowercase search tokens.
///
/// `"The Legend of Zelda: TotK"` -> `["legend", "zelda", "totk", "thelegendofzeldatotk"]`.
/// The last, compact token helps with folders like `ZeldaTotK`.
fn tokens(name: &str) -> Vec<String> {
    static NON_ALNUM_RE: OnceLock<Regex> = OnceLock::new();
    let lowercase = name.to_lowercase();
    let mut result: Vec<String> = cached_regex(&NON_ALNUM_RE, r"[^a-z0-9]+")
        .split(&lowercase)
        .filter(|word| word.len() >= MIN_TOKEN_LEN && !STOPWORDS.contains(word))
        .map(String::from)
        .collect();
    let compact: String = lowercase.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if compact.len() >= MIN_COMPACT_LEN {
        result.push(compact);
    }
    result
}

/// `"pc"` -> `"PC"`, everything else -> `"Thor"` (side names as shown to the user).
fn side_label(side: &str) -> &'static str {
    if side == "pc" { "PC" } else { "Thor" }
}
