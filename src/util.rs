//! Kleinkram: Formatierung, Hashes, Ordner kopieren und spiegeln (ersetzt rsync).

use md5::{Digest, Md5};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn human(n: u64) -> String {
    let mut f = n as f64;
    for u in ["B", "KB", "MB", "GB"] {
        if f < 1024.0 {
            return if u == "B" { format!("{n} B") } else { format!("{f:.1} {u}") };
        }
        f /= 1024.0;
    }
    format!("{f:.1} TB")
}

pub fn when(ts: f64) -> String {
    if ts <= 0.0 {
        return "—".into();
    }
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

pub fn mtime_of(p: &Path) -> f64 {
    fs::symlink_metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn md5_file(p: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(p)?;
    let mut h = Md5::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

/// Relativer Pfad mit '/' als Trenner (gleich auf Windows und Thor).
pub fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Pfad als String mit '/' (für Mustersuche, auch unter Windows).
pub fn slashed(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn files_under(root: &Path) -> Vec<String> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| rel(root, e.path()))
        .collect()
}

fn copy_file_keep_time(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(d) = dst.parent() {
        fs::create_dir_all(d)?;
    }
    fs::copy(src, dst)?;
    if let Ok(m) = fs::metadata(src) {
        let _ = filetime::set_file_mtime(dst, filetime::FileTime::from_last_modification_time(&m));
    }
    Ok(())
}

/// Ganzen Ordner kopieren (wie shutil.copytree), Zeitstempel bleiben.
/// `merge` = in einen vorhandenen Ordner hinein kopieren.
pub fn copy_tree(src: &Path, dst: &Path, merge: bool) -> Result<(), String> {
    if dst.exists() && !merge {
        return Err(format!("Already exists: {}", dst.display()));
    }
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for e in walkdir::WalkDir::new(src).min_depth(1).into_iter().filter_map(Result::ok) {
        let target = dst.join(e.path().strip_prefix(src).unwrap());
        if e.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else if e.file_type().is_file() {
            copy_file_keep_time(e.path(), &target).map_err(|er| format!("{}: {er}", e.path().display()))?;
        }
    }
    Ok(())
}

/// dst exakt wie src machen (rsync -a --checksum --delete).
pub fn mirror(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let mut keep: BTreeSet<PathBuf> = BTreeSet::new();
    for e in walkdir::WalkDir::new(src).min_depth(1).into_iter().filter_map(Result::ok) {
        let relp = e.path().strip_prefix(src).unwrap().to_path_buf();
        let target = dst.join(&relp);
        keep.insert(relp);
        if e.file_type().is_dir() {
            if target.is_file() {
                fs::remove_file(&target).map_err(|e| e.to_string())?;
            }
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if !e.file_type().is_file() {
            continue;
        }
        if target.is_dir() {
            fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
        }
        let same = target.is_file()
            && fs::metadata(&target).map(|m| m.len()).ok() == e.metadata().map(|m| m.len()).ok()
            && md5_file(&target).ok() == md5_file(e.path()).ok();
        if same {
            if let Ok(m) = e.metadata() {
                let _ = filetime::set_file_mtime(&target, filetime::FileTime::from_last_modification_time(&m));
            }
        } else {
            copy_file_keep_time(e.path(), &target).map_err(|er| format!("{}: {er}", e.path().display()))?;
        }
    }
    // Alles löschen, was es in src nicht gibt (tiefste Einträge zuerst)
    let mut extra: Vec<PathBuf> = walkdir::WalkDir::new(dst)
        .min_depth(1)
        .contents_first(true)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| e.path().to_path_buf())
        .filter(|p| !keep.contains(p.strip_prefix(dst).unwrap()))
        .collect();
    extra.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for p in extra {
        if p.is_dir() {
            let _ = fs::remove_dir_all(&p);
        } else {
            let _ = fs::remove_file(&p);
        }
    }
    Ok(())
}

pub fn which(name: &str) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        vec![".exe".into(), ".cmd".into(), ".bat".into(), String::new()]
    } else {
        vec![String::new()]
    };
    let mut dirs_: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(h) = dirs::home_dir() {
        dirs_.push(h.join(".local/bin"));
    }
    for d in dirs_ {
        for e in &exts {
            let p = d.join(format!("{name}{e}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Programm mit stdin-Text und Zeitlimit laufen lassen. Gibt stdout zurück.
pub fn run_with_timeout(mut c: Command, input: &str, timeout: Duration) -> Option<String> {
    use std::io::Write;
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    if let Some(mut s) = child.stdin.take() {
        let _ = s.write_all(input.as_bytes());
    }
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(_) => return None,
        }
    }
    reader.join().ok()
}

/// Ordner oder Datei im Dateimanager des Systems öffnen.
pub fn open_in_os(p: &Path) -> Result<(), String> {
    let prog = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(prog).arg(p).spawn().map(|_| ()).map_err(|e| e.to_string())
}
