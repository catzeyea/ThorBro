//! The game catalog for the Start page and the "Add a game save" dialog, built from the real
//! Thor: games in the libraries (ROM files, title IDs from file names / 3DS headers), games an
//! emulator brings itself (Peanuts packs), the installed emulators with a profile, their save
//! folders, and covers from iiSU.
//!
//! Nothing here is hardcoded for one setup: consoles come from the libraries and the profiles,
//! emulators from `pm list packages` + `emulators::load()`.

use crate::adb::{sh, sh_script, shell_quote};
use crate::art::{self, ArtRef};
use crate::emulators::{self, ids, thor, Profile, ProfileError, TitleIdKind};
use crate::library::{Library, System};
use crate::util;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Folders inside a system folder that are never a game (ES-DE, Azahar's user folder in
/// `ROMs/n3ds`, Switch updates).
const NOT_GAME_DIRS: &[&str] = &[
    "update", "updates", "dlc", "bios", "log", "logs", "config", "nand", "sdmc", "sysdata", "cheats", "shaders",
    "gpu_drivers", "media", "images", "videos", "manuals", "downloaded_media", "save", "saves", "states", "screenshots",
];

/// Archive extensions: a game file, but nothing to read a header from.
const ARCHIVE_EXTS: &[&str] = &[".zip", ".7z", ".rar"];

/// 3DS files with an NCSD header (title ID at 0x108).
const N3DS_HEADER_EXTS: &[&str] = &[".3ds", ".cci"];

/// One save folder an emulator has on the Thor.
#[derive(Serialize, Clone, Debug)]
pub struct SaveEntry {
    pub title_id: String,
    pub path: String,
    pub files: u64,
    pub newest: f64,
    pub when: String,
}

/// An emulator with a profile.
#[derive(Serialize, Clone, Debug)]
pub struct EmulatorInfo {
    pub id: String,
    pub name: String,
    pub package: String,
    pub console: String,
    pub installed: bool,
    /// `false` = ThorBro reads and backs up, but never writes (see `write_note`).
    pub write: bool,
    pub write_note: Option<String>,
    pub source: &'static str,
    /// Number of games the emulator brings itself (`None` = it plays the library's ROMs).
    pub games: Option<usize>,
    pub saves: Vec<SaveEntry>,
    pub error: Option<String>,
}

/// One game tile on the Start page.
#[derive(Serialize, Clone, Debug)]
pub struct GameEntry {
    pub title: String,
    pub title_id: Option<String>,
    /// The ROM on the Thor (`None` for a game an emulator brings itself).
    pub rom: Option<String>,
    /// The library the ROM is in (for "open in Files").
    pub lib: Option<String>,
    pub art: Option<ArtRef>,
    /// Ids of the emulators that have a save for this game.
    pub saves: Vec<String>,
}

/// One console block on the Start page.
#[derive(Serialize, Clone, Debug)]
pub struct ConsoleBlock {
    pub id: String,
    /// The system folder name (e.g. `switch`), if the console has one in a library.
    pub folder: Option<String>,
    pub full: String,
    pub games: Vec<GameEntry>,
    /// Ids of the installed emulators for this console.
    pub emulators: Vec<String>,
}

/// A game the "Add a game save" search can find (everything with a title ID).
#[derive(Serialize, Clone, Debug)]
pub struct SearchGame {
    pub console: String,
    pub console_name: String,
    pub title: String,
    pub title_id: String,
}

#[derive(Serialize, Debug)]
pub struct Catalog {
    pub consoles: Vec<ConsoleBlock>,
    pub emulators: Vec<EmulatorInfo>,
    pub search: Vec<SearchGame>,
    pub profile_errors: Vec<ProfileError>,
}

/// A game file or folder found in a library, before merging.
#[derive(Debug, Clone, PartialEq)]
struct RomEntry {
    /// System folder name inside the library.
    folder: String,
    /// Path inside the system folder: `Game.nsp` or `Folder/Game.3ds` (the ROM inside a game folder).
    rel: String,
    /// Name of the entry in the system folder (file or folder name).
    name: String,
}

fn ext_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => format!(".{}", ext.to_lowercase()),
        _ => String::new(),
    }
}

/// Is `name` a game file for `system`? Without known extensions every file but text files counts.
fn is_game_file(name: &str, system: &System) -> bool {
    if name.starts_with('.') || name.eq_ignore_ascii_case("systeminfo.txt") {
        return false;
    }
    let ext = ext_of(name);
    if system.exts.is_empty() {
        return !matches!(ext.as_str(), ".txt" | ".part" | ".thorbro-part" | ".nfo" | ".xml" | ".jpg" | ".png" | "");
    }
    system.exts.contains(&ext)
}

/// Parses the listing of one library (`@@F` = files at depth 2 and 3, `@@D` = folders at depth 2,
/// paths like `./switch/Game.nsp`) into game entries per system folder.
fn parse_library_files(output: &str, lib: &Library) -> Vec<RomEntry> {
    let mut files_in: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut top_files: Vec<(String, String)> = Vec::new();
    let mut dirs: Vec<(String, String)> = Vec::new();
    let mut section = "";
    for line in output.lines() {
        if line == "@@F" || line == "@@D" {
            section = line;
            continue;
        }
        let Some(rest) = line.strip_prefix("./") else { continue };
        let parts: Vec<&str> = rest.split('/').collect();
        match (section, parts.len()) {
            ("@@F", 2) => top_files.push((parts[0].into(), parts[1].into())),
            ("@@F", 3) => files_in.entry((parts[0].into(), parts[1].into())).or_default().push(parts[2].into()),
            ("@@D", 2) => dirs.push((parts[0].into(), parts[1].into())),
            _ => {}
        }
    }
    let mut entries = Vec::new();
    for (folder, name) in top_files {
        if lib.systems.get(&folder).is_some_and(|system| is_game_file(&name, system)) {
            entries.push(RomEntry { rel: name.clone(), folder, name });
        }
    }
    for (folder, dir) in dirs {
        let Some(system) = lib.systems.get(&folder) else { continue };
        if system.exts.is_empty() || dir.starts_with('.') || NOT_GAME_DIRS.contains(&dir.to_lowercase().as_str()) {
            continue;
        }
        let inner = files_in.get(&(folder.clone(), dir.clone())).cloned().unwrap_or_default();
        let mut roms: Vec<&String> = inner
            .iter()
            .filter(|name| is_game_file(name, system) && !ARCHIVE_EXTS.contains(&ext_of(name).as_str()) && ext_of(name) != ".m3u")
            .collect();
        roms.sort_by_key(|name| (!N3DS_HEADER_EXTS.contains(&ext_of(name).as_str()), (*name).clone()));
        if let Some(rom) = roms.first() {
            entries.push(RomEntry { rel: format!("{dir}/{rom}"), name: dir.clone(), folder });
        }
    }
    entries
}

/// Lists the game files of one library (one adb call).
fn list_library_files(lib: &Library) -> Vec<RomEntry> {
    let script = format!(
        "cd {} 2>/dev/null || exit 1; echo @@F; find . -mindepth 2 -maxdepth 3 -type f 2>/dev/null; echo @@D; find . -mindepth 2 -maxdepth 2 -type d 2>/dev/null",
        shell_quote(&lib.path)
    );
    parse_library_files(&sh(&script).stdout, lib)
}

/// 3DS title IDs from the ROM headers (one adb call for all files): Thor path -> title ID.
fn read_n3ds_ids(paths: &[String]) -> HashMap<String, String> {
    if paths.is_empty() {
        return HashMap::new();
    }
    let mut script = String::new();
    for path in paths {
        script += &format!("echo \"@@\"{q}; dd if={q} bs=16 skip=16 count=1 2>/dev/null | xxd -p\n", q = shell_quote(path));
    }
    parse_header_listing(&sh_script(&script).stdout)
}

fn parse_header_listing(output: &str) -> HashMap<String, String> {
    let mut ids_by_path = HashMap::new();
    let mut current: Option<String> = None;
    for line in output.lines() {
        if let Some(path) = line.strip_prefix("@@") {
            current = Some(path.to_string());
        } else if let (Some(path), Some(title_id)) = (&current, ids::n3ds_title_id(line)) {
            ids_by_path.insert(path.clone(), title_id);
        }
    }
    ids_by_path
}

/// The title ID of a library entry, `None` if it has none or isn't a game (update/DLC: `Err`).
fn title_id_for(system_id: &str, entry: &RomEntry, path: &str, n3ds_ids: &HashMap<String, String>) -> Result<Option<String>, ()> {
    match system_id {
        "switch" => match ids::switch_title(&entry.name) {
            Some((title_id, ids::SwitchKind::Base)) => Ok(Some(title_id)),
            Some(_) => Err(()), // updates and DLC are not games
            None => Ok(None),
        },
        "n3ds" => Ok(n3ds_ids.get(path).cloned()),
        _ => Ok(None),
    }
}

/// Short console name without maker / "(…)" (`"Nintendo Switch"` stays, the UI shortens).
fn console_full(id: &str, system: Option<&System>) -> String {
    system
        .map(|system| system.full.clone())
        .filter(|full| !full.is_empty())
        .or_else(|| crate::systems::by_id(id).map(|known| known.full.to_string()))
        .unwrap_or_else(|| id.to_string())
}

/// Game entries per library, plus 3DS title IDs by ROM path.
type Listed<'a> = (Vec<(&'a Library, Vec<RomEntry>)>, HashMap<String, String>);

/// The game entries of every library plus the 3DS title IDs from their headers.
fn list_libraries(libs: &[Library]) -> Listed<'_> {
    let mut n3ds_paths: Vec<String> = Vec::new();
    let mut listed: Vec<(&Library, Vec<RomEntry>)> = Vec::new();
    for lib in libs {
        let entries = list_library_files(lib);
        for entry in &entries {
            let system_id = lib.systems.get(&entry.folder).map(|system| system.id.as_str()).unwrap_or("");
            if system_id == "n3ds" && N3DS_HEADER_EXTS.contains(&ext_of(&entry.rel).as_str()) {
                n3ds_paths.push(format!("{}/{}/{}", lib.path, entry.folder, entry.rel));
            }
        }
        listed.push((lib, entries));
    }
    (listed, read_n3ds_ids(&n3ds_paths))
}

/// Builds the whole catalog. `libs` are the libraries the file manager already found.
pub fn build(libs: &[Library]) -> Catalog {
    let profiles = emulators::load();
    // Three independent parts, each a few adb calls: run them side by side (adb serves several
    // shell sessions at once). `thread::scope` lets the threads borrow `profiles` and `libs`.
    let ((emulators, packs), art, (listed, n3ds_ids)) = std::thread::scope(|scope| {
        let emulators = scope.spawn(|| emulator_infos(&profiles, &thor::installed_packages().unwrap_or_default()));
        let art = scope.spawn(art::index);
        let listed = scope.spawn(|| list_libraries(libs));
        (emulators.join().expect("emulator thread"), art.join().expect("art thread"), listed.join().expect("library thread"))
    });

    // Library games, per console id.
    let mut consoles: Vec<ConsoleBlock> = Vec::new();
    for (lib, entries) in listed {
        for entry in entries {
            let system = lib.systems.get(&entry.folder);
            let system_id = system.map(|system| system.id.clone()).filter(|id| !id.is_empty()).unwrap_or_else(|| entry.folder.clone());
            let rom = format!("{}/{}/{}", lib.path, entry.folder, entry.rel);
            let Ok(title_id) = title_id_for(&system_id, &entry, &rom, &n3ds_ids) else { continue };
            let block = console_block(&mut consoles, &system_id, Some(&entry.folder), system);
            let art = art.find(&[&system_id, &entry.folder], Some(&entry.rel), title_id.as_deref()).map(art::art_ref);
            block.games.push(GameEntry { title: ids::clean_title(&entry.name), title_id, rom: Some(rom), lib: Some(lib.path.clone()), art, saves: vec![] });
        }
    }

    // Games an emulator brings itself, and the emulators per console.
    for (emulator, games) in emulators.iter().zip(packs) {
        if !emulator.installed {
            continue;
        }
        let block = console_block(&mut consoles, &emulator.console, None, None);
        block.emulators.push(emulator.id.clone());
        for (title_id, name) in games {
            let art_file = art.find(&[&emulator.console], None, Some(&title_id));
            block.games.push(GameEntry {
                title: name.unwrap_or_else(|| title_id.clone()),
                art: art_file.map(art::art_ref),
                title_id: Some(title_id),
                rom: None,
                lib: None,
                saves: vec![],
            });
        }
    }

    let mut search: Vec<SearchGame> = Vec::new();
    for block in &mut consoles {
        dedupe_games(&mut block.games);
        for game in &mut block.games {
            if let Some(title_id) = &game.title_id {
                game.saves = emulators
                    .iter()
                    .filter(|emulator| emulator.console == block.id && emulator.saves.iter().any(|save| &save.title_id == title_id))
                    .map(|emulator| emulator.id.clone())
                    .collect();
            }
        }
        block.games.sort_by_key(|game| game.title.to_lowercase());
        search.extend(block.games.iter().filter_map(|game| {
            Some(SearchGame { console: block.id.clone(), console_name: block.full.clone(), title: game.title.clone(), title_id: game.title_id.clone()? })
        }));
        // Saves of games that are neither in a library nor brought by the emulator: findable by title ID.
        let known: BTreeSet<String> = block.games.iter().filter_map(|game| game.title_id.clone()).collect();
        let orphans: BTreeSet<String> = emulators
            .iter()
            .filter(|emulator| emulator.console == block.id)
            .flat_map(|emulator| emulator.saves.iter().map(|save| save.title_id.clone()))
            .filter(|title_id| !known.contains(title_id))
            .collect();
        search.extend(orphans.into_iter().map(|title_id| SearchGame { console: block.id.clone(), console_name: block.full.clone(), title: title_id.clone(), title_id }));
    }
    consoles.retain(|block| !block.games.is_empty() || !block.emulators.is_empty());
    consoles.sort_by_key(|block| std::cmp::Reverse(block.games.len()));
    Catalog { consoles, emulators, search, profile_errors: profiles.errors }
}

/// The block of console `id`, created if missing.
fn console_block<'a>(consoles: &'a mut Vec<ConsoleBlock>, id: &str, folder: Option<&str>, system: Option<&System>) -> &'a mut ConsoleBlock {
    let index = match consoles.iter().position(|block| block.id == id) {
        Some(index) => index,
        None => {
            consoles.push(ConsoleBlock { id: id.to_string(), folder: None, full: console_full(id, system), games: vec![], emulators: vec![] });
            consoles.len() - 1
        }
    };
    let block = &mut consoles[index];
    if block.folder.is_none() {
        block.folder = folder.map(String::from);
        if system.is_some() {
            block.full = console_full(id, system);
        }
    }
    block
}

/// Removes duplicates: the same title ID, or the same title (e.g. a `.cci`, a game folder and a
/// `.7z` of one game). The first entry wins; a later one may only add art it is missing.
fn dedupe_games(games: &mut Vec<GameEntry>) {
    let mut kept: Vec<GameEntry> = Vec::new();
    for game in games.drain(..) {
        let title_key = game.title.to_lowercase();
        let duplicate = kept.iter_mut().find(|other| {
            (game.title_id.is_some() && other.title_id == game.title_id) || other.title.to_lowercase() == title_key
        });
        match duplicate {
            Some(other) => {
                if other.art.is_none() {
                    other.art = game.art;
                }
                if other.title_id.is_none() {
                    other.title_id = game.title_id;
                }
            }
            None => kept.push(game),
        }
    }
    *games = kept;
}

/// The games an emulator brings itself, per profile (same order as the infos).
type PackGames = Vec<Vec<(String, Option<String>)>>;

/// Every profile with its install state and, if installed, its save folders and own games.
fn emulator_infos(profiles: &emulators::Profiles, installed: &BTreeSet<String>) -> (Vec<EmulatorInfo>, PackGames) {
    let mut infos: Vec<EmulatorInfo> = Vec::new();
    let mut packs: PackGames = Vec::new();
    let mut save_paths: Vec<String> = Vec::new();
    let mut found: Vec<Vec<(String, String)>> = Vec::new();
    for loaded in &profiles.list {
        let profile = &loaded.profile;
        let is_installed = installed.contains(&profile.package);
        let (saves, error, pack) = if is_installed {
            let resolved = thor::resolve(profile);
            (resolved.list_saves(), resolved.error.clone(), thor::pack_games(profile))
        } else {
            (vec![], None, vec![])
        };
        let games = profile.games.as_ref().map(|_| pack.len());
        packs.push(pack);
        save_paths.extend(saves.iter().map(|(_, path)| path.clone()));
        found.push(saves);
        infos.push(EmulatorInfo {
            id: profile.id.clone(),
            name: profile.name.clone(),
            package: profile.package.clone(),
            console: profile.console.clone(),
            installed: is_installed,
            write: profile.saves.write,
            write_note: profile.write_block(),
            source: loaded.source,
            games,
            saves: vec![],
            error,
        });
    }
    // One adb call for the file counts of all save folders; empty folders don't count as a save.
    let stats: HashMap<String, (u64, f64)> = crate::saves::thor_stats(&save_paths, false)
        .into_iter()
        .map(|(path, files, mtime)| (path, (files, mtime)))
        .collect();
    for (info, saves) in infos.iter_mut().zip(found) {
        info.saves = saves
            .into_iter()
            .filter_map(|(title_id, path)| {
                let (files, newest) = *stats.get(&path)?;
                Some(SaveEntry { title_id, path, files, newest, when: util::when(newest) })
            })
            .collect();
    }
    (infos, packs)
}

/// The save folder of `title_id` for emulator `profile` on the Thor, with its stats. Several
/// bases (a search can find more than one): the newest save wins.
pub fn find_save(profile: &Profile, title_id: &str) -> Option<SaveEntry> {
    let title_id = profile.saves.title_id.normalize(title_id)?;
    let resolved = thor::resolve(profile);
    let folders = resolved.folders_for(&title_id);
    let stats = crate::saves::thor_stats(&folders, false);
    stats
        .into_iter()
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(path, files, newest)| SaveEntry { title_id: title_id.clone(), path, files, newest, when: util::when(newest) })
}

/// For tests and the CLI: is this title ID the right kind for the profile?
pub fn title_id_fits(profile: &Profile, title_id: &str) -> bool {
    match profile.saves.title_id {
        TitleIdKind::Switch => profile.saves.title_id.normalize(title_id).is_some_and(|id| id.starts_with("0100")),
        TitleIdKind::N3ds => profile.saves.title_id.normalize(title_id).is_some_and(|id| id.starts_with("00040000")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn system(id: &str, exts: &[&str]) -> System {
        System { id: id.into(), origin: "systeminfo", full: String::new(), exts: exts.iter().map(|ext| ext.to_string()).collect(), count: 0 }
    }

    /// The library on the Thor's SD card (excerpt of the real listing from step 1).
    fn sd_library() -> Library {
        let mut systems = BTreeMap::new();
        systems.insert("switch".to_string(), system("switch", &[".nca", ".nro", ".nso", ".nsp", ".xci"]));
        systems.insert("n3ds".to_string(), system("n3ds", &[".3ds", ".3dsx", ".app", ".axf", ".cci", ".cxi", ".elf", ".z3dsx", ".zcci", ".zcxi", ".7z", ".zip"]));
        Library { path: "/storage/EEFF-BF6C/ROMs".into(), label: "SD card".into(), systems, total: 0, custom: false }
    }

    const LISTING: &str = "@@F
./switch/systeminfo.txt
./switch/The Legend of Zelda Breath of the Wild [01007EF00011E800][BASE][v1.5.0].xci
./switch/The Legend of Zelda Breath of the Wild [DLC Pack 1] [01007EF00011F001][v196608].nsp
./switch/Nine Sols [0100D810170D0800][v327680].nsp
./switch/Nine Sols [0100D810170D0000][v0][US].nsp
./switch/Mario Tennis Aces [0100BDE00862A800][v786432][US](axekin.fr).nsp.thorbro-part
./switch/update/Cult of the Lamb [01002E7016C46800][v1966080].nsp
./switch/SUPER MARIO ODYSSEY/SUPER MARIO ODYSSEY.nup
./n3ds/Pokemon X.3ds
./n3ds/LEGO Ninjago - Shadow of Ronin (USA) (En,Fr,Es,Pt) (Rev 1).cci
./n3ds/LEGO Ninjago - Shadow of Ronin (USA) (En_Fr_Es_Pt) (Rev 1).7z
./n3ds/LEGO Ninjago - Shadow of Ronin (USA) (En_Fr_Es_Pt) (Rev 1)/LEGO Ninjago - Shadow of Ronin (USA) (En_Fr_Es_Pt) (Rev 1).cci
./n3ds/Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11).3ds
./n3ds/Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11)/Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11).3ds
./n3ds/Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11)/Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11).m3u
./n3ds/sysdata/aes_keys.txt
./n3ds/config/config.ini
@@D
./switch/update
./switch/SUPER MARIO ODYSSEY
./n3ds/LEGO Ninjago - Shadow of Ronin (USA) (En_Fr_Es_Pt) (Rev 1)
./n3ds/Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11)
./n3ds/sdmc
./n3ds/sysdata
./n3ds/config
";

    #[test]
    fn library_files_skip_updates_parts_and_emulator_folders() {
        let entries = parse_library_files(LISTING, &sd_library());
        let rels: Vec<&str> = entries.iter().map(|entry| entry.rel.as_str()).collect();
        assert_eq!(
            rels,
            [
                "The Legend of Zelda Breath of the Wild [01007EF00011E800][BASE][v1.5.0].xci",
                "The Legend of Zelda Breath of the Wild [DLC Pack 1] [01007EF00011F001][v196608].nsp",
                "Nine Sols [0100D810170D0800][v327680].nsp",
                "Nine Sols [0100D810170D0000][v0][US].nsp",
                "Pokemon X.3ds",
                "LEGO Ninjago - Shadow of Ronin (USA) (En,Fr,Es,Pt) (Rev 1).cci",
                "LEGO Ninjago - Shadow of Ronin (USA) (En_Fr_Es_Pt) (Rev 1).7z",
                "Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11).3ds",
                "LEGO Ninjago - Shadow of Ronin (USA) (En_Fr_Es_Pt) (Rev 1)/LEGO Ninjago - Shadow of Ronin (USA) (En_Fr_Es_Pt) (Rev 1).cci",
                "Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11)/Super Smash Bros. for Nintendo 3DS (Europe) (Rev 11).3ds",
            ]
        );
        // Switch: updates and DLC are filtered by title ID, the [BASE] dump stays.
        let switch: Vec<Option<String>> = entries
            .iter()
            .filter(|entry| entry.folder == "switch")
            .filter_map(|entry| title_id_for("switch", entry, "", &HashMap::new()).ok())
            .collect();
        assert_eq!(switch, [Some("01007EF00011E000".to_string()), Some("0100D810170D0000".to_string())]);
    }

    #[test]
    fn duplicates_merge_by_title_id_or_title() {
        let game = |title: &str, title_id: Option<&str>, art: bool| GameEntry {
            title: title.into(),
            title_id: title_id.map(String::from),
            rom: None,
            lib: None,
            art: art.then(|| ArtRef { path: "p".into(), key: "k".into(), cached: false }),
            saves: vec![],
        };
        let mut games = vec![
            game("LEGO Ninjago - Shadow of Ronin", Some("0004000000154400"), false),
            game("LEGO Ninjago - Shadow of Ronin", None, true),
            game("Super Smash Bros. for Nintendo 3DS", Some("00040000000EE000"), true),
            game("Smash", Some("00040000000EE000"), false),
            game("Pokemon X", None, false),
        ];
        dedupe_games(&mut games);
        assert_eq!(games.len(), 3);
        assert!(games[0].art.is_some(), "the duplicate's art is kept");
    }

    #[test]
    fn header_listing_maps_paths_to_title_ids() {
        // `xxd -p` prints 16 bytes as one line of 32 hex digits.
        let output = "@@/storage/X/ROMs/n3ds/Pokemon X.3ds\n4e43534400000000005d050000000400\n@@/storage/X/ROMs/n3ds/broken.3ds\n\n";
        let ids = parse_header_listing(output);
        assert_eq!(ids.get("/storage/X/ROMs/n3ds/Pokemon X.3ds").map(String::as_str), Some("0004000000055D00"));
        assert_eq!(ids.len(), 1);
    }

    /// Real Thor (`cargo test device_catalog -- --ignored --nocapture`): builds the catalog and
    /// checks what step 1 verified by hand. Read-only.
    #[test]
    #[ignore]
    fn device_catalog_from_the_thor() {
        let libs = crate::library::find_libraries(&crate::adb::storages(), &crate::settings::load());
        let started = std::time::Instant::now();
        let catalog = build(&libs);
        println!("catalog in {:?}", started.elapsed());
        for emulator in &catalog.emulators {
            println!("{} installed={} games={:?} error={:?}", emulator.id, emulator.installed, emulator.games, emulator.error);
            for save in &emulator.saves {
                println!("   {} {} files {} {}", save.title_id, save.files, save.when, save.path);
            }
        }
        for block in &catalog.consoles {
            println!("[{}] {} – {} games, emulators {:?}", block.id, block.full, block.games.len(), block.emulators);
            for game in &block.games {
                println!("   {:45} {:18} art={} saves={:?}", game.title, game.title_id.clone().unwrap_or_default(), game.art.is_some(), game.saves);
            }
        }
        let emulator = |id: &str| catalog.emulators.iter().find(|emulator| emulator.id == id).unwrap();
        let save = |id: &str, title_id: &str| emulator(id).saves.iter().find(|save| save.title_id == title_id).cloned();
        assert_eq!(save("eden-duo", "01007EF00011E000").unwrap().files, 27);
        assert_eq!(save("peanuts", "01007EF00011E000").unwrap().files, 27);
        assert!(save("eden", "01007EF00011E000").is_none());
        assert!(save("azahar", "00040000000EE000").is_some());
        assert!(emulator("eden-duo").saves.iter().all(|save| save.path.contains("5D1B4CDA089A581DAA9E48BC2B43C120")));
        let switch = catalog.consoles.iter().find(|block| block.id == "switch").unwrap();
        let botw = switch.games.iter().find(|game| game.title_id.as_deref() == Some("01007EF00011E000")).unwrap();
        assert!(botw.art.is_some());
        assert_eq!(botw.saves, ["eden-duo", "peanuts"]);
    }

    #[test]
    fn title_ids_fit_their_console() {
        let profiles = emulators::load_from(None);
        assert!(title_id_fits(profiles.by_id("peanuts").unwrap(), "01007ef00011e000"));
        assert!(!title_id_fits(profiles.by_id("peanuts").unwrap(), "00040000000EE000"));
        assert!(title_id_fits(profiles.by_id("azahar").unwrap(), "00040000000EE000"));
        assert!(!title_id_fits(profiles.by_id("azahar").unwrap(), "000ee000"));
    }
}
