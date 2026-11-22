//! The actual "continue on PC / continue on Thor" sync.
//!
//! 1. `status`    – compares both sides for the game card in the UI.
//! 2. `plan_sync` – what would happen (which side is newer, is the emulator running?), so
//!    the UI can ask the user before anything is overwritten.
//! 3. `run_sync`  – back up the target, copy, then verify with a fresh snapshot.
//!
//! Used by `main.rs` (Tauri commands and CLI). Builds on `snapshot.rs` and `transfer.rs`.

use super::config::{state_dir, GameCfg};
use super::side_label;
use super::snapshot::{info, newest, pc_snapshot, same, thor_snapshot, Snap, SnapInfo};
use super::transfer::{backup_dir, backup_side, ensure_thor_parent, prune_backups, pull_dir, push_dir, stop_thor_pkg, thor_running_pkg};
use crate::util;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// State of both sides, for the game card.
#[derive(Serialize)]
pub struct Status {
    pub pc: SnapInfo,
    pub thor: SnapInfo,
    pub same: bool,
}

/// Compares PC and Thor. Without a connected Thor, the Thor side is reported as missing.
pub fn status(cfg: &GameCfg, connected: bool) -> Status {
    // `as_deref()` turns `Option<String>` into `Option<&str>` (borrows the text instead of copying it).
    let pc = pc_snapshot(cfg.pc.as_deref());
    let thor = if connected { thor_snapshot(cfg.thor.as_deref()) } else { None };
    Status { same: same(&pc, &thor), pc: info(&pc), thor: info(&thor) }
}

/// What a sync towards `target` would do. Shown to the user before `run_sync`.
#[derive(Serialize)]
pub struct Plan {
    pub target: String,
    pub src_name: String,
    pub dst_name: String,
    pub src: SnapInfo,
    pub dst: SnapInfo,
    /// The source has no save at all.
    pub empty: bool,
    /// Both sides are already identical.
    pub same: bool,
    /// The target's save is newer than the source's (overwriting it is probably a mistake).
    pub dst_newer: bool,
    /// Emulator running on the Thor that has to be closed first (only when syncing to the Thor).
    pub running_pkg: Option<String>,
}

/// Snapshots of both sides as `(source, destination)` for a sync towards `target`.
fn snaps_for(cfg: &GameCfg, target: &str) -> (Option<Snap>, Option<Snap>) {
    let pc = pc_snapshot(cfg.pc.as_deref());
    let thor = thor_snapshot(cfg.thor.as_deref());
    if target == "pc" { (thor, pc) } else { (pc, thor) }
}

/// True if the snapshot is missing or contains no files.
fn is_missing_or_empty(snap: &Option<Snap>) -> bool {
    snap.as_ref().is_none_or(|files| files.is_empty())
}

/// Plans a sync towards `target` (`"pc"` or `"thor"`).
pub fn plan_sync(cfg: &GameCfg, target: &str) -> Plan {
    let (src, dst) = snaps_for(cfg, target);
    let source_side = if target == "pc" { "thor" } else { "pc" };
    Plan {
        target: target.into(),
        src_name: side_label(source_side).into(),
        dst_name: side_label(target).into(),
        empty: is_missing_or_empty(&src),
        same: same(&src, &dst),
        dst_newer: !is_missing_or_empty(&dst) && newest(&dst) > newest(&src),
        running_pkg: if target == "thor" { thor_running_pkg(cfg.thor.as_deref()) } else { None },
        src: info(&src),
        dst: info(&dst),
    }
}

/// Syncs towards `target`: back up the target, copy, verify.
///
/// `stop_pkg`: emulator to close on the Thor first (from `Plan::running_pkg`, confirmed by the
/// user). Returns the backup folder, or `None` if the target had nothing to back up.
pub fn run_sync(game: &str, cfg: &GameCfg, target: &str, stop_pkg: Option<&str>) -> Result<Option<String>, String> {
    let (pc_path, thor_path) = match (&cfg.pc, &cfg.thor) {
        (Some(pc), Some(thor)) => (pc.clone(), thor.clone()),
        _ => return Err("This game is only set up on one side.".into()),
    };
    if let Some(pkg) = stop_pkg {
        stop_thor_pkg(pkg);
    }
    let (src, dst) = snaps_for(cfg, target);
    if is_missing_or_empty(&src) {
        return Err("There's no save on the source side.".into());
    }

    let (backup_folder, timestamp) = backup_dir(game)?;
    let backup_path = if dst.is_some() {
        let side = if target == "pc" { "pc" } else { "thor" };
        let target_path = if target == "pc" { &pc_path } else { &thor_path };
        let backup = backup_folder.join(format!("{timestamp}-{side}"));
        backup_side(side, target_path, &backup)?;
        prune_backups(&backup_folder);
        Some(backup.to_string_lossy().into_owned())
    } else {
        None
    };

    let after = if target == "pc" {
        copy_thor_to_pc(&thor_path, &pc_path, &timestamp)?
    } else {
        copy_pc_to_thor(&pc_path, &thor_path)?
    };
    if !same(&src, &after) {
        let backup_location = backup_path.clone().unwrap_or_else(|| backup_folder.to_string_lossy().into_owned());
        return Err(format!("The files don't match after copying!\nBackup is in {backup_location}"));
    }
    Ok(backup_path)
}

/// Downloads the Thor save into a temporary folder, then mirrors it onto the PC folder.
/// Returns the PC snapshot afterwards (for verification).
fn copy_thor_to_pc(thor_path: &str, pc_path: &str, timestamp: &str) -> Result<Option<Snap>, String> {
    let temp: PathBuf = state_dir().join(format!("neu-{timestamp}"));
    let result = pull_dir(thor_path, &temp).and_then(|_| util::mirror(&temp, Path::new(pc_path)));
    let _ = fs::remove_dir_all(&temp);
    result?;
    Ok(pc_snapshot(Some(pc_path)))
}

/// Uploads the PC save to the Thor. Returns the Thor snapshot afterwards (for verification).
fn copy_pc_to_thor(pc_path: &str, thor_path: &str) -> Result<Option<Snap>, String> {
    ensure_thor_parent(thor_path);
    push_dir(Path::new(pc_path), thor_path)?;
    Ok(thor_snapshot(Some(thor_path)))
}
