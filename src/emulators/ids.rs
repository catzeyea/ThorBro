//! Pure parsing helpers (no adb): title IDs from Switch file names and 3DS ROM headers, the
//! user folder from a yuzu-style `profiles.dat`, ini values, display titles from file names.
//!
//! Every format here was checked against files on a real Thor (step 1 of Update 9); the tests
//! use bytes and names copied from there.

use regex::Regex;
use std::sync::OnceLock;

/// What a Switch file is, judged by the title ID in its name.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SwitchKind {
    Base,
    Update,
    Dlc,
}

/// Switch title ID (`0100` + 12 hex digits in `[...]` or `(...)`) from a file name, uppercase.
pub fn switch_id(name: &str) -> Option<String> {
    static TITLE_ID_RE: OnceLock<Regex> = OnceLock::new();
    let regex = TITLE_ID_RE.get_or_init(|| Regex::new(r"[\[(](0100[0-9A-Fa-f]{12})[\])]").unwrap());
    regex.captures(name).map(|captures| captures[1].to_uppercase())
}

/// The base game's title ID and the kind of the file `name`.
///
/// Base games end in `000`, updates in `800`, DLC count up from base + `0x1000`. A file tagged
/// `[BASE]` is a base game even with an update ID in its name; on the Thor
/// `The Legend of Zelda Breath of the Wild [01007EF00011E800][BASE][v1.5.0].xci` is the base
/// game and its save folder is `01007EF00011E000`.
pub fn switch_title(name: &str) -> Option<(String, SwitchKind)> {
    let tid = switch_id(name)?;
    let value = u64::from_str_radix(&tid, 16).ok()?;
    let low = value & 0xFFF;
    let tagged_base = name.to_uppercase().contains("[BASE]");
    let (base, kind) = if low == 0 {
        (value, SwitchKind::Base)
    } else if low == 0x800 {
        (value & !0xFFF, if tagged_base { SwitchKind::Base } else { SwitchKind::Update })
    } else if tagged_base {
        (value & !0xFFF, SwitchKind::Base)
    } else {
        (value.wrapping_sub(0x1000) & !0xFFF, SwitchKind::Dlc)
    };
    Some((format!("{base:016X}"), kind))
}

/// 3DS title ID from the first 16 bytes at offset 0x100 of a `.3ds` / `.cci` file (hex, as
/// printed by `xxd -p`): `NCSD` magic, 4 bytes size, 8 bytes title ID (little endian).
/// Only application IDs (`00040000…`) are returned.
pub fn n3ds_title_id(header_hex: &str) -> Option<String> {
    let bytes = hex_bytes(header_hex)?;
    if bytes.len() < 16 || &bytes[0..4] != b"NCSD" {
        return None;
    }
    let id = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let text = format!("{id:016X}");
    text.starts_with("00040000").then_some(text)
}

/// Bytes from a hex dump (whitespace and line breaks are ignored).
pub fn hex_bytes(hex: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = hex.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// Size of the header in `profiles.dat` and of one user entry (UUID, UUID, timestamp, name, data).
const PROFILES_HEADER: usize = 0x10;
const PROFILES_ENTRY: usize = 0xC8;
const PROFILES_SLOTS: usize = 8;

/// The save folder name of a user in a yuzu-style `profiles.dat`: the entry's 16-byte UUID with
/// its bytes reversed, as uppercase hex. `index` picks the slot (`current_user` in config.ini);
/// if that slot is empty, the first user is used. `None` if the file has no user.
pub fn switch_user_folder(data: &[u8], index: Option<usize>) -> Option<String> {
    let uuid_at = |slot: usize| -> Option<&[u8]> {
        let start = PROFILES_HEADER + slot * PROFILES_ENTRY;
        let uuid = data.get(start..start + 16)?;
        uuid.iter().any(|byte| *byte != 0).then_some(uuid)
    };
    let uuid = index.and_then(uuid_at).or_else(|| (0..PROFILES_SLOTS).find_map(uuid_at))?;
    Some(uuid.iter().rev().map(|byte| format!("{byte:02X}")).collect())
}

/// Value of `key=` in an ini file, optionally only inside `[section]`. Empty values count as
/// missing. Keys must match exactly (`nand_directory` doesn't match `nand_directory\default`).
pub fn ini_value(text: &str, section: Option<&str>, key: &str) -> Option<String> {
    let mut current: Option<&str> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            current = Some(name);
            continue;
        }
        if section.is_some() && current != section {
            continue;
        }
        if let Some(value) = line.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')) {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

/// Game name from the first line of a Peanuts `game.tsv`. Seen on the Thor:
/// `# Game profile of The Legend of Zelda: Breath of the Wild (Switch, base game …` and
/// `# Super Mario Odyssey (Nintendo EPD's sead / agl engine, …`.
pub fn pack_title(first_line: &str) -> Option<String> {
    let rest = first_line.trim().strip_prefix('#')?.trim();
    let rest = rest.strip_prefix("Game profile of ").unwrap_or(rest);
    let name = rest.split(" (").next()?.trim();
    (!name.is_empty() && name.len() <= 120).then(|| name.to_string())
}

/// Display title from a ROM file or folder name: extension, `[…]` / `(…)` / `{…}` groups and
/// `_` removed, spaces collapsed. `"Mario & Luigi - Dream Team (USA) (En,Fr,Es).3ds"` ->
/// `"Mario & Luigi - Dream Team"`.
pub fn clean_title(name: &str) -> String {
    static EXT_RE: OnceLock<Regex> = OnceLock::new();
    static GROUP_RE: OnceLock<Regex> = OnceLock::new();
    let without_ext = EXT_RE.get_or_init(|| Regex::new(r"\.[A-Za-z0-9]{1,5}$").unwrap()).replace(name, "");
    let without_groups = GROUP_RE.get_or_init(|| Regex::new(r"\[[^\]]*\]|\([^)]*\)|\{[^}]*\}").unwrap()).replace_all(&without_ext, " ");
    let spaced = without_groups.replace('_', " ");
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c: char| c == '-' || c == '+' || c.is_whitespace()).to_string();
    if trimmed.is_empty() { without_ext.trim().to_string() } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_titles_from_real_file_names() {
        let base = |name: &str| switch_title(name).unwrap();
        assert_eq!(base("The Legend of Zelda Breath of the Wild [01007EF00011E800][BASE][v1.5.0].xci"), ("01007EF00011E000".into(), SwitchKind::Base));
        assert_eq!(base("Dead Cells [0100646009FBE000][v0].xci"), ("0100646009FBE000".into(), SwitchKind::Base));
        assert_eq!(base("Dead Cells [0100646009FBE800][v3014656][US].nsp"), ("0100646009FBE000".into(), SwitchKind::Update));
        assert_eq!(base("The Legend of Zelda Breath of the Wild [DLC Pack 1] [01007EF00011F001][v196608].nsp"), ("01007EF00011E000".into(), SwitchKind::Dlc));
        assert_eq!(base("Cult of the Lamb[01002E7016C47004].nsp"), ("01002E7016C46000".into(), SwitchKind::Dlc));
        assert_eq!(base("Super Mario Odyssey[0100000000010000][V0].nsp"), ("0100000000010000".into(), SwitchKind::Base));
        assert_eq!(base("Neon Abyss [0100BAB01113A000]+[v1.0.4].nsp"), ("0100BAB01113A000".into(), SwitchKind::Base));
        assert_eq!(switch_title("SUPER MARIO ODYSSEY.nup"), None);
    }

    /// Bytes 0x100..0x110 of three ROMs on the Thor; their saves are in Azahar's
    /// `title/00040000/000ee000`, `000d5a00` and `00055d00`.
    #[test]
    fn n3ds_title_ids_from_real_headers() {
        assert_eq!(n3ds_title_id("4e435344 00000000 00e00e0000000400").as_deref(), Some("00040000000EE000"));
        assert_eq!(n3ds_title_id("4e43534400000000005a0d0000000400").as_deref(), Some("00040000000D5A00"));
        assert_eq!(n3ds_title_id("4e435344000000\n00005d050000000400").as_deref(), Some("0004000000055D00"));
        assert_eq!(n3ds_title_id("00000000000000000000000000000000"), None, "no NCSD magic");
        assert_eq!(n3ds_title_id("4e43534400000000005a0d0000000E00"), None, "not an application");
        assert_eq!(n3ds_title_id("4e4353"), None);
    }

    /// The first 0x60 bytes of Eden Duo's profiles.dat on the Thor (one user, "Eden").
    fn eden_profiles_dat() -> Vec<u8> {
        let mut data = hex_bytes(
            "00000000000000000000000000000000\
             20c1432bbc489eaa1d589a08da4c1b5d\
             20c1432bbc489eaa1d589a08da4c1b5d\
             00000000000000004564656e00000000",
        )
        .unwrap();
        data.resize(PROFILES_HEADER + PROFILES_SLOTS * PROFILES_ENTRY, 0);
        data
    }

    #[test]
    fn eden_user_folder_is_the_reversed_uuid() {
        let data = eden_profiles_dat();
        // The save folder on the Thor is .../0000000000000000/5D1B4CDA089A581DAA9E48BC2B43C120/.
        assert_eq!(switch_user_folder(&data, Some(0)).as_deref(), Some("5D1B4CDA089A581DAA9E48BC2B43C120"));
        assert_eq!(switch_user_folder(&data, None).as_deref(), Some("5D1B4CDA089A581DAA9E48BC2B43C120"));
        // An empty slot falls back to the first user, never to an orphan folder.
        assert_eq!(switch_user_folder(&data, Some(3)).as_deref(), Some("5D1B4CDA089A581DAA9E48BC2B43C120"));
        let mut second = data.clone();
        let slot1 = PROFILES_HEADER + PROFILES_ENTRY;
        second[slot1..slot1 + 16].copy_from_slice(&[0x11; 16]);
        assert_eq!(switch_user_folder(&second, Some(1)).as_deref(), Some("11111111111111111111111111111111"));
        assert_eq!(switch_user_folder(&[0u8; 400], Some(0)), None);
        assert_eq!(switch_user_folder(&[1, 2, 3], None), None);
    }

    #[test]
    fn ini_values_from_eden_config() {
        let ini = "[Data%20Storage]\nnand_directory\\default=false\nnand_directory=/storage/emulated/0/Android/data/dev.igawa6.edenduo/files/nand\nsave_directory=\n\
                   [System]\ncurrent_user\\default=true\ncurrent_user=0\n[UI]\nPaths\\nand_directory=/elsewhere\n";
        assert_eq!(ini_value(ini, Some("Data%20Storage"), "nand_directory").as_deref(), Some("/storage/emulated/0/Android/data/dev.igawa6.edenduo/files/nand"));
        assert_eq!(ini_value(ini, Some("System"), "current_user").as_deref(), Some("0"));
        assert_eq!(ini_value(ini, Some("System"), "nand_directory"), None);
        assert_eq!(ini_value(ini, None, "save_directory"), None, "empty counts as missing");
        assert_eq!(ini_value(ini, None, "current_user").as_deref(), Some("0"));
    }

    #[test]
    fn titles_from_names() {
        assert_eq!(clean_title("Mario & Luigi - Dream Team (USA) (En,Fr,Es).3ds"), "Mario & Luigi - Dream Team");
        assert_eq!(clean_title("The Legend of Zelda Breath of the Wild [01007EF00011E800][BASE][v1.5.0].xci"), "The Legend of Zelda Breath of the Wild");
        assert_eq!(clean_title("Super Mario Odyssey[0100000000010000][V0].nsp"), "Super Mario Odyssey");
        assert_eq!(clean_title("LEGO Ninjago - Shadow of Ronin (USA) (En_Fr_Es_Pt) (Rev 1)"), "LEGO Ninjago - Shadow of Ronin");
        assert_eq!(clean_title("Neon Abyss [0100BAB01113A000]+[v1.0.4].nsp"), "Neon Abyss");
        assert_eq!(clean_title("Pokemon X.3ds"), "Pokemon X");
        assert_eq!(clean_title("[0100000000010000].nsp"), "[0100000000010000]");
        assert_eq!(pack_title("# Game profile of The Legend of Zelda: Breath of the Wild (Switch, base game v0 + update 1.9.0 = v1114112). In bring-up").as_deref(), Some("The Legend of Zelda: Breath of the Wild"));
        assert_eq!(pack_title("# Super Mario Odyssey (Nintendo EPD's sead / agl engine, NintendoWare G3d / Ui2d / Vfx, SDK 7.3.2), update 1.4.1.").as_deref(), Some("Super Mario Odyssey"));
        assert_eq!(pack_title("main\t0x1"), None);
    }
}
