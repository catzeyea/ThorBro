//! Archive (zip, 7z, tar, rar …) lokal entpacken, bevor sie auf den Thor gehen.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

const EXTS: &[&str] = &[
    ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz", ".tbz2", ".tar", ".zip", ".7z", ".rar",
];

/// Archiv-Endung (lowercase) oder None.
pub fn archive_ext(path: &Path) -> Option<&'static str> {
    let n = path.file_name()?.to_string_lossy().to_lowercase();
    EXTS.iter().copied().find(|e| n.ends_with(e))
}

pub fn is_archive(path: &Path) -> bool {
    path.is_file() && archive_ext(path).is_some()
}

/// Anzahl Dateien im Archiv (nur zip/7z, sonst None).
pub fn file_count(path: &Path) -> Option<usize> {
    match archive_ext(path)? {
        ".zip" => {
            let mut z = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
            Some((0..z.len()).filter(|&i| z.by_index_raw(i).map_or(false, |e| !e.is_dir())).count())
        }
        ".7z" => {
            let r = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty()).ok()?;
            Some(r.archive().files.iter().filter(|f| f.has_stream()).count())
        }
        _ => None,
    }
}

/// Dateiname so umbauen, dass er auf FAT/exFAT (SD-Karte im Thor) erlaubt ist.
pub fn safe_name(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            ':' => s.push_str(" -"),
            '*' | '?' | '"' | '<' | '>' | '|' | '\\' => s.push('_'),
            c if (c as u32) < 0x20 => s.push('_'),
            c => s.push(c),
        }
    }
    let s = s.replace("  -", " -");
    let t = s.trim_end_matches(['.', ' ']).trim_start();
    if t.is_empty() { "_".into() } else { t.to_string() }
}

/// Pfad aus dem Archiv sicher unter root abbilden (kein „..“, keine absoluten Pfade).
fn safe_join(root: &Path, inner: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    let mut any = false;
    for c in Path::new(&inner.replace('\\', "/")).components() {
        if let Component::Normal(p) = c {
            out.push(safe_name(&p.to_string_lossy()));
            any = true;
        }
    }
    any.then_some(out)
}

/// Kopieren mit Fortschritt.
fn copy_counted(r: &mut dyn Read, path: &Path, done: &mut u64, total: u64, prog: &mut dyn FnMut(u64, u64)) -> io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let mut buf = vec![0u8; 1 << 20];
    let mut last = 0u64;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        *done += n as u64;
        if *done - last > 8 << 20 {
            last = *done;
            prog(*done, total);
        }
    }
    w.flush()
}

/// Archiv nach dest entpacken. prog(fertig, gesamt) in Bytes (gesamt 0 = unbekannt).
pub fn extract(archive: &Path, dest: &Path, prog: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let ext = archive_ext(archive).ok_or("Not a known archive type")?;
    let res = match ext {
        ".zip" => zip(archive, dest, prog),
        ".7z" => sevenz(archive, dest, prog),
        ".tar" => tar(Box::new(File::open(archive).map_err(|e| e.to_string())?), dest, prog),
        ".tar.gz" | ".tgz" => {
            let f = File::open(archive).map_err(|e| e.to_string())?;
            tar(Box::new(flate2::read::GzDecoder::new(f)), dest, prog)
        }
        _ => Err(String::new()),
    };
    match res {
        Ok(()) => Ok(()),
        // rar, xz, bz2, zst oder exotische Kompression: externes Tool probieren
        Err(e) => external(archive, dest).map_err(|x| if e.is_empty() { x } else { format!("{e}\n{x}") }),
    }
}

fn zip(archive: &Path, dest: &Path, prog: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    let mut z = zip::ZipArchive::new(File::open(archive).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let total: u64 = (0..z.len()).filter_map(|i| z.by_index_raw(i).ok().map(|e| e.size())).sum();
    let mut done = 0u64;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        let Some(path) = safe_join(dest, e.name()) else { continue };
        if e.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        copy_counted(&mut e, &path, &mut done, total, prog).map_err(|x| format!("{}: {x}", path.display()))?;
    }
    Ok(())
}

fn sevenz(archive: &Path, dest: &Path, prog: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    let total = sevenz_rust::SevenZReader::open(archive, sevenz_rust::Password::empty())
        .map(|r| r.archive().files.iter().map(|f| f.size()).sum())
        .unwrap_or(0);
    let mut done = 0u64;
    sevenz_rust::decompress_file_with_extract_fn(archive, dest, |entry, reader, _| {
        let Some(path) = safe_join(dest, entry.name()) else { return Ok(true) };
        if entry.is_directory() {
            std::fs::create_dir_all(&path).map_err(sevenz_rust::Error::io)?;
        } else {
            copy_counted(reader, &path, &mut done, total, prog).map_err(sevenz_rust::Error::io)?;
        }
        Ok(true)
    })
    .map_err(|e| e.to_string())
}

fn tar(r: Box<dyn Read>, dest: &Path, prog: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    let mut a = ::tar::Archive::new(r);
    let mut done = 0u64;
    for e in a.entries().map_err(|e| e.to_string())? {
        let mut e = e.map_err(|e| e.to_string())?;
        let name = e.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        let Some(path) = safe_join(dest, &name) else { continue };
        match e.header().entry_type() {
            ::tar::EntryType::Directory => std::fs::create_dir_all(&path).map_err(|e| e.to_string())?,
            ::tar::EntryType::Regular | ::tar::EntryType::Continuous => {
                copy_counted(&mut e, &path, &mut done, 0, prog).map_err(|x| format!("{}: {x}", path.display()))?
            }
            _ => {}
        }
    }
    Ok(())
}

/// 7-Zip oder bsdtar aufrufen (für rar & Co.). Danach Namen FAT-tauglich machen.
fn external(archive: &Path, dest: &Path) -> Result<(), String> {
    let a = archive.to_string_lossy();
    let d = dest.to_string_lossy();
    let o_flag = format!("-o{d}");
    let d_slash = format!("{d}/");
    let seven = |t: &'static str| (t, vec!["x", "-y", "-bd", o_flag.as_str(), &*a]);
    let bsd = |t: &'static str| (t, vec!["-xf", &*a, "-C", &*d]);
    // tar.xz & Co.: bsdtar entpackt in einem Rutsch, 7z nur die äußere Schicht
    let tries: Vec<(&str, Vec<&str>)> = if archive_ext(archive).map_or(false, |e| e.starts_with(".t")) {
        vec![bsd("bsdtar"), bsd("tar"), seven("7z"), seven("7zz")]
    } else {
        vec![seven("7z"), seven("7zz"), bsd("bsdtar"), ("unrar", vec!["x", "-o+", &*a, d_slash.as_str()])]
    };
    let mut last = None;
    for (tool, args) in tries {
        let Some(exe) = crate::util::which(tool) else { continue };
        match std::process::Command::new(exe).args(&args).stdin(std::process::Stdio::null()).output() {
            Ok(o) if o.status.success() => {
                fix_names(dest);
                return Ok(());
            }
            Ok(o) => last = Some(format!("{tool}: {}", String::from_utf8_lossy(&o.stderr).trim())),
            Err(e) => last = Some(format!("{tool}: {e}")),
        }
    }
    Err(last.unwrap_or_else(|| "This archive needs 7-Zip (7z) on the PC.".into()))
}

/// Alle Namen unter dir FAT-tauglich umbenennen (von unten nach oben).
fn fix_names(dir: &Path) {
    let entries: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .min_depth(1)
        .contents_first(true)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| e.into_path())
        .collect();
    for p in entries {
        let Some(n) = p.file_name().map(|s| s.to_string_lossy().into_owned()) else { continue };
        let s = safe_name(&n);
        if s != n {
            let _ = std::fs::rename(&p, p.with_file_name(s));
        }
    }
}

/// Was nach dem Entpacken hochgeladen werden soll. Bei Games wird ein einzelner
/// Hüllordner („Spiel.nsp/Spiel.nsp“) übersprungen.
pub fn top_entries(dir: &Path, flatten: bool) -> Vec<PathBuf> {
    let mut root = dir.to_path_buf();
    loop {
        let list: Vec<PathBuf> = std::fs::read_dir(&root)
            .map(|r| r.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default();
        if flatten && list.len() == 1 && list[0].is_dir() {
            root = list[0].clone();
            continue;
        }
        let mut list = list;
        list.sort();
        return list;
    }
}

/// Arbeitsordner fürs Entpacken (nicht /tmp, das ist oft im RAM).
pub fn work_dir() -> Result<tempfile::TempDir, String> {
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("thorbro");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    tempfile::Builder::new().prefix("entpacken-").tempdir_in(base).map_err(|e| e.to_string())
}

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

