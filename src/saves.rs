//! Spielstände zwischen PC und Thor synchronisieren (Port von thor-save).
//!
//! Config und Backups liegen an denselben Stellen wie beim alten Python-Script,
//! also ~/.config/thor-save/games.json und ~/.local/share/thor-save/backups.

use crate::adb::{self, posix_basename, posix_dirname, shell_quote, sh, sh_script, tcmd};
use crate::util::{self, human, when};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const KEEP_BACKUPS: usize = 15;
const PC_MAX_DEPTH: usize = 14;
const THOR_MAX_DEPTH: usize = 12;
const IMPORT_MAX_FILE: u64 = 256 * 1024 * 1024;

const PC_SKIP: &[&str] = &[
    "node_modules", ".git", "__pycache__", "Trash", "flatpak", "baloo", "icons", "shadercache",
    "shader_cache", "GPUCache", "Code Cache", "windows", "mozilla", "Code", "Code - OSS", "Claude",
    "claude", "discord", "Vencord", "spotify", "Element", "net.imput.helium", "obsidian", "keyrings",
    "pki", "vulkan", "crash_dumps", "dump", "load", "Logs", "logs", "thor-save", "thorBro",
    "Program Files", "Program Files (x86)", "Microsoft", "Packages", "Temp", "CrashDumps",
];
const THOR_PRUNE: &[&str] = &[
    "DCIM", "Pictures", "Movies", "Music", "obb", ".thumbnails", "Ringtones", "Notifications",
    "Alarms", "Podcasts", "Audiobooks", "Recordings",
];
const STOPWORDS: &[&str] = &["the", "of", "and", "der", "die", "das", "und", "a", "an", "von", "to"];
const ARCHIVE_EXT: &[&str] = &[
    ".zip", ".7z", ".rar", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2",
];
const ROM_EXT: &[&str] = &[
    ".3ds", ".cci", ".cia", ".cxi", ".3dsx", ".nsp", ".nsz", ".xci", ".xcz", ".iso", ".cso", ".chd",
    ".rvz", ".wbfs", ".wua", ".wux", ".rpx", ".nds", ".gba", ".gbc", ".gb", ".sfc", ".smc", ".nes",
    ".z64", ".n64", ".v64", ".pkg", ".apk", ".m3u", ".cue",
];
const JUNK_EXT: &[&str] = &[
    ".txt", ".nfo", ".url", ".html", ".htm", ".md", ".diz", ".lnk", ".webloc", ".pdf", ".jpg",
    ".jpeg", ".png", ".gif", ".webp", ".mp4", ".exe",
];
const JUNK_NAMES: &[&str] = &["__MACOSX", ".DS_Store", "Thumbs.db", "desktop.ini"];

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

fn save_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"(?i)save|memcard|sram|spielst")
}

pub fn conf_file() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("thor-save").join("games.json")
}

pub fn state_dir() -> PathBuf {
    let d = dirs::data_dir().unwrap_or_default().join("thor-save");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn backups_root() -> PathBuf {
    state_dir().join("backups")
}

pub fn slug(name: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let s = re(&R, r"[^a-z0-9]+").replace_all(&name.to_lowercase(), "-").trim_matches('-').to_string();
    if s.is_empty() { "game".into() } else { s }
}

pub fn tokens(name: &str) -> Vec<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let low = name.to_lowercase();
    let mut t: Vec<String> = re(&R, r"[^a-z0-9]+")
        .split(&low)
        .filter(|w| w.len() >= 3 && !STOPWORDS.contains(w))
        .map(String::from)
        .collect();
    let compact: String = low.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if compact.len() >= 4 {
        t.push(compact);
    }
    t
}

// --- Konfig ------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct GameCfg {
    pub pc: Option<String>,
    pub thor: Option<String>,
}

impl GameCfg {
    pub fn side(&self, side: &str) -> Option<&String> {
        if side == "pc" { self.pc.as_ref() } else { self.thor.as_ref() }
    }
}

pub type Games = BTreeMap<String, GameCfg>;

pub fn load_games() -> Games {
    fs::read_to_string(conf_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_games(g: &Games) -> Result<(), String> {
    let f = conf_file();
    fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = f.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(g).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &f).map_err(|e| e.to_string())
}

pub fn get_game(name: &str) -> Result<GameCfg, String> {
    load_games().get(name).cloned().ok_or_else(|| format!("Game “{name}” doesn't exist"))
}

// --- Snapshots ---------------------------------------------------------------------

/// relpfad -> (md5, mtime, größe). None = Ordner gibt's nicht.
pub type Snap = BTreeMap<String, (String, f64, u64)>;

pub fn pc_snapshot(path: Option<&str>) -> Option<Snap> {
    let p = Path::new(path?);
    if !p.is_dir() {
        return None;
    }
    let mut s = Snap::new();
    for e in walkdir::WalkDir::new(p).into_iter().filter_map(Result::ok) {
        if !e.file_type().is_file() {
            continue;
        }
        let Ok(h) = util::md5_file(e.path()) else { continue };
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        s.insert(util::rel(p, e.path()), (h, util::mtime_of(e.path()), size));
    }
    Some(s)
}

pub fn thor_snapshot(path: Option<&str>) -> Option<Snap> {
    let path = path?;
    let out = sh(&tcmd(
        path,
        &format!(
            "cd {} 2>/dev/null || {{ echo NOPE; exit 0; }}; find . -type f -exec md5sum {{}} + 2>/dev/null; \
             echo ===; find . -type f -exec stat -c '%Y %s %n' {{}} + 2>/dev/null",
            shell_quote(path)
        ),
    ))
    .stdout;
    if out.trim() == "NOPE" {
        return None;
    }
    let (sums, stats) = out.split_once("===\n").unwrap_or((&out, ""));
    let strip = |s: &str| s.strip_prefix("./").unwrap_or(s).to_string();
    let mut meta: HashMap<String, (f64, u64)> = HashMap::new();
    for l in stats.lines() {
        let mut p = l.splitn(3, ' ');
        if let (Some(t), Some(sz), Some(n)) = (p.next(), p.next(), p.next()) {
            if let (Ok(t), Ok(sz)) = (t.parse::<f64>(), sz.parse::<u64>()) {
                meta.insert(strip(n), (t, sz));
            }
        }
    }
    static R: OnceLock<Regex> = OnceLock::new();
    let r = re(&R, r"^([0-9a-f]{32})\s+(.*)$");
    let mut s = Snap::new();
    for l in sums.lines() {
        if let Some(c) = r.captures(l) {
            let relp = strip(&c[2]);
            let (mt, sz) = meta.get(&relp).copied().unwrap_or((0.0, 0));
            s.insert(relp, (c[1].to_string(), mt, sz));
        }
    }
    Some(s)
}

pub fn snapshot(side: &str, path: Option<&str>) -> Option<Snap> {
    if side == "pc" { pc_snapshot(path) } else { thor_snapshot(path) }
}

pub fn summary(s: &Option<Snap>) -> String {
    match s {
        None => "doesn't exist (yet)".into(),
        Some(s) if s.is_empty() => "empty".into(),
        Some(s) => format!(
            "{} files, {}, last changed {}",
            s.len(),
            human(s.values().map(|v| v.2).sum()),
            when(newest(&Some(s.clone())))
        ),
    }
}

pub fn newest(s: &Option<Snap>) -> f64 {
    s.as_ref().map(|s| s.values().map(|v| v.1).fold(0.0, f64::max)).unwrap_or(0.0)
}

pub fn same(a: &Option<Snap>, b: &Option<Snap>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).map(|w| w.0 == v.0).unwrap_or(false)),
        _ => false,
    }
}

#[derive(Serialize)]
pub struct SnapInfo {
    pub exists: bool,
    pub files: usize,
    pub newest: f64,
    pub newest_text: String,
    pub summary: String,
    pub names: Vec<String>,
}

pub fn info(s: &Option<Snap>) -> SnapInfo {
    SnapInfo {
        exists: s.is_some(),
        files: s.as_ref().map_or(0, |s| s.len()),
        newest: newest(s),
        newest_text: when(newest(s)),
        summary: summary(s),
        names: s.as_ref().map(|s| s.keys().take(12).cloned().collect()).unwrap_or_default(),
    }
}

// --- Ordner suchen ------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct Cand {
    pub path: String,
    pub mtime: f64,
    pub when: String,
    pub files: u64,
    pub note: String,
    #[serde(rename = "match")]
    pub matched: bool,
}

fn interesting(path: &str, toks: &[String]) -> bool {
    static TID: OnceLock<Regex> = OnceLock::new();
    let low = path.to_lowercase();
    let parts: Vec<&str> = low.split('/').collect();
    let name = *parts.last().unwrap_or(&"");
    if save_re().is_match(name) || toks.iter().any(|t| name.contains(t.as_str())) {
        return true;
    }
    if re(&TID, r"^[0-9a-f]{16}$").is_match(name) && low.contains("save") {
        return true;
    }
    if name == "remote" && parts.contains(&"userdata") {
        return true;
    }
    // 3DS: …/title/00040000/<title-low>/data/00000001
    let n = parts.len();
    if name == "00000001" && n >= 5 && parts[n - 2] == "data" && parts[n - 5] == "title" {
        return true;
    }
    // bis zu 2 Ebenen unter einem Save-Ordner (z.B. savedata/1/CUSA12345)
    let lo = n.saturating_sub(3);
    parts[lo..n.saturating_sub(1)].iter().any(|p| save_re().is_match(p))
}

fn pc_roots() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut v = vec![
        dirs::data_dir(),
        dirs::config_dir(),
        dirs::data_local_dir(),
        dirs::document_dir(),
        Some(home.join("Games")),
        Some(home.join(".var/app")),
        Some(home.join("Dokumente")),
        Some(home.join("Saved Games")),
        Some(home.join("AppData/LocalLow")),
    ];
    if cfg!(windows) {
        v.push(Some(PathBuf::from("C:/Program Files (x86)/Steam/userdata")));
    }
    let mut seen = BTreeSet::new();
    v.into_iter().flatten().filter(|p| p.is_dir() && seen.insert(p.clone())).collect()
}

fn steam_names() -> HashMap<String, String> {
    static A: OnceLock<Regex> = OnceLock::new();
    static N: OnceLock<Regex> = OnceLock::new();
    let home = dirs::home_dir().unwrap_or_default();
    let mut names = HashMap::new();
    for lib in [
        home.join(".local/share/Steam/steamapps"),
        home.join("Games/steam/steamapps"),
        PathBuf::from("C:/Program Files (x86)/Steam/steamapps"),
    ] {
        let Ok(rd) = fs::read_dir(&lib) else { continue };
        for e in rd.flatten() {
            if !e.file_name().to_string_lossy().starts_with("appmanifest_") {
                continue;
            }
            let Ok(bytes) = fs::read(e.path()) else { continue };
            let txt = String::from_utf8_lossy(&bytes);
            if let (Some(a), Some(n)) = (re(&A, r#""appid"\s+"(\d+)""#).captures(&txt), re(&N, r#""name"\s+"([^"]+)""#).captures(&txt)) {
                names.insert(a[1].to_string(), n[1].to_string());
            }
        }
    }
    names
}

fn annotate(path: &str, steam: &HashMap<String, String>) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"/(?:compatdata|userdata/\d+)/(\d+)(?:/|$)")
        .captures(path)
        .and_then(|c| steam.get(&c[1]).map(|n| format!("[Steam: {n}]")))
        .unwrap_or_default()
}

fn pc_newest(p: &Path, limit: u64) -> (f64, u64) {
    let (mut newest, mut count) = (0.0f64, 0u64);
    for e in walkdir::WalkDir::new(p).into_iter().filter_map(Result::ok) {
        if e.file_type().is_file() {
            newest = newest.max(util::mtime_of(e.path()));
            count += 1;
            if count >= limit {
                break;
            }
        }
    }
    (newest, count)
}

fn rank(mut c: Vec<Cand>) -> Vec<Cand> {
    c.sort_by(|a, b| (!a.matched).cmp(&!b.matched).then(b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal)));
    c.truncate(300);
    c
}

pub fn scan_pc(game: &str) -> Vec<Cand> {
    let toks = tokens(game);
    let steam = steam_names();
    let mut found = Vec::new();
    for root in pc_roots() {
        let walker = walkdir::WalkDir::new(&root).max_depth(PC_MAX_DEPTH).into_iter().filter_entry(|e| {
            if !e.file_type().is_dir() {
                return false;
            }
            let n = e.file_name().to_string_lossy();
            if PC_SKIP.contains(&n.as_ref()) {
                return false;
            }
            !(n == "common" && e.path().parent().map_or(false, |p| p.ends_with("steamapps")))
        });
        for e in walker.filter_map(Result::ok) {
            if e.depth() == 0 {
                continue;
            }
            let s = util::slashed(e.path());
            if interesting(&s, &toks) {
                found.push(e.into_path());
            }
        }
    }
    let mut cands = Vec::new();
    for p in found {
        let (mtime, files) = pc_newest(&p, 5000);
        if files == 0 {
            continue;
        }
        let s = p.to_string_lossy().into_owned();
        let low = util::slashed(&p).to_lowercase();
        cands.push(Cand {
            note: annotate(&util::slashed(&p), &steam),
            matched: toks.iter().any(|t| low.contains(t.as_str())),
            when: when(mtime),
            path: s,
            mtime,
            files,
        });
    }
    rank(cands)
}

fn thor_roots() -> Vec<String> {
    adb::storages().into_iter().map(|(_, p)| p).collect()
}

/// Ordner in den privaten Datenordnern aller debuggable Apps (run-as klappt nur da).
fn private_dirs() -> Vec<String> {
    let script = format!(
        "for p in $(pm list packages -3 | cut -d: -f2); do run-as \"$p\" find \"/data/data/$p\" -maxdepth {THOR_MAX_DEPTH} -type d 2>/dev/null; done\n"
    );
    sh_script(&script).stdout.lines().map(String::from).collect()
}

/// Pro Thor-Pfad: Anzahl Dateien und neueste Änderung, alles in einer Shell-Session.
fn thor_stats(paths: &[String], files_too: bool) -> Vec<(String, u64, f64)> {
    let mut script = String::new();
    for p in paths {
        let body = if files_too {
            format!(
                "d={}; if [ -d \"$d\" ]; then n=$(find \"$d\" -type f 2>/dev/null | wc -l); \
                 t=$(find \"$d\" -type f -exec stat -c %Y {{}} + 2>/dev/null | sort -n | tail -1); \
                 else n=1; t=$(stat -c %Y \"$d\"); fi; echo \"$n|${{t:-0}}|$d\"",
                shell_quote(p)
            )
        } else {
            format!(
                "d={}; n=$(find \"$d\" -type f 2>/dev/null | wc -l); \
                 t=$(find \"$d\" -type f -exec stat -c %Y {{}} + 2>/dev/null | sort -n | tail -1); echo \"$n|${{t:-0}}|$d\"",
                shell_quote(p)
            )
        };
        script += &tcmd(p, &body);
        script.push('\n');
    }
    sh_script(&script)
        .stdout
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '|');
            let n = it.next()?.trim().parse::<u64>().ok()?;
            let t = it.next()?.trim().parse::<f64>().unwrap_or(0.0);
            Some((it.next()?.to_string(), n, t))
        })
        .filter(|x| x.1 > 0)
        .collect()
}

pub fn scan_thor(game: &str) -> Vec<Cand> {
    let toks = tokens(game);
    let prune = THOR_PRUNE.iter().map(|n| format!("-name {}", shell_quote(n))).collect::<Vec<_>>().join(" -o ");
    let roots = thor_roots().iter().map(|r| shell_quote(&format!("{r}/"))).collect::<Vec<_>>().join(" ");
    let out = sh(&format!("find {roots} -maxdepth {THOR_MAX_DEPTH} \\( {prune} \\) -prune -o -type d -print 2>/dev/null")).stdout;
    let mut found: Vec<String> = Vec::new();
    for line in out.lines().map(String::from).chain(private_dirs()) {
        let p = line.trim_end_matches('/').replace("//", "/");
        let low = p.to_lowercase();
        if p.is_empty() || low.contains("/cache") || low.contains("/shader") {
            continue;
        }
        if interesting(&p, &toks) {
            found.push(p);
        }
    }
    found.truncate(800);
    if found.is_empty() {
        return vec![];
    }
    let cands = thor_stats(&found, false)
        .into_iter()
        .map(|(p, n, t)| {
            let low = p.to_lowercase();
            Cand { matched: toks.iter().any(|tk| low.contains(tk.as_str())), when: when(t), path: p, mtime: t, files: n, note: String::new() }
        })
        .collect();
    rank(cands)
}

// --- KI ------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct AiPick {
    pub pc: Option<String>,
    pub pc_reason: Option<String>,
    pub thor: Option<String>,
    pub thor_reason: Option<String>,
}

pub fn ai_pick(game: &str, pc: &[Cand], thor: &[Cand]) -> AiPick {
    let Some(claude) = util::which("claude") else { return AiPick::default() };
    let model = std::env::var("THOR_SAVE_MODEL").unwrap_or_else(|_| "sonnet".into());
    let fmt = |cs: &[Cand]| {
        if cs.is_empty() {
            return "(keine Kandidaten)".to_string();
        }
        cs.iter()
            .map(|c| format!("{} | {} Dateien | {} {}", c.when, c.files, c.path, c.note).trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let os = if cfg!(windows) { "Windows" } else if cfg!(target_os = "macos") { "macOS" } else { "Linux" };
    let prompt = format!(
        r#"I want to sync the save files of "{game}" between my PC ({os}) and my
AYN Thor (Android handheld). Below are folder candidates from both devices
(last modified | number of files | path). On each side, pick the folder that directly
contains the saves of exactly this game.

Important:
- Both folders must match each other (same level/structure), because the contents are
  copied 1:1.
- Switch emulators (Eden, Yuzu, Citron, Sudachi, Ryujinx etc.) store saves under
  nand/user/save/0000000000000000/<user ID>/<TITLE ID>. The user ID differs on every
  device – pick the title ID folder itself. Use your knowledge of Switch title IDs
  (e.g. 0100F2C0115B6000 = Tears of the Kingdom). With several users, pick the most recently changed.
- 3DS emulators (Azahar, Citra, Lime3DS) store saves under
  sdmc/Nintendo 3DS/<id0>/<id1>/title/00040000/<title low, 8 hex>/data/00000001 – pick the
  00000001 folder. Use your knowledge of 3DS title IDs (e.g. Smash Bros. for 3DS:
  000ee000 = EU, 000edf00 = US, 000b8b00 = JP).
- For PS2/PS1/GameCube/Wii/PSP etc., pick the emulator's memory card / save folder accordingly.
- If the game isn't on one of the devices at all, return null for that side.
- On the Thor, paths may be inside a private app folder (/data/data/<package>/…),
  e.g. for native ports like bbport (Bloodborne). That's fine.
- NO ROM/install folders, caches, shader caches, logs or mods.
- Only return paths exactly as they appear in the list. If nothing fits: null.

Reply ONLY with JSON, no explanation around it:
{{"pc": "<path or null>", "pc_reason": "<short, English>", "thor": "<path or null>", "thor_reason": "<short, English>"}}

=== PC ===
{}

=== Thor ===
{}
"#,
        fmt(pc),
        fmt(thor)
    );
    let mut c = adb::cmd(claude);
    c.args(["-p", "--model", &model, "--tools", "", "--no-session-persistence"]);
    let Some(out) = util::run_with_timeout(c, &prompt, Duration::from_secs(240)) else { return AiPick::default() };
    let (Some(a), Some(b)) = (out.find('{'), out.rfind('}')) else { return AiPick::default() };
    let mut pick: AiPick = serde_json::from_str(&out[a..=b]).unwrap_or_default();
    if !pc.iter().any(|c| Some(&c.path) == pick.pc.as_ref()) {
        pick.pc = None;
    }
    if !thor.iter().any(|c| Some(&c.path) == pick.thor.as_ref()) {
        pick.thor = None;
    }
    pick
}

// --- Kopieren --------------------------------------------------------------------------

fn backup_dir(game: &str) -> Result<(PathBuf, String), String> {
    let d = backups_root().join(slug(game));
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let base = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let existing: Vec<String> = fs::read_dir(&d).map_err(|e| e.to_string())?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    let (mut ts, mut n) = (base.clone(), 1);
    while existing.iter().any(|e| e.starts_with(&format!("{ts}-"))) {
        n += 1;
        ts = format!("{base}.{n}");
    }
    Ok((d, ts))
}

fn prune_backups(d: &Path) {
    let Ok(rd) = fs::read_dir(d) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    if entries.len() > KEEP_BACKUPS {
        for old in &entries[..entries.len() - KEEP_BACKUPS] {
            let _ = fs::remove_dir_all(old);
        }
    }
}

fn tempdir(prefix: &str) -> Result<PathBuf, String> {
    tempfile::Builder::new().prefix(prefix).tempdir_in(state_dir()).map(|t| t.keep()).map_err(|e| e.to_string())
}

/// tar-Stream von `adb exec-out` lokal nach dest entpacken.
fn untar_from_thor(command: &str, dest: &Path) -> Result<(), String> {
    let mut child = adb::exec_out(command).map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut ar = tar::Archive::new(stdout);
    ar.set_preserve_mtime(true);
    let res = ar.unpack(dest);
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if let Err(e) = res {
        return Err(format!("Extraction failed: {e}\n{err}"));
    }
    if !status.success() {
        return Err(format!("Fetching from the Thor failed:\n{err}"));
    }
    Ok(())
}

/// Thor-Ordner nach dest holen (dest darf nicht existieren).
pub fn pull_dir(remote: &str, dest: &Path) -> Result<(), String> {
    let tmp = tempdir("pull-")?;
    let name = posix_basename(remote).to_string();
    let res = (|| {
        if adb::private_pkg(remote).is_some() {
            let parent = posix_dirname(remote);
            untar_from_thor(&tcmd(remote, &format!("tar -cf - -C {} {}", shell_quote(parent), shell_quote(&name))), &tmp)?;
        } else {
            let o = adb::run(&["pull", "-a", remote, &tmp.to_string_lossy()]);
            if !o.ok {
                return Err(format!("adb pull failed:\n{}", o.err_text()));
            }
        }
        let got = tmp.join(&name);
        if !got.is_dir() {
            return Err(format!("Fetching failed, {name} didn't arrive"));
        }
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        if fs::rename(&got, dest).is_err() {
            util::copy_tree(&got, dest, false)?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&tmp);
    res
}

fn tar_dir(local: &Path, out: &Path) -> Result<(), String> {
    let f = fs::File::create(out).map_err(|e| e.to_string())?;
    let mut b = tar::Builder::new(f);
    b.follow_symlinks(false);
    let name = local.file_name().ok_or("no folder name")?;
    b.append_dir_all(name, local).map_err(|e| e.to_string())?;
    b.finish().map_err(|e| e.to_string())
}

/// local als tar nach /data/local/tmp pushen, dann per run-as nach stage/<name> entpacken.
fn push_private(local: &Path, stage: &str) -> Result<(), String> {
    let remote_tar = "/data/local/tmp/thor-save-push.tar";
    let tmp = tempfile::Builder::new().prefix("push-").suffix(".tar").tempfile_in(state_dir()).map_err(|e| e.to_string())?;
    let res = (|| {
        tar_dir(local, tmp.path())?;
        let o = adb::run(&["push", &tmp.path().to_string_lossy(), remote_tar]);
        if !o.ok {
            return Err(format!("adb push failed:\n{}", o.err_text()));
        }
        sh(&format!("chmod 644 {}", shell_quote(remote_tar)));
        let o = sh(&tcmd(stage, &format!("rm -rf {s} && mkdir -p {s} && tar -xf {t} -C {s} && echo OK", s = shell_quote(stage), t = shell_quote(remote_tar))));
        if !o.stdout.contains("OK") {
            sh(&tcmd(stage, &format!("rm -rf {}", shell_quote(stage))));
            return Err(format!("Extracting via run-as failed:\n{}", o.err_text()));
        }
        Ok(())
    })();
    sh(&format!("rm -f {}", shell_quote(remote_tar)));
    res
}

pub fn push_dir(local: &Path, remote: &str) -> Result<(), String> {
    let remote = remote.trim_end_matches('/');
    let stage = format!("{remote}.thor-save-neu");
    let lname = local.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let inner = format!("{stage}/{lname}");
    if adb::private_pkg(remote).is_some() {
        push_private(local, &stage)?;
        let o = sh(&tcmd(remote, &format!(
            "test -d {i} && rm -rf {r} && mv {i} {r} && rmdir {s} && echo OK",
            i = shell_quote(&inner), r = shell_quote(remote), s = shell_quote(&stage)
        )));
        if !o.stdout.contains("OK") {
            return Err(format!("Couldn't move the new save into place on the Thor:\n{}\nThe new save may still be in {stage}", o.stderr));
        }
        return Ok(());
    }
    // adbd darf ab Android 11 keine Ordner unter Android/data anlegen, die Shell schon.
    // Also erst nach /data/local/tmp pushen, dann per cp rüber.
    let tmp = "/data/local/tmp/thor-save-push";
    sh(&format!("rm -rf {s} {t}; mkdir -p {s} {t}", s = shell_quote(&stage), t = shell_quote(tmp)));
    let o = adb::run(&["push", &local.to_string_lossy(), &format!("{tmp}/")]);
    if !o.ok {
        sh(&format!("rm -rf {} {}", shell_quote(&stage), shell_quote(tmp)));
        return Err(format!("adb push failed:\n{}", o.err_text()));
    }
    let src = format!("{tmp}/{lname}");
    let o = sh(&format!("cp -r --preserve=t {} {}/ && echo OK; rm -rf {}", shell_quote(&src), shell_quote(&stage), shell_quote(tmp)));
    if !o.stdout.contains("OK") {
        sh(&format!("rm -rf {}", shell_quote(&stage)));
        return Err(format!("Copying on the Thor failed:\n{}", o.err_text()));
    }
    let o = sh(&format!("rm -rf {r} && mv {i} {r} && rmdir {s} && echo OK", r = shell_quote(remote), i = shell_quote(&inner), s = shell_quote(&stage)));
    if !o.stdout.contains("OK") {
        return Err(format!("Couldn't move the new save into place on the Thor:\n{}\nThe new save is still in {stage}", o.stderr));
    }
    Ok(())
}

/// Paketname, wenn der Emulator für diesen Pfad gerade auf dem Thor läuft.
pub fn thor_running_pkg(path: Option<&str>) -> Option<String> {
    let pkg = adb::thor_pkg(path?)?;
    if sh(&format!("pidof {}", shell_quote(&pkg))).stdout.trim().is_empty() { None } else { Some(pkg) }
}

pub fn stop_thor_pkg(pkg: &str) {
    sh(&format!("am force-stop {}", shell_quote(pkg)));
}

// --- Sync ----------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct Status {
    pub pc: SnapInfo,
    pub thor: SnapInfo,
    pub same: bool,
}

pub fn status(cfg: &GameCfg, connected: bool) -> Status {
    let pc = pc_snapshot(cfg.pc.as_deref());
    let thor = if connected { thor_snapshot(cfg.thor.as_deref()) } else { None };
    Status { same: same(&pc, &thor), pc: info(&pc), thor: info(&thor) }
}

#[derive(Serialize)]
pub struct Plan {
    pub target: String,
    pub src_name: String,
    pub dst_name: String,
    pub src: SnapInfo,
    pub dst: SnapInfo,
    pub empty: bool,
    pub same: bool,
    pub dst_newer: bool,
    pub running_pkg: Option<String>,
}

fn snaps_for(cfg: &GameCfg, target: &str) -> (Option<Snap>, Option<Snap>) {
    let pc = pc_snapshot(cfg.pc.as_deref());
    let thor = thor_snapshot(cfg.thor.as_deref());
    if target == "pc" { (thor, pc) } else { (pc, thor) }
}

pub fn plan_sync(cfg: &GameCfg, target: &str) -> Plan {
    let (src, dst) = snaps_for(cfg, target);
    Plan {
        target: target.into(),
        src_name: if target == "pc" { "Thor" } else { "PC" }.into(),
        dst_name: if target == "pc" { "PC" } else { "Thor" }.into(),
        empty: src.as_ref().map_or(true, |s| s.is_empty()),
        same: same(&src, &dst),
        dst_newer: dst.as_ref().map_or(false, |d| !d.is_empty()) && newest(&dst) > newest(&src),
        running_pkg: if target == "thor" { thor_running_pkg(cfg.thor.as_deref()) } else { None },
        src: info(&src),
        dst: info(&dst),
    }
}

/// Backup vom Ziel + kopieren + prüfen. Gibt den Backup-Pfad zurück.
pub fn run_sync(game: &str, cfg: &GameCfg, target: &str, stop_pkg: Option<&str>) -> Result<Option<String>, String> {
    let (pc_path, thor_path) = match (&cfg.pc, &cfg.thor) {
        (Some(a), Some(b)) => (a.clone(), b.clone()),
        _ => return Err("This game is only set up on one side.".into()),
    };
    if let Some(p) = stop_pkg {
        stop_thor_pkg(p);
    }
    let (src, dst) = snaps_for(cfg, target);
    if src.as_ref().map_or(true, |s| s.is_empty()) {
        return Err("There's no save on the source side.".into());
    }
    let (bdir, ts) = backup_dir(game)?;
    let mut bpath = None;
    if dst.is_some() {
        let side = if target == "pc" { "pc" } else { "thor" };
        let b = bdir.join(format!("{ts}-{side}"));
        if target == "pc" {
            util::copy_tree(Path::new(&pc_path), &b, false)?;
        } else {
            pull_dir(&thor_path, &b)?;
        }
        prune_backups(&bdir);
        bpath = Some(b.to_string_lossy().into_owned());
    }
    let after = if target == "pc" {
        let tmp = state_dir().join(format!("neu-{ts}"));
        let r = pull_dir(&thor_path, &tmp).and_then(|_| util::mirror(&tmp, Path::new(&pc_path)));
        let _ = fs::remove_dir_all(&tmp);
        r?;
        pc_snapshot(Some(&pc_path))
    } else {
        sh(&tcmd(&thor_path, &format!("mkdir -p {}", shell_quote(posix_dirname(&thor_path)))));
        push_dir(Path::new(&pc_path), &thor_path)?;
        thor_snapshot(Some(&thor_path))
    };
    if !same(&src, &after) {
        return Err(format!(
            "The files don't match after copying!\nBackup is in {}",
            bpath.clone().unwrap_or_else(|| bdir.to_string_lossy().into_owned())
        ));
    }
    Ok(bpath)
}

// --- Import ------------------------------------------------------------------------------------

fn ext_lower(name: &str) -> String {
    let n = name.to_lowercase();
    n.rfind('.').map(|i| n[i..].to_string()).unwrap_or_default()
}

pub fn is_archive(p: &str) -> bool {
    let l = p.to_lowercase();
    ARCHIVE_EXT.iter().any(|e| l.ends_with(e))
}

fn is_junk(name: &str, size: u64) -> bool {
    if JUNK_NAMES.contains(&name) || name.starts_with("._") {
        return true;
    }
    let e = ext_lower(name);
    JUNK_EXT.contains(&e.as_str()) || ROM_EXT.contains(&e.as_str()) || is_archive(name) || size > IMPORT_MAX_FILE
}

/// Müll rauswerfen, leere Ordner weg. Gibt die rausgeworfenen Dateien zurück.
fn clean_import(root: &Path) -> Vec<String> {
    let mut dropped = Vec::new();
    let entries: Vec<_> = walkdir::WalkDir::new(root).min_depth(1).into_iter().filter_map(Result::ok).collect();
    for e in &entries {
        let name = e.file_name().to_string_lossy();
        if !e.path().exists() {
            continue;
        }
        if e.file_type().is_dir() {
            if JUNK_NAMES.contains(&name.as_ref()) {
                let _ = fs::remove_dir_all(e.path());
            }
            continue;
        }
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        if e.file_type().is_symlink() || is_junk(&name, size) {
            let _ = fs::remove_file(e.path());
            dropped.push(util::rel(root, e.path()));
        }
    }
    for e in walkdir::WalkDir::new(root).min_depth(1).contents_first(true).into_iter().filter_map(Result::ok) {
        if e.file_type().is_dir() && fs::read_dir(e.path()).map(|mut r| r.next().is_none()).unwrap_or(false) {
            let _ = fs::remove_dir(e.path());
        }
    }
    dropped
}

fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let low = archive.to_string_lossy().to_lowercase();
    if low.ends_with(".zip") {
        if let Ok(f) = fs::File::open(archive) {
            if let Ok(mut z) = zip::ZipArchive::new(f) {
                if z.extract(dest).is_ok() {
                    return Ok(());
                }
            }
        }
    } else if low.ends_with(".7z") {
        if sevenz_rust::decompress_file(archive, dest).is_ok() {
            return Ok(());
        }
    } else if low.ends_with(".tar") || low.ends_with(".tar.gz") || low.ends_with(".tgz") {
        let f = fs::File::open(archive).map_err(|e| e.to_string())?;
        let r: Box<dyn Read> = if low.ends_with(".tar") { Box::new(f) } else { Box::new(flate2::read::GzDecoder::new(f)) };
        return tar::Archive::new(r).unpack(dest).map_err(|e| format!("Extraction failed: {e}"));
    }
    // Rest (rar, tar.xz, tar.bz2, kaputte zips): externes 7z oder bsdtar
    let mut err = None;
    for (prog, args) in [
        ("7z", vec!["x".to_string(), "-y".into(), "-bd".into(), format!("-o{}", dest.display()), archive.to_string_lossy().into_owned()]),
        ("bsdtar", vec!["-xf".into(), archive.to_string_lossy().into_owned(), "-C".into(), dest.to_string_lossy().into_owned()]),
    ] {
        let Some(exe) = util::which(prog) else { continue };
        match adb::cmd(exe).args(&args).output() {
            Ok(o) if o.status.success() => return Ok(()),
            Ok(o) => err = Some(String::from_utf8_lossy(&o.stderr).to_string() + &String::from_utf8_lossy(&o.stdout)),
            Err(e) => err = Some(e.to_string()),
        }
    }
    Err(match err {
        None => "Can't extract the archive – rar and similar formats need 7-Zip.".into(),
        Some(e) => format!("Extraction failed:\n{}", &e[e.len().saturating_sub(600)..]),
    })
}

/// Datei/Ordner vom Thor nach dest_dir holen. Bei Ordnern nur Dateien unter IMPORT_MAX_FILE.
fn fetch_thor_source(remote: &str, dest_dir: &Path) -> Result<PathBuf, String> {
    let remote = remote.trim_end_matches('/');
    let kind = sh(&format!("if [ -d {r} ]; then echo D; elif [ -f {r} ]; then echo F; fi", r = shell_quote(remote))).stdout.trim().to_string();
    let name = posix_basename(remote);
    let local = dest_dir.join(if name.is_empty() { "thor" } else { name });
    match kind.as_str() {
        "F" => {
            let o = adb::run(&["pull", "-a", remote, &local.to_string_lossy()]);
            if !o.ok {
                return Err(format!("adb pull failed:\n{}", o.err_text()));
            }
        }
        "D" => {
            fs::create_dir_all(&local).map_err(|e| e.to_string())?;
            untar_from_thor(
                &format!("cd {} && find . -type f -size -{}k | tar -cf - -T -", shell_quote(remote), IMPORT_MAX_FILE / 1024),
                &local,
            )?;
        }
        _ => return Err(format!("Doesn't exist on the Thor:\n{remote}")),
    }
    Ok(local)
}

#[derive(Serialize)]
pub struct Prepared {
    pub work: String,
    pub content: String,
    pub dropped: Vec<String>,
    pub root: String,
    pub root_label: String,
    pub hits: usize,
    pub why: String,
    pub known: usize,
    pub missing: Vec<String>,
    pub root_info: SnapInfo,
    pub dirs: Vec<Cand>,
}

pub fn prepare_import(game: &str, source: &str, from_thor: bool) -> Result<Prepared, String> {
    let cfg = get_game(game)?;
    let work = tempdir("import-")?;
    let content = work.join("inhalt");
    let res = (|| {
        let local = if from_thor {
            let dl = work.join("dl");
            fs::create_dir_all(&dl).map_err(|e| e.to_string())?;
            fetch_thor_source(source, &dl)?
        } else {
            let p = PathBuf::from(source);
            if !p.exists() {
                return Err(format!("Doesn't exist: {source}"));
            }
            p
        };
        if local.is_dir() {
            util::copy_tree(&local, &content, false)?;
        } else if is_archive(&local.to_string_lossy()) {
            extract_archive(&local, &content)?;
        } else {
            fs::create_dir_all(&content).map_err(|e| e.to_string())?;
            fs::copy(&local, content.join(local.file_name().unwrap())).map_err(|e| e.to_string())?;
        }
        let dropped = clean_import(&content);
        if util::files_under(&content).is_empty() {
            let mut m = "There's nothing in there that looks like a save".to_string();
            if !dropped.is_empty() {
                m += &format!(" (filtered out: {})", dropped.iter().take(5).cloned().collect::<Vec<_>>().join(", "));
            }
            return Err(m);
        }
        let mut known: BTreeSet<String> = BTreeSet::new();
        let mut missing = Vec::new();
        for side in ["pc", "thor"] {
            if let Some(p) = cfg.side(side) {
                match snapshot(side, Some(p)) {
                    Some(s) => known.extend(s.into_keys()),
                    None => missing.push(side.to_string()),
                }
            }
        }
        let (root, hits, why) = guess_import_root(&content, &known);
        let dirs = import_dirs(&content)
            .into_iter()
            .map(|(p, sub)| Cand {
                note: if sub.iter().any(|s| known.contains(s)) { "← matches".into() } else { String::new() },
                path: p.to_string_lossy().into_owned(),
                mtime: 0.0,
                when: String::new(),
                files: sub.len() as u64,
                matched: false,
            })
            .collect();
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
    })();
    if res.is_err() {
        let _ = fs::remove_dir_all(&work);
    }
    res
}

fn root_label(content: &Path, root: &Path) -> String {
    match root.strip_prefix(content) {
        Ok(r) if r.as_os_str().is_empty() => "(top level)".into(),
        Ok(r) => util::slashed(r),
        Err(_) => "(adjusted to your save)".into(),
    }
}

#[derive(Serialize)]
pub struct RootCheck {
    pub label: String,
    pub info: SnapInfo,
    pub hits: usize,
    pub why: String,
}

/// Selbst gewählten Unterordner im Paket bewerten.
pub fn check_root(game: &str, content: &str, root: &str) -> Result<RootCheck, String> {
    let cfg = get_game(game)?;
    let mut known: BTreeSet<String> = BTreeSet::new();
    for side in ["pc", "thor"] {
        if let Some(s) = snapshot(side, cfg.side(side).map(|s| s.as_str())) {
            known.extend(s.into_keys());
        }
    }
    let snap = pc_snapshot(Some(root));
    let sub: BTreeSet<String> = snap.as_ref().map(|s| s.keys().cloned().collect()).unwrap_or_default();
    let hits = sub.intersection(&known).count();
    let why = if hits > 0 {
        format!("{hits} of {} files have the same names as in your current save", known.len())
    } else if !known.is_empty() {
        "No file name matches your current save.".into()
    } else {
        "Nothing to compare against, but it looks okay.".into()
    };
    Ok(RootCheck { label: root_label(Path::new(content), Path::new(root)), info: info(&snap), hits, why })
}

/// Alle Ordner im Paket, die Dateien enthalten: [(pfad, relpfade darunter)].
fn import_dirs(content: &Path) -> Vec<(PathBuf, BTreeSet<String>)> {
    let all = util::files_under(content);
    let mut dirs: BTreeSet<String> = BTreeSet::from([String::new()]);
    for f in &all {
        let parts: Vec<&str> = f.split('/').collect();
        for i in 1..parts.len() {
            dirs.insert(parts[..i].join("/"));
        }
    }
    dirs.into_iter()
        .filter_map(|d| {
            let pre = if d.is_empty() { String::new() } else { format!("{d}/") };
            let sub: BTreeSet<String> = all.iter().filter_map(|f| f.strip_prefix(&pre).map(String::from)).collect();
            if sub.is_empty() {
                return None;
            }
            Some((if d.is_empty() { content.to_path_buf() } else { content.join(&d) }, sub))
        })
        .collect()
}

fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// {import_relpfad: ziel_relpfad} für Dateien, die im echten Save woanders liegen.
fn remap_moves(sub: &BTreeSet<String>, known: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut by_base: HashMap<&str, Vec<&String>> = HashMap::new();
    for k in known {
        by_base.entry(basename(k)).or_default().push(k);
    }
    let mut moves = BTreeMap::new();
    for f in sub {
        if known.contains(f) {
            continue;
        }
        if let Some(h) = by_base.get(basename(f)) {
            if h.len() == 1 && !sub.contains(h[0]) {
                moves.insert(f.clone(), h[0].clone());
            }
        }
    }
    moves
}

fn remap_to_known(src: &Path, known: &BTreeSet<String>, dest: &Path) -> Option<(Vec<(String, String)>, Vec<String>)> {
    let sub: BTreeSet<String> = util::files_under(src).into_iter().collect();
    let moves = remap_moves(&sub, known);
    if moves.is_empty() {
        return None;
    }
    let known_dirs: BTreeSet<String> = known
        .iter()
        .flat_map(|k| {
            let parts: Vec<&str> = k.split('/').collect();
            (1..parts.len()).map(move |i| parts[..i].join("/")).collect::<Vec<_>>()
        })
        .collect();
    let _ = fs::remove_dir_all(dest);
    let mut dropped = Vec::new();
    for f in &sub {
        let target = moves.get(f).unwrap_or(f);
        let full = src.join(f);
        let empty = fs::metadata(&full).map(|m| m.len() == 0).unwrap_or(false);
        // leere Datei, die im echten Save ein Ordner ist (save_data als 0-Byte-Datei)
        if known_dirs.contains(target) || (!moves.contains_key(f) && empty && known_dirs.iter().any(|d| d == f || d.starts_with(&format!("{f}/")))) {
            dropped.push(f.clone());
            continue;
        }
        let out = dest.join(target);
        if let Some(p) = out.parent() {
            let _ = fs::create_dir_all(p);
        }
        let _ = fs::copy(&full, &out);
    }
    Some((moves.into_iter().collect(), dropped))
}

/// Welcher Unterordner im Paket entspricht dem Save-Ordner des Spiels?
fn guess_import_root(content: &Path, known: &BTreeSet<String>) -> (PathBuf, usize, String) {
    let cands = import_dirs(content);
    if !known.is_empty() && !cands.is_empty() {
        let known_base: BTreeSet<&str> = known.iter().map(|k| basename(k)).collect();
        let score = |sub: &BTreeSet<String>| {
            let exact = sub.intersection(known).count();
            let base = sub.iter().map(|s| basename(s)).collect::<BTreeSet<_>>().intersection(&known_base).count();
            (exact, base, -(sub.difference(known).count() as i64))
        };
        let best = cands.iter().max_by_key(|c| score(&c.1)).unwrap();
        let (exact, base, _) = score(&best.1);
        if exact == best.1.len() || (exact > 0 && remap_moves(&best.1, known).is_empty()) {
            return (best.0.clone(), exact, format!("{exact} of {} files have the same names as in your current save", known.len()));
        }
        if base > 0 {
            let fixed = content.parent().unwrap_or(content).join("angepasst");
            if let Some((moves, dropped)) = remap_to_known(&best.0, known, &fixed) {
                let fixed_files: BTreeSet<String> = util::files_under(&fixed).into_iter().collect();
                let hits = fixed_files.intersection(known).count();
                let mut why = format!(
                    "Folder structure adjusted to your save ({hits} of {} files match): {}{}",
                    known.len(),
                    moves.iter().take(4).map(|(a, b)| format!("{a} → {b}")).collect::<Vec<_>>().join(", "),
                    if moves.len() > 4 { " …" } else { "" }
                );
                if !dropped.is_empty() {
                    why += &format!("; left out empty placeholders: {}", dropped.join(", "));
                }
                return (fixed, hits, why);
            }
            return (best.0.clone(), 0, format!("{base} file names match your current save, but the folder structure is different – better double-check"));
        }
    }
    // Nix zum Vergleichen: Hüllordner wegschälen (Paket/Paket/… → Inhalt)
    let mut cur = content.to_path_buf();
    loop {
        let entries: Vec<_> = fs::read_dir(&cur).map(|r| r.flatten().collect()).unwrap_or_default();
        if entries.len() == 1 && entries[0].path().is_dir() {
            cur = entries[0].path();
        } else {
            break;
        }
    }
    let why = if !known.is_empty() {
        "No file name matches your current save – are you sure this is the right save?"
    } else {
        "There's no save to compare against yet, so I can't check whether the folder structure is right. \
         Better: start the game once and save, then import – then I'll move the files to the right place myself."
    };
    (cur, 0, why.into())
}

fn touch_tree(root: &Path) {
    let now = filetime::FileTime::now();
    for f in util::files_under(root) {
        let _ = filetime::set_file_mtime(root.join(f), now);
    }
}

/// src_dir in die Save-Ordner der targets schreiben. Vorher wird jedes Ziel gesichert.
pub fn import_save(game: &str, src_dir: &Path, targets: &[String], merge: bool, step: &dyn Fn(&str)) -> Result<(), String> {
    let cfg = get_game(game)?;
    if targets.iter().any(|t| t == "thor") {
        if let Some(pkg) = thor_running_pkg(cfg.thor.as_deref()) {
            step(&format!("Closing {pkg} …"));
            stop_thor_pkg(&pkg);
        }
    }
    touch_tree(src_dir);
    let (bdir, ts) = backup_dir(game)?;
    let res = (|| {
        for side in targets {
            let path = cfg.side(side).cloned().ok_or_else(|| format!("No folder is set up on the {} for “{game}”.", side.to_uppercase()))?;
            let name = if side == "pc" { "PC" } else { "Thor" };
            step(&format!("Backing up the old {name} save …"));
            let dst = snapshot(side, Some(&path));
            let mut bpath = None;
            if dst.as_ref().map_or(false, |d| !d.is_empty()) {
                let b = bdir.join(format!("{ts}-{side}-before-import"));
                if side == "pc" {
                    util::copy_tree(Path::new(&path), &b, false)?;
                } else {
                    pull_dir(&path, &b)?;
                }
                bpath = Some(b);
            }
            let mut final_ = src_dir.to_path_buf();
            if merge {
                if let Some(b) = &bpath {
                    final_ = state_dir().join(format!("merge-{ts}-{side}"));
                    util::copy_tree(b, &final_, false)?;
                    util::copy_tree(src_dir, &final_, true)?;
                }
            }
            step(&format!("Importing the save on the {name} …"));
            let r = (|| {
                let after = if side == "pc" {
                    util::mirror(&final_, Path::new(&path))?;
                    pc_snapshot(Some(&path))
                } else {
                    sh(&tcmd(&path, &format!("mkdir -p {}", shell_quote(posix_dirname(&path)))));
                    push_dir(&final_, &path)?;
                    thor_snapshot(Some(&path))
                };
                if !same(&pc_snapshot(Some(&final_.to_string_lossy())), &after) {
                    return Err(format!(
                        "The files on the {name} don't match after the import!\nBackup is in {}",
                        bpath.as_ref().unwrap_or(&bdir).display()
                    ));
                }
                Ok(())
            })();
            if final_ != src_dir {
                let _ = fs::remove_dir_all(&final_);
            }
            r?;
        }
        Ok(())
    })();
    if fs::read_dir(&bdir).map(|mut r| r.next().is_some()).unwrap_or(false) {
        prune_backups(&bdir);
    }
    res
}

/// Auf dem Thor nach Save-Paketen suchen (Download, Documents, SD-Karte).
pub fn scan_thor_sources(game: &str) -> Vec<Cand> {
    let toks = tokens(game);
    let mut roots = vec!["/sdcard/Download".to_string(), "/sdcard/Documents".to_string()];
    roots.extend(thor_roots().into_iter().filter(|r| r != "/sdcard"));
    let rq = roots.iter().map(|r| shell_quote(&format!("{r}/"))).collect::<Vec<_>>().join(" ");
    let out = sh(&format!("find {rq} -maxdepth 5 \\( -name Android -o -name sdmc -o -name nand \\) -prune -o \\( -type d -o -type f \\) -print 2>/dev/null")).stdout;
    let mut found: Vec<(String, usize)> = Vec::new();
    for line in out.lines() {
        let p = line.trim_end_matches('/').replace("//", "/");
        let name = posix_basename(&p).to_lowercase();
        if name.is_empty() || roots.iter().any(|r| r.trim_end_matches('/') == p) {
            continue;
        }
        let e = ext_lower(&name);
        if ROM_EXT.contains(&e.as_str()) || JUNK_EXT.contains(&e.as_str()) {
            continue;
        }
        let hits = toks.iter().filter(|t| name.contains(t.as_str())).count();
        if hits > 0 || save_re().is_match(&name) {
            found.push((p, hits));
        }
    }
    found.sort_by_key(|x| std::cmp::Reverse(x.1));
    found.truncate(150);
    if found.is_empty() {
        return vec![];
    }
    let hits_of: HashMap<String, usize> = found.iter().cloned().collect();
    let paths: Vec<String> = found.into_iter().map(|x| x.0).collect();
    let mut c: Vec<(usize, Cand)> = thor_stats(&paths, true)
        .into_iter()
        .map(|(p, n, t)| {
            (hits_of.get(&p).copied().unwrap_or(0), Cand {
                note: if is_archive(&p) { "[archive]".into() } else { String::new() },
                when: when(t),
                path: p,
                mtime: t,
                files: n,
                matched: true,
            })
        })
        .collect();
    c.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.mtime.partial_cmp(&a.1.mtime).unwrap_or(std::cmp::Ordering::Equal)));
    c.into_iter().map(|x| x.1).collect()
}
