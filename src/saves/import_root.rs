//! Finding the folder inside an import package that corresponds to the game's save folder.
//!
//! Downloaded saves come in all shapes: wrapped in extra folders, one level too deep or too
//! shallow, or with files in slightly different places. We compare the package against the
//! file names of the existing save ("known" files) and pick the best matching sub-folder. If the
//! names match but the layout doesn't, the files are copied into the known layout ("adjusted").
//!
//! Used by `import.rs`.

use crate::util;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

/// How many file moves are listed in the explanation text.
const MAX_LISTED_MOVES: usize = 4;

/// Name of the folder (next to the extracted package) that holds the re-arranged files.
const ADJUSTED_DIR_NAME: &str = "angepasst";

/// A folder in the package plus all files below it (relative to that folder).
pub type PackageDir = (PathBuf, BTreeSet<String>);

/// All folders in the package that contain files, including the package root itself.
pub fn import_dirs(content: &Path) -> Vec<PackageDir> {
    let all_files = util::files_under(content);
    // Every folder that appears in any file path ("" = the root).
    let mut folders: BTreeSet<String> = BTreeSet::from([String::new()]);
    for file in &all_files {
        let parts: Vec<&str> = file.split('/').collect();
        for depth in 1..parts.len() {
            folders.insert(parts[..depth].join("/"));
        }
    }
    folders
        .into_iter()
        .filter_map(|folder| {
            let prefix = if folder.is_empty() { String::new() } else { format!("{folder}/") };
            let files_below: BTreeSet<String> =
                all_files.iter().filter_map(|file| file.strip_prefix(&prefix).map(String::from)).collect();
            if files_below.is_empty() {
                return None;
            }
            let path = if folder.is_empty() { content.to_path_buf() } else { content.join(&folder) };
            Some((path, files_below))
        })
        .collect()
}

/// Last component of a `/`-separated relative path.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `{package path: known path}` for files that exist in the real save under the same file name
/// but at a different place. Only unambiguous moves are listed (the name occurs once in the
/// known save and the target isn't already taken in the package).
fn remap_moves(package_files: &BTreeSet<String>, known: &BTreeSet<String>) -> BTreeMap<String, String> {
    // `HashMap<&str, Vec<&String>>`: the keys and values *borrow* from `known` (no copies).
    // The compiler makes sure `known` outlives this map.
    let mut known_by_name: HashMap<&str, Vec<&String>> = HashMap::new();
    for known_path in known {
        known_by_name.entry(basename(known_path)).or_default().push(known_path);
    }
    let mut moves = BTreeMap::new();
    for file in package_files {
        if known.contains(file) {
            continue; // already in the right place
        }
        if let Some(same_name) = known_by_name.get(basename(file)) {
            if same_name.len() == 1 && !package_files.contains(same_name[0]) {
                moves.insert(file.clone(), same_name[0].clone());
            }
        }
    }
    moves
}

/// All folders that appear in the known file paths (`"a/b/c"` -> `"a"`, `"a/b"`).
fn known_folders(known: &BTreeSet<String>) -> BTreeSet<String> {
    let mut folders = BTreeSet::new();
    for known_path in known {
        let parts: Vec<&str> = known_path.split('/').collect();
        for depth in 1..parts.len() {
            folders.insert(parts[..depth].join("/"));
        }
    }
    folders
}

/// Result of `remap_to_known`: the applied moves and the files left out.
type Remapped = (Vec<(String, String)>, Vec<String>);

/// Copies the files of `src` into `dest`, moved into the layout of the known save.
///
/// Returns `None` if nothing needs to move. Files are left out if they would collide with a
/// known *folder*, e.g. a 0-byte placeholder file `save_data` where the real save has a folder
/// `save_data/`.
fn remap_to_known(src: &Path, known: &BTreeSet<String>, dest: &Path) -> Option<Remapped> {
    let package_files: BTreeSet<String> = util::files_under(src).into_iter().collect();
    let moves = remap_moves(&package_files, known);
    if moves.is_empty() {
        return None;
    }
    let folders = known_folders(known);
    let _ = fs::remove_dir_all(dest);
    let mut dropped = Vec::new();
    for file in &package_files {
        let target = moves.get(file).unwrap_or(file);
        let full_path = src.join(file);
        if collides_with_known_folder(file, target, &full_path, &moves, &folders) {
            dropped.push(file.clone());
            continue;
        }
        let out = dest.join(target);
        if let Some(parent) = out.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::copy(&full_path, &out);
    }
    Some((moves.into_iter().collect(), dropped))
}

/// True if copying `file` to `target` would put a file where the known save has a folder.
/// Also catches empty (0-byte) unmoved files whose name is a known folder or a parent of one.
fn collides_with_known_folder(
    file: &String,
    target: &String,
    full_path: &Path,
    moves: &BTreeMap<String, String>,
    folders: &BTreeSet<String>,
) -> bool {
    if folders.contains(target) {
        return true;
    }
    let is_empty_file = fs::metadata(full_path).map(|meta| meta.len() == 0).unwrap_or(false);
    let is_folder_placeholder = folders.iter().any(|folder| folder == file || folder.starts_with(&format!("{file}/")));
    !moves.contains_key(file) && is_empty_file && is_folder_placeholder
}

/// How well a package folder matches the known save:
/// `(exact path matches, file name matches, -files that don't exist in the known save)`.
/// Bigger is better; tuples compare element by element.
fn match_score(files: &BTreeSet<String>, known: &BTreeSet<String>, known_names: &BTreeSet<&str>) -> (usize, usize, i64) {
    let exact = files.intersection(known).count();
    let file_names: BTreeSet<&str> = files.iter().map(|file| basename(file)).collect();
    let by_name = file_names.intersection(known_names).count();
    let unknown = files.difference(known).count() as i64;
    (exact, by_name, -unknown)
}

/// Decides which folder of the package is the save folder.
///
/// Returns `(folder, number of matching files, explanation for the user)`. The folder may be
/// an "adjusted" copy with files moved into the known layout.
pub fn guess_import_root(content: &Path, known: &BTreeSet<String>) -> (PathBuf, usize, String) {
    let candidates = import_dirs(content);
    if !known.is_empty() && !candidates.is_empty() {
        if let Some(result) = guess_by_known_files(content, known, &candidates) {
            return result;
        }
    }
    // Nothing to compare against (or no file name matches): peel off wrapper folders
    // (Package/Package/... -> contents).
    let root = peel_wrapper_folders(content);
    let why = if !known.is_empty() {
        "No file name matches your current save – are you sure this is the right save?"
    } else {
        "There's no save to compare against yet, so I can't check whether the folder structure is right. \
         Better: start the game once and save, then import – then I'll move the files to the right place myself."
    };
    (root, 0, why.into())
}

/// The comparison part of `guess_import_root`. `None` if not even file names match.
fn guess_by_known_files(content: &Path, known: &BTreeSet<String>, candidates: &[PackageDir]) -> Option<(PathBuf, usize, String)> {
    let known_names: BTreeSet<&str> = known.iter().map(|known_path| basename(known_path)).collect();
    // `max_by_key` returns the *last* of several equal maxima; the order of `candidates` is
    // sorted by folder path, so this is deterministic.
    let (best_dir, best_files) = candidates.iter().max_by_key(|(_, files)| match_score(files, known, &known_names)).unwrap();
    let (exact, by_name, _) = match_score(best_files, known, &known_names);

    let all_match = exact == best_files.len();
    if all_match || (exact > 0 && remap_moves(best_files, known).is_empty()) {
        let why = format!("{exact} of {} files have the same names as in your current save", known.len());
        return Some((best_dir.clone(), exact, why));
    }
    if by_name == 0 {
        return None;
    }
    let adjusted = content.parent().unwrap_or(content).join(ADJUSTED_DIR_NAME);
    if let Some((moves, dropped)) = remap_to_known(best_dir, known, &adjusted) {
        let adjusted_files: BTreeSet<String> = util::files_under(&adjusted).into_iter().collect();
        let hits = adjusted_files.intersection(known).count();
        let why = adjusted_explanation(hits, known.len(), &moves, &dropped);
        return Some((adjusted, hits, why));
    }
    let why = format!(
        "{by_name} file names match your current save, but the folder structure is different – better double-check"
    );
    Some((best_dir.clone(), 0, why))
}

/// Explanation text after re-arranging files, e.g.
/// `"Folder structure adjusted to your save (3 of 3 files match): a → x/a, ..."`.
fn adjusted_explanation(hits: usize, known_count: usize, moves: &[(String, String)], dropped: &[String]) -> String {
    let listed_moves: Vec<String> =
        moves.iter().take(MAX_LISTED_MOVES).map(|(from, to)| format!("{from} → {to}")).collect();
    let ellipsis = if moves.len() > MAX_LISTED_MOVES { " …" } else { "" };
    let mut why = format!(
        "Folder structure adjusted to your save ({hits} of {known_count} files match): {}{ellipsis}",
        listed_moves.join(", ")
    );
    if !dropped.is_empty() {
        why += &format!("; left out empty placeholders: {}", dropped.join(", "));
    }
    why
}

/// Descends into single sub-folders as long as a folder contains nothing else.
fn peel_wrapper_folders(content: &Path) -> PathBuf {
    let mut current = content.to_path_buf();
    loop {
        let entries: Vec<_> = fs::read_dir(&current).map(|read_dir| read_dir.flatten().collect()).unwrap_or_default();
        if entries.len() == 1 && entries[0].path().is_dir() {
            current = entries[0].path();
        } else {
            break;
        }
    }
    current
}
