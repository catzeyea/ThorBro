//! The PC's addresses that Moonlight on the Thor can connect to (read-only).
//!
//! - LAN: the IPv4 addresses of this PC's network interfaces (`if-addrs`), private ranges first.
//! - Tailscale: the `tailscale` CLI. Its docs (<https://tailscale.com/kb/1080/cli>, read
//!   2026-11-22) document `tailscale ip -4` ("Only return an IPv4 address") and
//!   `tailscale status --json` ("Output in JSON format (WARNING: format subject to change)").
//!   So the JSON is read defensively: every field is optional, and `tailscale ip -4` is the main
//!   source for the address. The MagicDNS name comes from `Self.DNSName` in the JSON.
//!
//! Tailscale is optional. If the CLI is missing or Tailscale isn't connected, the tab says so and
//! shows the LAN addresses only; nothing here returns an error for that.

use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// How long one `tailscale` CLI call may take.
const TAILSCALE_TIMEOUT: Duration = Duration::from_secs(5);

/// `BackendState` when Tailscale is connected.
const TAILSCALE_RUNNING: &str = "Running";

/// Interface name prefixes of virtual adapters (containers, VMs, VPN bridges). Their addresses are
/// still listed, but never recommended for Moonlight.
const VIRTUAL_INTERFACE_PREFIXES: &[&str] =
    &["docker", "br-", "veth", "virbr", "vmnet", "vboxnet", "lxc", "lxd", "podman", "cni", "flannel", "zt", "vEthernet", "VMware", "VirtualBox"];

/// What kind of address an interface has.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AddressKind {
    /// RFC 1918 private address (192.168.x.x, 10.x.x.x, 172.16-31.x.x): the normal home LAN.
    Lan,
    /// 100.64.0.0/10 (CGNAT range, used by Tailscale).
    Tailscale,
    /// Anything else (public address, ...).
    Other,
}

/// One IPv4 address of this PC.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct LocalAddress {
    pub address: String,
    pub interface: String,
    pub kind: AddressKind,
    /// Interface looks like a container / VM / bridge adapter.
    pub virtual_adapter: bool,
}

/// What we know about Tailscale on this PC.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct TailscaleInfo {
    /// The `tailscale` CLI was found.
    pub cli_found: bool,
    /// `BackendState` from `tailscale status --json` ("Running", "Stopped", "NeedsLogin", ...).
    pub backend_state: String,
    /// Connected (`BackendState` == "Running").
    pub running: bool,
    /// From `tailscale ip -4`.
    pub ipv4: Option<String>,
    /// MagicDNS name without the trailing dot, only if MagicDNS is enabled for the tailnet.
    pub dns_name: Option<String>,
    /// Problem text for the UI (CLI missing, not logged in, ...). Not an error of the tab.
    pub note: Option<String>,
}

/// All addresses for the tab.
#[derive(Serialize, Clone, Debug)]
pub struct Addresses {
    pub lan: Vec<LocalAddress>,
    /// The LAN address we suggest typing into Moonlight.
    pub recommended_lan: Option<String>,
    pub tailscale: TailscaleInfo,
    /// Why the interface list couldn't be read, if it couldn't.
    pub error: Option<String>,
}

impl Addresses {
    /// Used when the lookup itself failed.
    pub fn unavailable(why: &str) -> Self {
        Addresses { lan: Vec::new(), recommended_lan: None, tailscale: TailscaleInfo::default(), error: Some(why.into()) }
    }
}

/// Looks up everything. Never fails; problems end up in `error` / `tailscale.note`.
pub fn addresses() -> Addresses {
    let (lan, error) = match if_addrs::get_if_addrs() {
        Ok(interfaces) => {
            let raw: Vec<(String, Ipv4Addr)> = interfaces
                .into_iter()
                .filter_map(|interface| match interface.addr {
                    if_addrs::IfAddr::V4(v4) => Some((interface.name, v4.ip)),
                    if_addrs::IfAddr::V6(_) => None,
                })
                .collect();
            (classify_addresses(&raw), None)
        }
        Err(e) => (Vec::new(), Some(format!("couldn't read the network interfaces: {e}"))),
    };
    let recommended_lan = recommended_lan(&lan);
    Addresses { lan, recommended_lan, tailscale: tailscale(), error }
}

/// Classifies and sorts interface addresses: drops loopback and link-local (169.254.x.x),
/// sorts real LAN adapters first, then virtual adapters, Tailscale and others.
pub fn classify_addresses(raw: &[(String, Ipv4Addr)]) -> Vec<LocalAddress> {
    let mut found: Vec<LocalAddress> = raw
        .iter()
        .filter(|(_, ip)| !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified())
        .map(|(name, ip)| LocalAddress {
            address: ip.to_string(),
            interface: name.clone(),
            kind: address_kind(*ip),
            virtual_adapter: VIRTUAL_INTERFACE_PREFIXES.iter().any(|prefix| name.starts_with(prefix)),
        })
        .collect();
    // The same address can appear on several interfaces (aliases); list it once.
    let mut seen = std::collections::HashSet::new();
    found.retain(|address| seen.insert(address.address.clone()));
    let rank = |address: &LocalAddress| match (address.kind, address.virtual_adapter) {
        (AddressKind::Lan, false) => 0,
        (AddressKind::Lan, true) => 1,
        (AddressKind::Other, false) => 2,
        (AddressKind::Tailscale, _) => 3,
        (AddressKind::Other, true) => 4,
    };
    found.sort_by_key(rank); // stable: keeps the OS order within a rank
    found
}

/// Kind of an IPv4 address.
pub fn address_kind(ip: Ipv4Addr) -> AddressKind {
    let [a, b, ..] = ip.octets();
    if ip.is_private() {
        AddressKind::Lan
    } else if a == 100 && (64..=127).contains(&b) {
        AddressKind::Tailscale
    } else {
        AddressKind::Other
    }
}

/// The first private, non-virtual address.
pub fn recommended_lan(addresses: &[LocalAddress]) -> Option<String> {
    addresses
        .iter()
        .find(|address| address.kind == AddressKind::Lan && !address.virtual_adapter)
        .map(|address| address.address.clone())
}

// ---------------------------------------------------------------- Tailscale

/// Finds the `tailscale` CLI. PATH first. The macOS App Store path is the one the Tailscale CLI
/// docs give. The Windows path is the installer's usual folder; the docs only say "executing the
/// .exe", so this fallback is not from the docs.
fn tailscale_cli() -> Option<PathBuf> {
    if let Some(path) = crate::util::which("tailscale") {
        return Some(path);
    }
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        std::env::var_os("ProgramFiles")
            .map(|dir| vec![PathBuf::from(dir).join("Tailscale").join("tailscale.exe")])
            .unwrap_or_default()
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale")]
    } else {
        Vec::new()
    };
    candidates.into_iter().find(|path| path.is_file())
}

/// Runs the CLI with `args` (no shell) and returns stdout, `None` if it couldn't run in time.
fn run_tailscale(cli: &PathBuf, args: &[&str]) -> Option<String> {
    let mut command: Command = crate::adb::cmd(cli);
    command.args(args);
    crate::util::run_with_timeout(command, "", TAILSCALE_TIMEOUT)
}

/// Asks the Tailscale CLI for this PC's address and name.
pub fn tailscale() -> TailscaleInfo {
    let Some(cli) = tailscale_cli() else {
        return TailscaleInfo { note: Some("Tailscale CLI not found, showing LAN addresses only.".into()), ..Default::default() };
    };
    let status = run_tailscale(&cli, &["status", "--json"]).map(|json| parse_status_json(&json)).unwrap_or_default();
    let ipv4 = run_tailscale(&cli, &["ip", "-4"]).and_then(|output| parse_ip_output(&output));
    let mut info = TailscaleInfo {
        cli_found: true,
        running: status.backend_state == TAILSCALE_RUNNING,
        backend_state: status.backend_state,
        // The JSON's own address list is the fallback if `tailscale ip -4` printed nothing usable.
        ipv4: ipv4.or(status.ipv4),
        dns_name: status.dns_name,
        note: None,
    };
    if !info.running {
        let state = if info.backend_state.is_empty() { "unknown".to_string() } else { info.backend_state.clone() };
        info.note = Some(format!("Tailscale is installed but not connected (state: {state})."));
    } else if info.ipv4.is_none() {
        info.note = Some("Tailscale is connected but reported no IPv4 address.".into());
    }
    info
}

/// First valid IPv4 address in the output of `tailscale ip -4` (one address per line).
pub fn parse_ip_output(output: &str) -> Option<String> {
    output.lines().filter_map(|line| line.trim().parse::<Ipv4Addr>().ok()).map(|ip| ip.to_string()).next()
}

/// The parts of `tailscale status --json` we use.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct StatusSummary {
    pub backend_state: String,
    pub dns_name: Option<String>,
    pub ipv4: Option<String>,
}

/// Only the fields we read; everything is optional because the format may change.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct StatusJson {
    backend_state: String,
    #[serde(rename = "Self")]
    self_node: Option<SelfJson>,
    current_tailnet: Option<TailnetJson>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct SelfJson {
    #[serde(rename = "DNSName")]
    dns_name: String,
    #[serde(rename = "TailscaleIPs")]
    tailscale_ips: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct TailnetJson {
    #[serde(rename = "MagicDNSEnabled")]
    magic_dns_enabled: Option<bool>,
}

/// Parses `tailscale status --json`. Broken JSON gives an empty summary.
pub fn parse_status_json(json: &str) -> StatusSummary {
    let Ok(status) = serde_json::from_str::<StatusJson>(json) else {
        return StatusSummary::default();
    };
    let self_node = status.self_node.unwrap_or_default();
    // An explicit "MagicDNSEnabled": false hides the name; a missing field (older clients) doesn't.
    let magic_dns = status.current_tailnet.and_then(|tailnet| tailnet.magic_dns_enabled).unwrap_or(true);
    let name = self_node.dns_name.trim().trim_end_matches('.').to_string();
    StatusSummary {
        backend_state: status.backend_state,
        dns_name: (magic_dns && !name.is_empty()).then_some(name),
        ipv4: self_node.tailscale_ips.iter().find_map(|ip| ip.parse::<Ipv4Addr>().ok()).map(|ip| ip.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(list: &[(&str, &str)]) -> Vec<(String, Ipv4Addr)> {
        list.iter().map(|(name, ip)| (name.to_string(), ip.parse().unwrap())).collect()
    }

    #[test]
    fn lan_addresses_are_classified_and_sorted() {
        let found = classify_addresses(&raw(&[
            ("lo", "127.0.0.1"),
            ("tailscale0", "100.101.102.103"),
            ("docker0", "172.17.0.1"),
            ("enp5s0", "192.168.1.20"),
            ("wlan0", "169.254.3.4"),
            ("eth9", "203.0.113.5"),
        ]));
        let summary: Vec<(&str, AddressKind, bool)> =
            found.iter().map(|a| (a.address.as_str(), a.kind, a.virtual_adapter)).collect();
        assert_eq!(
            summary,
            [
                ("192.168.1.20", AddressKind::Lan, false),
                ("172.17.0.1", AddressKind::Lan, true),
                ("203.0.113.5", AddressKind::Other, false),
                ("100.101.102.103", AddressKind::Tailscale, false),
            ]
        );
        assert_eq!(recommended_lan(&found).as_deref(), Some("192.168.1.20"));
        assert_eq!(recommended_lan(&found[1..2]), None, "a docker bridge is never recommended");
    }

    #[test]
    fn windows_interface_names() {
        let found = classify_addresses(&raw(&[("vEthernet (WSL)", "172.28.80.1"), ("Ethernet", "10.0.0.7")]));
        assert_eq!(found[0].address, "10.0.0.7");
        assert!(found[1].virtual_adapter);
    }

    #[test]
    fn cgnat_range_edges() {
        assert_eq!(address_kind("100.64.0.1".parse().unwrap()), AddressKind::Tailscale);
        assert_eq!(address_kind("100.127.255.254".parse().unwrap()), AddressKind::Tailscale);
        assert_eq!(address_kind("100.128.0.1".parse().unwrap()), AddressKind::Other);
        assert_eq!(address_kind("100.63.0.1".parse().unwrap()), AddressKind::Other);
    }

    #[test]
    fn tailscale_ip_output() {
        assert_eq!(parse_ip_output("100.101.102.103\n").as_deref(), Some("100.101.102.103"));
        assert_eq!(parse_ip_output("\r\n100.64.0.9\r\n").as_deref(), Some("100.64.0.9"));
        assert_eq!(parse_ip_output("fd7a:115c:a1e0::1\n100.64.0.2\n").as_deref(), Some("100.64.0.2"));
        assert_eq!(parse_ip_output(""), None);
        assert_eq!(parse_ip_output("Tailscale is stopped.\n"), None);
    }

    #[test]
    fn tailscale_status_json_running() {
        let json = r#"{
          "Version": "1.104.7", "BackendState": "Running", "TailscaleIPs": ["100.101.102.103"],
          "Self": { "HostName": "my-pc", "DNSName": "my-pc.tail1234.ts.net.",
                    "TailscaleIPs": ["fd7a:115c:a1e0::1", "100.101.102.103"], "Online": true },
          "CurrentTailnet": { "Name": "x", "MagicDNSSuffix": "tail1234.ts.net", "MagicDNSEnabled": true },
          "Peer": {}
        }"#;
        assert_eq!(
            parse_status_json(json),
            StatusSummary {
                backend_state: "Running".into(),
                dns_name: Some("my-pc.tail1234.ts.net".into()),
                ipv4: Some("100.101.102.103".into()),
            }
        );
    }

    #[test]
    fn tailscale_status_json_variants() {
        let no_magic = r#"{"BackendState":"Running","Self":{"DNSName":"pc.ts.net."},"CurrentTailnet":{"MagicDNSEnabled":false}}"#;
        assert_eq!(parse_status_json(no_magic).dns_name, None);
        let old_client = r#"{"BackendState":"Running","Self":{"DNSName":"pc.ts.net."}}"#;
        assert_eq!(parse_status_json(old_client).dns_name.as_deref(), Some("pc.ts.net"));
        let stopped = r#"{"BackendState":"Stopped","Self":null}"#;
        assert_eq!(parse_status_json(stopped), StatusSummary { backend_state: "Stopped".into(), ..Default::default() });
        assert_eq!(parse_status_json("not json"), StatusSummary::default());
        assert_eq!(parse_status_json(""), StatusSummary::default());
    }
}
