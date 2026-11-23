//! Sunshine's admin API, called from Rust (not from the webview).
//!
//! Facts from Sunshine's source and docs (research doc, section 2; `src/confighttp.cpp` at
//! v2026.914.233613):
//!
//! - HTTPS on `port + 1` (default 47990) with a self-signed certificate. ThorBro only ever talks
//!   to `127.0.0.1`, so certificate checks are switched off for this one local connection.
//! - HTTP Basic auth with the web UI login. If no login exists yet, every call answers with a
//!   redirect to `/welcome`; a caller outside `origin_web_ui_allowed` gets 403.
//! - CSRF: POST/DELETE need a token only if the request carries an `Origin` or `Referer` header.
//!   ureq sends neither, and `ApiRequest` never adds one (checked by a test).
//! - Pairing: `GET /api/pin` lists pending requests (`{"pairings":[{"id","name","address"}]}`),
//!   `POST /api/pin` takes `{"pairing_id","pin","name"}` and stays open until Moonlight finished the
//!   handshake. `pairing_id` and `GET /api/pin` first appeared in the stable release
//!   v2026.906.222525; older versions answer 404 to the GET and take only `{"pin","name"}` for the
//!   newest request. Old versions (v0.23) send `"status":"true"` as a string, new ones a bool.
//!
//! The login is kept in the OS keychain (Windows Credential Manager, macOS Keychain, Secret
//! Service on Linux) under service `ThorBro`, never in `settings.json` and never sent to the UI.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Keychain entry: service name and account.
const KEYCHAIN_SERVICE: &str = "ThorBro";
const KEYCHAIN_ACCOUNT: &str = "sunshine-admin";

/// Timeout for normal admin calls (local, should be instant).
const CALL_TIMEOUT: Duration = Duration::from_secs(8);

/// `POST /api/pin` stays open while Moonlight finishes the handshake (up to Sunshine's
/// `ping_timeout`), so it gets much longer.
const PIN_TIMEOUT: Duration = Duration::from_secs(90);

/// How long `sunshine --creds` may take.
const CREDS_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest username / password we accept (bytes).
const MAX_LOGIN_BYTES: usize = 256;

/// Sunshine's limit for the client name in `POST /api/pin` (1..=128 bytes).
const MAX_CLIENT_NAME_BYTES: usize = 128;

/// The Sunshine web UI login. `Debug` hides the password so it can't end up in a log.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Login {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login").field("username", &self.username).field("password", &"<hidden>").finish()
    }
}

impl Login {
    /// Checks a login before it is used or stored. `for_command_line`: it will be passed to
    /// `sunshine --creds`, so it must not look like an option either.
    pub fn validate(&self, for_command_line: bool) -> Result<(), String> {
        for (what, value) in [("Username", &self.username), ("Password", &self.password)] {
            if value.is_empty() {
                return Err(format!("{what} is empty"));
            }
            if value.len() > MAX_LOGIN_BYTES {
                return Err(format!("{what} is too long"));
            }
            if super::has_control_chars(value) {
                return Err(format!("{what} contains control characters"));
            }
            if for_command_line && value.starts_with('-') {
                return Err(format!("{what} must not start with '-'"));
            }
        }
        // Basic auth is "user:password"; Sunshine splits at the first ':'.
        if self.username.contains(':') {
            return Err("Username must not contain ':'".into());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- Keychain

fn keychain_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(keychain_error)
}

fn keychain_error(error: keyring::Error) -> String {
    let hint = if cfg!(target_os = "linux") { " (on Linux this needs a Secret Service like GNOME Keyring or KWallet, unlocked)" } else { "" };
    format!("Couldn't use the OS keychain{hint}: {error}")
}

/// The stored login, `None` if there is none.
pub fn load_login() -> Result<Option<Login>, String> {
    match keychain_entry()?.get_password() {
        Ok(json) => serde_json::from_str(&json).map(Some).map_err(|_| "The stored Sunshine login is damaged; save it again".into()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(keychain_error(e)),
    }
}

/// Stores the login in the OS keychain.
pub fn store_login(login: &Login) -> Result<(), String> {
    let json = serde_json::to_string(login).map_err(|e| e.to_string())?;
    keychain_entry()?.set_password(&json).map_err(keychain_error)
}

/// Removes the stored login (no error if there was none).
pub fn forget_login() -> Result<(), String> {
    match keychain_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(keychain_error(e)),
    }
}

// ---------------------------------------------------------------- Requests

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// One admin API request, fully described before it is sent (so it can be tested).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiRequest {
    pub method: Method,
    /// Path including the leading slash, e.g. `/api/apps`.
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub timeout: Duration,
}

/// `https://127.0.0.1:<port><path>`. Always loopback: the admin client never talks to another host.
pub fn admin_url(admin_port: u16, path: &str) -> String {
    format!("https://127.0.0.1:{admin_port}{path}")
}

/// Value of the `Authorization` header.
pub fn basic_auth(login: &Login) -> String {
    use base64::Engine;
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", login.username, login.password));
    format!("Basic {token}")
}

/// A GET request; `login` = None sends no `Authorization` (used to see whether a login exists).
pub fn get_request(login: Option<&Login>, path: &str) -> ApiRequest {
    let headers = login.map(|login| vec![("Authorization".to_string(), basic_auth(login))]).unwrap_or_default();
    ApiRequest { method: Method::Get, path: path.into(), headers, body: None, timeout: CALL_TIMEOUT }
}

/// A POST request with a JSON body.
pub fn post_json_request(login: &Login, path: &str, body: String, timeout: Duration) -> ApiRequest {
    ApiRequest {
        method: Method::Post,
        path: path.into(),
        headers: vec![
            ("Authorization".to_string(), basic_auth(login)),
            ("Content-Type".to_string(), "application/json".to_string()),
        ],
        body: Some(body),
        timeout,
    }
}

/// A response, reduced to what we look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiResponse {
    pub status: u16,
    pub location: Option<String>,
    pub body: String,
}

/// HTTPS agent for the admin API: accepts Sunshine's self-signed certificate (loopback only),
/// ignores proxy variables, follows no redirects (the `/welcome` redirect is information) and
/// returns HTTP errors as responses.
fn admin_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .proxy(None)
        .http_status_as_error(false)
        .max_redirects(0)
        .tls_config(ureq::tls::TlsConfig::builder().disable_verification(true).build())
        .build()
        .into()
}

/// Sends `request` to the admin API on `admin_port`.
pub fn send(admin_port: u16, request: &ApiRequest) -> Result<ApiResponse, String> {
    let url = admin_url(admin_port, &request.path);
    let agent = admin_agent(request.timeout);
    let result = match request.method {
        Method::Get => {
            let mut builder = agent.get(&url);
            for (name, value) in &request.headers {
                builder = builder.header(name, value);
            }
            builder.call()
        }
        Method::Post => {
            let mut builder = agent.post(&url);
            for (name, value) in &request.headers {
                builder = builder.header(name, value);
            }
            builder.send(request.body.clone().unwrap_or_default())
        }
    };
    let mut response = result.map_err(|e| match e {
        ureq::Error::Timeout(_) => format!("Sunshine's web UI (port {admin_port}) didn't answer in time"),
        other => format!("Sunshine's web UI is not reachable on port {admin_port} ({other})"),
    })?;
    let status = response.status().as_u16();
    let location = response.headers().get("location").and_then(|value| value.to_str().ok()).map(String::from);
    let body = response.body_mut().read_to_string().unwrap_or_default();
    Ok(ApiResponse { status, location, body })
}

// ---------------------------------------------------------------- Answers

/// Why an admin call didn't work, as far as the tab cares.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoginState {
    /// The login works.
    Ok,
    /// No login stored in the keychain.
    NotStored,
    /// Sunshine has no web UI login at all yet (redirect to /welcome).
    NoLoginYet,
    /// Sunshine has a login, but ours is wrong (401).
    Rejected,
    /// Sunshine refuses this PC's address (403, `origin_web_ui_allowed`).
    Forbidden,
    /// The web UI port doesn't answer.
    Unreachable,
    /// The keychain failed.
    KeychainError,
    /// Anything else (unexpected status, broken JSON).
    Failed,
}

/// An admin API failure: a category plus text for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiFailure {
    pub state: LoginState,
    pub message: String,
}

impl ApiFailure {
    fn new(state: LoginState, message: impl Into<String>) -> Self {
        ApiFailure { state, message: message.into() }
    }
}

impl From<ApiFailure> for String {
    fn from(failure: ApiFailure) -> String {
        failure.message
    }
}

/// Sunshine's `{"error": "..."}` text, if the body has one.
fn error_text(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body).ok()?.get("error")?.as_str().map(String::from)
}

/// Sorts a response into success (body) or a categorized failure.
pub fn classify(response: &ApiResponse) -> Result<&str, ApiFailure> {
    match response.status {
        200..=299 => Ok(&response.body),
        300..=399 if response.location.as_deref().is_some_and(|location| location.contains("welcome")) => Err(ApiFailure::new(
            LoginState::NoLoginYet,
            "Sunshine has no web UI login yet. Create one (below) or in Sunshine's web UI.",
        )),
        401 => Err(ApiFailure::new(LoginState::Rejected, "Sunshine rejected the login (wrong username or password).")),
        403 => Err(ApiFailure::new(
            LoginState::Forbidden,
            "Sunshine refused the request from this PC (check origin_web_ui_allowed in Sunshine's settings).",
        )),
        status => {
            let detail = error_text(&response.body).map(|text| format!(": {text}")).unwrap_or_default();
            Err(ApiFailure::new(LoginState::Failed, format!("Sunshine answered with HTTP {status}{detail}")))
        }
    }
}

/// Sends a request and classifies the answer.
fn call(admin_port: u16, request: &ApiRequest) -> Result<String, ApiFailure> {
    let response = send(admin_port, request).map_err(|message| ApiFailure::new(LoginState::Unreachable, message))?;
    classify(&response).map(String::from)
}

/// The stored login or a `NotStored` / `KeychainError` failure.
fn require_login() -> Result<Login, ApiFailure> {
    match load_login() {
        Ok(Some(login)) => Ok(login),
        Ok(None) => Err(ApiFailure::new(LoginState::NotStored, "Save your Sunshine login first (section Sunshine login).")),
        Err(message) => Err(ApiFailure::new(LoginState::KeychainError, message)),
    }
}

// ---------------------------------------------------------------- Login status

/// What the tab shows about the login. Never contains the password.
#[derive(Serialize, Clone, Debug)]
pub struct LoginStatus {
    pub state: LoginState,
    /// The stored username, if any.
    pub username: Option<String>,
    /// Sunshine's own version from `/api/config` (only with a working login).
    pub sunshine_version: Option<String>,
    pub message: Option<String>,
}

/// Checks a login against `GET /api/config`; on success returns Sunshine's version.
pub fn check_login(admin_port: u16, login: &Login) -> Result<Option<String>, ApiFailure> {
    let body = call(admin_port, &get_request(Some(login), "/api/config"))?;
    Ok(parse_config_version(&body))
}

/// `version` from the `/api/config` JSON.
pub fn parse_config_version(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body).ok()?.get("version")?.as_str().map(String::from)
}

/// The full login status for the tab. Without a stored login it still asks Sunshine (without
/// credentials) whether a login exists at all, so the UI can offer "save" or "create".
pub fn login_status(admin_port: u16) -> LoginStatus {
    let login = match load_login() {
        Ok(login) => login,
        Err(message) => return LoginStatus { state: LoginState::KeychainError, username: None, sunshine_version: None, message: Some(message) },
    };
    let Some(login) = login else {
        // 401 here just means "a login exists"; that is the normal NotStored case.
        let failure = match call(admin_port, &get_request(None, "/api/config")) {
            Err(failure) if matches!(failure.state, LoginState::NoLoginYet | LoginState::Unreachable | LoginState::Forbidden) => failure,
            _ => ApiFailure::new(LoginState::NotStored, "Sunshine has a login. Enter it below so ThorBro can pair and list games."),
        };
        return LoginStatus { state: failure.state, username: None, sunshine_version: None, message: Some(failure.message) };
    };
    match check_login(admin_port, &login) {
        Ok(version) => LoginStatus { state: LoginState::Ok, username: Some(login.username), sunshine_version: version, message: None },
        Err(failure) => LoginStatus { state: failure.state, username: Some(login.username), sunshine_version: None, message: Some(failure.message) },
    }
}

/// Checks the login against Sunshine and stores it only if it works.
pub fn save_login(admin_port: u16, login: &Login) -> Result<LoginStatus, String> {
    login.validate(false)?;
    let version = check_login(admin_port, login).map_err(String::from)?;
    store_login(login)?;
    Ok(LoginStatus { state: LoginState::Ok, username: Some(login.username.clone()), sunshine_version: version, message: None })
}

// ---------------------------------------------------------------- Creating a login

/// `sunshine --creds <user> <password>` arguments (no shell; each element is one argument).
pub fn creds_args(login: &Login) -> Result<Vec<String>, String> {
    login.validate(true)?;
    Ok(vec!["--creds".into(), login.username.clone(), login.password.clone()])
}

/// Runs `sunshine --creds` and stores the login in the keychain.
///
/// Notes from Sunshine's source: `--creds` *replaces* an existing login, it exits with 0 even if
/// the file couldn't be written (on Windows the file is under Program Files and needs admin
/// rights), and a running Sunshine reads the login only when it starts. So afterwards Sunshine
/// must be restarted and the login tested; the UI says so.
pub fn create_login(login: &Login) -> Result<String, String> {
    let args = creds_args(login)?;
    let exe = super::sunshine::executable().ok_or("Couldn't find the sunshine program")?;
    let mut command = crate::adb::cmd(&exe);
    command.args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|e| format!("couldn't start sunshine: {e}"))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > CREDS_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("sunshine --creds didn't finish in time".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(format!("couldn't wait for sunshine: {e}")),
        }
    };
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    // Sunshine logs this on failure but still exits with 0.
    if !status.success() || text.contains("error writing to the credentials file") || text.contains("Couldn't read user credentials") {
        let hint = if cfg!(windows) { " On Windows this needs administrator rights; use Sunshine's web UI instead." } else { "" };
        return Err(format!("Sunshine couldn't save the login.{hint}"));
    }
    store_login(login)?;
    Ok("Login created and saved in the keychain. Restart Sunshine so it uses the new login, then test it.".into())
}

/// Restarts the Sunshine user service on Linux (`systemctl --user restart <unit>`). Ends any
/// running stream. Not offered on Windows: restarting a service there needs admin rights.
pub fn restart_service() -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("Restart Sunshine yourself (on Windows: the SunshineService, needs admin rights)".into());
    }
    let output = crate::adb::cmd("systemctl")
        .args(["--user", "restart", super::sunshine::LINUX_UNIT])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run systemctl: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("systemctl couldn't restart Sunshine: {}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

// ---------------------------------------------------------------- Pairing

/// A Moonlight client waiting for its PIN (`GET /api/pin`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PendingPairing {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub address: String,
}

/// Answer of `pending_pairings`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Pairings {
    /// false = this Sunshine is older than v2026.906.222525 and has no pending list / pairing_id.
    pub supported: bool,
    pub pairings: Vec<PendingPairing>,
}

/// Parses `{"pairings":[...]}`.
pub fn parse_pairings(body: &str) -> Result<Vec<PendingPairing>, String> {
    #[derive(Deserialize)]
    struct Answer {
        #[serde(default)]
        pairings: Vec<PendingPairing>,
    }
    serde_json::from_str::<Answer>(body).map(|answer| answer.pairings).map_err(|e| format!("unexpected answer from /api/pin: {e}"))
}

/// Lists the Moonlight clients waiting for a PIN. A 404 means "older Sunshine".
pub fn pending_pairings(admin_port: u16) -> Result<Pairings, String> {
    let login = require_login()?;
    let response = send(admin_port, &get_request(Some(&login), "/api/pin"))?;
    if response.status == 404 {
        return Ok(Pairings { supported: false, pairings: Vec::new() });
    }
    let body = classify(&response)?;
    Ok(Pairings { supported: true, pairings: parse_pairings(body)? })
}

/// True for Sunshine's pairing id format (exactly 32 hex characters).
pub fn is_valid_pairing_id(id: &str) -> bool {
    id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// JSON body for `POST /api/pin`, checked like Sunshine checks it. `pairing_id` = None builds the
/// body for Sunshine versions before v2026.906.222525 (`{"pin","name"}` only).
pub fn pin_body(pairing_id: Option<&str>, pin: &str, name: &str) -> Result<String, String> {
    if pin.len() != 4 || !pin.chars().all(|c| c.is_ascii_digit()) {
        return Err("The PIN has exactly 4 digits (Moonlight shows it on the Thor)".into());
    }
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_CLIENT_NAME_BYTES || super::has_control_chars(name) {
        return Err(format!("The device name must have 1 to {MAX_CLIENT_NAME_BYTES} bytes and no control characters"));
    }
    let body = match pairing_id {
        Some(id) if !is_valid_pairing_id(id) => return Err("Invalid pairing request id".into()),
        Some(id) => serde_json::json!({ "pairing_id": id, "pin": pin, "name": name }),
        None => serde_json::json!({ "pin": pin, "name": name }),
    };
    Ok(body.to_string())
}

/// `status` of a POST answer: a bool in new Sunshine, the string "true"/"false" in old ones.
pub fn parse_status(body: &str) -> Option<bool> {
    match serde_json::from_str::<serde_json::Value>(body).ok()?.get("status")? {
        serde_json::Value::Bool(value) => Some(*value),
        serde_json::Value::String(text) => Some(text == "true"),
        _ => None,
    }
}

/// Sends the PIN. Returns whether Moonlight finished pairing.
pub fn submit_pin(admin_port: u16, pairing_id: Option<&str>, pin: &str, name: &str) -> Result<bool, String> {
    let body = pin_body(pairing_id, pin, name)?;
    let login = require_login()?;
    let response = send(admin_port, &post_json_request(&login, "/api/pin", body, PIN_TIMEOUT))?;
    if response.status == 400 {
        // Sunshine explains bad input (wrong PIN format, missing pairing_id on new versions, ...).
        let text = error_text(&response.body).unwrap_or_else(|| "bad request".into());
        return Err(format!("Sunshine refused the PIN: {text}"));
    }
    let body = classify(&response)?;
    parse_status(body).ok_or_else(|| "unexpected answer from /api/pin".into())
}

// ---------------------------------------------------------------- Apps

/// App names from `GET /api/apps` (`{"apps":[{"name": ...}, ...], ...}`), in Sunshine's order,
/// empty names skipped.
pub fn parse_app_names(body: &str) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct App {
        #[serde(default)]
        name: String,
    }
    #[derive(Deserialize)]
    struct Answer {
        #[serde(default)]
        apps: Vec<App>,
    }
    let answer: Answer = serde_json::from_str(body).map_err(|e| format!("unexpected answer from /api/apps: {e}"))?;
    Ok(answer.apps.into_iter().map(|app| app.name).filter(|name| !name.trim().is_empty()).collect())
}

/// Sunshine's app names.
pub fn app_names(admin_port: u16) -> Result<Vec<String>, String> {
    let login = require_login()?;
    let body = call(admin_port, &get_request(Some(&login), "/api/apps"))?;
    parse_app_names(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login() -> Login {
        Login { username: "admin".into(), password: "p:ss wörd".into() }
    }

    #[test]
    fn login_debug_hides_the_password() {
        let text = format!("{:?}", login());
        assert!(text.contains("admin") && !text.contains("p:ss"), "{text}");
    }

    #[test]
    fn basic_auth_header() {
        // base64("admin:p:ss wörd") with UTF-8 bytes
        assert_eq!(basic_auth(&login()), "Basic YWRtaW46cDpzcyB3w7ZyZA==");
        let simple = Login { username: "sunshine".into(), password: "secret".into() };
        assert_eq!(basic_auth(&simple), "Basic c3Vuc2hpbmU6c2VjcmV0");
    }

    #[test]
    fn requests_are_loopback_and_have_no_origin() {
        assert_eq!(admin_url(47990, "/api/apps"), "https://127.0.0.1:47990/api/apps");
        let get = get_request(Some(&login()), "/api/apps");
        let post = post_json_request(&login(), "/api/pin", "{}".into(), PIN_TIMEOUT);
        for request in [&get, &post] {
            assert!(request.headers.iter().all(|(name, _)| !name.eq_ignore_ascii_case("origin") && !name.eq_ignore_ascii_case("referer")));
            assert!(request.headers.iter().any(|(name, value)| name == "Authorization" && value.starts_with("Basic ")));
        }
        assert_eq!(get.method, Method::Get);
        assert_eq!(get.body, None);
        assert_eq!(post.method, Method::Post);
        assert!(post.headers.contains(&("Content-Type".into(), "application/json".into())));
        assert_eq!(post.timeout, PIN_TIMEOUT);
        assert!(get_request(None, "/api/config").headers.is_empty());
    }

    #[test]
    fn pin_bodies() {
        let id = "0123456789abcdef0123456789ABCDEF";
        let body: serde_json::Value = serde_json::from_str(&pin_body(Some(id), "0042", " Thor ").unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({ "pairing_id": id, "pin": "0042", "name": "Thor" }));
        let legacy: serde_json::Value = serde_json::from_str(&pin_body(None, "1234", "Thor \"2\"").unwrap()).unwrap();
        assert_eq!(legacy, serde_json::json!({ "pin": "1234", "name": "Thor \"2\"" }));
        assert!(pin_body(Some(id), "123", "Thor").is_err());
        assert!(pin_body(Some(id), "12a4", "Thor").is_err());
        assert!(pin_body(Some(id), "12345", "Thor").is_err());
        assert!(pin_body(Some(id), "1234", "").is_err());
        assert!(pin_body(Some(id), "1234", "a\nb").is_err());
        assert!(pin_body(Some(id), "1234", &"x".repeat(129)).is_err());
        assert!(pin_body(Some("short"), "1234", "Thor").is_err());
        assert!(pin_body(Some("0123456789abcdef0123456789abcdeg"), "1234", "Thor").is_err());
    }

    #[test]
    fn creds_arguments() {
        assert_eq!(creds_args(&login()).unwrap(), ["--creds", "admin", "p:ss wörd"]);
        assert!(creds_args(&Login { username: "-x".into(), password: "a".into() }).is_err());
        assert!(creds_args(&Login { username: "a".into(), password: "--help".into() }).is_err());
        assert!(creds_args(&Login { username: "a".into(), password: "".into() }).is_err());
        assert!(creds_args(&Login { username: "a:b".into(), password: "x".into() }).is_err());
        assert!(creds_args(&Login { username: "a".into(), password: "x\ny".into() }).is_err());
        // Saving an existing login allows a password starting with '-' (no command line involved).
        assert!(Login { username: "a".into(), password: "-x".into() }.validate(false).is_ok());
    }

    #[test]
    fn responses_are_classified() {
        let response = |status: u16, location: Option<&str>, body: &str| ApiResponse {
            status,
            location: location.map(String::from),
            body: body.into(),
        };
        assert_eq!(classify(&response(200, None, "{}")).unwrap(), "{}");
        assert_eq!(classify(&response(307, Some("/welcome"), "")).unwrap_err().state, LoginState::NoLoginYet);
        assert_eq!(classify(&response(401, None, r#"{"error":"Unauthorized"}"#)).unwrap_err().state, LoginState::Rejected);
        assert_eq!(classify(&response(403, None, "")).unwrap_err().state, LoginState::Forbidden);
        let other = classify(&response(500, None, r#"{"status_code":500,"error":"boom"}"#)).unwrap_err();
        assert_eq!(other.state, LoginState::Failed);
        assert!(other.message.contains("500") && other.message.contains("boom"), "{}", other.message);
        assert_eq!(classify(&response(302, Some("/elsewhere"), "")).unwrap_err().state, LoginState::Failed);
    }

    #[test]
    fn answers_are_parsed() {
        assert_eq!(parse_config_version(r#"{"status":true,"platform":"linux","version":"2026.914.233613"}"#).as_deref(), Some("2026.914.233613"));
        assert_eq!(parse_config_version("{}"), None);

        let pairings = parse_pairings(r#"{"pairings":[{"id":"0123456789abcdef0123456789abcdef","name":"Thor","address":"192.168.1.30"}]}"#).unwrap();
        assert_eq!(pairings.len(), 1);
        assert_eq!(pairings[0].name, "Thor");
        assert!(parse_pairings(r#"{"pairings":[]}"#).unwrap().is_empty());
        assert!(parse_pairings("nope").is_err());

        assert_eq!(parse_status(r#"{"status":true}"#), Some(true));
        assert_eq!(parse_status(r#"{"status":false}"#), Some(false));
        assert_eq!(parse_status(r#"{"status":"true"}"#), Some(true), "Sunshine v0.23 style");
        assert_eq!(parse_status(r#"{"status":"false"}"#), Some(false));
        assert_eq!(parse_status("{}"), None);
    }

    #[test]
    fn app_list() {
        let body = r#"{"env":{"PATH":"x"},"apps":[{"name":"Desktop","image-path":"desktop.png"},{"name":"Steam Big Picture","cmd":"steam"},{"name":""},{"cmd":"no name"}]}"#;
        assert_eq!(parse_app_names(body).unwrap(), ["Desktop", "Steam Big Picture"]);
        assert!(parse_app_names(r#"{"apps":[]}"#).unwrap().is_empty());
        assert!(parse_app_names("<html>").is_err());
    }

    #[test]
    fn pairing_ids() {
        assert!(is_valid_pairing_id("0123456789abcdef0123456789abcdef"));
        assert!(!is_valid_pairing_id("0123456789abcdef0123456789abcde"));
        assert!(!is_valid_pairing_id(""));
    }

    /// Talks to the real local Sunshine without credentials. Read-only, changes nothing.
    /// `cargo test local_ -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn local_admin_api_answers_without_login() {
        let port = super::super::sunshine::admin_port(super::super::sunshine::configured_port());
        let response = send(port, &get_request(None, "/api/config")).expect("admin API not reachable");
        eprintln!("status {} location {:?}", response.status, response.location);
        let state = classify(&response).map(|_| LoginState::Ok).unwrap_or_else(|failure| failure.state);
        eprintln!("classified as {state:?}");
        assert!(matches!(state, LoginState::Rejected | LoginState::NoLoginYet | LoginState::Forbidden), "{state:?}");
    }
}
