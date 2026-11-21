//! Bildschirme vom Thor per scrcpy auf den PC spiegeln (Port von thor-stream).

use std::process::Child;
use std::sync::Mutex;

const COMMON: &[&str] = &["--no-control", "--max-fps=60", "--video-bit-rate=16M"];

#[derive(Default)]
pub struct Streams {
    running: Mutex<Vec<(String, Child)>>,
}

fn scrcpy() -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap();
        // neben ThorBro oder im mitgelieferten tools/-Ordner (Windows-Release)
        for sub in [dir.to_path_buf(), dir.join("tools")] {
            for name in ["scrcpy", "scrcpy.exe"] {
                let p = sub.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    crate::util::which("scrcpy")
}

impl Streams {
    fn spawn(&self, which: &str) -> Result<(), String> {
        let exe = scrcpy().ok_or("scrcpy not found – please install scrcpy (see README)")?;
        let mut args: Vec<&str> = COMMON.to_vec();
        if which == "top" {
            args.extend(["--display-id=0", "--window-title=Thor – top"]);
        } else {
            args.extend(["--display-id=4", "--no-audio", "--window-title=Thor – bottom"]);
        }
        let mut c = std::process::Command::new(exe);
        c.args(&args);
        // scrcpy soll das gleiche adb nehmen wie ThorBro
        c.env("ADB", crate::adb::adb_path());
        let child = c.spawn().map_err(|e| e.to_string())?;
        self.running.lock().unwrap().push((which.to_string(), child));
        Ok(())
    }

    /// which: "both", "top" or "bottom" (the old German "beide"/"oben"/"unten" still work).
    /// A screen that is already running isn't started twice.
    pub fn start(&self, which: &str) -> Result<(), String> {
        self.cleanup();
        let want: &[&str] = match which {
            "top" | "oben" => &["top"],
            "bottom" | "unten" => &["bottom"],
            _ => &["top", "bottom"],
        };
        for w in want {
            let running = self.running.lock().unwrap().iter().any(|(n, _)| n == w);
            if !running {
                self.spawn(w)?;
            }
        }
        Ok(())
    }

    pub fn stop(&self) {
        for (_, mut c) in self.running.lock().unwrap().drain(..) {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Beendete Fenster aus der Liste werfen. Gibt die laufenden zurück.
    pub fn cleanup(&self) -> Vec<String> {
        let mut r = self.running.lock().unwrap();
        r.retain_mut(|(_, c)| matches!(c.try_wait(), Ok(None)));
        r.iter().map(|(n, _)| n.clone()).collect()
    }
}
