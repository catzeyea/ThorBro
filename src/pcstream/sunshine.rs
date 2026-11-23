//! Detecting Sunshine on this PC (read-only).
//!
//! 1. Is it installed and is its service running? Linux: the systemd *user* unit
//!    `app-dev.lizardbyte.app.Sunshine`; Windows: the service `SunshineService`
//!    (research doc, section 3).
//! 2. Does it answer? `GET http://127.0.0.1:<port>/serverinfo` is unauthenticated and is the same
//!    request Moonlight sends (research doc, section 4). It returns `uniqueid`, which Moonlight uses
//!    as the host UUID.
//!
//! A Sunshine started by hand (no service) still counts as reachable: the service state is only
//! informational, `/serverinfo` is what matters.

use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// Default GameStream HTTP port (`port` in sunshine.conf). The admin API is on port + 1.
pub const DEFAULT_PORT: u16 = 47989;

/// Offset of the web UI / admin API port from `port` (`confighttp::PORT_HTTPS = 1`).
pub const ADMIN_PORT_OFFSET: u16 = 1;

/// Valid range for `port` per Sunshine's configuration docs.
const PORT_RANGE: std::ops::RangeInclusive<u16> = 1029..=65514;

/// Name of the systemd user unit on Linux (Sunshine docs, "Getting Started").
pub const LINUX_UNIT: &str = "app-dev.lizardbyte.app.Sunshine";

/// Name of the Windows service installed by the MSI.
pub const WINDOWS_SERVICE: &str = "SunshineService";

/// How long `systemctl` / `sc` may take.
const SERVICE_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `/serverinfo` may take. It is local, so anything slow means "not answering".
const SERVERINFO_TIMEOUT: Duration = Duration::from_secs(3);

/// `state` values Sunshine sends in `/serverinfo`.
const STATE_BUSY: &str = "SUNSHINE_SERVER_BUSY";

/// Service state as far as the OS tells us.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Running,
    /// Installed (unit / service exists) but not running.
    Stopped,
    /// Starting or stopping right now.
    Changing,
    /// No unit / service with that name.
    NotInstalled,
    /// We couldn't ask (no systemctl, macOS, timeout, ...).
    Unknown,
}

/// Result of the service check.
#[derive(Serialize, Clone, Debug)]
pub struct Service {
    pub state: ServiceState,
    /// What the OS reported, for the UI ("active", "4 RUNNING", ...).
    pub detail: String,
}

/// The fields of `/serverinfo` the tab uses.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct ServerInfo {
    /// `hostname`: the name Moonlight shows (`sunshine_name` in sunshine.conf).
    pub hostname: String,
    /// `appversion`: the GameStream protocol version Sunshine pretends to be (e.g. `7.1.431.-1`).
    /// This is *not* Sunshine's own version; that comes from the admin API (`/api/config`).
    pub gamestream_version: String,
    /// `uniqueid`: the host UUID Moonlight stores and the trampoline's `UUID` extra.
    pub uniqueid: String,
    /// `HttpsPort`.
    pub https_port: Option<u16>,
    /// `currentgame`: app id of the running stream, 0 = nothing.
    pub current_game: u64,
    /// `state` == SUNSHINE_SERVER_BUSY.
    pub busy: bool,
}

/// Everything about Sunshine on this PC.
#[derive(Serialize, Clone, Debug)]
pub struct SunshineStatus {
    /// Path of the `sunshine` executable if found (PATH or the default install folder).
    pub executable: Option<String>,
    pub service: Service,
    /// GameStream HTTP port in use (from sunshine.conf, else the default).
    pub port: u16,
    /// Admin API / web UI port (port + 1).
    pub admin_port: u16,
    /// Answer of `/serverinfo`, if Sunshine answered.
    pub server: Option<ServerInfo>,
    /// Why `/serverinfo` failed, if it did.
    pub error: Option<String>,
}

/// Runs all checks.
pub fn status() -> SunshineStatus {
    let port = configured_port();
    let (server, error) = match fetch_serverinfo(port) {
        Ok(info) => (Some(info), None),
        Err(e) => (None, Some(e)),
    };
    SunshineStatus {
        executable: executable().map(|path| path.display().to_string()),
        service: service(),
        port,
        admin_port: admin_port(port),
        server,
        error,
    }
}

/// Admin API port for a GameStream port.
pub fn admin_port(port: u16) -> u16 {
    port.saturating_add(ADMIN_PORT_OFFSET)
}

// ---------------------------------------------------------------- Executable and config

/// Finds the `sunshine` program: PATH first, then the MSI's default folder on Windows.
pub fn executable() -> Option<PathBuf> {
    if let Some(path) = crate::util::which("sunshine") {
        return Some(path);
    }
    if cfg!(windows) {
        let program_files = std::env::var_os("ProgramFiles")?;
        let candidate = PathBuf::from(program_files).join("Sunshine").join("sunshine.exe");
        return candidate.is_file().then_some(candidate);
    }
    None
}

/// Where sunshine.conf lives (Sunshine docs, configuration.md "Configuration file").
fn config_file() -> Option<PathBuf> {
    if cfg!(windows) {
        let program_files = std::env::var_os("ProgramFiles")?;
        Some(PathBuf::from(program_files).join("Sunshine").join("config").join("sunshine.conf"))
    } else {
        Some(dirs::config_dir()?.join("sunshine").join("sunshine.conf"))
    }
}

/// The `port` from sunshine.conf, or the default if the file or the key is missing / invalid.
pub fn configured_port() -> u16 {
    config_file()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| parse_port(&text))
        .unwrap_or(DEFAULT_PORT)
}

/// Reads `port = <number>` from the text of sunshine.conf. Comments (`#`) and other keys are
/// ignored; a value outside Sunshine's allowed range counts as missing.
pub fn parse_port(conf: &str) -> Option<u16> {
    // Searched from the end: if the key appears twice, the last one wins.
    conf.lines()
        .rev()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| key.trim() == "port")
        .filter_map(|(_, value)| value.trim().parse::<u16>().ok())
        .find(|port| PORT_RANGE.contains(port))
}

// ---------------------------------------------------------------- Service

/// Asks the OS about the Sunshine service.
pub fn service() -> Service {
    if cfg!(windows) {
        windows_service()
    } else if cfg!(target_os = "linux") {
        linux_service()
    } else {
        Service { state: ServiceState::Unknown, detail: "No service check on this OS".into() }
    }
}

/// Runs a program (no shell) and returns its stdout, or `None` if it couldn't run in time.
fn query(program: &str, args: &[&str]) -> Option<String> {
    let mut command: Command = crate::adb::cmd(program);
    command.args(args);
    crate::util::run_with_timeout(command, "", SERVICE_QUERY_TIMEOUT)
}

fn linux_service() -> Service {
    // `LoadState` tells whether the unit exists at all ("loaded" vs "not-found"), `ActiveState`
    // whether it runs. `show` exits 0 for unknown units too, so its output is what counts.
    let Some(output) = query("systemctl", &["--user", "show", "--property=LoadState,ActiveState", LINUX_UNIT]) else {
        return Service { state: ServiceState::Unknown, detail: "systemctl --user did not answer".into() };
    };
    parse_systemctl_show(&output)
}

/// Parses `systemctl --user show --property=LoadState,ActiveState <unit>`.
pub fn parse_systemctl_show(output: &str) -> Service {
    let value = |key: &str| {
        output
            .lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    let (load, active) = (value("LoadState"), value("ActiveState"));
    let state = match (load.as_str(), active.as_str()) {
        ("not-found", _) => ServiceState::NotInstalled,
        ("", "") => ServiceState::Unknown,
        (_, "active" | "reloading") => ServiceState::Running,
        (_, "activating" | "deactivating") => ServiceState::Changing,
        (_, "inactive" | "failed") => ServiceState::Stopped,
        _ => ServiceState::Unknown,
    };
    Service { state, detail: if active.is_empty() { load } else { active } }
}

fn windows_service() -> Service {
    // `sc query` needs no admin rights for reading the state.
    let Some(output) = query("sc", &["query", WINDOWS_SERVICE]) else {
        return Service { state: ServiceState::Unknown, detail: "sc query did not answer".into() };
    };
    parse_sc_query(&output)
}

/// Parses `sc query SunshineService`. The numeric state code is used because the words after it
/// may be localized. Error 1060 = "The specified service does not exist".
pub fn parse_sc_query(output: &str) -> Service {
    if output.contains("1060") && !output.contains("STATE") {
        return Service { state: ServiceState::NotInstalled, detail: "service not installed".into() };
    }
    let state_line = output.lines().find(|line| line.trim_start().starts_with("STATE"));
    let Some(after_colon) = state_line.and_then(|line| line.split_once(':')).map(|(_, rest)| rest.trim()) else {
        return Service { state: ServiceState::Unknown, detail: output.trim().lines().last().unwrap_or("").trim().to_string() };
    };
    let code = after_colon.split_whitespace().next().and_then(|code| code.parse::<u32>().ok());
    let state = match code {
        Some(4) => ServiceState::Running,          // RUNNING
        Some(1) => ServiceState::Stopped,          // STOPPED
        Some(2 | 3 | 5 | 6) => ServiceState::Changing, // START/STOP/CONTINUE/PAUSE_PENDING
        Some(7) => ServiceState::Stopped,          // PAUSED
        _ => ServiceState::Unknown,
    };
    Service { state, detail: after_colon.split_whitespace().collect::<Vec<_>>().join(" ") }
}

// ---------------------------------------------------------------- /serverinfo

/// A plain-HTTP agent for local requests: short timeout, no proxy from the environment
/// (`HTTP_PROXY` must never see a 127.0.0.1 request), HTTP errors returned as responses.
pub(crate) fn local_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .proxy(None)
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into()
}

/// `GET http://127.0.0.1:<port>/serverinfo`.
pub fn fetch_serverinfo(port: u16) -> Result<ServerInfo, String> {
    let url = format!("http://127.0.0.1:{port}/serverinfo");
    let mut response = local_agent(SERVERINFO_TIMEOUT)
        .get(&url)
        .call()
        .map_err(|e| format!("Sunshine is not answering on port {port} ({e})"))?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|e| format!("couldn't read /serverinfo: {e}"))?;
    if status != 200 {
        return Err(format!("/serverinfo answered with HTTP {status}"));
    }
    parse_serverinfo(&body)
}

/// Parses the XML of `/serverinfo` (Sunshine's `nvhttp::serverinfo`):
///
/// ```xml
/// <root status_code="200"><hostname>pc</hostname><appversion>7.1.431.-1</appversion>
///   <uniqueid>…</uniqueid><HttpsPort>47984</HttpsPort><currentgame>0</currentgame>
///   <state>SUNSHINE_SERVER_FREE</state></root>
/// ```
///
/// `uniqueid` is required (Moonlight treats it as mandatory too); everything else is optional.
pub fn parse_serverinfo(xml: &str) -> Result<ServerInfo, String> {
    let document = roxmltree::Document::parse(xml).map_err(|e| format!("/serverinfo is not valid XML: {e}"))?;
    let root = document.root_element();
    if root.tag_name().name() != "root" {
        return Err("/serverinfo has an unexpected format".into());
    }
    if let Some(code) = root.attribute("status_code").filter(|code| *code != "200") {
        let message = root.attribute("status_message").unwrap_or("");
        return Err(format!("/serverinfo reported status {code} {message}").trim().to_string());
    }
    let text = |name: &str| -> String {
        root.children()
            .find(|node| node.is_element() && node.tag_name().name() == name)
            .and_then(|node| node.text())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let uniqueid = text("uniqueid");
    if uniqueid.is_empty() {
        return Err("/serverinfo has no uniqueid".into());
    }
    Ok(ServerInfo {
        hostname: text("hostname"),
        gamestream_version: text("appversion"),
        uniqueid,
        https_port: text("HttpsPort").parse().ok(),
        current_game: text("currentgame").parse().unwrap_or(0),
        busy: text("state") == STATE_BUSY,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape of a real answer of Sunshine (Linux, 2026-11-22) with the ids replaced.
    const SERVERINFO: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<root status_code="200"><hostname>my-pc</hostname><appversion>7.1.431.-1</appversion><GfeVersion>3.23.0.74</GfeVersion><uniqueid>0123ABCD-4567-89EF-0123-456789ABCDEF</uniqueid><HttpsPort>47984</HttpsPort><ExternalPort>47989</ExternalPort><MaxLumaPixelsHEVC>1869449984</MaxLumaPixelsHEVC><mac>00:00:00:00:00:00</mac><LocalIP>127.0.0.1</LocalIP><ServerCodecModeSupport>197377</ServerCodecModeSupport><PairStatus>0</PairStatus><currentgame>881448767</currentgame><state>SUNSHINE_SERVER_BUSY</state></root>"#;

    #[test]
    fn parses_a_real_serverinfo() {
        let info = parse_serverinfo(SERVERINFO).unwrap();
        assert_eq!(
            info,
            ServerInfo {
                hostname: "my-pc".into(),
                gamestream_version: "7.1.431.-1".into(),
                uniqueid: "0123ABCD-4567-89EF-0123-456789ABCDEF".into(),
                https_port: Some(47984),
                current_game: 881448767,
                busy: true,
            }
        );
    }

    #[test]
    fn idle_server_and_missing_optional_fields() {
        let xml = "<root status_code=\"200\"><uniqueid> abc </uniqueid><state>SUNSHINE_SERVER_FREE</state></root>";
        let info = parse_serverinfo(xml).unwrap();
        assert_eq!(info.uniqueid, "abc");
        assert!(!info.busy);
        assert_eq!(info.current_game, 0);
        assert_eq!(info.https_port, None);
        assert_eq!(info.hostname, "");
    }

    #[test]
    fn hostname_with_xml_entities() {
        let xml = "<root status_code=\"200\"><hostname>Tom &amp; Jerry&apos;s PC</hostname><uniqueid>x</uniqueid></root>";
        assert_eq!(parse_serverinfo(xml).unwrap().hostname, "Tom & Jerry's PC");
    }

    #[test]
    fn rejects_broken_or_unexpected_serverinfo() {
        assert!(parse_serverinfo("").is_err());
        assert!(parse_serverinfo("<html>nope</html>").is_err());
        assert!(parse_serverinfo("<root status_code=\"200\"><hostname>x</hostname></root>").unwrap_err().contains("uniqueid"));
        let error = parse_serverinfo("<root status_code=\"503\" status_message=\"busy\"><uniqueid>x</uniqueid></root>").unwrap_err();
        assert!(error.contains("503") && error.contains("busy"), "{error}");
    }

    #[test]
    fn port_from_sunshine_conf() {
        assert_eq!(parse_port(""), None);
        assert_eq!(parse_port("sunshine_name = pc\nport = 48000\n"), Some(48000));
        assert_eq!(parse_port("port=50000 # custom"), Some(50000));
        assert_eq!(parse_port("# port = 48000\n"), None);
        assert_eq!(parse_port("port = 80\n"), None, "outside Sunshine's range");
        assert_eq!(parse_port("port = abc\n"), None);
        assert_eq!(parse_port("upnp_port = 1\nport = 48100\nport = 48200"), Some(48200), "last one wins");
        assert_eq!(admin_port(DEFAULT_PORT), 47990);
    }

    #[test]
    fn systemctl_show_states() {
        let parse = |text: &str| parse_systemctl_show(text).state;
        assert_eq!(parse("LoadState=loaded\nActiveState=active\n"), ServiceState::Running);
        assert_eq!(parse("LoadState=loaded\nActiveState=inactive\n"), ServiceState::Stopped);
        assert_eq!(parse("LoadState=loaded\nActiveState=failed\n"), ServiceState::Stopped);
        assert_eq!(parse("LoadState=loaded\nActiveState=activating\n"), ServiceState::Changing);
        assert_eq!(parse("LoadState=not-found\nActiveState=inactive\n"), ServiceState::NotInstalled);
        assert_eq!(parse(""), ServiceState::Unknown);
        assert_eq!(parse_systemctl_show("LoadState=loaded\nActiveState=active").detail, "active");
    }

    #[test]
    fn sc_query_states() {
        let running = "\r\nSERVICE_NAME: SunshineService\r\n        TYPE               : 10  WIN32_OWN_PROCESS\r\n        STATE              : 4  RUNNING\r\n                                (STOPPABLE, NOT_PAUSABLE, ACCEPTS_SHUTDOWN)\r\n        WIN32_EXIT_CODE    : 0  (0x0)\r\n";
        assert_eq!(parse_sc_query(running).state, ServiceState::Running);
        assert_eq!(parse_sc_query(running).detail, "4 RUNNING");
        let stopped = "SERVICE_NAME: SunshineService\n        STATE              : 1  STOPPED\n";
        assert_eq!(parse_sc_query(stopped).state, ServiceState::Stopped);
        // German Windows: the words differ, the code doesn't.
        let pending = "SERVICE_NAME: SunshineService\n        STATE              : 2  START_AUSSTEHEND\n";
        assert_eq!(parse_sc_query(pending).state, ServiceState::Changing);
        let missing = "[SC] EnumQueryServicesStatus:OpenService FEHLER 1060:\r\n\r\nThe specified service does not exist as an installed service.\r\n";
        assert_eq!(parse_sc_query(missing).state, ServiceState::NotInstalled);
        assert_eq!(parse_sc_query("garbage").state, ServiceState::Unknown);
    }
}
