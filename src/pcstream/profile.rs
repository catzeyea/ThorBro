//! The "stream profile": a small, versioned JSON document with everything the ThorBro app on
//! the Thor needs to start a stream through Moonlight, built from data the PC side already has.
//! "Send to Thor app" hands it over (`thor::send_profile`).
//! The format is specified in `docs/STREAM-PROFILE.md`; keep both in sync.
//!
//! It never contains credentials: no Sunshine login, no pairing data, no keys.

use super::network::Addresses;
use super::sunshine::ServerInfo;
use serde::{Deserialize, Serialize};

/// Value of `format`, so a reader can tell this JSON from any other.
pub const FORMAT: &str = "thorbro.stream-profile";

/// Current format version. Bump it only for changes old readers would misread (see the doc).
pub const VERSION: u32 = 1;

/// The whole document.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StreamProfile {
    pub format: String,
    pub version: u32,
    pub pc: PcEntry,
    /// Sunshine app names, exactly as Sunshine has them (Moonlight's `AppName` extra).
    pub games: Vec<String>,
}

/// The PC that runs Sunshine.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PcEntry {
    /// Sunshine's `hostname` from `/serverinfo` (what Moonlight shows).
    pub name: String,
    /// Sunshine's `uniqueid` = Moonlight's host UUID (the trampoline's `UUID` extra).
    pub uniqueid: String,
    /// Recommended LAN IPv4 address, if any.
    pub lan_address: Option<String>,
    /// Tailscale IPv4 address, if Tailscale is connected on the PC.
    pub tailscale_address: Option<String>,
    /// Tailscale MagicDNS name, if MagicDNS is on.
    pub tailscale_name: Option<String>,
    /// Sunshine's GameStream HTTP port (`port` in sunshine.conf, default 47989); Moonlight's
    /// "Add PC" takes it as `address:port` when it isn't the default.
    pub sunshine_port: u16,
}

/// Builds the profile from the detection results and the game list.
pub fn build_profile(server: &ServerInfo, sunshine_port: u16, addresses: &Addresses, games: Vec<String>) -> StreamProfile {
    let tailscale = &addresses.tailscale;
    let connected = tailscale.running;
    StreamProfile {
        format: FORMAT.into(),
        version: VERSION,
        pc: PcEntry {
            name: server.hostname.clone(),
            uniqueid: server.uniqueid.clone(),
            lan_address: addresses.recommended_lan.clone(),
            tailscale_address: tailscale.ipv4.clone().filter(|_| connected),
            tailscale_name: tailscale.dns_name.clone().filter(|_| connected),
            sunshine_port,
        },
        games,
    }
}

/// Pretty JSON of the profile.
pub fn to_json(profile: &StreamProfile) -> String {
    serde_json::to_string_pretty(profile).unwrap_or_default()
}

/// Compact JSON of the profile (no indentation), for sending it to the Thor app: same document as
/// `to_json`, about a quarter smaller.
pub fn to_compact_json(profile: &StreamProfile) -> String {
    serde_json::to_string(profile).unwrap_or_default()
}

/// Parts of field names that hint at credentials. A superset of what the Thor app refuses
/// (`ProfileParser.FORBIDDEN_KEYS` in ThorBroAndroid, matched exactly there; here as substrings).
const CREDENTIAL_KEY_PARTS: &[&str] =
    &["password", "passwd", "pin", "token", "secret", "credential", "apikey", "api_key", "auth", "login", "username", "cert", "key"];

/// The first field name in `json` (at any depth) that looks like a credential, if any. Only names
/// are checked, not values, so a game called "Token Quest" is fine.
pub fn credential_field(json: &serde_json::Value) -> Option<String> {
    match json {
        serde_json::Value::Object(map) => map.iter().find_map(|(key, value)| {
            let lower = key.to_lowercase();
            if CREDENTIAL_KEY_PARTS.iter().any(|part| lower.contains(part)) {
                Some(key.clone())
            } else {
                credential_field(value)
            }
        }),
        serde_json::Value::Array(items) => items.iter().find_map(credential_field),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::network::{AddressKind, LocalAddress, TailscaleInfo};
    use super::*;

    fn server() -> ServerInfo {
        ServerInfo {
            hostname: "my-pc".into(),
            gamestream_version: "7.1.431.-1".into(),
            uniqueid: "0123ABCD-4567-89EF-0123-456789ABCDEF".into(),
            https_port: Some(47984),
            current_game: 0,
            busy: false,
        }
    }

    fn addresses(tailscale_running: bool) -> Addresses {
        Addresses {
            lan: vec![LocalAddress { address: "192.168.1.20".into(), interface: "eth0".into(), kind: AddressKind::Lan, virtual_adapter: false }],
            recommended_lan: Some("192.168.1.20".into()),
            tailscale: TailscaleInfo {
                cli_found: true,
                backend_state: if tailscale_running { "Running" } else { "Stopped" }.into(),
                running: tailscale_running,
                ipv4: Some("100.101.102.103".into()),
                dns_name: Some("my-pc.tail1234.ts.net".into()),
                note: None,
            },
            error: None,
        }
    }

    #[test]
    fn profile_json_matches_the_documented_example() {
        let profile = build_profile(&server(), 47989, &addresses(true), vec!["Desktop".into(), "Steam Big Picture".into()]);
        let json: serde_json::Value = serde_json::from_str(&to_json(&profile)).unwrap();
        // Same document as the example in docs/STREAM-PROFILE.md.
        let expected = serde_json::json!({
            "format": "thorbro.stream-profile",
            "version": 1,
            "pc": {
                "name": "my-pc",
                "uniqueid": "0123ABCD-4567-89EF-0123-456789ABCDEF",
                "lan_address": "192.168.1.20",
                "tailscale_address": "100.101.102.103",
                "tailscale_name": "my-pc.tail1234.ts.net",
                "sunshine_port": 47989
            },
            "games": ["Desktop", "Steam Big Picture"]
        });
        assert_eq!(json, expected);
        // Round trip.
        let back: StreamProfile = serde_json::from_value(json).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn tailscale_fields_are_null_when_not_connected() {
        let profile = build_profile(&server(), 48000, &addresses(false), Vec::new());
        let json: serde_json::Value = serde_json::from_str(&to_json(&profile)).unwrap();
        assert_eq!(json["pc"]["tailscale_address"], serde_json::Value::Null);
        assert_eq!(json["pc"]["tailscale_name"], serde_json::Value::Null);
        assert_eq!(json["pc"]["sunshine_port"], 48000);
        assert_eq!(json["games"], serde_json::json!([]));
    }

    #[test]
    fn profile_never_contains_secrets() {
        let profile = build_profile(&server(), 47989, &addresses(true), vec!["Game".into()]);
        let json = to_json(&profile).to_lowercase();
        for word in ["password", "username", "login", "token", "secret", "cert", "key"] {
            assert!(!json.contains(word), "profile contains {word}: {json}");
        }
    }

    #[test]
    fn credential_fields_are_found_at_any_depth() {
        let profile = build_profile(&server(), 47989, &addresses(true), vec!["Token Quest".into(), "Keys & Secrets".into()]);
        let json: serde_json::Value = serde_json::from_str(&to_compact_json(&profile)).unwrap();
        assert_eq!(credential_field(&json), None, "game names (values) must not count");
        for (doc, key) in [
            (serde_json::json!({ "password": "x" }), "password"),
            (serde_json::json!({ "pc": { "Auth": "x" } }), "Auth"),
            (serde_json::json!({ "games": [{ "name": "a", "apiKey": "x" }] }), "apiKey"),
            (serde_json::json!({ "pc": { "pairing_pin": "1234" } }), "pairing_pin"),
            (serde_json::json!({ "client_cert": "x" }), "client_cert"),
        ] {
            assert_eq!(credential_field(&doc).as_deref(), Some(key), "{doc}");
        }
    }

    #[test]
    fn compact_json_is_the_same_document() {
        let profile = build_profile(&server(), 47989, &addresses(true), vec!["Desktop".into()]);
        let compact = to_compact_json(&profile);
        assert!(!compact.contains('\n'));
        let a: serde_json::Value = serde_json::from_str(&compact).unwrap();
        let b: serde_json::Value = serde_json::from_str(&to_json(&profile)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn game_names_with_quotes_survive() {
        let names = vec!["Tom's \"Game\"".to_string(), "Ünïcödé – 2".to_string()];
        let profile = build_profile(&server(), 47989, &addresses(true), names.clone());
        let back: StreamProfile = serde_json::from_str(&to_json(&profile)).unwrap();
        assert_eq!(back.games, names);
    }
}
