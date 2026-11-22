//! Built-in list of known systems, used for system folders that have no ES-DE `systeminfo.txt`
//! (folders the user created by hand, e.g. `Wii` or `Game Boy Advance`).
//!
//! A folder name is matched case-insensitively against the aliases, ignoring spaces, dashes etc.
//! The extension lists are a practical subset of what ES-DE uses; they only feed the guessing
//! in `library.rs`, the user can always pick another target in the sort dialog.
//!
//! Used by `library.rs` (folder name fallback, manual choice) and `commands.rs` (`known_systems`).

use serde::Serialize;

/// One known system.
#[derive(Serialize, Clone, Copy)]
pub struct KnownSystem {
    /// Canonical ES-DE folder name, e.g. "snes".
    pub id: &'static str,
    /// Display name.
    pub full: &'static str,
    /// Normalized folder names that mean this system (see `normalize`).
    #[serde(skip)]
    pub aliases: &'static [&'static str],
    /// Supported extensions, lowercase with dot, separated by spaces.
    #[serde(skip)]
    pub exts: &'static str,
}

/// Shorthand to keep the table below compact.
const fn sys(
    id: &'static str,
    full: &'static str,
    aliases: &'static [&'static str],
    exts: &'static str,
) -> KnownSystem {
    KnownSystem {
        id,
        full,
        aliases,
        exts,
    }
}

/// All systems ThorBro knows without a `systeminfo.txt`. One line per system on purpose.
#[rustfmt::skip]
pub const KNOWN: &[KnownSystem] = &[
    sys("nes", "Nintendo Entertainment System", &["nes", "famicom", "fc"], ".nes .unf .unif .fds .zip .7z"),
    sys("snes", "Super Nintendo", &["snes", "superfamicom", "sfc", "supernintendo"], ".sfc .smc .fig .swc .bs .zip .7z"),
    sys("n64", "Nintendo 64", &["n64", "nintendo64"], ".n64 .z64 .v64 .ndd .zip .7z"),
    sys("gc", "Nintendo GameCube", &["gc", "gamecube", "ngc", "nintendogamecube"], ".iso .gcm .gcz .ciso .rvz .wia .dol .elf"),
    sys("wii", "Nintendo Wii", &["wii", "nintendowii"], ".iso .wbfs .rvz .wia .gcz .ciso .wad .dol .elf"),
    sys("wiiu", "Nintendo Wii U", &["wiiu", "nintendowiiu"], ".wua .wud .wux .rpx .iso"),
    sys("switch", "Nintendo Switch", &["switch", "nintendoswitch", "ns"], ".nsp .xci .nsz .xcz .nro"),
    sys("gb", "Nintendo Game Boy", &["gb", "gameboy"], ".gb .zip .7z"),
    sys("gbc", "Nintendo Game Boy Color", &["gbc", "gameboycolor"], ".gbc .gb .zip .7z"),
    sys("gba", "Nintendo Game Boy Advance", &["gba", "gameboyadvance"], ".gba .zip .7z"),
    sys("nds", "Nintendo DS", &["nds", "ds", "nintendods"], ".nds .zip .7z"),
    sys("n3ds", "Nintendo 3DS", &["n3ds", "3ds", "nintendo3ds"], ".3ds .3dsx .cci .cia .cxi .app .zcci .zcxi .z3dsx"),
    sys("virtualboy", "Nintendo Virtual Boy", &["virtualboy", "vb"], ".vb .zip .7z"),
    sys("psx", "Sony PlayStation", &["psx", "ps1", "psone", "playstation", "playstation1"], ".cue .bin .chd .pbp .iso .img .m3u .ecm .ccd .zip .7z"),
    sys("ps2", "Sony PlayStation 2", &["ps2", "playstation2"], ".iso .chd .cso .zso .bin .cue .gz .m3u"),
    sys("ps3", "Sony PlayStation 3", &["ps3", "playstation3"], ".ps3 .ps3dir .iso"),
    sys("psp", "Sony PlayStation Portable", &["psp", "playstationportable"], ".iso .cso .pbp .chd"),
    sys("psvita", "Sony PlayStation Vita", &["psvita", "vita", "playstationvita"], ".vpk .psvita"),
    sys("mastersystem", "Sega Master System", &["mastersystem", "sms", "segamastersystem"], ".sms .bin .zip .7z"),
    sys("gamegear", "Sega Game Gear", &["gamegear", "gg", "segagamegear"], ".gg .bin .zip .7z"),
    sys("megadrive", "Sega Mega Drive", &["megadrive", "md", "segamegadrive"], ".md .gen .smd .bin .zip .7z"),
    sys("genesis", "Sega Genesis", &["genesis", "segagenesis"], ".md .gen .smd .bin .zip .7z"),
    sys("segacd", "Sega CD", &["segacd", "megacd", "segamegacd"], ".cue .chd .iso .m3u"),
    sys("sega32x", "Sega 32X", &["sega32x", "32x"], ".32x .bin .zip .7z"),
    sys("saturn", "Sega Saturn", &["saturn", "segasaturn"], ".cue .chd .iso .ccd .m3u .zip .7z"),
    sys("dreamcast", "Sega Dreamcast", &["dreamcast", "dc", "segadreamcast"], ".cdi .chd .gdi .cue .m3u .zip .7z"),
    sys("pcengine", "NEC PC Engine", &["pcengine", "pce", "turbografx16", "turbografx", "tg16"], ".pce .cue .chd .zip .7z"),
    sys("ngpc", "SNK Neo Geo Pocket Color", &["ngpc", "neogeopocketcolor", "ngp", "neogeopocket"], ".ngc .ngp .zip .7z"),
    sys("neogeo", "SNK Neo Geo", &["neogeo"], ".zip .7z"),
    sys("wonderswan", "Bandai WonderSwan", &["wonderswan", "wswan", "wonderswancolor", "wswanc"], ".ws .wsc .zip .7z"),
    sys("atari2600", "Atari 2600", &["atari2600", "2600"], ".a26 .bin .zip .7z"),
    sys("arcade", "Arcade", &["arcade"], ".zip .7z"),
    sys("mame", "MAME", &["mame"], ".zip .7z"),
    sys("fbneo", "FinalBurn Neo", &["fbneo", "finalburnneo", "fba"], ".zip .7z"),
    sys("dos", "DOS", &["dos", "msdos"], ".bat .com .exe .dosz .zip .7z"),
    sys("scummvm", "ScummVM", &["scummvm"], ".scummvm .svm"),
];

/// Lowercase letters and digits only: `"Game Boy Advance"` -> `"gameboyadvance"`.
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The system with this canonical id (`"wii"`).
pub fn by_id(id: &str) -> Option<&'static KnownSystem> {
    KNOWN.iter().find(|system| system.id == id)
}

/// The system a folder name stands for (`"Wii"`, `"PS1"`, `"Game Boy Advance"`), if any.
pub fn by_folder_name(name: &str) -> Option<&'static KnownSystem> {
    let normalized = normalize(name);
    KNOWN
        .iter()
        .find(|system| system.aliases.contains(&normalized.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_match_case_insensitively() {
        assert_eq!(by_folder_name("Wii").map(|s| s.id), Some("wii"));
        assert_eq!(by_folder_name("WII U").map(|s| s.id), Some("wiiu"));
        assert_eq!(
            by_folder_name("Game Boy Advance").map(|s| s.id),
            Some("gba")
        );
        assert_eq!(by_folder_name("PS1").map(|s| s.id), Some("psx"));
        assert!(by_folder_name("BIOS").is_none());
    }

    #[test]
    fn aliases_are_normalized_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for system in KNOWN {
            for alias in system.aliases {
                assert_eq!(normalize(alias), *alias, "alias {alias} is not normalized");
                assert!(seen.insert(*alias), "alias {alias} is used twice");
            }
        }
    }
}
