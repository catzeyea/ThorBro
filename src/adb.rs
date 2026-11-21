//! Alles rund um adb: Gerät finden, Shell-Befehle, Dateien schieben.
//!
//! adb wird als externes Programm aufgerufen. Gesucht wird neben der ThorBro-Datei,
//! im PATH und in den üblichen Android-SDK-Ordnern, damit es auch unter Windows klappt.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const EXE: &str = if cfg!(windows) { "adb.exe" } else { "adb" };

fn find_adb() -> PathBuf {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join(EXE));
            cands.push(dir.join("tools").join(EXE)); // vom Windows-Release mitgeliefert
            cands.push(dir.join("platform-tools").join(EXE));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            cands.push(dir.join(EXE));
        }
    }
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(sdk) = std::env::var_os(var) {
            cands.push(PathBuf::from(sdk).join("platform-tools").join(EXE));
        }
    }
    if let Some(local) = dirs::data_local_dir() {
        cands.push(local.join("Android").join("Sdk").join("platform-tools").join(EXE));
    }
    if let Some(home) = dirs::home_dir() {
        cands.push(home.join("Library/Android/sdk/platform-tools").join(EXE));
        cands.push(home.join("Android/Sdk/platform-tools").join(EXE));
    }
    cands.into_iter().find(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(EXE))
}

pub fn adb_path() -> &'static PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(find_adb)
}

/// Ein Command ohne aufpoppendes Konsolenfenster (Windows).
pub fn cmd(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

fn adb_cmd() -> Command {
    cmd(adb_path())
}

pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn err_text(&self) -> String {
        let s = if self.stderr.trim().is_empty() { &self.stdout } else { &self.stderr };
        s.trim().to_string()
    }
}

fn to_out(o: std::io::Result<Output>) -> Out {
    match o {
        Ok(o) => Out {
            ok: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Err(e) => Out { ok: false, stdout: String::new(), stderr: format!("couldn't start adb: {e}") },
    }
}

pub fn run<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Out {
    to_out(adb_cmd().args(args).stdin(Stdio::null()).output())
}

/// Einen Befehl in der Thor-Shell ausführen.
pub fn sh(command: &str) -> Out {
    run(&["shell", command])
}

/// Längeres Shell-Script über stdin an `adb shell sh` geben (umgeht Längenlimits).
pub fn sh_script(script: &str) -> Out {
    let child = adb_cmd()
        .args(["shell", "sh"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return Out { ok: false, stdout: String::new(), stderr: e.to_string() },
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }
    to_out(child.wait_with_output())
}

/// `adb exec-out <cmd>` als laufender Prozess, stdout ist ein Byte-Stream (z.B. tar).
pub fn exec_out(command: &str) -> std::io::Result<Child> {
    adb_cmd()
        .args(["exec-out", command])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// Shell-Quoting für die Thor-Seite (sh).
pub fn q(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+=:,@".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[derive(serde::Serialize, Clone)]
pub struct Device {
    pub connected: bool,
    pub message: String,
}

pub fn device_status() -> Device {
    let o = run(&["devices"]);
    if !o.ok && o.stdout.is_empty() {
        return Device {
            connected: false,
            message: if cfg!(windows) {
                "adb not found – keep the tools folder next to ThorBro.exe or install the Android Platform-Tools".into()
            } else {
                "adb not found – please install adb / android-tools (see README)".into()
            },
        };
    }
    let devs: Vec<(String, String)> = o
        .stdout
        .lines()
        .skip(1)
        .filter_map(|l| l.split_once('\t'))
        .map(|(a, b)| (a.trim().to_string(), b.trim().to_string()))
        .collect();
    let Some((_, state)) = devs.first() else {
        return Device { connected: false, message: "No Thor found – is the cable plugged in and USB debugging on?".into() };
    };
    match state.as_str() {
        "device" => {
            let model = sh("getprop ro.product.model").stdout.trim().to_string();
            Device { connected: true, message: if model.is_empty() { "Thor".into() } else { model } }
        }
        "unauthorized" => Device { connected: false, message: "Confirm the USB debugging prompt on the Thor".into() },
        s => Device { connected: false, message: format!("Thor status: {s}") },
    }
}

/// [(Anzeigename, Pfad)] – interner Speicher plus SD-Karten/USB-Sticks.
pub fn storages() -> Vec<(String, String)> {
    let mut s = vec![("Internal storage".to_string(), "/sdcard".to_string())];
    for n in sh("ls /storage").stdout.split_whitespace() {
        if n != "emulated" && n != "self" {
            s.push((format!("SD card ({n})"), format!("/storage/{n}")));
        }
    }
    s
}

/// Paketname, wenn der Pfad im privaten App-Ordner liegt (nur per run-as erreichbar).
pub fn private_pkg(path: &str) -> Option<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"^/data/(?:data|user/\d+)/([^/]+)(?:/|$)").unwrap());
    re.captures(path).map(|c| c[1].to_string())
}

/// Shell-Befehl für einen Thor-Pfad – im privaten App-Ordner über run-as.
pub fn tcmd(path: &str, command: &str) -> String {
    match private_pkg(path) {
        Some(pkg) => format!("run-as {} sh -c {}", q(&pkg), q(command)),
        None => command.to_string(),
    }
}

/// Paket, zu dem ein Thor-Pfad gehört (Android/data/<paket> oder privat).
pub fn thor_pkg(path: &str) -> Option<String> {
    let p = format!("{path}/");
    if let Some(i) = p.find("/Android/data/") {
        let rest = &p[i + "/Android/data/".len()..];
        return rest.split('/').next().filter(|s| !s.is_empty()).map(String::from);
    }
    private_pkg(path)
}

pub fn posix_basename(p: &str) -> &str {
    p.trim_end_matches('/').rsplit('/').next().unwrap_or(p)
}

pub fn posix_dirname(p: &str) -> &str {
    let t = p.trim_end_matches('/');
    match t.rfind('/') {
        Some(0) => "/",
        Some(i) => &t[..i],
        None => ".",
    }
}
