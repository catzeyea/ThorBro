//! Dateimanager für den Thor und die Kopier-Warteschlange PC -> Thor.

use crate::adb::{self, shell_quote, sh};
use crate::util::human;
use crate::unpack;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

pub fn listdir(path: &str) -> Result<Vec<Entry>, String> {
    let out = sh(&format!("cd {} && stat -c '%F|%s|%n' -- * .[!.]* 2>/dev/null", shell_quote(path)));
    let mut items: Vec<Entry> = out
        .stdout
        .lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, '|');
            let (kind, size, name) = (p.next()?, p.next()?, p.next()?);
            Some(Entry { name: name.to_string(), dir: kind.contains("directory"), size: size.parse().unwrap_or(0) })
        })
        .collect();
    items.sort_by(|a, b| (!a.dir, a.name.to_lowercase()).cmp(&(!b.dir, b.name.to_lowercase())));
    Ok(items)
}

fn check(o: adb::Out) -> Result<(), String> {
    if o.ok { Ok(()) } else { Err(o.err_text()) }
}

pub fn mkdir(path: &str) -> Result<(), String> {
    check(sh(&format!("mkdir -p {}", shell_quote(path))))
}

pub fn rename(from: &str, to: &str) -> Result<(), String> {
    check(sh(&format!("mv -- {} {}", shell_quote(from), shell_quote(to))))
}

pub fn delete(path: &str) -> Result<(), String> {
    if path.trim_end_matches('/').matches('/').count() < 2 {
        return Err("I'd rather not delete that (too high up in the file system).".into());
    }
    check(sh(&format!("rm -rf -- {}", shell_quote(path))))
}

pub fn pull_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Downloads")).join("thor")
}

pub fn pull(src: &str) -> Result<String, String> {
    let dest = pull_dir();
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    check(adb::run(&["pull", src, &dest.to_string_lossy()]))?;
    Ok(dest.to_string_lossy().into_owned())
}

// --- letzter Ordner ------------------------------------------------------------

fn last_path_file() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("thorBro").join("last_path")
}

pub fn load_last() -> Option<String> {
    std::fs::read_to_string(last_path_file()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn save_last(path: &str) {
    let f = last_path_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, path);
}

// --- Kopieren ----------------------------------------------------------------------

pub fn size_of(path: &Path) -> u64 {
    if path.is_file() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .sum()
}

#[derive(Serialize, Clone)]
pub struct Progress {
    pub busy: bool,
    pub text: String,
    pub detail: String,
    pub fraction: f64,
    pub dests: Vec<String>,
    pub error: Option<String>,
}

/// Ein Kopierauftrag. `unpack`: Archiv vorher entpacken. `flatten`: einzelnen Hüllordner
/// im Archiv überspringen (bei Games). `remote`: src liegt auf dem Thor (dort entpacken).
#[derive(Deserialize, Clone)]
pub struct Job {
    pub src: String,
    pub dest: String,
    #[serde(default)]
    pub unpack: bool,
    #[serde(default)]
    pub flatten: bool,
    #[serde(default)]
    pub remote: bool,
}

#[derive(Default)]
pub struct Queue {
    jobs: VecDeque<Job>,
    busy: bool,
}

pub type SharedQueue = Arc<Mutex<Queue>>;

pub fn enqueue(app: AppHandle, queue: SharedQueue, jobs: Vec<Job>) {
    let start = {
        let mut q = queue.lock().unwrap();
        q.jobs.extend(jobs);
        if q.busy { false } else { q.busy = true; true }
    };
    if start {
        std::thread::spawn(move || worker(app, queue));
    }
}

fn emit(app: &AppHandle, p: Progress) {
    let _ = app.emit("transfer", p);
}

fn status(app: &AppHandle, text: String, detail: String, fraction: f64) {
    emit(app, Progress { busy: true, text, detail, fraction, dests: vec![], error: None });
}

/// Größe einer Datei/eines Ordners auf dem Thor (für den Fortschritt).
fn remote_size(path: &str) -> u64 {
    sh(&format!("du -sk {} 2>/dev/null | cut -f1", shell_quote(path))).stdout.trim().parse::<u64>().unwrap_or(0) * 1024
}

/// Eine Datei/einen Ordner per adb push nach dest/<name> schieben, mit Fortschritt.
/// Der Zielname wird FAT-tauglich gemacht (kein „:“ usw.), sonst lehnt die SD-Karte ab.
fn push_one(app: &AppHandle, src: &Path, dest: &str, more: &str) -> Result<(), String> {
    let name = src.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let tname = unpack::safe_name(&name);
    let total = size_of(src).max(1);
    let target = format!("{}/{}", dest.trim_end_matches('/'), tname);
    status(app, format!("Copying {tname} ({})", human(total)), format!("to {dest}{more}"), 0.0);
    let _ = sh(&format!("mkdir -p {}", shell_quote(dest)));
    let push_to = if src.is_dir() { format!("{}/", dest.trim_end_matches('/')) } else { target.clone() };
    // Ordner mit ungültigem Namen: unter neuem Namen hochladen geht bei adb nur über den Zielordner
    let (push_src, _tmp) = if src.is_dir() && tname != name {
        let t = unpack::work_dir()?;
        let link = t.path().join(&tname);
        #[cfg(unix)]
        std::os::unix::fs::symlink(src, &link).map_err(|e| e.to_string())?;
        #[cfg(not(unix))]
        copy_dir(src, &link)?;
        (link, Some(t))
    } else {
        (src.to_path_buf(), None)
    };
    let mut child = adb::cmd(adb::adb_path())
        .args(["push", &push_src.to_string_lossy(), &push_to])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{name}: {e}"))?;
    // Fortschritt: alle ~0,7 s nachschauen, wie viel schon auf dem Thor liegt
    let started = Instant::now();
    let mut last_poll = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => {}
        }
        std::thread::sleep(Duration::from_millis(150));
        if last_poll.elapsed() >= Duration::from_millis(700) {
            last_poll = Instant::now();
            let done = remote_size(&target).min(total);
            let secs = started.elapsed().as_secs_f64().max(0.1);
            let speed = done as f64 / secs;
            let eta = if speed > 0.0 { ((total - done) as f64 / speed) as u64 } else { 0 };
            status(
                app,
                format!("Copying {tname}"),
                format!(
                    "{} of {} · {}/s · about {}:{:02} min left{more}",
                    human(done), human(total), human(speed as u64), eta / 60, eta % 60
                ),
                done as f64 / total as f64,
            );
        }
    }
    match child.wait_with_output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let t = String::from_utf8_lossy(&o.stderr).to_string() + &String::from_utf8_lossy(&o.stdout);
            Err(format!("{name}: {}", t.trim().lines().last().unwrap_or("?")))
        }
        Err(e) => Err(format!("{name}: {e}")),
    }
}

#[cfg(not(unix))]
fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    for e in walkdir::WalkDir::new(src).into_iter().filter_map(Result::ok) {
        let rel = e.path().strip_prefix(src).unwrap();
        let to = dst.join(rel);
        if e.file_type().is_dir() {
            std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
        } else {
            std::fs::copy(e.path(), &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Archiv entpacken (vorher vom Thor holen, falls remote) und den Inhalt hochladen.
fn unpack_job(app: &AppHandle, job: &Job, more: &str) -> Result<u32, String> {
    let work = unpack::work_dir()?;
    let local = if job.remote {
        let name = job.src.rsplit('/').next().unwrap_or("archiv").to_string();
        status(app, format!("Fetching {name} from the Thor"), format!("to extract it{more}"), 0.0);
        let o = adb::run(&["pull", &job.src, &work.path().to_string_lossy()]);
        if !o.ok {
            return Err(format!("{name}: {}", o.err_text()));
        }
        work.path().join(&name)
    } else {
        PathBuf::from(&job.src)
    };
    let name = local.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let out = work.path().join("inhalt");
    status(app, format!("Extracting {name}"), format!("on the PC{more}"), 0.0);
    let started = Instant::now();
    unpack::extract(&local, &out, &mut |done, total| {
        let speed = done as f64 / started.elapsed().as_secs_f64().max(0.1);
        let (detail, frac) = if total > 0 {
            (format!("{} of {} · {}/s{more}", human(done), human(total), human(speed as u64)), done as f64 / total as f64)
        } else {
            (format!("{} · {}/s{more}", human(done), human(speed as u64)), 0.0)
        };
        status(app, format!("Extracting {name}"), detail, frac);
    })
    .map_err(|e| format!("{name}: extraction failed: {e}"))?;
    if job.remote {
        let _ = std::fs::remove_file(&local);
    }
    let items = unpack::top_entries(&out, job.flatten);
    if items.is_empty() {
        return Err(format!("{name}: archive is empty"));
    }
    let mut n = 0;
    for it in &items {
        push_one(app, it, &job.dest, more)?;
        n += 1;
    }
    Ok(n)
    // work (TempDir) räumt beim Verlassen auf
}

fn worker(app: AppHandle, queue: SharedQueue) {
    let (mut ok, mut fail) = (0u32, 0u32);
    let mut dests: BTreeSet<String> = BTreeSet::new();
    let mut last_err = None;
    loop {
        let (job, left) = {
            let mut q = queue.lock().unwrap();
            let j = q.jobs.pop_front();
            if j.is_none() {
                q.busy = false;
            }
            (j, q.jobs.len())
        };
        let Some(job) = job else { break };
        dests.insert(job.dest.clone());
        let more = if left > 0 { format!(" · {left} more waiting") } else { String::new() };
        let res = if job.unpack {
            unpack_job(&app, &job, &more)
        } else {
            push_one(&app, Path::new(&job.src), &job.dest, &more).map(|_| 1)
        };
        match res {
            Ok(n) => ok += n,
            Err(e) => {
                fail += 1;
                last_err = Some(e);
            }
        }
    }
    // Medienscanner anstoßen, damit Bilder/Musik sofort in Apps auftauchen
    for d in &dests {
        let _ = sh(&format!(
            "find {} -type f -newer /proc/self 2>/dev/null | head -n 2000 | while read -r f; do \
             am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d \"file://$f\" >/dev/null; done",
            shell_quote(d)
        ));
    }
    let where_ = if dests.len() == 1 { dests.iter().next().cloned().unwrap_or_default() } else { format!("{} folders", dests.len()) };
    let mut text = format!("Done: {ok} copied to {where_}");
    if fail > 0 {
        text += &format!(", {fail} failed");
    }
    emit(&app, Progress { busy: false, text, detail: String::new(), fraction: 1.0, dests: dests.into_iter().collect(), error: last_err });
}

// --- Rausziehen Thor -> PC ---------------------------------------------------------------
//
// Zum Rausziehen muss die Datei lokal liegen. Sie wird erst in den Cache geholt
// (~/.cache/thorbro/ziehen/<hash>/<name>), danach startet ein echter System-Drag.
// Schon geholte Sachen werden wiederverwendet, solange sie sich auf dem Thor nicht geändert haben.

pub fn drag_root() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("thorbro").join("ziehen")
}

/// Alte Sachen (älter als einen Tag) aus dem Zieh-Cache löschen.
pub fn drag_cleanup() {
    let Ok(rd) = std::fs::read_dir(drag_root()) else { return };
    let day = Duration::from_secs(24 * 3600);
    for e in rd.filter_map(Result::ok) {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(true, |a| a > day);
        if old {
            let p = e.path();
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
    }
}

/// Fingerabdruck einer Datei/eines Ordners auf dem Thor: Änderungszeit + Größe.
fn remote_stamp(path: &str) -> String {
    let o = sh(&format!("stat -c '%Y' -- {p} 2>/dev/null; du -sk -- {p} 2>/dev/null | cut -f1", p = shell_quote(path)));
    o.stdout.split_whitespace().collect::<Vec<_>>().join("|")
}

#[derive(Serialize)]
pub struct Fetched {
    pub local: String,
    /// true = musste erst vom Thor geholt werden (dauert), false = lag schon im Cache
    pub fetched: bool,
}

/// Datei/Ordner vom Thor in den Zieh-Cache holen (mit Fortschritt) und den lokalen Pfad liefern.
pub fn drag_fetch(app: &AppHandle, src: &str) -> Result<Fetched, String> {
    use md5::{Digest, Md5};
    let src = src.trim_end_matches('/');
    let name = src.rsplit('/').next().filter(|s| !s.is_empty()).ok_or("Invalid path")?.to_string();
    let key = format!("{:x}", Md5::digest(src.as_bytes()));
    let root = drag_root();
    let dir = root.join(&key);
    let stamp_file = root.join(format!("{key}.stamp"));
    let local = dir.join(&name);
    let stamp = remote_stamp(src);
    if stamp.is_empty() {
        return Err(format!("{name} doesn't exist on the Thor (anymore)."));
    }
    if local.exists() && std::fs::read_to_string(&stamp_file).ok().as_deref() == Some(stamp.as_str()) {
        // Zeitstempel auffrischen, damit das Aufräumen es nicht gleich wegwirft
        let _ = filetime::set_file_mtime(&dir, filetime::FileTime::now());
        return Ok(Fetched { local: local.to_string_lossy().into_owned(), fetched: false });
    }
    let part = root.join(format!("{key}.part"));
    let _ = std::fs::remove_dir_all(&part);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&part).map_err(|e| e.to_string())?;

    let total = remote_size(src).max(1);
    status(app, format!("Fetching {name} from the Thor ({})", human(total)), "to drag out".into(), 0.0);
    let mut child = adb::cmd(adb::adb_path())
        .args(["pull", src, &part.to_string_lossy()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{name}: {e}"))?;
    let started = Instant::now();
    let mut last_poll = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => {}
        }
        std::thread::sleep(Duration::from_millis(100));
        if last_poll.elapsed() >= Duration::from_millis(500) {
            last_poll = Instant::now();
            let done = size_of(&part).min(total);
            let speed = done as f64 / started.elapsed().as_secs_f64().max(0.1);
            let eta = if speed > 0.0 { ((total - done) as f64 / speed) as u64 } else { 0 };
            status(
                app,
                format!("Fetching {name} from the Thor"),
                format!("{} of {} · {}/s · about {}:{:02} min left · then drag it out", human(done), human(total), human(speed as u64), eta / 60, eta % 60),
                done as f64 / total as f64,
            );
        }
    }
    let out = child.wait_with_output().map_err(|e| format!("{name}: {e}"))?;
    let finish = |text: String, error: Option<String>| {
        emit(app, Progress { busy: false, text, detail: String::new(), fraction: 1.0, dests: vec![], error });
    };
    if !out.status.success() {
        let t = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
        let e = format!("{name}: {}", t.trim().lines().last().unwrap_or("?"));
        let _ = std::fs::remove_dir_all(&part);
        finish(format!("{name} couldn't be fetched"), Some(e.clone()));
        return Err(e);
    }
    std::fs::rename(&part, &dir).map_err(|e| e.to_string())?;
    let _ = std::fs::write(&stamp_file, &stamp);
    finish(format!("{name} is ready to drag out"), None);
    Ok(Fetched { local: local.to_string_lossy().into_owned(), fetched: true })
}
