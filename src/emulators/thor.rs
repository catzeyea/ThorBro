//! Resolving profiles on the connected Thor (all adb): which emulators are installed, what
//! `{base}` and `{user}` are, which save folders exist, which games an emulator brings itself.
//!
//! Every path read from the Thor (ini values, search hits) passes `is_allowed_thor_path` before
//! it is used. All paths are shell-quoted.

use super::{expand, ids, is_allowed_thor_path, normalize_thor_path, IniRef, Profile, Search, UserResolver, Vars};
use crate::adb::{self, sh, sh_script, shell_quote};
use std::collections::BTreeSet;

/// Most folders listed by `browse_dirs`.
const MAX_BROWSE_DIRS: usize = 400;

/// How deep `browse_dirs` looks below the emulator's data folder.
const BROWSE_DEPTH: usize = 7;

/// Third-party packages installed on the Thor (`pm list packages -3`). `None` if adb failed.
pub fn installed_packages() -> Option<BTreeSet<String>> {
    let output = sh("pm list packages -3");
    if !output.ok {
        return None;
    }
    Some(parse_packages(&output.stdout))
}

fn parse_packages(output: &str) -> BTreeSet<String> {
    output.lines().filter_map(|line| line.trim().strip_prefix("package:")).map(String::from).collect()
}

/// A profile resolved on the Thor.
pub struct Resolved<'a> {
    pub profile: &'a Profile,
    /// `{base}` candidates (empty if the profile has no base). A search can find several.
    pub bases: Vec<String>,
    pub user: Option<String>,
    /// Why the profile can't be used right now (shown on the emulator chip).
    pub error: Option<String>,
}

fn read_text(path: &str) -> String {
    sh(&format!("cat {} 2>/dev/null", shell_quote(path))).stdout
}

fn read_hex(path: &str) -> String {
    sh(&format!("xxd -p {} 2>/dev/null", shell_quote(path))).stdout
}

fn read_ini(profile: &Profile, ini: &IniRef, base: Option<&str>) -> Result<Option<String>, String> {
    let file = expand(&ini.file, &profile.vars(base, None, None))?;
    Ok(ids::ini_value(&read_text(&file), ini.section.as_deref(), &ini.key))
}

/// Resolves `{base}` and `{user}` of `profile` on the Thor.
pub fn resolve(profile: &Profile) -> Resolved<'_> {
    match resolve_parts(profile) {
        Ok((bases, user)) => {
            let error = (profile.saves.base.is_some() && bases.is_empty()).then(|| "Its data folder wasn't found on the Thor.".to_string());
            Resolved { profile, bases, user, error }
        }
        Err(error) => Resolved { profile, bases: vec![], user: None, error: Some(error) },
    }
}

fn resolve_parts(profile: &Profile) -> Result<(Vec<String>, Option<String>), String> {
    let bases = match &profile.saves.base {
        None => vec![],
        Some(base) => match (&base.path, &base.search) {
            (_, Some(search)) => search_bases(search),
            (Some(path), None) => {
                let default = expand(path, &profile.vars(None, None, None))?;
                let configured = match &base.ini {
                    Some(ini) => read_ini(profile, ini, None)?,
                    None => None,
                };
                let chosen = match configured {
                    Some(value) => {
                        let value = normalize_thor_path(&value);
                        if !is_allowed_thor_path(&value) {
                            return Err(format!("Its config points to {value}, which ThorBro doesn't read."));
                        }
                        value
                    }
                    None => default,
                };
                vec![chosen]
            }
            (None, None) => vec![],
        },
    };
    let user = match &profile.saves.user {
        None => None,
        Some(user) => {
            let base = bases.first().map(String::as_str);
            let file = expand(&user.file, &profile.vars(base, None, None))?;
            let index = match &user.index {
                Some(ini) => read_ini(profile, ini, base)?.and_then(|value| value.parse::<usize>().ok()),
                None => None,
            };
            let data = ids::hex_bytes(&read_hex(&file)).unwrap_or_default();
            match user.resolver {
                UserResolver::SwitchProfilesDat => {
                    Some(ids::switch_user_folder(&data, index).ok_or("Couldn't read the emulator's user (profiles.dat).")?)
                }
            }
        }
    };
    Ok((bases, user))
}

/// Folders on any storage whose path ends in `search.marker`; returns the part before it.
fn search_bases(search: &Search) -> Vec<String> {
    let pattern = shell_quote(&format!("*/{}", search.marker));
    let mut script = String::new();
    for (_, root) in adb::storages() {
        script += &format!(
            "find {} -maxdepth {} \\( -name Android -o -name .thumbnails \\) -prune -o -type d -path {pattern} -print 2>/dev/null\n",
            shell_quote(&format!("{root}/")),
            search.depth
        );
    }
    let suffix = format!("/{}", search.marker);
    let mut bases: Vec<String> = Vec::new();
    for line in sh_script(&script).stdout.lines() {
        let path = normalize_thor_path(&line.replace("//", "/"));
        let Some(base) = path.strip_suffix(&suffix) else { continue };
        if is_allowed_thor_path(base) && !bases.iter().any(|known| known == base) {
            bases.push(base.to_string());
        }
    }
    bases
}

/// Placeholder that marks the title segment while listing (never a valid path character run).
const WILD_SWITCH: &str = "????????????????";
const WILD_LOW: &str = "????????";

impl Resolved<'_> {
    /// `[None]` without a base, else one entry per base.
    fn base_options(&self) -> Vec<Option<&str>> {
        if self.profile.saves.base.is_none() {
            vec![None]
        } else {
            self.bases.iter().map(|base| Some(base.as_str())).collect()
        }
    }

    fn vars(&self, base: Option<&str>, title_id: Option<&str>) -> Vars {
        self.profile.vars(base, self.user.as_deref(), title_id)
    }

    /// The save folder of `title_id`, one per base.
    pub fn folders_for(&self, title_id: &str) -> Vec<String> {
        if self.error.is_some() {
            return vec![];
        }
        self.base_options()
            .into_iter()
            .filter_map(|base| expand(&self.profile.saves.folder, &self.vars(base, Some(title_id))).ok())
            .map(|path| normalize_thor_path(&path))
            .collect()
    }

    /// The folder template with the title segment as `?` wildcards, per base.
    fn wildcard_folders(&self) -> Vec<String> {
        if self.error.is_some() {
            return vec![];
        }
        self.base_options()
            .into_iter()
            .filter_map(|base| {
                let mut vars = self.vars(base, None);
                vars.insert(super::PH_TITLE_ID, WILD_SWITCH.into());
                vars.insert(super::PH_TITLE_ID_LOW, WILD_LOW.into());
                expand(&self.profile.saves.folder, &vars).ok().map(|path| normalize_thor_path(&path))
            })
            .collect()
    }

    /// All existing save folders as `(title ID, path)`. Siblings like Peanuts' `<id>.work` and
    /// `<id>.lock` never match, because the title segment must be exactly 16 (or 8) hex digits.
    pub fn list_saves(&self) -> Vec<(String, String)> {
        let wildcards = self.wildcard_folders();
        if wildcards.is_empty() {
            return vec![];
        }
        let mut script = String::new();
        for wildcard in &wildcards {
            script += &format!("for d in {}; do [ -d \"$d\" ] && echo \"$d\"; done\n", shell_glob(wildcard));
        }
        let output = sh_script(&script).stdout;
        let mut found: Vec<(String, String)> = Vec::new();
        for line in output.lines() {
            if let Some(hit) = self.parse_listed(&wildcards, line) {
                if !found.contains(&hit) {
                    found.push(hit);
                }
            }
        }
        found.sort();
        found
    }

    /// `(title ID, path)` if `line` is one of the wildcard folders with a valid title ID.
    fn parse_listed(&self, wildcards: &[String], line: &str) -> Option<(String, String)> {
        let path = normalize_thor_path(line);
        let parts: Vec<&str> = path.split('/').collect();
        for wildcard in wildcards {
            let pattern: Vec<&str> = wildcard.split('/').collect();
            if pattern.len() != parts.len() {
                continue;
            }
            let index = pattern.iter().position(|segment| *segment == WILD_SWITCH || *segment == WILD_LOW)?;
            let segment = parts[index];
            let title_id = if pattern[index] == WILD_LOW {
                let low_ok = segment.len() == 8 && segment.chars().all(|c| c.is_ascii_hexdigit());
                if !low_ok {
                    continue;
                }
                format!("00040000{}", segment.to_uppercase())
            } else {
                let Some(title_id) = self.profile.saves.title_id.normalize(segment) else { continue };
                if segment != title_id {
                    continue; // Switch folders are uppercase; anything else isn't a save folder.
                }
                title_id
            };
            if self.folders_for(&title_id).contains(&path) {
                return Some((title_id, path));
            }
        }
        None
    }

    /// The emulator's data folder, where "Wrong folder? Browse" starts: `{appdata}` if the
    /// saves are inside it, otherwise the first base.
    pub fn data_root(&self) -> Option<String> {
        let appdata = self.profile.appdata();
        let first = self.wildcard_folders().into_iter().next()?;
        if first.starts_with(&format!("{appdata}/")) {
            return Some(appdata);
        }
        self.bases.first().cloned()
    }

    /// The folder that directly holds the per-game save folders (`…/saves`, `…/<user>`).
    pub fn save_root(&self) -> Option<String> {
        let first = self.wildcard_folders().into_iter().next()?;
        let index = first.find(WILD_SWITCH).or_else(|| first.find(WILD_LOW))?;
        Some(first[..index].trim_end_matches('/').to_string())
    }
}

/// A Thor path with `?` wildcard segments as a shell glob: literal segments quoted.
fn shell_glob(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.is_empty() {
                String::new()
            } else if segment.chars().all(|c| c == '?') {
                segment.to_string()
            } else {
                shell_quote(segment)
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Games the emulator brings itself (`games.dirs`): `(title ID, name from name_file)`.
pub fn pack_games(profile: &Profile) -> Vec<(String, Option<String>)> {
    let Some(games) = &profile.games else { return vec![] };
    let Ok(dir) = expand(&games.dirs, &profile.vars(None, None, None)) else { return vec![] };
    let name_file = games.name_file.clone().unwrap_or_default();
    let script = format!(
        "for d in {}/*/; do n=${{d%/}}; echo \"@@${{n##*/}}\"; [ -n {nf} ] && head -n1 \"$d\"{nf} 2>/dev/null; done",
        shell_quote(&dir),
        nf = shell_quote(&name_file)
    );
    parse_pack_listing(&sh(&script).stdout, profile)
}

fn parse_pack_listing(output: &str, profile: &Profile) -> Vec<(String, Option<String>)> {
    let mut games: Vec<(String, Option<String>)> = Vec::new();
    for line in output.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            if let Some(title_id) = profile.saves.title_id.normalize(name).filter(|title_id| title_id == name) {
                games.push((title_id, None));
            } else {
                games.push((String::new(), None)); // not a game folder (e.g. Peanuts' "peanuts")
            }
        } else if let Some(last) = games.last_mut() {
            if last.1.is_none() {
                last.1 = ids::pack_title(line);
            }
        }
    }
    games.retain(|(title_id, _)| !title_id.is_empty());
    games
}

/// Folders below the emulator's data folder for the "Wrong folder? Browse" picker, as
/// `(path, file count, newest mtime)` (only folders with files). Folders inside the save root
/// come first, then newest first. Caches and shader folders are skipped.
pub fn browse_dirs(resolved: &Resolved) -> Vec<(String, u64, f64)> {
    let Some(root) = resolved.data_root() else { return vec![] };
    let save_root = resolved.save_root().unwrap_or_else(|| root.clone());
    let output = sh(&format!(
        "find {} -maxdepth {BROWSE_DEPTH} \\( -name cache -o -name shader -o -name shaders -o -name log -o -name logs -o -name gpu_drivers \\) -prune -o -type d -print 2>/dev/null",
        shell_quote(&format!("{root}/"))
    ))
    .stdout;
    let mut dirs: Vec<String> = output
        .lines()
        .map(|line| normalize_thor_path(&line.replace("//", "/")))
        .filter(|path| path != &root && is_allowed_thor_path(path))
        .collect();
    dirs.truncate(MAX_BROWSE_DIRS);
    let mut stats = crate::saves::thor_stats(&dirs, false);
    let inside = |path: &str| path.starts_with(&format!("{save_root}/"));
    stats.sort_by(|a, b| {
        let by_time = b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal);
        inside(&b.0).cmp(&inside(&a.0)).then(by_time)
    });
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emulators::load_from;

    #[test]
    fn packages_from_pm_output() {
        let packages = parse_packages("package:dev.eden.eden_emulator\npackage:dev.igawa6.edenduo\r\nnoise\n");
        assert!(packages.contains("dev.igawa6.edenduo") && packages.contains("dev.eden.eden_emulator") && packages.len() == 2);
    }

    #[test]
    fn globs_quote_everything_but_the_wildcard() {
        assert_eq!(
            shell_glob("/storage/X/ROMs/n3ds/sdmc/Nintendo 3DS/title/00040000/????????/data/00000001"),
            "/storage/X/ROMs/n3ds/sdmc/'Nintendo 3DS'/title/00040000/????????/data/00000001"
        );
        assert_eq!(shell_glob("/sdcard/a b/????????????????"), "/sdcard/'a b'/????????????????");
    }

    /// Listing output on the Thor for Peanuts and Eden Duo (`for d in <glob>` echoes the hits).
    #[test]
    fn listed_save_folders_ignore_work_and_lock() {
        let profiles = load_from(None);
        let peanuts = profiles.by_id("peanuts").unwrap();
        let resolved = Resolved { profile: peanuts, bases: vec![], user: None, error: None };
        let wildcards = resolved.wildcard_folders();
        assert_eq!(wildcards, ["/sdcard/Android/data/dev.nxnative.android.peanuts/files/saves/????????????????"]);
        let root = "/sdcard/Android/data/dev.nxnative.android.peanuts/files/saves";
        let parse = |line: String| resolved.parse_listed(&wildcards, &line);
        assert_eq!(parse(format!("{root}/01007EF00011E000")), Some(("01007EF00011E000".into(), format!("{root}/01007EF00011E000"))));
        assert_eq!(parse(format!("{root}/01007EF00011E000.work")), None);
        assert_eq!(parse(format!("{root}/01007EF00011E000.lock")), None);
        assert_eq!(parse(format!("{root}/01007ef00011e000")), None, "lowercase isn't a Switch save folder");
        assert_eq!(resolved.save_root().as_deref(), Some(root));
        assert_eq!(resolved.data_root().as_deref(), Some("/sdcard/Android/data/dev.nxnative.android.peanuts"));

        let eden_duo = profiles.by_id("eden-duo").unwrap();
        let base = "/sdcard/Android/data/dev.igawa6.edenduo/files/nand".to_string();
        let resolved = Resolved { profile: eden_duo, bases: vec![base.clone()], user: Some("5D1B4CDA089A581DAA9E48BC2B43C120".into()), error: None };
        let wildcards = resolved.wildcard_folders();
        let good = format!("{base}/user/save/0000000000000000/5D1B4CDA089A581DAA9E48BC2B43C120/01007EF00011E000");
        let orphan = format!("{base}/user/save/0000000000000000/1DAA9E48BC2B43C12000000000000000/01006A800016E000");
        assert_eq!(resolved.parse_listed(&wildcards, &good).map(|hit| hit.0).as_deref(), Some("01007EF00011E000"));
        assert_eq!(resolved.parse_listed(&wildcards, &orphan), None, "orphan user folders are ignored");
        assert_eq!(resolved.data_root().as_deref(), Some("/sdcard/Android/data/dev.igawa6.edenduo"));
    }

    #[test]
    fn azahar_listing_uses_the_low_title_id() {
        let profiles = load_from(None);
        let azahar = profiles.by_id("azahar").unwrap();
        let base = "/storage/EEFF-BF6C/ROMs/n3ds".to_string();
        let resolved = Resolved { profile: azahar, bases: vec![base.clone()], user: None, error: None };
        let wildcards = resolved.wildcard_folders();
        let folder = format!("{base}/sdmc/Nintendo 3DS/00000000000000000000000000000000/00000000000000000000000000000000/title/00040000/000ee000/data/00000001");
        assert_eq!(resolved.parse_listed(&wildcards, &folder), Some(("00040000000EE000".into(), folder.clone())));
        assert_eq!(resolved.folders_for("00040000000EE000"), [folder]);
        assert_eq!(resolved.data_root().as_deref(), Some(base.as_str()));
        // Nothing resolved = nothing listed.
        let broken = Resolved { profile: azahar, bases: vec![], user: None, error: Some("x".into()) };
        assert!(broken.folders_for("00040000000EE000").is_empty() && broken.wildcard_folders().is_empty());
    }

    #[test]
    fn peanuts_packs_from_listing() {
        let profiles = load_from(None);
        let peanuts = profiles.by_id("peanuts").unwrap();
        let output = "@@0100000000010000\n# Super Mario Odyssey (Nintendo EPD's sead / agl engine), update 1.4.1.\n\
                      @@01007EF00011E000\n# Game profile of The Legend of Zelda: Breath of the Wild (Switch, base game v0)\n@@peanuts\n";
        assert_eq!(
            parse_pack_listing(output, peanuts),
            [("0100000000010000".to_string(), Some("Super Mario Odyssey".to_string())), ("01007EF00011E000".to_string(), Some("The Legend of Zelda: Breath of the Wild".to_string()))]
        );
    }
}
