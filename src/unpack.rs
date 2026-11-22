//! Extracts archives (zip, 7z, tar, rar …) on the PC before their contents go to the Thor.
//!
//! zip, 7z, tar and tar.gz are handled in Rust. Everything else (rar, tar.xz, tar.bz2, tar.zst,
//! unusual compression methods) falls back to an external `7z`, `bsdtar`, `tar` or `unrar`.
//! All file names are made FAT/exFAT-safe, because the Thor's SD card rejects names like `a:b`.
//!
//! Used by `files.rs` (the copy queue extracts before pushing, and uses `safe_name` for every
//! upload) and by `main.rs` (`classify` checks `is_archive` / `file_count` for dropped files).

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

/// Archive extensions we recognize (lowercase, matched against the end of the file name).
const EXTS: &[&str] = &[
    ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz", ".tbz2", ".tar", ".zip", ".7z", ".rar",
];

/// Buffer size for reading and writing extracted files (1 MiB).
const COPY_BUFFER_SIZE: usize = 1 << 20;

/// Report progress at most every 8 MiB, so the UI isn't flooded with events.
const PROGRESS_STEP_BYTES: u64 = 8 << 20;

/// Callback type for progress reports: `progress(bytes_done, bytes_total)`, total `0` = unknown.
///
/// `&mut dyn FnMut(u64, u64)` is a reference to "any callable that takes two `u64`" — similar to
/// `std::function<void(uint64_t, uint64_t)>&` in C++. `FnMut` means the callable may change its
/// own captured state when called.
type Progress<'a> = &'a mut dyn FnMut(u64, u64);

/// The archive extension of `path` (lowercase, with dot), or `None` if it isn't an archive.
pub fn archive_ext(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    EXTS.iter().copied().find(|ext| name.ends_with(ext))
}

/// True if `path` is an existing file with an archive extension.
pub fn is_archive(path: &Path) -> bool {
    path.is_file() && archive_ext(path).is_some()
}

/// Number of files inside the archive. Only zip and 7z can be counted cheaply; other types
/// (and unreadable archives) return `None`.
pub fn file_count(path: &Path) -> Option<usize> {
    match archive_ext(path)? {
        ".zip" => {
            let mut zip = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
            let is_file = |index: usize, zip: &mut zip::ZipArchive<File>| {
                zip.by_index_raw(index).is_ok_and(|entry| !entry.is_dir())
            };
            Some((0..zip.len()).filter(|&index| is_file(index, &mut zip)).count())
        }
        ".7z" => {
            let reader = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty()).ok()?;
            Some(reader.archive().files.iter().filter(|file| file.has_stream()).count())
        }
        _ => None,
    }
}

/// Changes a file name so it is allowed on FAT/exFAT (the Thor's SD card).
///
/// `:` becomes ` -` (so "Zelda: BotW" reads "Zelda - BotW"), other forbidden characters and
/// control characters become `_`, and trailing dots/spaces are removed.
pub fn safe_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            ':' => result.push_str(" -"),
            '*' | '?' | '"' | '<' | '>' | '|' | '\\' => result.push('_'),
            // A *match guard*: this arm only matches if the condition after `if` is true.
            c if (c as u32) < 0x20 => result.push('_'),
            c => result.push(c),
        }
    }
    // "Zelda : BotW" would otherwise become "Zelda  - BotW" (two spaces).
    let result = result.replace("  -", " -");
    let trimmed = result.trim_end_matches(['.', ' ']).trim_start();
    if trimmed.is_empty() { "_".into() } else { trimmed.to_string() }
}

/// Maps a path from inside an archive safely below `root`.
///
/// Only normal path components are kept, so `..` and absolute paths can't escape `root`
/// ("zip slip"). Every component is made FAT-safe. Returns `None` if nothing is left.
fn safe_join(root: &Path, inner: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    let mut has_component = false;
    for component in Path::new(&inner.replace('\\', "/")).components() {
        if let Component::Normal(part) = component {
            out.push(safe_name(&part.to_string_lossy()));
            has_component = true;
        }
    }
    // `bool::then_some(x)` = `if self { Some(x) } else { None }`.
    has_component.then_some(out)
}

/// Copies `reader` into a new file at `path` and reports progress.
///
/// `bytes_done` is shared across all files of one archive, so it is passed by `&mut` (like a
/// non-const reference in C++) and updated here.
fn copy_counted(reader: &mut dyn Read, path: &Path, bytes_done: &mut u64, total: u64, progress: Progress) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::with_capacity(COPY_BUFFER_SIZE, File::create(path)?);
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    let mut last_report = 0u64;
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        writer.write_all(&buffer[..bytes_read])?;
        // `*bytes_done` dereferences the reference, just like `*ptr` in C++.
        *bytes_done += bytes_read as u64;
        if *bytes_done - last_report > PROGRESS_STEP_BYTES {
            last_report = *bytes_done;
            progress(*bytes_done, total);
        }
    }
    writer.flush()
}

/// Extracts `archive` into `dest`.
///
/// `progress(done, total)` is called with byte counts while extracting (total `0` = unknown).
/// If the built-in extractor fails or doesn't support the type, an external tool is tried.
/// On failure both error messages are returned so the user sees what went wrong.
pub fn extract(archive: &Path, dest: &Path, progress: Progress) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let ext = archive_ext(archive).ok_or("Not a known archive type")?;
    let builtin_result = match ext {
        ".zip" => extract_zip(archive, dest, progress),
        ".7z" => extract_7z(archive, dest, progress),
        ".tar" => {
            let file = File::open(archive).map_err(|e| e.to_string())?;
            extract_tar(Box::new(file), dest, progress)
        }
        ".tar.gz" | ".tgz" => {
            let file = File::open(archive).map_err(|e| e.to_string())?;
            extract_tar(Box::new(flate2::read::GzDecoder::new(file)), dest, progress)
        }
        // Empty error = "not supported in Rust", see below.
        _ => Err(String::new()),
    };
    match builtin_result {
        Ok(()) => Ok(()),
        // rar, xz, bz2, zst or exotic compression: try an external tool.
        Err(builtin_error) => extract_external(archive, dest).map_err(|external_error| {
            if builtin_error.is_empty() { external_error } else { format!("{builtin_error}\n{external_error}") }
        }),
    }
}

/// Built-in zip extraction.
fn extract_zip(archive: &Path, dest: &Path, progress: Progress) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let total: u64 = (0..zip.len()).filter_map(|index| zip.by_index_raw(index).ok().map(|entry| entry.size())).sum();
    let mut bytes_done = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let Some(path) = safe_join(dest, entry.name()) else { continue };
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        copy_counted(&mut entry, &path, &mut bytes_done, total, progress)
            .map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(())
}

/// Built-in 7z extraction.
fn extract_7z(archive: &Path, dest: &Path, progress: Progress) -> Result<(), String> {
    let total = sevenz_rust::SevenZReader::open(archive, sevenz_rust::Password::empty())
        .map(|reader| reader.archive().files.iter().map(|file| file.size()).sum())
        .unwrap_or(0);
    let mut bytes_done = 0u64;
    // The library calls our closure once per entry; returning `Ok(true)` means "continue".
    sevenz_rust::decompress_file_with_extract_fn(archive, dest, |entry, reader, _| {
        let Some(path) = safe_join(dest, entry.name()) else { return Ok(true) };
        if entry.is_directory() {
            std::fs::create_dir_all(&path).map_err(sevenz_rust::Error::io)?;
        } else {
            copy_counted(reader, &path, &mut bytes_done, total, progress).map_err(sevenz_rust::Error::io)?;
        }
        Ok(true)
    })
    .map_err(|e| e.to_string())
}

/// Built-in tar extraction from any byte stream (plain file or gzip decoder).
///
/// `Box<dyn Read>` is a heap-allocated object behind a trait interface, like
/// `std::unique_ptr<std::istream>` in C++. A *trait* (`Read`) is Rust's version of an
/// interface / abstract base class.
fn extract_tar(stream: Box<dyn Read>, dest: &Path, progress: Progress) -> Result<(), String> {
    // `::tar` means the external `tar` crate, not this module's own `tar` names.
    let mut tar_archive = ::tar::Archive::new(stream);
    let mut bytes_done = 0u64;
    for entry in tar_archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        let Some(path) = safe_join(dest, &name) else { continue };
        match entry.header().entry_type() {
            ::tar::EntryType::Directory => std::fs::create_dir_all(&path).map_err(|e| e.to_string())?,
            ::tar::EntryType::Regular | ::tar::EntryType::Continuous => {
                copy_counted(&mut entry, &path, &mut bytes_done, 0, progress)
                    .map_err(|err| format!("{}: {err}", path.display()))?
            }
            _ => {} // links, devices etc. are skipped
        }
    }
    Ok(())
}

/// Calls 7-Zip, bsdtar, tar or unrar (for rar and friends), then makes all names FAT-safe.
///
/// The tools are tried in order; the first one that exists and succeeds wins.
/// If none works, the last error (or a hint to install 7-Zip) is returned.
fn extract_external(archive: &Path, dest: &Path) -> Result<(), String> {
    let archive_str = archive.to_string_lossy();
    let dest_str = dest.to_string_lossy();
    let seven_zip_out_flag = format!("-o{dest_str}");
    let dest_with_slash = format!("{dest_str}/");

    // Small closures that build `(tool name, arguments)` for the two command-line styles.
    let seven_zip = |tool: &'static str| (tool, vec!["x", "-y", "-bd", seven_zip_out_flag.as_str(), &*archive_str]);
    let bsd_tar = |tool: &'static str| (tool, vec!["-xf", &*archive_str, "-C", &*dest_str]);

    // tar.xz & co.: bsdtar extracts everything in one go, 7z only removes the outer layer.
    let is_tar_family = archive_ext(archive).is_some_and(|ext| ext.starts_with(".t"));
    let attempts: Vec<(&str, Vec<&str>)> = if is_tar_family {
        vec![bsd_tar("bsdtar"), bsd_tar("tar"), seven_zip("7z"), seven_zip("7zz")]
    } else {
        vec![
            seven_zip("7z"),
            seven_zip("7zz"),
            bsd_tar("bsdtar"),
            ("unrar", vec!["x", "-o+", &*archive_str, dest_with_slash.as_str()]),
        ]
    };

    let mut last_error = None;
    for (tool, args) in attempts {
        let Some(exe) = crate::util::which(tool) else { continue };
        match std::process::Command::new(exe).args(&args).stdin(std::process::Stdio::null()).output() {
            Ok(output) if output.status.success() => {
                fix_names(dest);
                return Ok(());
            }
            Ok(output) => last_error = Some(format!("{tool}: {}", String::from_utf8_lossy(&output.stderr).trim())),
            Err(e) => last_error = Some(format!("{tool}: {e}")),
        }
    }
    Err(last_error.unwrap_or_else(|| "This archive needs 7-Zip (7z) on the PC.".into()))
}

/// Renames everything below `dir` to FAT-safe names, deepest entries first
/// (so renaming a folder doesn't invalidate the paths of its children we still have to visit).
fn fix_names(dir: &Path) {
    let entries: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .min_depth(1)
        .contents_first(true)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .collect();
    for path in entries {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let fixed = safe_name(&name);
        if fixed != name {
            let _ = std::fs::rename(&path, path.with_file_name(fixed));
        }
    }
}

/// What should be uploaded after extracting: the entries directly inside `dir`, sorted.
///
/// With `flatten == true` (games), single wrapper folders are skipped: if an archive contains
/// only `Game.nsp/Game.nsp`, the inner file is uploaded instead of the folder.
pub fn top_entries(dir: &Path, flatten: bool) -> Vec<PathBuf> {
    let mut root = dir.to_path_buf();
    loop {
        let entries: Vec<PathBuf> = std::fs::read_dir(&root)
            .map(|read_dir| read_dir.filter_map(Result::ok).map(|entry| entry.path()).collect())
            .unwrap_or_default();
        if flatten && entries.len() == 1 && entries[0].is_dir() {
            root = entries[0].clone();
            continue;
        }
        let mut entries = entries;
        entries.sort();
        return entries;
    }
}

/// Creates a temporary work folder for extracting, in the user's cache folder
/// (not `/tmp`, which is often a RAM disk too small for games).
///
/// The returned `TempDir` deletes the folder automatically when it is dropped (RAII).
pub fn work_dir() -> Result<tempfile::TempDir, String> {
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("thorbro");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    tempfile::Builder::new().prefix("entpacken-").tempdir_in(base).map_err(|e| e.to_string())
}

// `#[cfg(test)]` = this module is only compiled for `cargo test`, like code inside
// `#ifdef UNIT_TESTS` in C++.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(safe_name("The Legend of Zelda: Breath of the Wild.nsp"), "The Legend of Zelda - Breath of the Wild.nsp");
        assert_eq!(safe_name("a?b*c."), "a_b_c");
        assert!(safe_join(Path::new("/r"), "../../etc/passwd").unwrap().starts_with("/r"));
    }
}
