//! Live status of the Thor for the Start page (DESIGN-UPDATE-7): battery, internal storage, SD
//! card, CPU counters, RAM and CPU temperature.
//!
//! Everything is read in ONE `adb shell` call (about 0.1 s on the Thor) that only uses the
//! shell's built-ins plus `dumpsys battery` and `df`. Each part of the output starts with an
//! `@@<name>` marker line, so a part that fails or is missing only costs that one value.
//!
//! Rule: a value that can't be read or doesn't make sense is `None` and is not shown by the UI.
//! There are no defaults or placeholder numbers here.
//!
//! The CPU load needs two samples: this module only returns the cumulative `/proc/stat`
//! counters; the UI (ui/js/status.js) turns two of them into a percentage.

use serde::Serialize;

use crate::adb;

/// The shell script, sent to `adb shell sh` via stdin (see `adb::sh_script`).
///
/// - Temperatures: only the CPU zones `cpu-0-*` / `cpu-1-*`, read with the built-in `read`
///   (a `cat` per zone takes over a second with ~100 zones). Zones whose `temp` can't be read
///   ("Invalid argument") are skipped.
/// - Volumes: every entry in `/storage` except `emulated` and `self`; the parser keeps only
///   names that look like SD card ids and whose `df` really reports that mount point.
const SCRIPT: &str = r#"
echo @@battery; dumpsys battery
echo @@internal; df -k /sdcard
for d in /storage/*; do n=${d##*/}; case $n in emulated|self) continue;; esac; echo "@@volume $n"; df -k "$d"; done
echo @@cpu; read l < /proc/stat; echo "$l"
echo @@mem; while read k v u; do case $k in MemTotal:|MemAvailable:) echo "$k $v";; esac; done < /proc/meminfo
echo @@temp; for z in /sys/class/thermal/thermal_zone*; do read t < $z/type || continue; case $t in cpu-0-*|cpu-1-*) read v < $z/temp 2>/dev/null && echo "$t $v";; esac; done
echo @@end
"#;

/// Battery as `dumpsys battery` reports it.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Battery {
    /// 0–100.
    pub percent: u8,
    /// Android's `BatteryManager.BATTERY_STATUS_*`: 1 unknown, 2 charging, 3 discharging,
    /// 4 not charging, 5 full. Passed on raw; the UI decides what counts as "charging".
    pub status: Option<u32>,
}

/// Size of a storage in bytes.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Space {
    pub total: u64,
    pub free: u64,
}

/// Cumulative CPU time from the first line of `/proc/stat`, in clock ticks.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct CpuTicks {
    /// Everything except idle and iowait.
    pub busy: u64,
    pub total: u64,
}

/// RAM in bytes. "Used" is `total - available` (what apps can't get without swapping).
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Ram {
    pub total: u64,
    pub available: u64,
}

/// One status reading. Every field is `None` when it couldn't be read.
#[derive(Serialize, Debug, PartialEq, Default)]
pub struct ThorStatus {
    pub battery: Option<Battery>,
    pub internal: Option<Space>,
    /// The first SD card that is mounted. `None` also means "no SD card".
    pub sd: Option<Space>,
    pub cpu: Option<CpuTicks>,
    pub ram: Option<Ram>,
    /// Hottest CPU thermal zone in °C.
    pub cpu_temp_c: Option<f64>,
}

/// Reads the status from the connected Thor (one adb call).
pub fn read() -> Result<ThorStatus, String> {
    let out = adb::sh_script(SCRIPT);
    if !out.stdout.contains("@@end") {
        let err = out.err_text();
        return Err(if err.is_empty() { "The Thor didn't answer the status request".into() } else { err });
    }
    Ok(parse(&out.stdout))
}

/// Splits the script output into its `@@` parts: (marker line without "@@", body lines).
fn sections(output: &str) -> Vec<(&str, Vec<&str>)> {
    let mut parts: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(name) = line.strip_prefix("@@") {
            parts.push((name.trim(), Vec::new()));
        } else if let Some((_, body)) = parts.last_mut() {
            body.push(line);
        }
    }
    parts
}

/// Turns the script output into a `ThorStatus`. Pure, so it can be tested with captured output.
pub fn parse(output: &str) -> ThorStatus {
    let mut status = ThorStatus::default();
    for (name, body) in sections(output) {
        match name {
            "battery" => status.battery = parse_battery(&body),
            "internal" => status.internal = parse_df(&body, None),
            "cpu" => status.cpu = body.first().and_then(|line| parse_cpu(line)),
            "mem" => status.ram = parse_mem(&body),
            "temp" => status.cpu_temp_c = parse_temps(&body),
            _ => {
                if let Some(id) = name.strip_prefix("volume ") {
                    if status.sd.is_none() && adb::is_volume_id(id) {
                        status.sd = parse_df(&body, Some(&format!("/storage/{id}")));
                    }
                }
            }
        }
    }
    status
}

/// `level:` / `scale:` / `status:` / `present:` lines of `dumpsys battery`.
fn parse_battery(lines: &[&str]) -> Option<Battery> {
    let field = |key: &str| {
        lines.iter().find_map(|line| {
            let (k, v) = line.trim().split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    };
    if field("present").as_deref() == Some("false") {
        return None;
    }
    let level: u32 = field("level")?.parse().ok()?;
    let scale: u32 = field("scale").and_then(|s| s.parse().ok()).unwrap_or(100);
    if scale == 0 || level > scale {
        return None;
    }
    let percent = ((level as f64 / scale as f64) * 100.0).round() as u8;
    Some(Battery { percent, status: field("status").and_then(|s| s.parse().ok()) })
}

/// One `df -k` data row. With `mount`, the row must report exactly that mount point (an SD
/// card folder that isn't mounted would otherwise show the size of the file system above it).
fn parse_df(lines: &[&str], mount: Option<&str>) -> Option<Space> {
    let row = lines.iter().find(|line| !line.trim().is_empty() && !line.starts_with("Filesystem"))?;
    let cols: Vec<&str> = row.split_whitespace().collect();
    if cols.len() < 6 {
        return None;
    }
    if let Some(mount) = mount {
        if cols[5] != mount {
            return None;
        }
    }
    let total_kb: u64 = cols[1].parse().ok()?;
    let free_kb: u64 = cols[3].parse().ok()?;
    if total_kb == 0 || free_kb > total_kb {
        return None;
    }
    Some(Space { total: total_kb * 1024, free: free_kb * 1024 })
}

/// `cpu  user nice system idle iowait irq softirq steal ...` (the summary line of /proc/stat).
fn parse_cpu(line: &str) -> Option<CpuTicks> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = parts.map(|v| v.parse().ok()).collect::<Option<_>>()?;
    if values.len() < 5 {
        return None;
    }
    // user nice system idle iowait irq softirq steal (guest time is already inside user/nice).
    let total: u64 = values.iter().take(8).sum();
    let idle = values[3] + values[4];
    (total > 0).then(|| CpuTicks { busy: total - idle, total })
}

/// `MemTotal: <kB>` and `MemAvailable: <kB>` (as printed by the script).
fn parse_mem(lines: &[&str]) -> Option<Ram> {
    let value = |key: &str| {
        lines.iter().find_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next()? == key).then(|| parts.next()?.parse::<u64>().ok()).flatten()
        })
    };
    let total = value("MemTotal:")?;
    let available = value("MemAvailable:")?;
    (total > 0 && available <= total).then(|| Ram { total: total * 1024, available: available * 1024 })
}

/// Hottest of the `<zone type> <millidegrees>` lines. Values outside 0–150 °C are sensor
/// errors and are ignored.
fn parse_temps(lines: &[&str]) -> Option<f64> {
    lines
        .iter()
        .filter_map(|line| line.split_whitespace().nth(1)?.parse::<i64>().ok())
        .map(|milli| milli as f64 / 1000.0)
        .filter(|c| (0.0..=150.0).contains(c))
        .reduce(f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a real AYN Thor (Android 13, SD card inserted, USB cable plugged in).
    const THOR_OUTPUT: &str = "@@battery
Current Battery Service state:
  AC powered: true
  USB powered: false
  Wireless powered: false
  Max charging current: 3000000
  Max charging voltage: 5000000
  Charge counter: 5072565
  status: 3
  health: 2
  present: true
  level: 80
  scale: 100
  voltage: 4214
  temperature: 230
  technology: Li-ion
@@internal
Filesystem     1K-blocks      Used Available Use% Mounted on
/dev/fuse      487651696 199189000 288331624  41% /storage/emulated
@@volume EEFF-BF6C
Filesystem     1K-blocks     Used Available Use% Mounted on
/dev/fuse      124862464 99838720  25023744  80% /storage/EEFF-BF6C
@@cpu
cpu  127293 5444 132784 10342718 1497 30741 4052 486 0 0
@@mem
MemTotal: 15621188
MemAvailable: 10491360
@@temp
cpu-1-0 30700
cpu-1-1 30700
cpu-1-2 31900
cpu-0-0 34000
cpu-0-1 34400
cpu-0-2 34000
@@end
";

    #[test]
    fn parses_real_thor_output() {
        let status = parse(THOR_OUTPUT);
        assert_eq!(status.battery, Some(Battery { percent: 80, status: Some(3) }));
        assert_eq!(status.internal, Some(Space { total: 487651696 * 1024, free: 288331624 * 1024 }));
        assert_eq!(status.sd, Some(Space { total: 124862464 * 1024, free: 25023744 * 1024 }));
        let idle = 10342718 + 1497;
        let total = 127293 + 5444 + 132784 + 10342718 + 1497 + 30741 + 4052 + 486;
        assert_eq!(status.cpu, Some(CpuTicks { busy: total - idle, total }));
        assert_eq!(status.ram, Some(Ram { total: 15621188 * 1024, available: 10491360 * 1024 }));
        assert_eq!(status.cpu_temp_c, Some(34.4));
    }

    /// ui/js/status.js and the UI test stub read exactly these JSON names.
    #[test]
    fn json_field_names_match_the_ui() {
        let json = serde_json::to_value(parse(THOR_OUTPUT)).unwrap();
        let keys = |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>().join(",");
        assert_eq!(keys(&json), "battery,cpu,cpu_temp_c,internal,ram,sd");
        assert_eq!(keys(&json["battery"]), "percent,status");
        assert_eq!(keys(&json["internal"]), "free,total");
        assert_eq!(keys(&json["cpu"]), "busy,total");
        assert_eq!(keys(&json["ram"]), "available,total");
    }

    #[test]
    fn no_sd_card_means_no_sd_value() {
        let without_sd: String = THOR_OUTPUT
            .lines()
            .filter(|l| !l.contains("EEFF-BF6C") && !l.contains("124862464"))
            .map(|l| format!("{l}\n"))
            .collect();
        let status = parse(&without_sd);
        assert_eq!(status.sd, None);
        assert!(status.internal.is_some() && status.ram.is_some());
    }

    #[test]
    fn unmounted_or_odd_volumes_are_not_sd_cards() {
        // The folder exists but df reports the file system above it: not mounted.
        let unmounted = "@@volume 1A2B-3C4D\nFilesystem 1K-blocks Used Available Use% Mounted on\ntmpfs 5695520 0 5695520 0% /storage\n@@end\n";
        assert_eq!(parse(unmounted).sd, None);
        // Not an SD card id.
        let odd = "@@volume something\nFilesystem 1K-blocks Used Available Use% Mounted on\n/dev/fuse 100 50 50 50% /storage/something\n@@end\n";
        assert_eq!(parse(odd).sd, None);
    }

    #[test]
    fn unreadable_parts_stay_empty() {
        let broken = "@@battery\nCan't find service: battery\n@@internal\ndf: /sdcard: Permission denied\n@@cpu\n\n@@mem\n@@temp\n@@end\n";
        assert_eq!(parse(broken), ThorStatus::default());
        assert_eq!(parse(""), ThorStatus::default());
    }

    #[test]
    fn battery_edge_cases() {
        assert_eq!(parse_battery(&["  present: false", "  level: 50", "  scale: 100"]), None);
        assert_eq!(parse_battery(&["  level: 120", "  scale: 100"]), None);
        assert_eq!(parse_battery(&["  level: 7", "  scale: 10", "  status: 2"]), Some(Battery { percent: 70, status: Some(2) }));
        assert_eq!(parse_battery(&["  level: 50"]), Some(Battery { percent: 50, status: None }));
    }

    /// Needs a connected Thor: `cargo test device_ -- --ignored --test-threads=1`. Read-only.
    #[test]
    #[ignore]
    fn device_status_reads_every_value() {
        let status = read().expect("status read failed");
        eprintln!("THOR_STATUS_JSON {}", serde_json::to_string(&status).unwrap());
        let battery = status.battery.expect("battery");
        assert!(battery.percent <= 100 && battery.status.is_some());
        let internal = status.internal.expect("internal storage");
        assert!(internal.free <= internal.total);
        assert!(status.cpu.is_some() && status.ram.is_some() && status.cpu_temp_c.is_some());
    }

    #[test]
    fn temperatures_ignore_sensor_errors() {
        assert_eq!(parse_temps(&["cpu-0-0 -40960", "cpu-0-1 999999", "cpu-1-0 41200"]), Some(41.2));
        assert_eq!(parse_temps(&["cpu-0-0 nonsense"]), None);
        assert_eq!(parse_temps(&[]), None);
    }
}
