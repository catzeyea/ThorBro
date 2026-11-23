//! The Thor side, over adb: are Moonlight and Tailscale installed, and "Play on Thor".
//!
//! Device selection works like everywhere else in ThorBro: adb talks to "the" device (no `-s`),
//! and nothing runs unless `adb::device_status()` reports exactly one connected, authorized
//! device (see `adb.rs`).
//!
//! Package names:
//! - Moonlight: `com.limelight` (normal build); `com.limelight.root` is the root flavor. Both use
//!   the Java namespace `com.limelight`, so the trampoline class is `com.limelight.ShortcutTrampoline`
//!   (moonlight-android `app/build.gradle`, research doc section 1).
//! - Tailscale: `com.tailscale.ipn`. Confirmed 2026-11-22 from tailscale.com/download/android
//!   (Play link `id=com.tailscale.ipn`) and the `namespace` in tailscale-android's
//!   `android/build.gradle.kts`.
//!
//! "Play on Thor" starts Moonlight's exported `ShortcutTrampoline` with the host UUID and the app
//! name, the same intent Moonlight's own pinned shortcuts use (`ServerHelper.createAppShortcut`):
//! `am start -n com.limelight/.ShortcutTrampoline -a android.intent.action.VIEW --es UUID <uuid>
//! --es AppName <name>`. `--es` (string) matters: Moonlight reads every extra with
//! `getStringExtra`, so `--ei` would be ignored.

use crate::adb;
use serde::Serialize;

/// Package of the normal Moonlight build.
pub const MOONLIGHT_PACKAGE: &str = "com.limelight";
/// Package of Moonlight's root build flavor.
pub const MOONLIGHT_ROOT_PACKAGE: &str = "com.limelight.root";
/// Fully qualified trampoline class (same namespace for both flavors).
const TRAMPOLINE_CLASS: &str = "com.limelight.ShortcutTrampoline";
/// Package of the Tailscale Android app.
pub const TAILSCALE_PACKAGE: &str = "com.tailscale.ipn";

/// Intent action Moonlight's shortcuts use (`Intent.ACTION_VIEW`).
const VIEW_ACTION: &str = "android.intent.action.VIEW";

/// Longest app name we pass on (bytes). Sunshine has no documented limit; this only guards
/// against garbage.
const MAX_APP_NAME_BYTES: usize = 512;

/// An app found on the Thor.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    pub package: String,
    /// `versionName` from `dumpsys package`, if it could be read.
    pub version: Option<String>,
}

/// Answer of `thor_apps`.
#[derive(Serialize, Clone, Debug)]
pub struct ThorApps {
    pub connected: bool,
    /// Device model, or why it isn't connected (same text as the top bar).
    pub device: String,
    pub moonlight: Option<InstalledApp>,
    pub tailscale: Option<InstalledApp>,
    /// adb answered, but the package list couldn't be read.
    pub error: Option<String>,
}

/// Checks which of the apps are installed on the connected Thor.
pub fn thor_apps() -> ThorApps {
    let device = adb::device_status();
    let mut result = ThorApps { connected: device.connected, device: device.message, moonlight: None, tailscale: None, error: None };
    if !device.connected {
        return result;
    }
    let output = adb::sh("pm list packages");
    if !output.ok {
        result.error = Some(format!("couldn't list the apps on the Thor: {}", output.err_text()));
        return result;
    }
    let packages = parse_packages(&output.stdout);
    let has = |name: &str| packages.iter().any(|package| package == name);
    let found = |name: &str| InstalledApp { package: name.to_string(), version: package_version(name) };
    // Prefer the normal build if both flavors are installed.
    result.moonlight = [MOONLIGHT_PACKAGE, MOONLIGHT_ROOT_PACKAGE].into_iter().find(|name| has(name)).map(found);
    result.tailscale = has(TAILSCALE_PACKAGE).then(|| found(TAILSCALE_PACKAGE));
    result
}

/// The installed Moonlight package (normal build preferred), checked with one `pm list packages`.
pub fn moonlight_package() -> Result<String, String> {
    let output = adb::sh("pm list packages com.limelight");
    if !output.ok {
        return Err(format!("couldn't list the apps on the Thor: {}", output.err_text()));
    }
    let packages = parse_packages(&output.stdout);
    [MOONLIGHT_PACKAGE, MOONLIGHT_ROOT_PACKAGE]
        .into_iter()
        .find(|name| packages.iter().any(|package| package == name))
        .map(String::from)
        .ok_or_else(|| "Moonlight is not installed on the Thor".to_string())
}

/// Package names from `pm list packages` (lines `package:<name>`).
pub fn parse_packages(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("package:"))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// `versionName` of an installed package.
fn package_version(package: &str) -> Option<String> {
    let output = adb::sh(&format!("dumpsys package {}", adb::shell_quote(package)));
    output.ok.then(|| parse_version_name(&output.stdout)).flatten()
}

/// First `versionName=...` in `dumpsys package` output.
pub fn parse_version_name(dumpsys: &str) -> Option<String> {
    dumpsys
        .lines()
        .find_map(|line| line.trim().strip_prefix("versionName="))
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

// ---------------------------------------------------------------- Play on Thor

/// What to start on the Thor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayRequest {
    /// Moonlight package (`com.limelight` or `com.limelight.root`).
    pub package: String,
    /// Sunshine's `uniqueid` from `/serverinfo`.
    pub uuid: String,
    /// App name exactly as in Sunshine's app list.
    pub app_name: String,
    /// Experimental: `am start --display <id>` (a display id from the screen detection).
    pub display: Option<u32>,
}

/// True for the 8-4-4-4-12 hex form that Java's `UUID.fromString` accepts from Sunshine.
pub fn is_valid_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, len)| group.len() == len && group.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Checks the request before anything is sent to the Thor.
pub fn validate(request: &PlayRequest) -> Result<(), String> {
    if request.package != MOONLIGHT_PACKAGE && request.package != MOONLIGHT_ROOT_PACKAGE {
        return Err(format!("Unknown Moonlight package: {}", request.package));
    }
    if !is_valid_uuid(&request.uuid) {
        return Err("Sunshine's host ID doesn't look like a UUID; check Sunshine again".into());
    }
    let name = &request.app_name;
    if name.trim().is_empty() {
        return Err("The app name is empty".into());
    }
    if name.len() > MAX_APP_NAME_BYTES {
        return Err("The app name is too long".into());
    }
    if super::has_control_chars(name) {
        return Err("The app name contains control characters (like a line break); rename it in Sunshine".into());
    }
    Ok(())
}

/// The `am start` arguments, one element per argument (before shell quoting).
pub fn play_args(request: &PlayRequest) -> Vec<String> {
    let mut args: Vec<String> = vec!["am".into(), "start".into()];
    // `am help`: start options like `--display <DISPLAY_ID>` come before the intent arguments.
    if let Some(display) = request.display {
        args.push("--display".into());
        args.push(display.to_string());
    }
    args.extend(["-n".into(), format!("{}/{TRAMPOLINE_CLASS}", request.package), "-a".into(), VIEW_ACTION.into()]);
    args.extend(["--es".into(), "UUID".into(), request.uuid.clone(), "--es".into(), "AppName".into(), request.app_name.clone()]);
    args
}

/// The command line for the Thor's shell. `adb shell` joins its arguments with spaces and hands
/// them to `sh` on the device, so every argument is quoted for that shell (`adb::shell_quote`).
pub fn play_command(request: &PlayRequest) -> Result<String, String> {
    validate(request)?;
    Ok(play_args(request).iter().map(|arg| adb::shell_quote(arg)).collect::<Vec<_>>().join(" "))
}

/// Result of "Play on Thor".
#[derive(Serialize, Clone, Debug)]
pub struct PlayResult {
    /// `am start`'s "Starting: Intent { ... }" line.
    pub message: String,
    /// Sunshine is already streaming something: Moonlight will ask whether to quit it (unless it
    /// is the same app).
    pub pc_busy: bool,
}

/// Starts `app_name` from Sunshine in Moonlight on the Thor. The host UUID is read fresh from
/// `/serverinfo` and the Moonlight package from the Thor, so nothing the UI sends besides the name
/// and the display id ends up in the command.
pub fn play(app_name: &str, display: Option<u32>) -> Result<PlayResult, String> {
    let server = super::sunshine::fetch_serverinfo(super::sunshine::configured_port())?;
    let device = adb::device_status();
    if !device.connected {
        return Err(device.message);
    }
    let package = moonlight_package()?;
    let request = PlayRequest { package, uuid: server.uniqueid.clone(), app_name: app_name.to_string(), display };
    let command = play_command(&request)?;
    let output = adb::sh(&command);
    let message = interpret_am_output("Moonlight", output.ok, &output.stdout, &output.stderr)?;
    Ok(PlayResult { message, pc_busy: server.busy || server.current_game != 0 })
}

/// Turns the output of `am start` into a result for the UI. `app` is the app's display name for
/// the error messages ("Moonlight", "The ThorBro app").
pub fn interpret_am_output(app: &str, ok: bool, stdout: &str, stderr: &str) -> Result<String, String> {
    let all = format!("{stdout}\n{stderr}");
    let error_line = all
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("Error") || line.contains("Exception") || line.starts_with("Security"));
    if let Some(line) = error_line {
        if line.contains("does not exist") || line.contains("Error type 3") {
            return Err(format!("{app} is not installed on the Thor (or its start screen changed)"));
        }
        return Err(format!("The Thor refused to start {app}: {line}"));
    }
    if !ok {
        let text = all.trim();
        return Err(if text.is_empty() { format!("adb couldn't start {app}") } else { text.to_string() });
    }
    Ok(all.lines().map(str::trim).find(|line| line.starts_with("Starting")).unwrap_or("Started").to_string())
}

// ---------------------------------------------------------------- Send to Thor app

/// The ThorBro Android app's main activity, as `am start -n` wants it: `<applicationId>/<class>`.
/// Taken from the ThorBroAndroid repo, not guessed:
/// - `dev.catzeyea.thorbro` is `defaultConfig.applicationId` in `android/build.gradle.kts`.
/// - The manifest (`android/src/main/AndroidManifest.xml`) declares the activity as `.MainActivity`.
///   A leading dot is resolved against the Gradle `namespace` (`dev.catzeyea.thorbro.android`),
///   not against the applicationId, so the class is `dev.catzeyea.thorbro.android.MainActivity`.
///
/// The app has the same value as `MainActivity.COMPONENT_FOR_ADB`. This is the only copy on the
/// PC side; the package name is derived from it (`thor_app_package`).
pub const THOR_APP_COMPONENT: &str = "dev.catzeyea.thorbro/dev.catzeyea.thorbro.android.MainActivity";

/// Name of the string extra that carries the base64 profile (`MainActivity.EXTRA_PROFILE_B64`).
pub const PROFILE_EXTRA: &str = "profile_b64";

/// Display name of the Thor app in messages.
const THOR_APP_NAME: &str = "The ThorBro app";

/// Longest base64 profile ThorBro sends, in characters. The base64 text goes into a single
/// `adb shell` command line, which passes three limits on its way (smallest first):
/// - Windows: `CreateProcess` takes at most 32,767 characters for the whole adb command line,
///   including the adb path and the rest of the `am start` command.
/// - adb client → adb server: the request is framed with a 4-digit hex length, so at most
///   65,535 bytes for `shell:<command>`.
/// - On the Thor, `sh -c <command>`: Linux allows 131,072 bytes per argument (`MAX_ARG_STRLEN`).
///
/// 24 KiB stays clear of all three. The Thor app itself accepts up to 256 KiB of JSON. A typical
/// profile is far below this (numbers in docs/STREAM-PROFILE.md, "Size").
pub const MAX_PROFILE_B64_CHARS: usize = 24 * 1024;

/// Package part of `THOR_APP_COMPONENT` (for `pm list packages`).
pub fn thor_app_package() -> &'static str {
    THOR_APP_COMPONENT.split_once('/').map_or(THOR_APP_COMPONENT, |(package, _)| package)
}

/// Message when the Thor app is missing.
const THOR_APP_MISSING: &str = "The ThorBro app is not installed on the Thor. Install the ThorBro APK on the Thor first \
     (for example with \"adb install <path to the APK>\", or copy the APK to the Thor and open it there), then send the profile again.";

/// Checks the output of `pm list packages <package>` for the Thor app. The filter of `pm` matches
/// substrings, so the package must appear exactly.
pub fn check_thor_app(ok: bool, stdout: &str, stderr: &str) -> Result<(), String> {
    if !ok {
        let text = if stderr.trim().is_empty() { stdout } else { stderr };
        return Err(format!("couldn't list the apps on the Thor: {}", text.trim()));
    }
    if parse_packages(stdout).iter().any(|package| package == thor_app_package()) {
        Ok(())
    } else {
        Err(THOR_APP_MISSING.into())
    }
}

/// The profile as base64 of its compact JSON (standard alphabet with `=` padding, no line breaks;
/// what the Thor app's `ProfileParser.parseBase64` reads). Refuses a profile with a field that
/// looks like a credential, and one above `MAX_PROFILE_B64_CHARS`.
pub fn encode_profile(profile: &super::profile::StreamProfile) -> Result<String, String> {
    use base64::Engine;
    let json = super::profile::to_compact_json(profile);
    let value: serde_json::Value = serde_json::from_str(&json).map_err(|e| format!("couldn't build the stream profile: {e}"))?;
    if let Some(field) = super::profile::credential_field(&value) {
        return Err(format!("The stream profile contains the field \"{field}\", which looks like a credential. It was not sent."));
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());
    if encoded.len() > MAX_PROFILE_B64_CHARS {
        return Err(format!(
            "The stream profile is too large to send over adb ({} characters encoded, the limit is {MAX_PROFILE_B64_CHARS}). \
             It has {} games; remove apps you don't stream from Sunshine's app list and send it again.",
            encoded.len(),
            profile.games.len()
        ));
    }
    Ok(encoded)
}

/// The `am start` arguments for the Thor app, one element per argument (before shell quoting).
pub fn profile_args(encoded: &str) -> Vec<String> {
    ["am", "start", "-n", THOR_APP_COMPONENT, "--es", PROFILE_EXTRA, encoded].into_iter().map(String::from).collect()
}

/// The command line for the Thor's shell, quoted like "Play on Thor" (`play_command`). Base64
/// only has characters the shell leaves alone, so the quoting is a safety net, not a necessity.
pub fn profile_command(encoded: &str) -> Result<String, String> {
    if encoded.is_empty() {
        return Err("The stream profile is empty".into());
    }
    if encoded.len() > MAX_PROFILE_B64_CHARS {
        return Err(format!("The stream profile is too large to send over adb (limit {MAX_PROFILE_B64_CHARS} characters)"));
    }
    if !encoded.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')) {
        return Err("The stream profile is not valid base64".into());
    }
    Ok(profile_args(encoded).iter().map(|arg| adb::shell_quote(arg)).collect::<Vec<_>>().join(" "))
}

/// Result of "Send to Thor app". `am start` gives no answer from the app, so this only says that
/// the profile was handed to Android, not that the app accepted it.
#[derive(Serialize, Clone, Debug)]
pub struct SendProfileResult {
    /// `am start`'s "Starting: Intent { ... }" line.
    pub message: String,
    /// Number of games in the sent profile.
    pub games: usize,
    /// Length of the base64 text that was sent.
    pub encoded_chars: usize,
    /// Why the game list is empty (no working Sunshine login), if so.
    pub note: Option<String>,
}

/// Sends the current stream profile to the ThorBro app on the Thor:
/// `am start -n <THOR_APP_COMPONENT> --es profile_b64 <base64>`. Needs no file access on the Thor;
/// the app validates and stores the profile itself and shows the result on its Status tab.
pub fn send_profile() -> Result<SendProfileResult, String> {
    let device = adb::device_status();
    if !device.connected {
        return Err(device.message);
    }
    let packages = adb::sh(&format!("pm list packages {}", adb::shell_quote(thor_app_package())));
    check_thor_app(packages.ok, &packages.stdout, &packages.stderr)?;
    let (profile, note) = super::current_profile()?;
    let encoded = encode_profile(&profile)?;
    let command = profile_command(&encoded)?;
    let output = adb::sh(&command);
    let message = interpret_am_output(THOR_APP_NAME, output.ok, &output.stdout, &output.stderr)?;
    Ok(SendProfileResult { message, games: profile.games.len(), encoded_chars: encoded.len(), note })
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "0123ABCD-4567-89EF-0123-456789ABCDEF";

    fn request(name: &str) -> PlayRequest {
        PlayRequest { package: MOONLIGHT_PACKAGE.into(), uuid: UUID.into(), app_name: name.into(), display: None }
    }

    #[test]
    fn package_list() {
        let output = "package:com.android.settings\r\npackage:com.limelight\npackage:com.limelight.root\n\npackage:com.tailscale.ipn\n";
        assert_eq!(parse_packages(output), ["com.android.settings", "com.limelight", "com.limelight.root", "com.tailscale.ipn"]);
        assert!(parse_packages("").is_empty());
    }

    #[test]
    fn version_name() {
        let dumpsys = "Packages:\n  Package [com.limelight] (abc):\n    versionCode=322 minSdk=21 targetSdk=35\n    versionName=12.2\n    versionName=old\n";
        assert_eq!(parse_version_name(dumpsys).as_deref(), Some("12.2"));
        assert_eq!(parse_version_name("Unable to find package: x"), None);
    }

    #[test]
    fn uuid_check() {
        assert!(is_valid_uuid(UUID));
        assert!(is_valid_uuid("0123abcd-4567-89ef-0123-456789abcdef"));
        assert!(!is_valid_uuid(""));
        assert!(!is_valid_uuid("0123ABCD-4567-89EF-0123-456789ABCDE"));
        assert!(!is_valid_uuid("0123ABCD-4567-89EF-0123-456789ABCDEFG"));
        assert!(!is_valid_uuid("0123ABCD 4567-89EF-0123-456789ABCDEF"));
        assert!(!is_valid_uuid("'; reboot; '-4567-89EF-0123-456789ABCDEF"));
    }

    #[test]
    fn play_command_simple() {
        assert_eq!(
            play_command(&request("Desktop")).unwrap(),
            format!("am start -n com.limelight/com.limelight.ShortcutTrampoline -a android.intent.action.VIEW --es UUID {UUID} --es AppName Desktop")
        );
    }

    #[test]
    fn play_command_quotes_spaces_and_quotes() {
        let command = play_command(&request("Steam Big Picture")).unwrap();
        assert!(command.ends_with("--es AppName 'Steam Big Picture'"), "{command}");

        let command = play_command(&request("Tom's \"Game\" $HOME `id`; rm -rf /")).unwrap();
        assert!(command.ends_with(r#"--es AppName 'Tom'\''s "Game" $HOME `id`; rm -rf /'"#), "{command}");
        // The whole name is still exactly one argument after the device shell has parsed it.
        let args = play_args(&request("Tom's \"Game\" $HOME `id`; rm -rf /"));
        assert_eq!(args.last().unwrap(), "Tom's \"Game\" $HOME `id`; rm -rf /");
        assert_eq!(args[args.len() - 2], "AppName");
        assert_eq!(args[args.len() - 3], "--es", "string extra, not --ei");

        // A name that looks like an option is quoted-safe too (am takes the next argument as value).
        let command = play_command(&request("--ei")).unwrap();
        assert!(command.ends_with("--es AppName --ei"), "{command}");
    }

    #[test]
    fn play_command_display_and_root_flavor() {
        let mut req = request("Desktop");
        req.display = Some(4);
        req.package = MOONLIGHT_ROOT_PACKAGE.into();
        let command = play_command(&req).unwrap();
        assert!(command.starts_with("am start --display 4 -n com.limelight.root/com.limelight.ShortcutTrampoline -a android.intent.action.VIEW --es UUID"), "{command}");
    }

    #[test]
    fn play_request_is_validated() {
        assert!(play_command(&request("")).is_err());
        assert!(play_command(&request("   ")).is_err());
        assert!(play_command(&request("a\nreboot")).is_err());
        assert!(play_command(&request(&"x".repeat(600))).is_err());
        let mut bad_uuid = request("Desktop");
        bad_uuid.uuid = "not-a-uuid".into();
        assert!(play_command(&bad_uuid).is_err());
        let mut bad_package = request("Desktop");
        bad_package.package = "com.evil".into();
        assert!(play_command(&bad_package).is_err());
    }

    #[test]
    fn am_output() {
        let ok = "Starting: Intent { act=android.intent.action.VIEW cmp=com.limelight/.ShortcutTrampoline (has extras) }\n";
        assert_eq!(interpret_am_output("Moonlight", true, ok, "").unwrap(), ok.trim());
        let missing = "Starting: Intent { ... }\nError type 3\nError: Activity class {com.limelight/com.limelight.ShortcutTrampoline} does not exist.\n";
        assert_eq!(interpret_am_output("Moonlight", true, missing, "").unwrap_err(), "Moonlight is not installed on the Thor (or its start screen changed)");
        let security = "Exception occurred while executing 'start':\njava.lang.SecurityException: Permission Denial\n";
        assert!(interpret_am_output("Moonlight", true, "", security).unwrap_err().contains("refused to start Moonlight"));
        assert!(interpret_am_output("Moonlight", false, "", "error: no devices/emulators found").is_err());
        assert_eq!(interpret_am_output("Moonlight", false, "", "").unwrap_err(), "adb couldn't start Moonlight");
        assert_eq!(
            interpret_am_output("Moonlight", true, "Warning: Activity not started, intent has been delivered to currently running top-most instance.", "").unwrap(),
            "Started"
        );
    }

    // ---- Send to Thor app

    use crate::pcstream::network::{AddressKind, Addresses, LocalAddress, TailscaleInfo};
    use crate::pcstream::profile::{build_profile, StreamProfile};
    use crate::pcstream::sunshine::ServerInfo;
    use base64::Engine;

    fn profile_with(games: Vec<String>) -> StreamProfile {
        let server = ServerInfo {
            hostname: "my-pc".into(),
            gamestream_version: "7.1.431.-1".into(),
            uniqueid: UUID.into(),
            https_port: Some(47984),
            current_game: 0,
            busy: false,
        };
        let addresses = Addresses {
            lan: vec![LocalAddress { address: "192.168.1.20".into(), interface: "eth0".into(), kind: AddressKind::Lan, virtual_adapter: false }],
            recommended_lan: Some("192.168.1.20".into()),
            tailscale: TailscaleInfo {
                cli_found: true,
                backend_state: "Running".into(),
                running: true,
                ipv4: Some("100.101.102.103".into()),
                dns_name: Some("my-pc.tail1234.ts.net".into()),
                note: None,
            },
            error: None,
        };
        build_profile(&server, 47989, &addresses, games)
    }

    /// The decoded JSON of the last `am start` argument.
    fn decode_last_arg(command: &str) -> serde_json::Value {
        let encoded = command.rsplit(' ').next().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn thor_app_component_matches_the_android_repo() {
        // Values from ThorBroAndroid: applicationId (build.gradle.kts) / namespace + ".MainActivity" (manifest).
        assert_eq!(THOR_APP_COMPONENT, "dev.catzeyea.thorbro/dev.catzeyea.thorbro.android.MainActivity");
        assert_eq!(thor_app_package(), "dev.catzeyea.thorbro");
        assert_eq!(PROFILE_EXTRA, "profile_b64");
    }

    #[test]
    fn send_profile_command_layout() {
        let encoded = encode_profile(&profile_with(vec!["Desktop".into(), "Steam Big Picture".into()])).unwrap();
        let command = profile_command(&encoded).unwrap();
        assert_eq!(command, format!("am start -n dev.catzeyea.thorbro/dev.catzeyea.thorbro.android.MainActivity --es profile_b64 {encoded}"));
        // Standard base64 with padding, no line breaks, nothing the shell would touch.
        assert!(!encoded.contains(['\n', ' ', '\'', '"', '$', '`', '\\']));
        assert_eq!(encoded.len() % 4, 0);
        let json = decode_last_arg(&command);
        assert_eq!(json["format"], "thorbro.stream-profile");
        assert_eq!(json["version"], 1);
        assert_eq!(json["pc"]["uniqueid"], UUID);
        assert_eq!(json["games"], serde_json::json!(["Desktop", "Steam Big Picture"]));
    }

    #[test]
    fn send_profile_special_characters_in_game_names() {
        let names: Vec<String> = [
            "Tom's \"Game\" $HOME `id`; rm -rf / && reboot",
            "--es evil 1",
            "Ünïcödé – 2 ★ 日本語",
            "back\\slash | pipe > out < in * ? [x] {y} # ~ !",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let encoded = encode_profile(&profile_with(names.clone())).unwrap();
        let command = profile_command(&encoded).unwrap();
        // Exactly the seven `am start` arguments after the Thor's shell splits it at spaces, and no
        // character that `sh` would interpret: the game names are hidden in the base64 text.
        let words: Vec<&str> = command.split(' ').collect();
        assert_eq!(words.len(), 7, "{command}");
        assert_eq!(words[..6], ["am", "start", "-n", THOR_APP_COMPONENT, "--es", "profile_b64"]);
        assert!(command.chars().all(|c| c.is_ascii_alphanumeric() || " /.+=-_".contains(c)), "{command}");
        // The names arrive unchanged.
        let json = decode_last_arg(&command);
        assert_eq!(json["games"], serde_json::json!(names));
        // The quoting is the same as for "Play on Thor": shell_quote on every argument.
        let quoted: Vec<String> = profile_args(&encoded).iter().map(|arg| adb::shell_quote(arg)).collect();
        assert_eq!(quoted.join(" "), command);
    }

    #[test]
    fn send_profile_command_refuses_bad_input() {
        assert!(profile_command("").is_err());
        assert!(profile_command("abc'; reboot; '").is_err());
        assert!(profile_command("YWJj\nZA==").is_err());
        assert!(profile_command(&"A".repeat(MAX_PROFILE_B64_CHARS + 4)).is_err());
        assert!(profile_command(&"A".repeat(MAX_PROFILE_B64_CHARS)).is_ok());
    }

    #[test]
    fn send_profile_size_limit() {
        // A realistic library fits with a wide margin.
        let realistic: Vec<String> = (0..100).map(|i| format!("Some Game Title: Remastered Edition {i}")).collect();
        let encoded = encode_profile(&profile_with(realistic)).unwrap();
        assert!(encoded.len() < MAX_PROFILE_B64_CHARS / 2, "{}", encoded.len());
        // Too many games: readable error with the numbers, nothing is built.
        let huge: Vec<String> = (0..2000).map(|i| format!("Some Game Title: Remastered Edition {i}")).collect();
        let error = encode_profile(&profile_with(huge)).unwrap_err();
        assert!(error.contains("too large") && error.contains("2000 games") && error.contains(&MAX_PROFILE_B64_CHARS.to_string()), "{error}");
        // The whole command line, with the longest allowed profile, stays below the Windows
        // command line limit (32,767) with room for the adb path.
        let longest = profile_command(&"A".repeat(MAX_PROFILE_B64_CHARS)).unwrap();
        assert!(longest.len() + "adb.exe shell \"\"".len() + 1024 < 32_767, "{}", longest.len());
    }

    #[test]
    fn send_profile_never_contains_credentials() {
        let encoded = encode_profile(&profile_with(vec!["Game".into()])).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(&encoded).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(crate::pcstream::profile::credential_field(&json), None);
        // Same words as profile::tests::profile_never_contains_secrets, checked on what is sent.
        let lower = text.to_lowercase();
        for word in ["password", "username", "login", "token", "secret", "cert", "key", "pin", "auth"] {
            assert!(!lower.contains(word), "sent profile contains {word}: {text}");
        }
    }

    #[test]
    fn thor_app_install_check() {
        assert!(check_thor_app(true, "package:dev.catzeyea.thorbro\n", "").is_ok());
        assert!(check_thor_app(true, "package:com.limelight\r\npackage:dev.catzeyea.thorbro\r\n", "").is_ok());
        // Not installed: empty list, or only packages that merely contain the name (pm filters by substring).
        let missing = check_thor_app(true, "", "").unwrap_err();
        assert!(missing.contains("not installed") && missing.contains("APK"), "{missing}");
        assert!(check_thor_app(true, "package:dev.catzeyea.thorbro.other\npackage:x.dev.catzeyea.thorbro\n", "").is_err());
        // pm itself failed.
        assert!(check_thor_app(false, "", "error: closed").unwrap_err().contains("couldn't list"));
    }

    #[test]
    fn thor_app_am_output() {
        let missing = "Error type 3\nError: Activity class {dev.catzeyea.thorbro/dev.catzeyea.thorbro.android.MainActivity} does not exist.\n";
        assert_eq!(interpret_am_output(THOR_APP_NAME, true, missing, "").unwrap_err(), "The ThorBro app is not installed on the Thor (or its start screen changed)");
        let delivered = "Starting: Intent { cmp=dev.catzeyea.thorbro/.android.MainActivity (has extras) }\nWarning: Activity not started, intent has been delivered to currently running top-most instance.\n";
        assert!(interpret_am_output(THOR_APP_NAME, true, delivered, "").unwrap().starts_with("Starting"));
    }

    /// Measures the encoded size of typical profiles (no device, no Sunshine needed).
    /// `cargo test measure_profile_size -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_profile_size() {
        let titles = [
            "Desktop", "Steam Big Picture", "Elden Ring", "Cyberpunk 2077", "Baldur's Gate 3", "The Witcher 3: Wild Hunt",
            "Red Dead Redemption 2", "Hollow Knight", "Forza Horizon 5", "Hades", "Stardew Valley", "Monster Hunter: World",
            "Sekiro: Shadows Die Twice", "Final Fantasy VII Remake Intergrade", "Disco Elysium - The Final Cut",
            "Star Wars Jedi: Survivor", "Persona 5 Royal", "Death Stranding Director's Cut", "Halo: The Master Chief Collection", "Celeste",
        ];
        let avg = titles.iter().map(|t| t.len()).sum::<usize>() as f64 / titles.len() as f64;
        for count in [2usize, 20, 100, 250, 500] {
            let games: Vec<String> = (0..count).map(|i| titles[i % titles.len()].to_string()).collect();
            let profile = profile_with(games);
            let pretty = crate::pcstream::profile::to_json(&profile).len();
            let compact = crate::pcstream::profile::to_compact_json(&profile).len();
            let encoded = encode_profile(&profile).map(|e| e.len());
            let command = encoded.as_ref().ok().map(|_| profile_command(&encode_profile(&profile).unwrap()).unwrap().len());
            eprintln!("{count:>4} games: pretty JSON {pretty:>6} B, compact {compact:>6} B, base64 {encoded:?}, shell command {command:?}");
        }
        eprintln!("average title length {avg:.1} chars, limit {MAX_PROFILE_B64_CHARS} base64 chars");
        let mut count = 0;
        while encode_profile(&profile_with((0..count).map(|i| titles[i % titles.len()].to_string()).collect())).is_ok() {
            count += 10;
        }
        eprintln!("first count over the limit (step 10): {count} games");
    }
}
