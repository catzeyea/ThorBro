//! Emulator profiles: small, purely declarative JSON files that say where an emulator keeps its
//! saves on the Thor and how a save folder is tied to a game (title ID).
//!
//! - Built-in profiles live in `profiles/*.json` in the repo and are compiled in (`BUILTIN`).
//!   Only emulators whose layout was checked on a real Thor are shipped.
//! - User profiles in `~/.config/thorBro/profiles/*.json` replace a built-in one with the same id.
//! - Every profile is validated (`validate`): unknown fields are rejected, `format` must be 1,
//!   only the fixed placeholders and resolvers are allowed, and every path must stay below
//!   `/sdcard` or `/storage/<volume>` without `..`. A profile can't run commands.
//!
//! The format is documented in `profiles/README.md`.
//!
//! Sub-modules:
//! - `ids`  – pure parsing: title IDs from file names / 3DS headers, `profiles.dat`, ini files.
//! - `thor` – resolving a profile on the connected Thor (adb): installed apps, `{base}`,
//!   `{user}`, the save folders that exist.
//!
//! Used by `catalog.rs` (Start page + "Add a game save"), `saves/` (sync guards, games.json
//! migration) and `commands.rs`.

pub mod ids;
pub mod thor;

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The only profile format version so far.
pub const FORMAT: u32 = 1;

/// Cover sources a profile may list in `art`.
pub const ART_SOURCES: &[&str] = &["iisu"];

/// Built-in profiles: `(file name, JSON)`. `include_str!` embeds the file at compile time.
const BUILTIN: &[(&str, &str)] = &[
    ("eden.json", include_str!("../../profiles/eden.json")),
    ("eden-duo.json", include_str!("../../profiles/eden-duo.json")),
    ("peanuts.json", include_str!("../../profiles/peanuts.json")),
    ("azahar.json", include_str!("../../profiles/azahar.json")),
];

/// Longest allowed path template / note (characters).
const MAX_TEMPLATE_LEN: usize = 400;
const MAX_NOTE_LEN: usize = 400;
const MAX_NAME_LEN: usize = 40;

/// Placeholders and where they may be used.
const PH_APPDATA: &str = "appdata";
const PH_BASE: &str = "base";
const PH_USER: &str = "user";
const PH_TITLE_ID: &str = "title_id";
const PH_TITLE_ID_LOW: &str = "title_id_low";

/// One emulator profile (see `profiles/README.md`).
///
/// `#[serde(deny_unknown_fields)]` makes parsing fail on any field that isn't listed here, so a
/// typo in a community profile is an error instead of being silently ignored.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub package: String,
    pub console: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub games: Option<GamesDirs>,
    pub saves: Saves,
    #[serde(default)]
    pub art: Vec<String>,
}

/// Games the emulator brings itself: one sub folder per game, named by its title ID.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct GamesDirs {
    pub dirs: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_file: Option<String>,
}

/// Where the saves are.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Saves {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Base>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,
    pub folder: String,
    pub title_id: TitleIdKind,
    #[serde(default = "default_true")]
    pub write: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_note: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Defines `{base}`: a fixed path (optionally overridden by an ini value) or a folder search.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Base {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ini: Option<IniRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<Search>,
}

/// A `key=value` line in an ini file on the Thor.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct IniRef {
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub key: String,
}

/// Search every storage for a folder whose path ends in `marker` (at most `depth` levels deep).
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub marker: String,
    pub depth: u32,
}

/// Defines `{user}`.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct User {
    pub resolver: UserResolver,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<IniRef>,
}

/// The fixed set of user resolvers (code in `ids.rs`).
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub enum UserResolver {
    /// yuzu-style `profiles.dat`: the folder name is the user UUID with its bytes reversed.
    #[serde(rename = "switch-profiles-dat")]
    SwitchProfilesDat,
}

/// How a save folder names its game.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub enum TitleIdKind {
    /// 16 hex digits, uppercase (`01007EF00011E000`).
    #[serde(rename = "switch")]
    Switch,
    /// 3DS: full ID `00040000000EE000`, folders often use the low part `000ee000`.
    #[serde(rename = "3ds")]
    N3ds,
}

impl TitleIdKind {
    /// Normalizes a title ID for this kind (uppercase, 16 digits), or `None` if it doesn't fit.
    pub fn normalize(&self, title_id: &str) -> Option<String> {
        let upper = title_id.trim().to_uppercase();
        let valid = upper.len() == 16 && upper.chars().all(|c| c.is_ascii_hexdigit());
        valid.then_some(upper)
    }
}

impl Profile {
    /// `/sdcard/Android/data/<package>`.
    pub fn appdata(&self) -> String {
        format!("/sdcard/Android/data/{}", self.package)
    }

    /// The fixed default of `{base}` (`base.path` expanded), if the profile has one.
    pub fn default_base(&self) -> Option<String> {
        let path = self.saves.base.as_ref()?.path.as_ref()?;
        expand(path, &self.vars(None, None, None)).ok()
    }

    /// Placeholder values. `base`, `user` and `title_id` are only set when known.
    pub fn vars(&self, base: Option<&str>, user: Option<&str>, title_id: Option<&str>) -> Vars {
        let mut vars = Vars::new();
        vars.insert(PH_APPDATA, self.appdata());
        if let Some(base) = base {
            vars.insert(PH_BASE, base.to_string());
        }
        if let Some(user) = user {
            vars.insert(PH_USER, user.to_string());
        }
        if let Some(title_id) = title_id {
            vars.insert(PH_TITLE_ID, title_id.to_uppercase());
            let low = &title_id[title_id.len().saturating_sub(8)..];
            vars.insert(PH_TITLE_ID_LOW, low.to_lowercase());
        }
        vars
    }

    /// Regex that matches a save folder of this profile and captures the title ID (`tid`, full)
    /// or its low part (`low`). `{base}` is the fixed default (or anything for a search base),
    /// `{user}` any 32 hex digits. Used to tag existing games.json entries.
    pub fn folder_regex(&self) -> Regex {
        let base_pattern = match self.default_base() {
            Some(base) => regex::escape(&base),
            None => ".+".to_string(),
        };
        let mut pattern = String::from("^");
        for piece in split_template(&self.saves.folder) {
            pattern += &match piece {
                Piece::Text(text) => regex::escape(text),
                Piece::Var(PH_APPDATA) => regex::escape(&self.appdata()),
                Piece::Var(PH_BASE) => base_pattern.clone(),
                Piece::Var(PH_USER) => "[0-9A-Fa-f]{32}".to_string(),
                Piece::Var(PH_TITLE_ID) => "(?P<tid>[0-9A-Fa-f]{16})".to_string(),
                Piece::Var(PH_TITLE_ID_LOW) => "(?P<low>[0-9A-Fa-f]{8})".to_string(),
                Piece::Var(_) => "\\x00".to_string(), // can't happen after validation
            };
        }
        pattern.push('$');
        Regex::new(&pattern).expect("escaped pattern is valid")
    }

    /// Title ID of `path` if it is a save folder of this profile (see `folder_regex`).
    pub fn match_folder(&self, path: &str) -> Option<String> {
        let normalized = normalize_thor_path(path);
        let captures = self.folder_regex().captures(&normalized)?;
        if let Some(tid) = captures.name("tid") {
            return self.saves.title_id.normalize(tid.as_str());
        }
        let low = captures.name("low")?.as_str().to_uppercase();
        self.saves.title_id.normalize(&format!("00040000{low}"))
    }

    /// The note shown when writing is turned off.
    pub fn write_block(&self) -> Option<String> {
        if self.saves.write {
            return None;
        }
        Some(self.saves.write_note.clone().unwrap_or_else(|| format!("ThorBro doesn't write saves to {} yet.", self.name)))
    }
}

/// Placeholder name -> value.
pub type Vars = BTreeMap<&'static str, String>;

// --- Templates ---------------------------------------------------------------------------------

/// A piece of a path template: literal text or a `{placeholder}`.
#[derive(Debug, PartialEq)]
enum Piece<'a> {
    Text(&'a str),
    Var(&'a str),
}

/// Splits a template into text and placeholders. Doesn't check anything (see `check_template`).
fn split_template(template: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            pieces.push(Piece::Text(&rest[..open]));
        }
        match rest[open..].find('}') {
            Some(close) => {
                pieces.push(Piece::Var(&rest[open + 1..open + close]));
                rest = &rest[open + close + 1..];
            }
            None => {
                pieces.push(Piece::Text(&rest[open..]));
                rest = "";
            }
        }
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest));
    }
    pieces
}

/// Replaces all placeholders. Fails if one has no value.
pub fn expand(template: &str, vars: &Vars) -> Result<String, String> {
    let mut out = String::new();
    for piece in split_template(template) {
        match piece {
            Piece::Text(text) => out += text,
            Piece::Var(name) => out += vars.get(name).ok_or_else(|| format!("{{{name}}} isn't known here"))?,
        }
    }
    Ok(out)
}

/// The placeholders used in a template.
fn placeholders(template: &str) -> Vec<&str> {
    split_template(template)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Var(name) => Some(name),
            Piece::Text(_) => None,
        })
        .collect()
}

fn has_control_chars(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// Checks one path template of field `field`: only `allowed` placeholders, braces balanced,
/// starts with a placeholder, `/sdcard/` or `/storage/<volume>/`, no `..`, `.` or empty parts.
fn check_template(field: &str, template: &str, allowed: &[&str]) -> Result<(), String> {
    let fail = |why: &str| Err(format!("{field}: {why}"));
    if template.is_empty() || template.len() > MAX_TEMPLATE_LEN || has_control_chars(template) {
        return fail("empty, too long or contains control characters");
    }
    for piece in split_template(template) {
        match piece {
            Piece::Var(name) if !allowed.contains(&name) => return fail(&format!("placeholder {{{name}}} isn't allowed here")),
            Piece::Text(text) if text.contains('{') || text.contains('}') => return fail("unbalanced { }"),
            _ => {}
        }
    }
    let starts_with_root_var = template.starts_with(&format!("{{{PH_APPDATA}}}")) || template.starts_with(&format!("{{{PH_BASE}}}"));
    if !starts_with_root_var {
        // A literal start must be an allowed Thor path; placeholders count as one harmless word.
        let literal = expand_opaque(template);
        if !is_allowed_thor_path(&literal) {
            return fail("must start with {appdata}, {base}, /sdcard/ or /storage/<volume>/");
        }
    }
    let segments: Vec<&str> = template.split('/').collect();
    let inner = if template.starts_with('/') { &segments[1..] } else { &segments[..] };
    if inner.iter().any(|segment| segment.is_empty() || *segment == "." || *segment == "..") {
        return fail("no empty parts, '.' or '..' allowed");
    }
    Ok(())
}

/// The template with every placeholder replaced by `x` (to check its literal parts).
fn expand_opaque(template: &str) -> String {
    split_template(template)
        .into_iter()
        .map(|piece| match piece {
            Piece::Text(text) => text,
            Piece::Var(_) => "x",
        })
        .collect()
}

/// Checks an ini reference (file template plus plain section/key names).
fn check_ini(field: &str, ini: &IniRef, allowed: &[&str]) -> Result<(), String> {
    check_template(&format!("{field}.file"), &ini.file, allowed)?;
    let plain = |text: &str| !text.trim().is_empty() && text.len() <= 100 && !has_control_chars(text);
    if !plain(&ini.key) || ini.key.contains('=') || ini.section.as_deref().is_some_and(|section| !plain(section)) {
        return Err(format!("{field}: key/section must be short plain text"));
    }
    Ok(())
}

// --- Thor paths --------------------------------------------------------------------------------

/// `/storage/emulated/0/...` and `/storage/self/primary/...` are the internal storage; ThorBro
/// writes them as `/sdcard/...`. Trailing slashes are removed.
pub fn normalize_thor_path(path: &str) -> String {
    let mut path = path.trim().to_string();
    for alias in ["/storage/emulated/0", "/storage/self/primary"] {
        if path == alias || path.starts_with(&format!("{alias}/")) {
            path = format!("/sdcard{}", &path[alias.len()..]);
        }
    }
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    path
}

/// True if a profile may point at `path`: below `/sdcard` or `/storage/<volume>`, no `..`,
/// `.` or empty parts and no control characters. Checked for templates and for every value read
/// from the Thor (ini values, search results).
pub fn is_allowed_thor_path(path: &str) -> bool {
    if has_control_chars(path) || path.len() > 1000 {
        return false;
    }
    let path = normalize_thor_path(path);
    let segments: Vec<&str> = path.split('/').collect();
    // segments[0] is the empty string before the leading '/'.
    if segments.first() != Some(&"") || segments[1..].iter().any(|segment| segment.is_empty() || *segment == "." || *segment == "..") {
        return false;
    }
    match segments.get(1) {
        Some(&"sdcard") => true,
        Some(&"storage") => segments.get(2).is_some_and(|volume| crate::adb::is_volume_id(volume)),
        _ => false,
    }
}

// --- Validation --------------------------------------------------------------------------------

fn id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,39}$").unwrap())
}

fn package_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$").unwrap())
}

/// Checks everything serde can't: values, placeholders, paths. See the module comment.
pub fn validate(profile: &Profile) -> Result<(), String> {
    if profile.format != FORMAT {
        return Err(format!("format must be {FORMAT}"));
    }
    if !id_re().is_match(&profile.id) {
        return Err("id: lowercase letters, digits and '-' only (max 40)".into());
    }
    let name = profile.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN || has_control_chars(name) {
        return Err(format!("name: 1 to {MAX_NAME_LEN} characters"));
    }
    if profile.package.len() > 200 || !package_re().is_match(&profile.package) {
        return Err("package: not an Android package name".into());
    }
    if crate::systems::by_id(&profile.console).is_none() {
        return Err(format!("console: unknown system id “{}”", profile.console));
    }
    if let Some(unknown) = profile.art.iter().find(|source| !ART_SOURCES.contains(&source.as_str())) {
        return Err(format!("art: unknown source “{unknown}” (known: {})", ART_SOURCES.join(", ")));
    }
    if let Some(games) = &profile.games {
        check_template("games.dirs", &games.dirs, &[PH_APPDATA])?;
        if let Some(name_file) = &games.name_file {
            let plain = !name_file.is_empty() && name_file.len() <= 64 && !name_file.contains(['/', '\\', '{', '}']) && name_file != ".." && !has_control_chars(name_file);
            if !plain {
                return Err("games.name_file: a plain file name".into());
            }
        }
    }
    validate_saves(&profile.saves)
}

fn validate_saves(saves: &Saves) -> Result<(), String> {
    if let Some(base) = &saves.base {
        match (&base.path, &base.search) {
            (Some(path), None) => {
                check_template("saves.base.path", path, &[PH_APPDATA])?;
                if let Some(ini) = &base.ini {
                    check_ini("saves.base.ini", ini, &[PH_APPDATA])?;
                }
            }
            (None, Some(search)) => {
                if base.ini.is_some() {
                    return Err("saves.base: ini only works together with path".into());
                }
                let marker_ok = !search.marker.is_empty()
                    && !search.marker.starts_with('/')
                    && !search.marker.contains(['{', '}', '\\', '*', '?', '[', ']'])
                    && !has_control_chars(&search.marker)
                    && search.marker.split('/').all(|segment| !segment.is_empty() && segment != "." && segment != "..");
                if !marker_ok {
                    return Err("saves.base.search.marker: a relative folder path without wildcards or '..'".into());
                }
                if !(1..=8).contains(&search.depth) {
                    return Err("saves.base.search.depth: 1 to 8".into());
                }
            }
            _ => return Err("saves.base: needs exactly one of path or search".into()),
        }
    }
    if let Some(user) = &saves.user {
        check_template("saves.user.file", &user.file, &[PH_APPDATA, PH_BASE])?;
        if let Some(index) = &user.index {
            check_ini("saves.user.index", index, &[PH_APPDATA, PH_BASE])?;
        }
        let uses_base = placeholders(&user.file).contains(&PH_BASE) || user.index.as_ref().is_some_and(|ini| placeholders(&ini.file).contains(&PH_BASE));
        if uses_base && saves.base.is_none() {
            return Err("saves.user uses {base} but saves.base is missing".into());
        }
    }
    check_template("saves.folder", &saves.folder, &[PH_APPDATA, PH_BASE, PH_USER, PH_TITLE_ID, PH_TITLE_ID_LOW])?;
    let used = placeholders(&saves.folder);
    if used.contains(&PH_BASE) && saves.base.is_none() {
        return Err("saves.folder uses {base} but saves.base is missing".into());
    }
    if used.contains(&PH_USER) && saves.user.is_none() {
        return Err("saves.folder uses {user} but saves.user is missing".into());
    }
    let title_vars: Vec<&&str> = used.iter().filter(|name| **name == PH_TITLE_ID || **name == PH_TITLE_ID_LOW).collect();
    if title_vars.len() != 1 {
        return Err("saves.folder: needs the title ID placeholder exactly once".into());
    }
    if saves.title_id == TitleIdKind::Switch && *title_vars[0] == PH_TITLE_ID_LOW {
        return Err("saves.folder: {title_id_low} is only for 3ds".into());
    }
    let title_segment = format!("{{{}}}", title_vars[0]);
    if !saves.folder.split('/').any(|segment| segment == title_segment) {
        return Err("saves.folder: the title ID placeholder must be a whole path part".into());
    }
    if saves.write_note.as_ref().is_some_and(|note| note.len() > MAX_NOTE_LEN || note.trim().is_empty()) {
        return Err(format!("saves.write_note: 1 to {MAX_NOTE_LEN} characters"));
    }
    Ok(())
}

/// Parses and validates one profile.
pub fn parse(json: &str) -> Result<Profile, String> {
    let profile: Profile = serde_json::from_str(json).map_err(|e| e.to_string())?;
    validate(&profile)?;
    Ok(profile)
}

// --- Loading -----------------------------------------------------------------------------------

/// A profile that couldn't be used, with the reason (shown in "+ Add emulator").
#[derive(Serialize, Clone, Debug)]
pub struct ProfileError {
    pub file: String,
    pub error: String,
}

/// A usable profile plus where it came from (`"builtin"` or `"user"`).
#[derive(Clone, Debug)]
pub struct Loaded {
    pub profile: Profile,
    pub source: &'static str,
}

/// All usable profiles (built-in order, user profiles replace by id or come after) and the
/// reasons for the skipped ones.
#[derive(Default)]
pub struct Profiles {
    pub list: Vec<Loaded>,
    pub errors: Vec<ProfileError>,
}

impl Profiles {
    pub fn by_id(&self, id: &str) -> Option<&Profile> {
        self.list.iter().map(|loaded| &loaded.profile).find(|profile| profile.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Profile> {
        self.list.iter().map(|loaded| &loaded.profile)
    }

    /// `(profile id, title ID)` if exactly one profile claims `path` as a save folder.
    pub fn match_save_path(&self, path: &str) -> Option<(String, String)> {
        let mut hits = self.iter().filter_map(|profile| profile.match_folder(path).map(|tid| (profile.id.clone(), tid)));
        let first = hits.next()?;
        hits.next().is_none().then_some(first)
    }
}

/// Folder for the user's own profiles (`~/.config/thorBro/profiles`).
pub fn user_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("thorBro").join("profiles"))
}

/// Built-in profiles plus the user's.
pub fn load() -> Profiles {
    load_from(user_dir().as_deref())
}

/// Built-in profiles plus the valid `*.json` files in `dir` (sorted by file name).
pub fn load_from(dir: Option<&Path>) -> Profiles {
    let mut profiles = Profiles::default();
    for (file, json) in BUILTIN {
        match parse(json) {
            Ok(profile) => profiles.list.push(Loaded { profile, source: "builtin" }),
            Err(error) => profiles.errors.push(ProfileError { file: file.to_string(), error }),
        }
    }
    let mut files: Vec<PathBuf> = dir
        .and_then(|dir| fs::read_dir(dir).ok())
        .map(|read_dir| read_dir.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    files.retain(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json")));
    files.sort();
    let mut user_ids: Vec<String> = Vec::new();
    for path in files {
        let file = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let parsed = fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|json| parse(&json));
        let profile = match parsed {
            Ok(profile) => profile,
            Err(error) => {
                profiles.errors.push(ProfileError { file, error });
                continue;
            }
        };
        if user_ids.contains(&profile.id) {
            profiles.errors.push(ProfileError { file, error: format!("id “{}” is used by another of your profiles", profile.id) });
            continue;
        }
        user_ids.push(profile.id.clone());
        let loaded = Loaded { profile, source: "user" };
        match profiles.list.iter_mut().find(|existing| existing.profile.id == loaded.profile.id) {
            Some(existing) => *existing = loaded,
            None => profiles.list.push(loaded),
        }
    }
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin(id: &str) -> Profile {
        load_from(None).by_id(id).cloned().unwrap()
    }

    #[test]
    fn builtin_profiles_are_valid() {
        let profiles = load_from(None);
        assert!(profiles.errors.is_empty(), "{:?}", profiles.errors);
        let ids: Vec<&str> = profiles.iter().map(|profile| profile.id.as_str()).collect();
        assert_eq!(ids, ["eden", "eden-duo", "peanuts", "azahar"]);
        assert!(profiles.list.iter().all(|loaded| loaded.source == "builtin"));
    }

    /// The Eden Duo profile as JSON value, to break it in the tests below.
    fn eden_duo_json() -> serde_json::Value {
        serde_json::from_str(BUILTIN[1].1).unwrap()
    }

    fn parse_value(value: &serde_json::Value) -> Result<Profile, String> {
        parse(&value.to_string())
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let mut value = eden_duo_json();
        value["savez"] = serde_json::json!(1);
        assert!(parse_value(&value).unwrap_err().contains("unknown field"));
        let mut value = eden_duo_json();
        value["saves"]["comand"] = serde_json::json!("rm -rf /");
        assert!(parse_value(&value).unwrap_err().contains("unknown field"));
    }

    #[test]
    fn format_version_must_be_1() {
        let mut value = eden_duo_json();
        value["format"] = serde_json::json!(2);
        assert_eq!(parse_value(&value).unwrap_err(), "format must be 1");
    }

    #[test]
    fn placeholders_and_resolvers_are_fixed() {
        let mut value = eden_duo_json();
        value["saves"]["folder"] = serde_json::json!("{base}/user/save/{home}/{title_id}");
        assert!(parse_value(&value).unwrap_err().contains("{home} isn't allowed"));
        let mut value = eden_duo_json();
        value["saves"]["user"]["resolver"] = serde_json::json!("shell");
        assert!(parse_value(&value).unwrap_err().contains("unknown variant"));
        let mut value = eden_duo_json();
        value["saves"]["folder"] = serde_json::json!("{base}/user/save/x{title_id}");
        assert!(parse_value(&value).unwrap_err().contains("whole path part"));
        let mut value = eden_duo_json();
        value["saves"]["folder"] = serde_json::json!("{base}/user/save/{user}");
        assert!(parse_value(&value).unwrap_err().contains("exactly once"));
        let mut value = eden_duo_json();
        value["art"] = serde_json::json!(["cocoon"]);
        assert!(parse_value(&value).unwrap_err().contains("unknown source"));
        let mut value = eden_duo_json();
        value["console"] = serde_json::json!("toaster");
        assert!(parse_value(&value).unwrap_err().contains("unknown system"));
    }

    #[test]
    fn paths_must_stay_on_sdcard_or_a_volume() {
        for bad in [
            "/data/data/x/{title_id}",
            "/sdcard/../data/{title_id}",
            "{appdata}/../../{title_id}",
            "/storage/emulated/10/{title_id}",
            "/storage/self/../{title_id}",
            "relative/{title_id}",
            "{title_id}/x",
            "/sdcard//x/{title_id}",
        ] {
            let mut value = serde_json::from_str::<serde_json::Value>(BUILTIN[2].1).unwrap();
            value["saves"]["folder"] = serde_json::json!(bad);
            assert!(parse_value(&value).is_err(), "{bad} was accepted");
        }
        for good in ["/sdcard/Saves/{title_id}", "/storage/EEFF-BF6C/saves/{title_id}", "/storage/emulated/0/x/{title_id}"] {
            let mut value = serde_json::from_str::<serde_json::Value>(BUILTIN[2].1).unwrap();
            value["saves"]["folder"] = serde_json::json!(good);
            assert!(parse_value(&value).is_ok(), "{good} was rejected");
        }
        let mut value = serde_json::from_str::<serde_json::Value>(BUILTIN[3].1).unwrap();
        value["saves"]["base"]["search"]["marker"] = serde_json::json!("../sdmc");
        assert!(parse_value(&value).is_err());
    }

    #[test]
    fn thor_path_checks() {
        assert!(is_allowed_thor_path("/sdcard/Android/data/x"));
        assert!(is_allowed_thor_path("/storage/emulated/0/Android/data/x"));
        assert!(is_allowed_thor_path("/storage/EEFF-BF6C/ROMs/n3ds"));
        assert!(!is_allowed_thor_path("/storage/emulated/0/../x"));
        assert!(!is_allowed_thor_path("/data/local/tmp"));
        assert!(!is_allowed_thor_path("/sdcard/a\nrm -rf /"));
        assert!(!is_allowed_thor_path("sdcard/x"));
        assert_eq!(normalize_thor_path("/storage/emulated/0/Android/"), "/sdcard/Android");
    }

    #[test]
    fn user_profiles_override_builtins_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = eden_duo_json();
        value["name"] = serde_json::json!("My Eden Duo");
        fs::write(dir.path().join("a.json"), value.to_string()).unwrap();
        let mut other = eden_duo_json();
        other["id"] = serde_json::json!("my-emu");
        fs::write(dir.path().join("b.json"), other.to_string()).unwrap();
        fs::write(dir.path().join("c.json"), "{ not json").unwrap();
        fs::write(dir.path().join("d.json"), other.to_string()).unwrap(); // duplicate id
        fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let profiles = load_from(Some(dir.path()));
        let ids: Vec<&str> = profiles.iter().map(|profile| profile.id.as_str()).collect();
        assert_eq!(ids, ["eden", "eden-duo", "peanuts", "azahar", "my-emu"]);
        assert_eq!(profiles.by_id("eden-duo").unwrap().name, "My Eden Duo");
        assert_eq!(profiles.list[1].source, "user");
        let failed: Vec<&str> = profiles.errors.iter().map(|error| error.file.as_str()).collect();
        assert_eq!(failed, ["c.json", "d.json"]);
    }

    /// The five entries of the real games.json (step 1) and what they must be tagged as.
    #[test]
    fn real_save_paths_match_exactly_one_profile() {
        let profiles = load_from(None);
        let eden_duo = "/sdcard/Android/data/dev.igawa6.edenduo/files/nand/user/save/0000000000000000/5D1B4CDA089A581DAA9E48BC2B43C120";
        for tid in ["01007EF00011E000", "0100646009FBE000", "01006A800016E000", "0100F2C0115B6000"] {
            assert_eq!(profiles.match_save_path(&format!("{eden_duo}/{tid}")), Some(("eden-duo".into(), tid.into())));
        }
        let azahar = "/storage/EEFF-BF6C/ROMs/n3ds/sdmc/Nintendo 3DS/00000000000000000000000000000000/00000000000000000000000000000000/title/00040000/000ee000/data/00000001";
        assert_eq!(profiles.match_save_path(azahar), Some(("azahar".into(), "00040000000EE000".into())));
        let eden = "/storage/emulated/0/Android/data/dev.eden.eden_emulator/files/nand/user/save/0000000000000000/5D1B4CDA089A581DAA9E48BC2B43C120/0100125019A14000";
        assert_eq!(profiles.match_save_path(eden), Some(("eden".into(), "0100125019A14000".into())));
        let peanuts = "/sdcard/Android/data/dev.nxnative.android.peanuts/files/saves/01007EF00011E000";
        assert_eq!(profiles.match_save_path(peanuts), Some(("peanuts".into(), "01007EF00011E000".into())));
        // .work / .lock siblings and PC paths are not save folders.
        assert_eq!(profiles.match_save_path(&format!("{peanuts}.work")), None);
        assert_eq!(profiles.match_save_path("/home/me/.local/share/eden/nand/user/save/0000000000000000/57F9DD99C7E62A7AE71954A31BC3B628/01007EF00011E000"), None);
    }

    #[test]
    fn expanding_a_save_folder() {
        let azahar = builtin("azahar");
        let vars = azahar.vars(Some("/storage/EEFF-BF6C/ROMs/n3ds"), None, Some("00040000000EE000"));
        assert_eq!(
            expand(&azahar.saves.folder, &vars).unwrap(),
            "/storage/EEFF-BF6C/ROMs/n3ds/sdmc/Nintendo 3DS/00000000000000000000000000000000/00000000000000000000000000000000/title/00040000/000ee000/data/00000001"
        );
        let peanuts = builtin("peanuts");
        assert!(expand(&peanuts.saves.folder, &peanuts.vars(None, None, None)).is_err(), "title ID missing");
        assert!(peanuts.write_block().unwrap().contains("can't write"));
        assert_eq!(builtin("eden").write_block(), None);
        assert_eq!(builtin("eden-duo").default_base().unwrap(), "/sdcard/Android/data/dev.igawa6.edenduo/files/nand");
    }
}
