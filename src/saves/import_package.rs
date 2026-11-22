//! Preparing an import package: fetching it (from the Thor if needed), extracting it and
//! throwing out everything that isn't a save (readmes, images, ROMs, nested archives, huge files).
//!
//! This is the "get the files" half of importing; `import_root.rs` then decides which sub-folder
//! is the actual save, and `import.rs` ties it all together.

use crate::adb::{self, posix_basename, sh, shell_quote};
use crate::util;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Files bigger than this are never part of a save (probably a ROM or video).
pub const IMPORT_MAX_FILE: u64 = 256 * 1024 * 1024;

/// How many bytes of an external tool's error output are shown.
const MAX_ERROR_CHARS: usize = 600;

/// ROM / game file extensions: never part of a save.
pub const ROM_EXT: &[&str] = &[
    ".3ds", ".cci", ".cia", ".cxi", ".3dsx", ".nsp", ".nsz", ".xci", ".xcz", ".iso", ".cso", ".chd",
    ".rvz", ".wbfs", ".wua", ".wux", ".rpx", ".nds", ".gba", ".gbc", ".gb", ".sfc", ".smc", ".nes",
    ".z64", ".n64", ".v64", ".pkg", ".apk", ".m3u", ".cue",
];
/// Typical extra files in downloaded save packages.
pub const JUNK_EXT: &[&str] = &[
    ".txt", ".nfo", ".url", ".html", ".htm", ".md", ".diz", ".lnk", ".webloc", ".pdf", ".jpg",
    ".jpeg", ".png", ".gif", ".webp", ".mp4", ".exe",
];
/// OS metadata files and folders.
const JUNK_NAMES: &[&str] = &["__MACOSX", ".DS_Store", "Thumbs.db", "desktop.ini"];

/// Lowercase extension including the dot (`"a.SAV"` -> `".sav"`), or `""`.
/// Unlike `Path::extension` this also works on plain strings / Thor paths.
pub fn ext_lower(name: &str) -> String {
    let lowercase = name.to_lowercase();
    lowercase.rfind('.').map(|index| lowercase[index..].to_string()).unwrap_or_default()
}

/// True if the name ends with a known archive extension. Works for local and Thor paths.
/// Uses the same extension list as the copy queue (`unpack::archive_ext`).
pub fn is_archive(path: &str) -> bool {
    crate::unpack::archive_ext(Path::new(path)).is_some()
}

/// True for files that are certainly not part of a save.
fn is_junk(name: &str, size: u64) -> bool {
    if JUNK_NAMES.contains(&name) || name.starts_with("._") {
        return true;
    }
    let ext = ext_lower(name);
    JUNK_EXT.contains(&ext.as_str()) || ROM_EXT.contains(&ext.as_str()) || is_archive(name) || size > IMPORT_MAX_FILE
}

/// Removes junk files and folders below `root`, then all empty folders.
/// Returns the relative paths of the removed files (shown to the user as "left out").
pub fn clean_import(root: &Path) -> Vec<String> {
    let mut dropped = Vec::new();
    // Collect first, then delete: deleting while walking would confuse the walker.
    let entries: Vec<_> = walkdir::WalkDir::new(root).min_depth(1).into_iter().filter_map(Result::ok).collect();
    for entry in &entries {
        let name = entry.file_name().to_string_lossy();
        if !entry.path().exists() {
            continue; // already deleted together with a junk folder
        }
        if entry.file_type().is_dir() {
            if JUNK_NAMES.contains(&name.as_ref()) {
                let _ = fs::remove_dir_all(entry.path());
            }
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        if entry.file_type().is_symlink() || is_junk(&name, size) {
            let _ = fs::remove_file(entry.path());
            dropped.push(util::rel(root, entry.path()));
        }
    }
    remove_empty_dirs(root);
    dropped
}

/// Removes empty folders below `root`, deepest first (so parents that become empty go too).
fn remove_empty_dirs(root: &Path) {
    for entry in walkdir::WalkDir::new(root).min_depth(1).contents_first(true).into_iter().filter_map(Result::ok) {
        let is_empty_dir = entry.file_type().is_dir()
            && fs::read_dir(entry.path()).map(|mut read_dir| read_dir.next().is_none()).unwrap_or(false);
        if is_empty_dir {
            let _ = fs::remove_dir(entry.path());
        }
    }
}

/// Extracts an import archive into `dest`.
///
/// zip, 7z, tar and tar.gz are tried in Rust first; everything else (rar, tar.xz, tar.bz2,
/// broken zips) goes to an external `7z` or `bsdtar`.
pub fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let lowercase = archive.to_string_lossy().to_lowercase();
    if lowercase.ends_with(".zip") {
        if extract_zip(archive, dest) {
            return Ok(());
        }
    } else if lowercase.ends_with(".7z") {
        if sevenz_rust::decompress_file(archive, dest).is_ok() {
            return Ok(());
        }
    } else if lowercase.ends_with(".tar") || lowercase.ends_with(".tar.gz") || lowercase.ends_with(".tgz") {
        // tar errors are final, there is no external fallback for them.
        let file = fs::File::open(archive).map_err(|e| e.to_string())?;
        let stream: Box<dyn Read> = if lowercase.ends_with(".tar") {
            Box::new(file)
        } else {
            Box::new(flate2::read::GzDecoder::new(file))
        };
        return tar::Archive::new(stream).unpack(dest).map_err(|e| format!("Extraction failed: {e}"));
    }
    extract_external(archive, dest)
}

/// Built-in zip extraction. `true` on success; any failure means "try the external tool".
fn extract_zip(archive: &Path, dest: &Path) -> bool {
    let Ok(file) = fs::File::open(archive) else { return false };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return false };
    zip.extract(dest).is_ok()
}

/// Extracts with an external `7z` or `bsdtar`, whichever is installed and works first.
fn extract_external(archive: &Path, dest: &Path) -> Result<(), String> {
    let archive_str = archive.to_string_lossy().into_owned();
    let attempts = [
        ("7z", vec!["x".to_string(), "-y".into(), "-bd".into(), format!("-o{}", dest.display()), archive_str.clone()]),
        ("bsdtar", vec!["-xf".into(), archive_str, "-C".into(), dest.to_string_lossy().into_owned()]),
    ];
    let mut last_error = None;
    for (program, args) in attempts {
        let Some(exe) = util::which(program) else { continue };
        match adb::cmd(exe).args(&args).output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                last_error = Some(String::from_utf8_lossy(&output.stderr).to_string() + &String::from_utf8_lossy(&output.stdout))
            }
            Err(e) => last_error = Some(e.to_string()),
        }
    }
    Err(match last_error {
        None => "Can't extract the archive – rar and similar formats need 7-Zip.".into(),
        // Only the end of the output: that's where the actual error usually is.
        Some(error) => format!("Extraction failed:\n{}", tail(&error, MAX_ERROR_CHARS)),
    })
}

/// The last `max_bytes` bytes of `text`, moved forward to the next character boundary.
///
/// Rust strings are UTF-8; slicing in the middle of a multi-byte character (e.g. "ä") panics,
/// so the cut position must be a char boundary.
fn tail(text: &str, max_bytes: usize) -> &str {
    let mut cut = text.len().saturating_sub(max_bytes);
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    &text[cut..]
}

/// Fetches a file or folder from the Thor into `dest_dir`. Returns the local path.
///
/// For folders, only files below `IMPORT_MAX_FILE` are fetched (no point downloading ROMs).
pub fn fetch_thor_source(remote: &str, dest_dir: &Path) -> Result<PathBuf, String> {
    let remote = remote.trim_end_matches('/');
    let kind_command = format!("if [ -d {r} ]; then echo D; elif [ -f {r} ]; then echo F; fi", r = shell_quote(remote));
    let kind = sh(&kind_command).stdout.trim().to_string();
    let name = posix_basename(remote);
    let local = dest_dir.join(if name.is_empty() { "thor" } else { name });
    match kind.as_str() {
        "F" => {
            let output = adb::run(&["pull", "-a", remote, &local.to_string_lossy()]);
            if !output.ok {
                return Err(format!("adb pull failed:\n{}", output.err_text()));
            }
        }
        "D" => {
            fs::create_dir_all(&local).map_err(|e| e.to_string())?;
            let tar_command = format!(
                "cd {} && find . -type f -size -{}k | tar -cf - -T -",
                shell_quote(remote),
                IMPORT_MAX_FILE / 1024
            );
            super::transfer::untar_from_thor(&tar_command, &local)?;
        }
        _ => return Err(format!("Doesn't exist on the Thor:\n{remote}")),
    }
    Ok(local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_are_recognized_like_in_the_copy_queue() {
        assert!(is_archive("/sdcard/Download/Zelda Save.ZIP"));
        assert!(is_archive("save.tar.zst"));
        assert!(!is_archive("/sdcard/Download/save.sav"));
    }

    #[test]
    fn tail_never_cuts_inside_a_character() {
        // "ä" is 2 bytes; a cut after 3 bytes from the end would land inside it.
        let text = "xxäyy";
        assert_eq!(tail(text, 3), "yy");
        assert_eq!(tail(text, 4), "äyy");
        assert_eq!(tail(text, 100), text);
    }
}
