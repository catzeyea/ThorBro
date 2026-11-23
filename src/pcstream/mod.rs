//! "Stream PC games to your Thor": glue around an already installed Sunshine (PC), Moonlight
//! (Thor) and optionally Tailscale. ThorBro does not bundle or install any of them.
//!
//! Background and source links for every protocol detail used here are in
//! `docs/PC-STREAMING-RESEARCH.md`. The parts:
//!
//! - `sunshine.rs`: is Sunshine installed / running (service state), and its unauthenticated
//!   `GET /serverinfo` (version, `uniqueid`, current game). Read-only.
//! - `network.rs`: the PC's LAN addresses and its Tailscale address / MagicDNS name (via the
//!   `tailscale` CLI). Read-only; a missing Tailscale never fails anything.
//! - `admin.rs`: Sunshine's admin API (HTTPS on port+1, Basic auth) called from Rust, not from the
//!   webview, so no `Origin` header is sent and no CSRF token is needed. The login lives in the OS
//!   keychain, never in `settings.json`.
//! - `thor.rs`: Moonlight / Tailscale app checks over adb, the "Play on Thor" intent
//!   (`com.limelight/.ShortcutTrampoline`) and "Send to Thor app" (the stream profile as the
//!   `profile_b64` extra of the ThorBro Android app's main activity).
//! - `profile.rs`: the versioned "stream profile" JSON for the ThorBro app on the Thor
//!   (`docs/STREAM-PROFILE.md`).
//!
//! The Tauri commands that use these live in `commands.rs` (section "Stream PC games").

pub mod admin;
pub mod network;
pub mod profile;
pub mod sunshine;
pub mod thor;

use serde::Serialize;

/// Everything the tab shows about the PC side, collected in one call.
#[derive(Serialize, Clone, Debug)]
pub struct PcStatus {
    pub sunshine: sunshine::SunshineStatus,
    pub addresses: network::Addresses,
    /// `linux`, `windows`, `macos`, ... (some hints and buttons differ per OS).
    pub os: &'static str,
}

/// Runs the Sunshine checks and the address lookup in parallel (the `tailscale` CLI and
/// `systemctl` / `sc` each take a moment) and returns both.
pub fn detect() -> PcStatus {
    std::thread::scope(|scope| {
        let addresses = scope.spawn(network::addresses);
        let sunshine = sunshine::status();
        // A panic in the address thread must not take the Sunshine result with it.
        let addresses = addresses.join().unwrap_or_else(|_| network::Addresses::unavailable("address lookup crashed"));
        PcStatus { sunshine, addresses, os: std::env::consts::OS }
    })
}

/// Answer of `stream_profile`.
#[derive(Serialize, Clone, Debug)]
pub struct ProfileAnswer {
    /// The stream profile as pretty JSON (`docs/STREAM-PROFILE.md`).
    pub json: String,
    /// Why `games` is empty although Sunshine may have apps (no working login), if so.
    pub note: Option<String>,
}

/// Builds the current stream profile: `/serverinfo` (required), the addresses and, if a working
/// login is stored, Sunshine's app names. The second value says why the game list is empty, if so.
/// Used by "Show stream profile" and by "Send to Thor app" (`thor::send_profile`).
pub(crate) fn current_profile() -> Result<(profile::StreamProfile, Option<String>), String> {
    let port = sunshine::configured_port();
    let server = sunshine::fetch_serverinfo(port)?;
    let addresses = network::addresses();
    let (games, note) = match admin::app_names(sunshine::admin_port(port)) {
        Ok(games) => (games, None),
        Err(e) => (Vec::new(), Some(format!("Without the game list: {e}"))),
    };
    Ok((profile::build_profile(&server, port, &addresses, games), note))
}

/// The current stream profile as pretty JSON, for "Show stream profile".
pub fn stream_profile() -> Result<ProfileAnswer, String> {
    let (profile, note) = current_profile()?;
    Ok(ProfileAnswer { json: profile::to_json(&profile), note })
}

/// Official pages the tab links to. The UI only sends an id, so it can't open arbitrary URLs.
/// Sources: Moonlight's README (store links), tailscale.com/download/android (Play link),
/// Sunshine and Moonlight docs (see docs/PC-STREAMING-RESEARCH.md).
pub const LINKS: &[(&str, &str)] = &[
    ("sunshine_getting_started", "https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2getting__started.html"),
    ("sunshine_releases", "https://github.com/LizardByte/Sunshine/releases/latest"),
    ("sunshine_performance", "https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2performance__tuning.html"),
    ("moonlight_play", "https://play.google.com/store/apps/details?id=com.limelight"),
    ("moonlight_fdroid", "https://f-droid.org/packages/com.limelight"),
    ("moonlight_releases", "https://github.com/moonlight-stream/moonlight-android/releases"),
    ("moonlight_setup", "https://github.com/moonlight-stream/moonlight-docs/wiki/Setup-Guide"),
    ("moonlight_troubleshooting", "https://github.com/moonlight-stream/moonlight-docs/wiki/Troubleshooting"),
    (
        "moonlight_stats",
        "https://github.com/moonlight-stream/moonlight-docs/wiki/Frequently-Asked-Questions#what-do-the-numbers-mean-in-the-on-screen-connection-statistics",
    ),
    ("tailscale_android", "https://play.google.com/store/apps/details?id=com.tailscale.ipn"),
    ("tailscale_download", "https://tailscale.com/download"),
];

/// The URL for a link id from `LINKS`.
pub fn link_url(id: &str) -> Option<&'static str> {
    LINKS.iter().find(|(key, _)| *key == id).map(|(_, url)| *url)
}

/// Opens `url` in the default browser. The program gets the URL as one argument, no shell.
pub fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) || has_control_chars(url) {
        return Err(format!("Not a web address: {url}"));
    }
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    crate::adb::cmd(program).arg(url).spawn().map(|_| ()).map_err(|e| format!("couldn't open the browser: {e}"))
}

/// True if `text` contains a control character (newline, NUL, escape, ...). Such text is never
/// passed on to Sunshine, adb or another process.
pub(crate) fn has_control_chars(text: &str) -> bool {
    text.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_chars_are_detected() {
        assert!(!has_control_chars("Steam Big Picture – \"quoted\" 'x' $HOME `id`"));
        assert!(has_control_chars("a\nb"));
        assert!(has_control_chars("a\0b"));
        assert!(has_control_chars("\u{1b}[31m"));
    }

    #[test]
    fn links_are_known_and_https() {
        assert!(LINKS.iter().all(|(_, url)| url.starts_with("https://")));
        assert_eq!(link_url("moonlight_play"), Some("https://play.google.com/store/apps/details?id=com.limelight"));
        assert_eq!(link_url("tailscale_android"), Some("https://play.google.com/store/apps/details?id=com.tailscale.ipn"));
        assert_eq!(link_url("https://evil.example"), None);
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("https://x\n--flag").is_err());
    }

    /// Builds the real stream profile on this PC (Sunshine, stored login) and prints only its sizes.
    /// Nothing is sent. `cargo test local_profile_size -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn local_profile_size() {
        let (profile, note) = current_profile().expect("Sunshine must answer /serverinfo");
        let pretty = profile::to_json(&profile).len();
        let compact = profile::to_compact_json(&profile).len();
        let encoded = thor::encode_profile(&profile).map(|e| e.len());
        eprintln!("games: {}, game list note: {}", profile.games.len(), note.is_some());
        eprintln!("pretty JSON {pretty} B, compact {compact} B, base64 {encoded:?}, limit {}", thor::MAX_PROFILE_B64_CHARS);
    }

    /// Runs the real detection on this PC (Sunshine, interfaces, Tailscale CLI). Read-only.
    /// `cargo test local_ -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn local_detection_runs() {
        let status = detect();
        let sunshine = &status.sunshine;
        eprintln!("executable: {:?}", sunshine.executable);
        eprintln!("service: {:?}", sunshine.service);
        eprintln!("port: {} / admin {}", sunshine.port, sunshine.admin_port);
        match &sunshine.server {
            Some(server) => eprintln!(
                "serverinfo: hostname set: {}, gamestream {}, uniqueid length {}, busy {}, game {}",
                !server.hostname.is_empty(),
                server.gamestream_version,
                server.uniqueid.len(),
                server.busy,
                server.current_game
            ),
            None => eprintln!("serverinfo error: {:?}", sunshine.error),
        }
        let addresses = &status.addresses;
        let kinds: Vec<String> =
            addresses.lan.iter().map(|a| format!("{}:{:?}{}", a.interface, a.kind, if a.virtual_adapter { "(virtual)" } else { "" })).collect();
        eprintln!("interfaces: {kinds:?}, recommended set: {}", addresses.recommended_lan.is_some());
        let tailscale = &addresses.tailscale;
        eprintln!(
            "tailscale: cli {}, state {:?}, running {}, ipv4 set {}, dns name set {}, note {:?}",
            tailscale.cli_found,
            tailscale.backend_state,
            tailscale.running,
            tailscale.ipv4.is_some(),
            tailscale.dns_name.is_some(),
            tailscale.note
        );
    }
}
