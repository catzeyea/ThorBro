//! "Import ES-DE info": writes ES-DE style `systeminfo.txt` files into the system folders of an
//! existing games folder that was not created by ES-DE (e.g. `/sdcard/lisu/Roms/Wii`).
//!
//! Flow: `preview` scans the direct sub folders and returns a plan (nothing is written), the UI
//! shows it, and `apply` carries out what the user confirmed. `apply` scans again instead of
//! trusting the old preview, so a file that appeared in the meantime is still protected.
//!
//! Safety rules: ROM files are never touched; unknown folders are left alone; an existing
//! `systeminfo.txt` is only replaced with `overwrite`; every file is written to a temp name and
//! then moved into place; a failure in one folder doesn't stop the others.
//!
//! Note: ES-DE itself never reads `systeminfo.txt`, it finds systems by folder name only. The
//! file mainly helps ThorBro (see `library.rs`) and humans. That's why the preview also offers
//! renaming folders whose name only differs by case from the ES-DE name (`Wii` -> `wii`).
//!
//! Used by `commands.rs` (`esde_preview`, `esde_import`).

use crate::adb::{sh, shell_quote};
use crate::systems::{self, KnownSystem};
use crate::{files, settings};
use serde::{Deserialize, Serialize};

/// File name ES-DE uses in every system folder.
pub const INFO_FILE: &str = "systeminfo.txt";

/// Suffix of the temp file that is moved over `systeminfo.txt` once it is completely written.
const TEMP_SUFFIX: &str = ".thorbro-tmp";

/// Content of a `systeminfo.txt` for `system`. The whole format lives in this one function.
///
/// Layout as written by ES-DE (`SystemData::createSystemDirectories()` in ES-DE's
/// `es-app/src/SystemData.cpp`): a header line, the value on the next line, a blank line between
/// sections, and a single newline after the last value. ES-DE copies the extension list from
/// its `es_systems.xml` (lower- and uppercase variants, separated by spaces), we do the same with
/// the list from `systems.rs`.
///
/// ThorBro doesn't know ES-DE's launch command, scraping platform or theme of a system, so those
/// get `None defined` (the text ES-DE itself writes for a missing platform) and the theme folder
/// is the system name, which is what ES-DE's default systems use.
pub fn systeminfo_content(system: &KnownSystem) -> String {
    let extensions: Vec<String> = system
        .exts
        .split_whitespace()
        .flat_map(|ext| [ext.to_string(), ext.to_uppercase()])
        .collect();
    let sections = [
        ("System name:", system.id.to_string()),
        ("Full system name:", system.full.to_string()),
        ("Supported file extensions:", extensions.join(" ")),
        ("Launch command:", "None defined".to_string()),
        ("Platform (for scraping):", "None defined".to_string()),
        ("Theme folder:", system.id.to_string()),
    ];
    let blocks: Vec<String> = sections
        .iter()
        .map(|(header, value)| format!("{header}\n{value}\n"))
        .collect();
    blocks.join("\n")
}

// --- Plan ------------------------------------------------------------------------------------

/// What `apply` would do with `systeminfo.txt` in one folder.
#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Recognized system without `systeminfo.txt`: create it.
    Create,
    /// Already has a `systeminfo.txt`: kept, unless the user picks "Override existing".
    Exists,
    /// System unknown: the folder is left alone.
    Unknown,
}

/// One line of the preview.
#[derive(Serialize, Clone, Debug)]
pub struct PlanItem {
    pub folder: String,
    /// Recognized system id, e.g. "wii".
    pub system: Option<String>,
    /// Display name of the system (empty if unknown).
    pub full: String,
    /// "name" (folder name matched), "manual" (set by the user) or "" (unknown).
    pub origin: &'static str,
    pub action: Action,
    /// The folder name ES-DE expects, if it differs from `folder`.
    pub esde_name: Option<String>,
    /// `esde_name` differs only by upper/lower case: renaming is offered.
    pub case_only: bool,
    /// Another folder already has the ES-DE name, so renaming is not possible.
    pub rename_conflict: bool,
}

/// A direct sub folder of the games folder, as found on the Thor.
#[derive(Debug, PartialEq)]
pub struct Subfolder {
    pub name: String,
    pub has_info: bool,
}

/// Shell script that prints `<1|0>/<folder name>` for every direct sub folder (1 = has
/// `systeminfo.txt`). `/` can't occur in a folder name, so it is a safe separator.
fn scan_script(library_path: &str) -> String {
    format!(
        "cd {} || exit 1; for d in */; do [ -d \"$d\" ] || continue; n=\"${{d%/}}\"; \
         if [ -e \"$n/{INFO_FILE}\" ]; then echo \"1/$n\"; else echo \"0/$n\"; fi; done",
        shell_quote(library_path)
    )
}

/// Parses the output of `scan_script`.
fn parse_scan(output: &str) -> Vec<Subfolder> {
    output
        .lines()
        .filter_map(|line| line.split_once('/'))
        .filter(|(_, name)| !name.is_empty())
        .map(|(flag, name)| Subfolder {
            name: name.to_string(),
            has_info: flag == "1",
        })
        .collect()
}

/// True if the names are different but equal when upper/lower case is ignored.
fn differs_only_by_case(a: &str, b: &str) -> bool {
    a != b && a.to_lowercase() == b.to_lowercase()
}

/// Builds the plan for all sub folders. A system set by hand wins over the folder name.
pub fn build_plan(
    library_path: &str,
    folders: &[Subfolder],
    config: &settings::Settings,
) -> Vec<PlanItem> {
    folders
        .iter()
        .map(|folder| {
            let manual = config
                .folder_systems
                .get(&format!("{library_path}/{}", folder.name))
                .and_then(|id| systems::by_id(id));
            let (known, origin) = match manual {
                Some(known) => (Some(known), "manual"),
                None => (systems::by_folder_name(&folder.name), "name"),
            };
            let action = match known {
                None => Action::Unknown,
                Some(_) if folder.has_info => Action::Exists,
                Some(_) => Action::Create,
            };
            let esde_name = known
                .map(|k| k.id.to_string())
                .filter(|id| *id != folder.name);
            let case_only = esde_name
                .as_deref()
                .is_some_and(|id| differs_only_by_case(&folder.name, id));
            let rename_conflict = case_only
                && esde_name
                    .as_deref()
                    .is_some_and(|id| folders.iter().any(|other| other.name == id));
            PlanItem {
                folder: folder.name.clone(),
                system: known.map(|k| k.id.to_string()),
                full: known.map(|k| k.full.to_string()).unwrap_or_default(),
                origin: if known.is_some() { origin } else { "" },
                action,
                esde_name,
                case_only,
                rename_conflict,
            }
        })
        .collect()
}

/// Scans the games folder on the Thor and returns the plan. Writes nothing.
pub fn preview(library_path: &str) -> Result<Vec<PlanItem>, String> {
    let library_path = settings::clean_path(library_path)?;
    let output = sh(&scan_script(&library_path));
    if !output.ok {
        return Err(format!(
            "Couldn't open {library_path}: {}",
            output.err_text()
        ));
    }
    Ok(build_plan(
        &library_path,
        &parse_scan(&output.stdout),
        &settings::load(),
    ))
}

// --- Writing ---------------------------------------------------------------------------------

/// Turns text into a `printf` format string that prints it literally (`\` and `%` escaped,
/// newlines as `\n`). Keeps the shell command on one line, which avoids newline handling
/// differences of `adb shell` on Windows.
fn printf_format(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "%%")
        .replace('\n', "\\n")
}

/// Shell command that writes `content` atomically to `<dir>/systeminfo.txt`: first into a temp
/// file, then `mv` over the target. Prints `OK`, `EXISTS` (target exists and `overwrite` is off;
/// checked on the Thor right before writing) or `FAILED` (temp file removed again).
fn write_command(dir: &str, content: &str, overwrite: bool) -> String {
    let file = format!("{dir}/{INFO_FILE}");
    let temp = format!("{file}{TEMP_SUFFIX}");
    let (file, temp) = (shell_quote(&file), shell_quote(&temp));
    let guard = if overwrite {
        String::new()
    } else {
        format!("if [ -e {file} ]; then echo EXISTS; exit 0; fi; ")
    };
    format!(
        "{guard}if printf {} > {temp} && mv -f {temp} {file}; then echo OK; else rm -f {temp}; echo FAILED; fi",
        shell_quote(&printf_format(content))
    )
}

/// What the user confirmed in the preview.
#[derive(Deserialize)]
pub struct ImportRequest {
    pub path: String,
    /// Folders to write `systeminfo.txt` into.
    #[serde(default)]
    pub write: Vec<String>,
    /// Replace existing `systeminfo.txt` files ("Override existing").
    #[serde(default)]
    pub overwrite: bool,
    /// Folders to rename to their ES-DE name (case-only renames).
    #[serde(default)]
    pub rename: Vec<String>,
}

/// Outcome for one folder, shown in the result dialog.
#[derive(Serialize, Debug)]
pub struct ImportResult {
    pub folder: String,
    pub ok: bool,
    pub text: String,
}

/// Carries out the confirmed import. Each folder is handled on its own: an error is recorded in
/// its result and the next folder is processed.
pub fn apply(request: &ImportRequest) -> Result<Vec<ImportResult>, String> {
    let library_path = settings::clean_path(&request.path)?;
    let plan = preview(&library_path)?;
    let mut config = settings::load();
    let mut config_changed = false;
    let mut results = Vec::new();

    for item in &plan {
        let wants_rename = request.rename.contains(&item.folder);
        let wants_write = request.write.contains(&item.folder);
        if !wants_rename && !wants_write {
            continue;
        }
        let mut folder = item.folder.clone();
        let mut notes: Vec<String> = Vec::new();
        let mut ok = true;

        if wants_rename {
            match rename_to_esde(&library_path, item) {
                Ok(new_name) => {
                    // Keep a system set by hand attached to the folder under its new name.
                    let old_key = format!("{library_path}/{folder}");
                    if let Some(id) = config.folder_systems.remove(&old_key) {
                        config
                            .folder_systems
                            .insert(format!("{library_path}/{new_name}"), id);
                        config_changed = true;
                    }
                    notes.push(format!("renamed to {new_name}"));
                    folder = new_name;
                }
                Err(e) => {
                    ok = false;
                    notes.push(format!("not renamed: {e}"));
                }
            }
        }

        if wants_write {
            match write_info(&library_path, &folder, item, request.overwrite) {
                Ok(text) => notes.push(text),
                Err(e) => {
                    ok = false;
                    notes.push(e);
                }
            }
        }
        results.push(ImportResult {
            folder: item.folder.clone(),
            ok,
            text: notes.join(", "),
        });
    }

    if config_changed {
        if let Err(e) = settings::save(&config) {
            results.push(ImportResult {
                folder: String::new(),
                ok: false,
                text: format!("Settings not saved: {e}"),
            });
        }
    }
    Ok(results)
}

/// Case-only rename of a folder to its ES-DE name, via the temp-name rename in `files.rs`.
fn rename_to_esde(library_path: &str, item: &PlanItem) -> Result<String, String> {
    let Some(new_name) = item.esde_name.as_ref().filter(|_| item.case_only) else {
        return Err("only renames that change upper/lower case are offered".into());
    };
    if item.rename_conflict {
        return Err(format!("a folder named {new_name} already exists"));
    }
    files::rename(
        &format!("{library_path}/{}", item.folder),
        &format!("{library_path}/{new_name}"),
    )?;
    Ok(new_name.clone())
}

/// Writes `systeminfo.txt` into `<library>/<folder>` according to the plan item.
fn write_info(
    library_path: &str,
    folder: &str,
    item: &PlanItem,
    overwrite: bool,
) -> Result<String, String> {
    let known = match item.system.as_deref().and_then(systems::by_id) {
        Some(known) if item.action != Action::Unknown => known,
        _ => return Ok("unknown system, left alone".into()),
    };
    if item.action == Action::Exists && !overwrite {
        return Ok("has a systeminfo.txt already, kept".into());
    }
    let dir = format!("{library_path}/{folder}");
    let output = sh(&write_command(&dir, &systeminfo_content(known), overwrite));
    match output.stdout.lines().map(str::trim).next_back() {
        Some("OK") if item.action == Action::Exists => Ok("systeminfo.txt replaced".into()),
        Some("OK") => Ok("systeminfo.txt created".into()),
        Some("EXISTS") => Ok("a systeminfo.txt appeared in the meantime, kept".into()),
        _ => Err(format!("systeminfo.txt not written: {}", output.err_text())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(name: &str, has_info: bool) -> Subfolder {
        Subfolder {
            name: name.into(),
            has_info,
        }
    }

    #[test]
    fn content_has_the_es_de_layout() {
        let wii = systems::by_id("wii").unwrap();
        let content = systeminfo_content(wii);
        assert!(content.starts_with("System name:\nwii\n\nFull system name:\nNintendo Wii\n\n"));
        assert!(content.contains("\n\nSupported file extensions:\n.iso .ISO .wbfs .WBFS "));
        assert!(content.ends_with("\n\nTheme folder:\nwii\n"));
        assert!(!content.ends_with("\n\n"));
        // Every section except the last is followed by exactly one blank line.
        assert_eq!(content.matches("\n\n").count(), 5);
    }

    #[test]
    fn plan_creates_skips_and_flags_names() {
        let folders = [
            sub("Wii", false),
            sub("ps2", true),
            sub("My Stuff", false),
            sub("Game Boy Advance", false),
            sub("PS1", false),
        ];
        let plan = build_plan(
            "/sdcard/lisu/Roms",
            &folders,
            &settings::Settings::default(),
        );
        let get = |name: &str| plan.iter().find(|item| item.folder == name).unwrap();

        let wii = get("Wii");
        assert_eq!(wii.action, Action::Create);
        assert_eq!(
            (wii.esde_name.as_deref(), wii.case_only, wii.rename_conflict),
            (Some("wii"), true, false)
        );
        assert_eq!(get("ps2").action, Action::Exists);
        assert_eq!(get("ps2").esde_name, None);
        assert_eq!(get("My Stuff").action, Action::Unknown);
        assert_eq!(get("My Stuff").esde_name, None);
        // Name differs by more than case: reported, but no rename offered.
        let gba = get("Game Boy Advance");
        assert_eq!(
            (gba.action, gba.esde_name.as_deref(), gba.case_only),
            (Action::Create, Some("gba"), false)
        );
        assert_eq!(get("PS1").esde_name.as_deref(), Some("psx"));
    }

    #[test]
    fn manual_system_wins_and_rename_conflicts_are_flagged() {
        let mut config = settings::Settings::default();
        config
            .folder_systems
            .insert("/r/My Stuff".into(), "gba".into());
        let plan = build_plan(
            "/r",
            &[sub("My Stuff", false), sub("Wii", false), sub("wii", false)],
            &config,
        );
        assert_eq!(
            (plan[0].action, plan[0].origin, plan[0].system.as_deref()),
            (Action::Create, "manual", Some("gba"))
        );
        assert!(plan[1].case_only && plan[1].rename_conflict);
        assert_eq!(plan[2].esde_name, None);
    }

    #[test]
    fn case_only_names_are_detected() {
        assert!(differs_only_by_case("Wii", "wii"));
        assert!(differs_only_by_case("PS2", "ps2"));
        assert!(!differs_only_by_case("wii", "wii"));
        assert!(!differs_only_by_case("PS1", "psx"));
    }

    #[test]
    fn scan_output_is_parsed() {
        let parsed = parse_scan("1/snes\n0/My Stuff\n0/\ngarbage\n");
        assert_eq!(parsed, vec![sub("snes", true), sub("My Stuff", false)]);
    }

    #[test]
    fn names_with_spaces_and_quotes_are_quoted() {
        let command = write_command("/sdcard/lisu/Roms/Game Boy Advance", "a\n50% b\\c\n", false);
        assert!(command.starts_with(
            "if [ -e '/sdcard/lisu/Roms/Game Boy Advance/systeminfo.txt' ]; then echo EXISTS;"
        ));
        assert!(command.contains("printf 'a\\n50%% b\\\\c\\n' > '/sdcard/lisu/Roms/Game Boy Advance/systeminfo.txt.thorbro-tmp'"));
        assert!(command.contains("&& mv -f '/sdcard/lisu/Roms/Game Boy Advance/systeminfo.txt.thorbro-tmp' '/sdcard/lisu/Roms/Game Boy Advance/systeminfo.txt'"));
        assert!(!command.contains('\n'), "command must stay on one line");

        let quoted = write_command("/r/It's Mine", "x", true);
        assert!(
            quoted.starts_with("if printf"),
            "no existence guard when overwriting"
        );
        assert!(quoted.contains("'/r/It'\\''s Mine/systeminfo.txt'"));

        let script = scan_script("/sdcard/my roms");
        assert!(script.starts_with("cd '/sdcard/my roms' || exit 1;"));
    }
}
