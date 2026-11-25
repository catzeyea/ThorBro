//! Importing a save package (e.g. downloaded from the internet) into a game's save folders.
//!
//! Flow, driven by the import dialog in the UI:
//! 1. `scan_thor_sources` – optional: find packages on the Thor (Download, Documents, SD card).
//! 2. `prepare_import`    – fetch + extract + clean the package into a work folder and guess
//!    the matching sub-folder (`import_root.rs`).
//! 3. `check_root`        – optional: the user picked another sub-folder, rate that one.
//! 4. `import_save`       – back up each target, write the save, verify.
//!
//! `commands.rs` deletes the work folder afterwards (`cleanup_import`).

use super::config::{get_game, state_dir, GameCfg};
use super::import_package::{clean_import, ext_lower, extract_archive, fetch_thor_source, is_archive, JUNK_EXT, ROM_EXT};
use super::import_root::{guess_import_root, import_dirs};
use super::scan::{thor_roots, thor_stats, Cand};
use super::snapshot::{info, pc_snapshot, same, snapshot, thor_snapshot, SnapInfo};
use super::sync::{running_package, thor_write_block};
use super::transfer::{backup_dir, backup_side, ensure_thor_parent, prune_backups, push_dir, stop_thor_pkg, tempdir};
use super::{save_re, side_label, tokens};
use crate::adb::{posix_basename, sh, shell_quote};
use crate::util;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

/// Search depth for packages on the Thor.
const SOURCE_SEARCH_DEPTH: usize = 5;

/// At most this many package candidates are examined.
const MAX_SOURCES: usize = 150;

/// How many dropped file names are mentioned in the "nothing found" error.
const MAX_DROPPED_IN_ERROR: usize = 5;

/// Result of `prepare_import`, shown in the import review dialog.
#[derive(Serialize)]
pub struct Prepared {
    /// Work folder (deleted by `cleanup_import` when the dialog is done).
    pub work: String,
    /// The extracted, cleaned package inside `work`.
    pub content: String,
    /// Files that were filtered out.
    pub dropped: Vec<String>,
    /// Suggested save folder inside the package (may be an adjusted copy).
    pub root: String,
    pub root_label: String,
    /// Number of files whose path matches the existing save.
    pub hits: usize,
    pub why: String,
    /// Number of files in the existing save(s), `0` = nothing to compare against.
    pub known: usize,
    /// Sides (`"pc"` / `"thor"`) whose save folder doesn't exist yet.
    pub missing: Vec<String>,
    pub root_info: SnapInfo,
    /// All folders of the package, for "choose another folder".
    pub dirs: Vec<Cand>,
}

/// File paths of the existing save on all configured sides (the union), plus the configured
/// sides whose folder doesn't exist.
fn known_save_files(cfg: &GameCfg) -> (BTreeSet<String>, Vec<String>) {
    let mut known: BTreeSet<String> = BTreeSet::new();
    let mut missing = Vec::new();
    for side in ["pc", "thor"] {
        if let Some(path) = cfg.side(side) {
            match snapshot(side, Some(path)) {
                // `into_keys()` consumes the map and yields its keys by value (no copies).
                Some(snap) => known.extend(snap.into_keys()),
                None => missing.push(side.to_string()),
            }
        }
    }
    (known, missing)
}

/// Fetches/extracts/cleans the package and suggests the save folder inside it.
///
/// `source` is a local path, or a Thor path if `from_thor` is set.
/// On error the work folder is removed again.
pub fn prepare_import(game: &str, source: &str, from_thor: bool) -> Result<Prepared, String> {
    let cfg = get_game(game)?;
    let work = tempdir("import-")?;
    let result = prepare_import_steps(&cfg, source, from_thor, &work);
    if result.is_err() {
        let _ = fs::remove_dir_all(&work);
    }
    result
}

/// The actual work of `prepare_import` (separate so the caller can clean up on any error).
fn prepare_import_steps(cfg: &GameCfg, source: &str, from_thor: bool, work: &Path) -> Result<Prepared, String> {
    let content = work.join("inhalt");
    let local = local_source(source, from_thor, work)?;
    unpack_into(&local, &content)?;

    let dropped = clean_import(&content);
    if util::files_under(&content).is_empty() {
        return Err(nothing_found_error(&dropped));
    }

    let (known, missing) = known_save_files(cfg);
    let (root, hits, why) = guess_import_root(&content, &known);
    let dirs = package_folder_candidates(&content, &known);
    Ok(Prepared {
        work: work.to_string_lossy().into_owned(),
        content: content.to_string_lossy().into_owned(),
        root_label: root_label(&content, &root),
        root_info: info(&pc_snapshot(Some(&root.to_string_lossy()))),
        root: root.to_string_lossy().into_owned(),
        dropped,
        hits,
        why,
        known: known.len(),
        missing,
        dirs,
    })
}

/// The package as a local path: downloaded into `work/dl` if it is on the Thor.
fn local_source(source: &str, from_thor: bool, work: &Path) -> Result<PathBuf, String> {
    if from_thor {
        let download_dir = work.join("dl");
        fs::create_dir_all(&download_dir).map_err(|e| e.to_string())?;
        return fetch_thor_source(source, &download_dir);
    }
    let path = PathBuf::from(source);
    if !path.exists() {
        return Err(format!("Doesn't exist: {source}"));
    }
    Ok(path)
}

/// Puts the package contents into `content`: folders are copied, archives extracted, and a
/// single file is copied into a new folder.
fn unpack_into(local: &Path, content: &Path) -> Result<(), String> {
    if local.is_dir() {
        util::copy_tree(local, content, false)
    } else if is_archive(&local.to_string_lossy()) {
        extract_archive(local, content)
    } else {
        fs::create_dir_all(content).map_err(|e| e.to_string())?;
        fs::copy(local, content.join(local.file_name().unwrap())).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Error text when the cleaned package is empty, mentioning a few of the filtered files.
fn nothing_found_error(dropped: &[String]) -> String {
    let mut message = "There's nothing in there that looks like a save".to_string();
    if !dropped.is_empty() {
        let examples: Vec<String> = dropped.iter().take(MAX_DROPPED_IN_ERROR).cloned().collect();
        message += &format!(" (filtered out: {})", examples.join(", "));
    }
    message
}

/// All package folders as `Cand`s for the folder picker; folders containing a known file
/// name are marked "← matches".
fn package_folder_candidates(content: &Path, known: &BTreeSet<String>) -> Vec<Cand> {
    import_dirs(content)
        .into_iter()
        .map(|(path, files)| Cand {
            note: if files.iter().any(|file| known.contains(file)) { "← matches".into() } else { String::new() },
            path: path.to_string_lossy().into_owned(),
            mtime: 0.0,
            when: String::new(),
            files: files.len() as u64,
            matched: false,
        })
        .collect()
}

/// Label for a save folder inside the package, relative to the package root.
fn root_label(content: &Path, root: &Path) -> String {
    match root.strip_prefix(content) {
        Ok(relative) if relative.as_os_str().is_empty() => "(top level)".into(),
        Ok(relative) => util::slashed(relative),
        // Not inside `content` = the "adjusted" copy next to it.
        Err(_) => "(adjusted to your save)".into(),
    }
}

/// Rating of a folder the user picked manually.
#[derive(Serialize)]
pub struct RootCheck {
    pub label: String,
    pub info: SnapInfo,
    pub hits: usize,
    pub why: String,
}

/// Rates a sub-folder of the package that the user chose themselves.
pub fn check_root(game: &str, content: &str, root: &str) -> Result<RootCheck, String> {
    let cfg = get_game(game)?;
    let (known, _missing) = known_save_files(&cfg);
    let snap = pc_snapshot(Some(root));
    let files: BTreeSet<String> = snap.as_ref().map(|files| files.keys().cloned().collect()).unwrap_or_default();
    let hits = files.intersection(&known).count();
    let why = if hits > 0 {
        format!("{hits} of {} files have the same names as in your current save", known.len())
    } else if !known.is_empty() {
        "No file name matches your current save.".into()
    } else {
        "Nothing to compare against, but it looks okay.".into()
    };
    Ok(RootCheck { label: root_label(Path::new(content), Path::new(root)), info: info(&snap), hits, why })
}

/// Sets all file modification times below `root` to now, so the imported save counts as the
/// newest one on both sides afterwards.
fn touch_tree(root: &Path) {
    let now = filetime::FileTime::now();
    for file in util::files_under(root) {
        let _ = filetime::set_file_mtime(root.join(file), now);
    }
}

/// Writes `src_dir` into the save folders of `targets` (`"pc"` / `"thor"`).
///
/// Every target is backed up first. With `merge`, existing files are kept and only the
/// imported files overwrite. `step` reports progress texts to the UI.
///
/// `&dyn Fn(&str)` is a reference to any callable taking a `&str` (like `const std::function<void(std::string_view)>&`).
pub fn import_save(game: &str, src_dir: &Path, targets: &[String], merge: bool, step: &dyn Fn(&str)) -> Result<(), String> {
    let cfg = get_game(game)?;
    if targets.iter().any(|target| target == "thor") {
        if let Some(reason) = thor_write_block(&cfg) {
            return Err(reason);
        }
        if let Some(pkg) = running_package(&cfg) {
            step(&format!("Closing {pkg} …"));
            stop_thor_pkg(&pkg);
        }
    }
    touch_tree(src_dir);
    let (backup_folder, timestamp) = backup_dir(game)?;
    let mut result = Ok(());
    for side in targets {
        result = import_to_side(game, &cfg, side, src_dir, merge, &backup_folder, &timestamp, step);
        if result.is_err() {
            break;
        }
    }
    let has_backups = fs::read_dir(&backup_folder).map(|mut read_dir| read_dir.next().is_some()).unwrap_or(false);
    if has_backups {
        prune_backups(&backup_folder);
    }
    result
}

/// Imports into one side: backup, optional merge, write, verify.
#[allow(clippy::too_many_arguments)] // all of them are needed; a struct would only hide that
fn import_to_side(
    game: &str,
    cfg: &GameCfg,
    side: &str,
    src_dir: &Path,
    merge: bool,
    backup_folder: &Path,
    timestamp: &str,
    step: &dyn Fn(&str),
) -> Result<(), String> {
    let path = cfg
        .side(side)
        .cloned()
        .ok_or_else(|| format!("No folder is set up on the {} for “{game}”.", side.to_uppercase()))?;
    let name = side_label(side);

    step(&format!("Backing up the old {name} save …"));
    let existing = snapshot(side, Some(&path));
    let mut backup = None;
    if existing.as_ref().is_some_and(|files| !files.is_empty()) {
        let backup_path = backup_folder.join(format!("{timestamp}-{side}-before-import"));
        backup_side(side, &path, &backup_path)?;
        backup = Some(backup_path);
    }

    // With merge: start from the backup and copy the import on top of it.
    let mut to_write = src_dir.to_path_buf();
    if merge {
        if let Some(backup_path) = &backup {
            to_write = state_dir().join(format!("merge-{timestamp}-{side}"));
            // If the folder already exists, copy_tree refuses before touching anything, and
            // we must not delete a folder we didn't create.
            let already_existed = to_write.exists();
            let merged = util::copy_tree(backup_path, &to_write, false).and_then(|_| util::copy_tree(src_dir, &to_write, true));
            if let Err(e) = merged {
                if !already_existed {
                    let _ = fs::remove_dir_all(&to_write); // don't leave a half-merged folder behind
                }
                return Err(e);
            }
        }
    }

    step(&format!("Importing the save on the {name} …"));
    let result = write_and_verify(side, &path, &to_write, name, backup.as_deref().unwrap_or(backup_folder));
    if to_write != src_dir {
        let _ = fs::remove_dir_all(&to_write);
    }
    result
}

/// Writes `source` into the save folder `path` on `side` and checks that it arrived intact.
/// `backup_location` is mentioned in the error message.
fn write_and_verify(side: &str, path: &str, source: &Path, name: &str, backup_location: &Path) -> Result<(), String> {
    let after = if side == "pc" {
        util::mirror(source, Path::new(path))?;
        pc_snapshot(Some(path))
    } else {
        ensure_thor_parent(path);
        push_dir(source, path)?;
        thor_snapshot(Some(path))
    };
    if !same(&pc_snapshot(Some(&source.to_string_lossy())), &after) {
        return Err(format!(
            "The files on the {name} don't match after the import!\nBackup is in {}",
            backup_location.display()
        ));
    }
    Ok(())
}

/// Searches the Thor for save packages of `game` (Download, Documents and the SD cards).
///
/// Candidates whose name contains more tokens of the game name come first, then newest first.
pub fn scan_thor_sources(game: &str) -> Vec<Cand> {
    let game_tokens = tokens(game);
    let mut found = find_source_paths(&game_tokens);
    found.sort_by_key(|(_, hits)| std::cmp::Reverse(*hits));
    found.truncate(MAX_SOURCES);
    if found.is_empty() {
        return vec![];
    }

    let hits_by_path: HashMap<String, usize> = found.iter().cloned().collect();
    let paths: Vec<String> = found.into_iter().map(|(path, _)| path).collect();
    let mut ranked: Vec<(usize, Cand)> = thor_stats(&paths, true)
        .into_iter()
        .map(|(path, files, mtime)| {
            let hits = hits_by_path.get(&path).copied().unwrap_or(0);
            let cand = Cand {
                note: if is_archive(&path) { "[archive]".into() } else { String::new() },
                when: util::when(mtime),
                path,
                mtime,
                files,
                matched: true,
            };
            (hits, cand)
        })
        .collect();
    ranked.sort_by(|(hits_a, a), (hits_b, b)| {
        let by_time = b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal);
        hits_b.cmp(hits_a).then(by_time)
    });
    ranked.into_iter().map(|(_, cand)| cand).collect()
}

/// Files and folders on the Thor whose name contains a game token or looks like a save,
/// as `(path, number of matching tokens)`. ROMs, junk and emulator folders are skipped.
fn find_source_paths(game_tokens: &[String]) -> Vec<(String, usize)> {
    let mut roots = vec!["/sdcard/Download".to_string(), "/sdcard/Documents".to_string()];
    roots.extend(thor_roots().into_iter().filter(|root| root != "/sdcard"));
    let quoted_roots: Vec<String> = roots.iter().map(|root| shell_quote(&format!("{root}/"))).collect();
    let find_output = sh(&format!(
        "find {} -maxdepth {SOURCE_SEARCH_DEPTH} \\( -name Android -o -name sdmc -o -name nand \\) -prune -o \\( -type d -o -type f \\) -print 2>/dev/null",
        quoted_roots.join(" ")
    ))
    .stdout;

    let mut found = Vec::new();
    for line in find_output.lines() {
        let path = line.trim_end_matches('/').replace("//", "/");
        let name = posix_basename(&path).to_lowercase();
        let is_root = roots.iter().any(|root| root.trim_end_matches('/') == path);
        if name.is_empty() || is_root {
            continue;
        }
        let ext = ext_lower(&name);
        if ROM_EXT.contains(&ext.as_str()) || JUNK_EXT.contains(&ext.as_str()) {
            continue;
        }
        let hits = game_tokens.iter().filter(|token| name.contains(token.as_str())).count();
        if hits > 0 || save_re().is_match(&name) {
            found.push((path, hits));
        }
    }
    found
}
