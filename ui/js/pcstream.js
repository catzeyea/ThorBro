// ThorBro UI – Stream tab, sub-tab "Stream PC games" and the Help page (src/pcstream/).
// Sunshine runs on the PC, Moonlight on the Thor. This checks both sides, helps with the
// login and pairing, lists Sunshine's apps and starts one on the Thor over adb.
// All Sunshine requests are made in Rust (no Origin header, login from the OS keychain).
//
// Layout: the "Stream PC games" view shows only status chips, "Play on Thor", the Thor app card
// (Send to Thor app, Pair Moonlight) and whether something streams. Details (Sunshine, login,
// addresses, Thor apps, screen picker, checklist, stream profile) are sections of the Help page.
"use strict";

/** State of the PC streaming page. */
const PCS = {
  sub: "mirror",     // visible part of the Stream page: "mirror", "pc" or "help"
  helpFrom: "pc",    // sub-tab "Back to Stream" returns to
  pc: null,          // last answer of pc_stream_detect (pcstream::PcStatus), null = not checked
  detecting: false,
  thor: null,        // last answer of pc_stream_thor_apps (pcstream::thor::ThorApps)
  login: null,       // last answer of pc_stream_login_status (pcstream::admin::LoginStatus)
  pairings: null,    // last answer of pc_stream_pairings (pcstream::admin::Pairings), null = unknown
  pairing: false,    // a PIN is being submitted right now
  pairSelected: "",  // id of the selected waiting Moonlight client
  apps: null,        // Sunshine app names from pc_stream_apps, null = not loaded
  appsError: "",
  display: "",       // experimental: display id for `am start --display`, "" = Moonlight's default
};

// ---------------------------------------------------------------- Sub-tabs

document.querySelectorAll(".subtab").forEach((button) => (button.onclick = () => showStreamSub(button.dataset.sub)));

/** Switches the Stream page between "mirror" (scrcpy), "pc" (Sunshine + Moonlight) and "help". */
function showStreamSub(sub) {
  if (sub === "help" && PCS.sub !== "help") PCS.helpFrom = PCS.sub;
  PCS.sub = sub;
  document.querySelectorAll(".subtab").forEach((button) => button.classList.toggle("active", button.dataset.sub === sub));
  document.querySelectorAll(".subpage").forEach((page) => page.classList.toggle("active", page.id === "stream-sub-" + sub));
  $("#stream-nav").classList.toggle("hidden", sub === "help");
  if (sub !== "mirror" && !PCS.pc && !PCS.detecting) pcRefresh();
}

/** Opens the Help page, optionally with one section (a <details> id) opened and scrolled to. */
function openHelp(sectionId) {
  showStreamSub("help");
  const section = sectionId && document.getElementById(sectionId);
  if (section) {
    section.open = true;
    section.scrollIntoView({ block: "start" });
  } else {
    $("#page-stream").scrollTop = 0;
  }
}

$("#stream-help").onclick = () => openHelp();
$("#help-back").onclick = () => showStreamSub(PCS.helpFrom === "help" ? "pc" : PCS.helpFrom);
// Buttons with data-help="<section id>" (status chips, "set up the login" hints) open that Help section.
$("#page-stream").addEventListener("click", (e) => {
  const button = e.target.closest("[data-help]");
  if (button) openHelp(button.dataset.help);
});

// "Pair Moonlight" shows the pairing box (PIN, device name, Pair) inside the Thor app card.
$("#pc-pair-toggle").onclick = () => {
  const open = $("#pc-pair").classList.toggle("hidden") === false;
  $("#pc-pair-toggle").setAttribute("aria-expanded", String(open));
  if (open) setTimeout(() => { const pin = $("#pc-pin"); if (pin && !pin.disabled) pin.focus(); }, 0);
};

$("#pc-refresh").onclick = () => pcRefresh();

/** Re-runs every check of the page. */
async function pcRefresh() {
  await Promise.all([pcDetect(), pcThorApps(), pcLoginStatus()]);
}

// ---------------------------------------------------------------- Small helpers

/** A "key: value" row; `value` is HTML (escape it yourself). */
const pcRow = (key, valueHtml) => `<div class="pc-row"><span class="k">${esc(key)}</span><span class="v">${valueHtml}</span></div>`;

/** A coloured badge: kind = "ok" | "warn" | "bad" | "". */
const pcBadge = (text, kind = "") => `<span class="pc-badge ${kind}">${esc(text)}</span>`;

/** Buttons that open official pages by id (→ commands::pc_stream_open_link, ids in pcstream::LINKS). */
const pcLinks = (links) =>
  `<div class="pc-links">${links.map(([id, label]) => `<button data-link="${esc(id)}">${esc(label)} ↗</button>`).join("")}</div>`;

/** Fills the body of a card and wires its link buttons. */
function pcFill(cardSelector, html) {
  const body = $(`${cardSelector} .pc-body`);
  body.classList.remove("dim");
  body.innerHTML = html;
  body.querySelectorAll("[data-link]").forEach((button) => (button.onclick = () => call("pc_stream_open_link", { id: button.dataset.link }).catch(() => {})));
  return body;
}

// ---------------------------------------------------------------- Step 1: Sunshine and addresses

/** Asks Rust about Sunshine and the PC's addresses (→ commands::pc_stream_detect). */
async function pcDetect() {
  if (PCS.detecting) return;
  PCS.detecting = true;
  $("#pc-refresh").disabled = true;
  pcFill("#pc-sunshine", `<span class="dim"><span class="spin"></span> Checking …</span>`);
  renderChips();
  try {
    PCS.pc = await invoke("pc_stream_detect");
  } catch (e) {
    PCS.pc = null;
    pcFill("#pc-sunshine", `<span class="hint-warn">Couldn't check: ${esc(e)}</span>`);
    return;
  } finally {
    PCS.detecting = false;
    $("#pc-refresh").disabled = false;
    renderChips();
  }
  renderSunshine();
  renderAddresses();
  renderPairing();
  renderGames();
  renderPcState();
}

// ---------------------------------------------------------------- Status chips and streaming state

/** One status chip; kind = "ok" | "warn" | "bad" | "" (unknown). Clicking opens its Help section. */
const pcChip = (text, kind, helpId) =>
  `<button type="button" class="chip ${kind}" data-help="${helpId}" title="Details in Help"><span class="chip-dot"></span>${esc(text)}</button>`;

/** The chips above "Play on Thor": Sunshine, Tailscale on this PC, Moonlight on the Thor (real state, as checked). */
function renderChips() {
  const chips = [];
  const pc = PCS.pc;
  if (!pc) chips.push(pcChip(PCS.detecting ? "Checking Sunshine …" : "Sunshine not checked", "", "help-sunshine"));
  else if (pc.sunshine.server) chips.push(pcChip("Sunshine is running", "ok", "help-sunshine"));
  else if (pc.sunshine.executable || pc.sunshine.service.state !== "not_installed") chips.push(pcChip("Sunshine is not answering", "warn", "help-sunshine"));
  else chips.push(pcChip("Sunshine not found", "bad", "help-sunshine"));

  if (pc) {
    const ts = pc.addresses.tailscale;
    chips.push(ts.running && ts.ipv4 ? pcChip("Tailscale connected", "ok", "help-addresses") : pcChip("Tailscale not connected", "", "help-addresses"));
  }

  const thor = PCS.thor;
  if (!thor) chips.push(pcChip("Checking the Thor …", "", "help-thor"));
  else if (!thor.connected) chips.push(pcChip("Thor not connected", "warn", "help-thor"));
  else if (thor.moonlight) chips.push(pcChip("Moonlight installed on the Thor", "ok", "help-thor"));
  else chips.push(pcChip("Moonlight not installed on the Thor", "bad", "help-thor"));
  $("#pc-chips").innerHTML = chips.join("");
}

/** Mascot next to the cards: waiting, or busy while Sunshine says something is streaming. */
function renderPcState() {
  const server = PCS.pc && PCS.pc.sunshine.server;
  const streaming = !!(server && (server.busy || server.current_game));
  $("#pc-state-img").src = `assets/mascot/thorbro-${streaming ? "busy" : "wait"}.svg`;
  $("#pc-state-text").textContent = streaming
    ? "PC is streaming. Moonlight will ask whether to quit the running app."
    : "Nothing is streaming right now.";
  $("#pc-state").classList.toggle("streaming", streaming);
}

/** Text and badge kind for the service state (pcstream::sunshine::ServiceState). */
function serviceText(service) {
  switch (service.state) {
    case "running": return [`running (${service.detail})`, "ok"];
    case "stopped": return [`installed, not running (${service.detail})`, "warn"];
    case "changing": return [`starting or stopping (${service.detail})`, "warn"];
    case "not_installed": return ["no Sunshine service found", "bad"];
    default: return [service.detail || "unknown", ""];
  }
}

function renderSunshine() {
  const sun = PCS.pc.sunshine;
  const server = sun.server;
  const [serviceLabel, serviceKind] = serviceText(sun.service);
  let headline;
  if (server) headline = pcBadge("Sunshine is running", "ok");
  else if (sun.executable || sun.service.state !== "not_installed") headline = pcBadge("Sunshine is not answering", "warn");
  else headline = pcBadge("Sunshine not found", "bad");

  let html = `<div>${headline}</div>`;
  html += pcRow("Service", pcBadge(serviceLabel, serviceKind));
  if (server) {
    html += pcRow("PC name", esc(server.hostname || "—"));
    html += pcRow("Host ID", `<span class="pc-addr">${esc(server.uniqueid)}</span>`);
    html += pcRow("Right now", server.busy || server.current_game
      ? pcBadge(`streaming (app id ${server.current_game})`, "warn")
      : pcBadge("idle", "ok"));
    html += pcRow("Ports", `${sun.port} (Moonlight) · ${sun.admin_port} (web UI)`);
    html += pcRow("GameStream", `${esc(server.gamestream_version)} <span class="dim">(protocol version, not Sunshine's own)</span>`);
  } else {
    html += pcRow("Answer", `<span class="hint-warn">${esc(sun.error || "no answer")}</span>`);
    if (!sun.executable && sun.service.state === "not_installed") {
      html += `<div class="pc-hint">ThorBro doesn't ship Sunshine. Install it from the official page, then click Check again.</div>`;
      html += pcLinks([["sunshine_getting_started", "Install guide"], ["sunshine_releases", "Downloads"]]);
    } else {
      html += `<div class="pc-hint">Start Sunshine (Linux: <code>systemctl --user start app-dev.lizardbyte.app.Sunshine</code>, Windows: the SunshineService), then click Check again.</div>`;
    }
  }
  if (sun.executable) html += pcRow("Program", `<span class="pc-addr">${esc(sun.executable)}</span>`);
  if (server) html += `<div class="pc-links"><button id="pc-web-ui">Open Sunshine web UI ↗</button></div>`;
  const body = pcFill("#pc-sunshine", html);
  const webUi = $("#pc-web-ui", body);
  if (webUi) webUi.onclick = () => call("pc_stream_open_web_ui").catch(() => {});
}

function renderAddresses() {
  const addresses = PCS.pc.addresses;
  let html = "";
  const lan = addresses.lan.filter((a) => a.kind !== "tailscale");
  if (lan.length) {
    html += pcRow("LAN", lan.map((a) =>
      `<span class="pc-addr">${esc(a.address)}</span> <span class="dim">${esc(a.interface)}${a.virtual_adapter ? ", virtual" : ""}</span>` +
      (a.address === addresses.recommended_lan ? " " + pcBadge("use this", "ok") : "")).join("<br>"));
  } else {
    html += pcRow("LAN", `<span class="hint-warn">${esc(addresses.error || "no LAN address found")}</span>`);
  }
  const ts = addresses.tailscale;
  if (ts.ipv4 && ts.running) {
    html += pcRow("Tailscale", `<span class="pc-addr">${esc(ts.ipv4)}</span> ${pcBadge("connected", "ok")}`);
    if (ts.dns_name) html += pcRow("MagicDNS", `<span class="pc-addr">${esc(ts.dns_name)}</span>`);
  } else {
    html += pcRow("Tailscale", `<span class="dim">${esc(ts.note || "not available")}</span>`);
    if (!ts.cli_found) html += pcLinks([["tailscale_download", "Get Tailscale"]]);
  }
  html += addressAdvice();
  pcFill("#pc-addresses", html);
}

// ---------------------------------------------------------------- Step 2: apps on the Thor

/** Asks Rust which apps the Thor has (→ commands::pc_stream_thor_apps, adb `pm list packages`). */
async function pcThorApps() {
  pcFill("#pc-thor", `<span class="dim"><span class="spin"></span> Checking …</span>`);
  try {
    PCS.thor = await invoke("pc_stream_thor_apps");
  } catch (e) {
    PCS.thor = null;
    pcFill("#pc-thor", `<span class="hint-warn">Couldn't check: ${esc(e)}</span>`);
    renderChips();
    return;
  }
  renderThorApps();
  renderGames();
  renderChips();
}

function renderThorApps() {
  const thor = PCS.thor;
  if (!thor.connected) {
    pcFill("#pc-thor", `${pcBadge("Thor not connected", "warn")}<div class="pc-hint">${esc(thor.device)}</div>` +
      `<div class="pc-hint">Connect the Thor over USB to check Moonlight and to start games from here.</div>`);
    return;
  }
  let html = pcRow("Device", esc(thor.device));
  if (thor.error) html += `<div class="pc-hint warn">${esc(thor.error)}</div>`;
  const ml = thor.moonlight;
  html += pcRow("Moonlight", ml
    ? pcBadge(`installed${ml.version ? " " + ml.version : ""}`, "ok") + (ml.package !== "com.limelight" ? ` <span class="dim">${esc(ml.package)}</span>` : "")
    : pcBadge("not installed", "bad"));
  if (!ml) {
    html += `<div class="pc-hint">Moonlight plays the stream on the Thor. Install it from the Play Store (or F-Droid / GitHub), then click Check again.</div>`;
    html += pcLinks([["moonlight_play", "Play Store"], ["moonlight_fdroid", "F-Droid"], ["moonlight_releases", "GitHub"]]);
  }
  const ts = thor.tailscale;
  html += pcRow("Tailscale", ts ? pcBadge(`installed${ts.version ? " " + ts.version : ""}`, "ok") : pcBadge("not installed", ""));
  if (!ts) {
    html += `<div class="pc-hint">Only needed to play away from home. At home, the LAN is enough.</div>`;
    html += pcLinks([["tailscale_android", "Tailscale on Play Store"]]);
  } else {
    html += `<div class="pc-hint">ThorBro can't see whether Tailscale is switched on on the Thor. Open the Tailscale app there and check it's connected.</div>`;
  }
  pcFill("#pc-thor", html);
}

// ---------------------------------------------------------------- Step 3: Sunshine login (keychain)

/** Is a login stored and does it work? (→ commands::pc_stream_login_status). */
async function pcLoginStatus() {
  pcFill("#pc-login", `<span class="dim"><span class="spin"></span> Checking …</span>`);
  try {
    PCS.login = await invoke("pc_stream_login_status");
  } catch (e) {
    PCS.login = { state: "failed", username: null, sunshine_version: null, message: String(e) };
  }
  renderLogin();
  pcLoginChanged();
}

/** Called whenever the login state may have changed: reloads what needs the login. */
function pcLoginChanged() {
  PCS.pairings = null;
  PCS.apps = null;
  PCS.appsError = "";
  renderPairing();
  renderGames();
  if (PCS.login && PCS.login.state === "ok") {
    pcLoadPairings();
    pcLoadApps();
  }
}

/** Badge for a pcstream::admin::LoginState. */
function loginBadge(login) {
  switch (login.state) {
    case "ok": return pcBadge(`logged in as ${login.username}`, "ok");
    case "not_stored": return pcBadge("no login saved in ThorBro", "warn");
    case "no_login_yet": return pcBadge("Sunshine has no login yet", "warn");
    case "rejected": return pcBadge("login rejected", "bad");
    case "forbidden": return pcBadge("Sunshine refuses this PC", "bad");
    case "unreachable": return pcBadge("Sunshine web UI not reachable", "bad");
    case "keychain_error": return pcBadge("keychain problem", "bad");
    default: return pcBadge("login check failed", "bad");
  }
}

function renderLogin() {
  const login = PCS.login;
  let html = `<div>${loginBadge(login)}</div>`;
  if (login.sunshine_version) html += pcRow("Sunshine", esc(login.sunshine_version));
  if (login.message) html += `<div class="pc-hint ${login.state === "ok" ? "" : "warn"}">${esc(login.message)}</div>`;
  html += `<div class="pc-hint">Needed for pairing and the game list. ThorBro talks to Sunshine's admin API directly and keeps the login in your system's keychain, not in its settings file.</div>`;

  const showForm = login.state !== "ok" && login.state !== "unreachable" && login.state !== "no_login_yet";
  html += `<details class="pc-login-form" ${showForm ? "open" : ""}>
      <summary>${login.state === "ok" ? "Use a different login" : "Enter your Sunshine login"}</summary>
      <div class="pc-form">
        <label for="pc-user">Username</label><input type="text" id="pc-user" autocomplete="off" value="${esc(login.username || "")}">
        <label for="pc-pass">Password</label><input type="password" id="pc-pass" autocomplete="off">
      </div>
      <div class="pc-links"><button class="primary" id="pc-save-login">Test and save</button>
        ${login.username ? `<button id="pc-forget-login">Forget saved login</button>` : ""}</div>
    </details>`;

  const showCreate = login.state === "no_login_yet" || login.state === "rejected" || login.state === "not_stored";
  html += `<details class="pc-create" ${login.state === "no_login_yet" ? "open" : ""} ${showCreate ? "" : "hidden"}>
      <summary>Create a new Sunshine login</summary>
      <div class="pc-hint">This runs <code>sunshine --creds &lt;username&gt; &lt;password&gt;</code> on this PC.
• It <b>replaces</b> any existing Sunshine web UI login.
• Sunshine reads the login only when it starts, so it has to be restarted afterwards.
• The password is passed to Sunshine on its command line for that moment.
• On Windows this needs administrator rights; if it fails there, create the login in Sunshine's web UI instead.</div>
      <div class="pc-form">
        <label for="pc-new-user">Username</label><input type="text" id="pc-new-user" autocomplete="off">
        <label for="pc-new-pass">Password</label><input type="password" id="pc-new-pass" autocomplete="off">
      </div>
      <div class="pc-links"><button id="pc-create-login">Create login …</button></div>
    </details>`;

  const body = pcFill("#pc-login", html);
  $("#pc-save-login", body).onclick = pcSaveLogin;
  const forget = $("#pc-forget-login", body);
  if (forget) forget.onclick = pcForgetLogin;
  $("#pc-create-login", body).onclick = pcCreateLogin;
  $("#pc-pass", body).onkeydown = (e) => { if (e.key === "Enter") pcSaveLogin(); };
}

async function pcSaveLogin() {
  const username = $("#pc-user").value.trim();
  const password = $("#pc-pass").value;
  if (!username || !password) return toast("Enter username and password", true);
  $("#pc-save-login").disabled = true;
  try {
    PCS.login = await call("pc_stream_save_login", { username, password }); // → admin::save_login (tests first)
    toast("Login works and is saved in the keychain");
  } catch {
    $("#pc-save-login").disabled = false;
    return;
  }
  renderLogin();
  pcLoginChanged();
}

async function pcForgetLogin() {
  await call("pc_stream_forget_login").catch(() => {});
  await pcLoginStatus();
}

async function pcCreateLogin() {
  const username = $("#pc-new-user").value.trim();
  const password = $("#pc-new-pass").value;
  if (!username || !password) return toast("Enter a username and a password for the new login", true);
  const answer = await dialog({
    title: "Replace Sunshine's login?",
    body: "ThorBro will run 'sunshine --creds' and set this as Sunshine's web UI login. An existing login is replaced. Sunshine has to be restarted afterwards.",
    buttons: [["Cancel", null], ["Create login", "create", "primary"]],
  });
  if (answer !== "create") return;
  let message;
  try {
    message = await call("pc_stream_create_login", { username, password }); // → admin::create_login
  } catch {
    return;
  }
  const linux = PCS.pc && PCS.pc.os === "linux";
  const next = await dialog({
    title: "Login created",
    body: message + (linux ? " Restarting Sunshine ends a running stream." : " Restart the SunshineService (or reboot), then click Check again."),
    buttons: linux ? [["Later", null], ["Restart Sunshine now", "restart", "primary"]] : [["OK", null, "primary"]],
  });
  if (next === "restart") {
    await call("pc_stream_restart_sunshine").then(() => toast("Sunshine restarted")).catch(() => {});
    // Sunshine needs a moment before its web UI answers again.
    await new Promise((resolve) => setTimeout(resolve, 2500));
  }
  await pcRefresh();
}

// ---------------------------------------------------------------- Step 4: pairing

/** Polled by main.js while this page is visible: picks up new pairing requests from Moonlight. */
function pcPoll() {
  if (PCS.login && PCS.login.state === "ok" && !PCS.pairing && PCS.pairings && PCS.pairings.supported) pcLoadPairings(true);
}

/** Loads the waiting Moonlight clients (→ commands::pc_stream_pairings, GET /api/pin). */
async function pcLoadPairings(quiet = false) {
  let pairings;
  try {
    pairings = await invoke("pc_stream_pairings");
  } catch (e) {
    if (!quiet) toast(String(e), true);
    return;
  }
  const before = JSON.stringify(PCS.pairings);
  PCS.pairings = pairings;
  // Re-render only on a change, so typing a PIN isn't interrupted by the poll.
  if (JSON.stringify(pairings) !== before) renderPairing();
}

/** Which address to type into Moonlight's "Add PC", as HTML. */
function addressAdvice() {
  if (!PCS.pc) return "";
  const { addresses } = PCS.pc;
  const ts = addresses.tailscale;
  let html = `<div class="pc-hint"><b>Address for Moonlight</b> (only needed if the PC doesn't show up by itself):</div>`;
  html += pcRow("At home", addresses.recommended_lan
    ? `<span class="pc-addr">${esc(addresses.recommended_lan)}</span> <span class="dim">same Wi-Fi / LAN as the PC</span>`
    : `<span class="dim">no LAN address found</span>`);
  if (ts.running && ts.ipv4) {
    html += pcRow("Away", `<span class="pc-addr">${esc(ts.ipv4)}</span>${ts.dns_name ? ` <span class="dim">or</span> <span class="pc-addr">${esc(ts.dns_name)}</span>` : ""} <span class="dim">Tailscale on PC and Thor</span>`);
  } else {
    html += pcRow("Away", `<span class="dim">set up Tailscale on PC and Thor to play away from home</span>`);
  }
  html += `<div class="pc-hint">Moonlight finds PCs in the same local network by itself (mDNS). That discovery may not work over Tailscale: if the PC isn't listed, tap + in Moonlight and type the Tailscale address.
Moonlight keeps one entry per PC (by its host ID) and stores a LAN address and a manually added address separately; it tries all of them and prefers the LAN one. Adding the Tailscale address therefore doesn't replace the LAN entry.</div>`;
  return html;
}

/** "Open Help" button for a hint; section = id of a Help <details>. */
const helpLink = (label, section) => `<button type="button" class="link-btn" data-help="${section}">${esc(label)}</button>`;

/** The pairing box of the Thor app card: waiting devices, PIN, device name, Pair. Steps and addresses are in Help. */
function renderPairing() {
  let html = `<div class="pc-hint">Tap this PC in Moonlight on the Thor, then type the PIN it shows. ${helpLink("How pairing works", "help-pair")}</div>`;

  const login = PCS.login;
  if (!login || login.state !== "ok") {
    html += `<div class="pc-hint warn">Save a working Sunshine login first (Help, Sunshine on this PC). ${helpLink("Set up the login", "help-sunshine")}</div>`;
    pcFill("#pc-pair", html);
    return;
  }
  const pairings = PCS.pairings;
  if (!pairings) {
    html += `<div class="dim"><span class="spin"></span> Asking Sunshine for waiting devices …</div>`;
    pcFill("#pc-pair", html);
    return;
  }
  if (!pairings.supported) {
    html += `<div class="pc-hint warn">This Sunshine is older than v2026.906.222525 and can't list waiting devices. ThorBro sends the PIN the old way (without a pairing id); Sunshine then pairs the device that asked last. Updating Sunshine is recommended.</div>`;
  } else if (!pairings.pairings.length) {
    html += `<div class="pc-hint">${pcBadge("no device waiting", "")} Tap the PC in Moonlight on the Thor; it appears here within a few seconds.</div>`;
  } else {
    if (!pairings.pairings.some((p) => p.id === PCS.pairSelected)) PCS.pairSelected = pairings.pairings[0].id;
    html += `<div class="pc-hint">Waiting for a PIN:</div>` + pairings.pairings.map((p) =>
      `<label class="check"><input type="radio" name="pc-pairing" value="${esc(p.id)}" ${p.id === PCS.pairSelected ? "checked" : ""}>
        <span><b>${esc(p.name || "Moonlight")}</b> <span class="dim pc-addr">${esc(p.address)}</span></span></label>`).join("");
  }
  const canPair = !pairings.supported || pairings.pairings.length > 0;
  const selected = (pairings.pairings || []).find((p) => p.id === PCS.pairSelected);
  html += `<div class="pc-form">
      <label for="pc-pin">PIN</label><input type="text" id="pc-pin" class="pc-pin" inputmode="numeric" maxlength="4" autocomplete="off" ${canPair ? "" : "disabled"}>
      <label for="pc-pair-name">Device name</label><input type="text" id="pc-pair-name" maxlength="128" value="${esc((selected && selected.name) || "Thor")}" ${canPair ? "" : "disabled"}>
    </div>
    <div class="pc-links"><button class="primary" id="pc-pair-go" ${canPair && !PCS.pairing ? "" : "disabled"}>${PCS.pairing ? "Waiting for Moonlight …" : "Pair"}</button></div>`;
  const body = pcFill("#pc-pair", html);
  body.querySelectorAll('input[name="pc-pairing"]').forEach((radio) => (radio.onchange = () => { PCS.pairSelected = radio.value; }));
  $("#pc-pair-go", body).onclick = pcSubmitPin;
  $("#pc-pin", body).onkeydown = (e) => { if (e.key === "Enter") pcSubmitPin(); };
}

/** Sends the PIN (→ commands::pc_stream_submit_pin, POST /api/pin). Waits for Moonlight. */
async function pcSubmitPin() {
  const pin = $("#pc-pin").value.trim();
  const name = $("#pc-pair-name").value.trim();
  if (!/^\d{4}$/.test(pin)) return toast("The PIN has 4 digits; Moonlight shows it on the Thor", true);
  if (!name) return toast("Enter a name for the Thor", true);
  const supported = PCS.pairings && PCS.pairings.supported;
  const pairingId = supported ? PCS.pairSelected : null;
  if (supported && !pairingId) return toast("Tap the PC in Moonlight first", true);
  PCS.pairing = true;
  $("#pc-pair-go").disabled = true;
  $("#pc-pair-go").textContent = "Waiting for Moonlight …";
  let paired = false;
  try {
    paired = await call("pc_stream_submit_pin", { pairingId, pin, name });
  } catch {
    // The error was shown as a toast.
  } finally {
    PCS.pairing = false;
  }
  if (paired) toast("Paired. Moonlight on the Thor can now stream from this PC.");
  else toast("Pairing didn't finish: wrong PIN, or Moonlight cancelled or timed out. Start again in Moonlight.", true);
  PCS.pairings = null;
  renderPairing();
  pcLoadPairings(true);
}

// ---------------------------------------------------------------- Step 5: game list and Play on Thor

$("#pc-games-reload").onclick = () => pcLoadApps();

/** Loads Sunshine's app names (→ commands::pc_stream_apps, GET /api/apps). */
async function pcLoadApps() {
  if (!PCS.login || PCS.login.state !== "ok") return renderGames();
  PCS.apps = null;
  PCS.appsError = "";
  renderGames();
  try {
    PCS.apps = await invoke("pc_stream_apps");
  } catch (e) {
    PCS.apps = [];
    PCS.appsError = String(e);
  }
  renderGames();
}

/** Why "Play on Thor" can't work right now, or "" if it can. */
function playBlocker() {
  if (!PCS.thor || !PCS.thor.connected) return "Connect the Thor over USB to start games from here.";
  if (!PCS.thor.moonlight) return "Install Moonlight on the Thor first.";
  if (PCS.pc && !PCS.pc.sunshine.server) return "Sunshine isn't answering on this PC.";
  return "";
}

/** The experimental display picker; ids come from the scrcpy screen detection (stream.js). */
function displayPicker() {
  const displays = (typeof STREAM !== "undefined" && STREAM.displays) || [];
  const options = [`<option value="">Moonlight's default</option>`].concat(displays.map((d) =>
    `<option value="${d.id}" ${String(d.id) === PCS.display ? "selected" : ""}>${esc(d.label)} (display ${d.id})</option>`));
  return `<details class="pc-display">
      <summary>Experimental: choose the Thor screen</summary>
      <div class="pc-hint">Starts Moonlight with <code>am start --display &lt;id&gt;</code>. Untested on the Thor: Moonlight may ignore it, or the controls may not reach the stream.</div>
      <div class="pc-links"><select id="pc-display">${options.join("")}</select>
        <button id="pc-display-detect">${displays.length ? "Detect again" : "Detect screens"}</button></div>
    </details>`;
}

/** The "Play on Thor" card: one row per Sunshine app. Picker and Moonlight hints go to Help (renderGamesHelp). */
function renderGames() {
  const login = PCS.login;
  let html = "";
  if (!login || login.state !== "ok") {
    html += `<div class="pc-hint warn">The game list comes from Sunshine; save a working Sunshine login first. ${helpLink("Set up the login", "help-sunshine")}</div>`;
    pcFill("#pc-games", html);
    return;
  }
  renderGamesHelp();
  const blocker = playBlocker();
  if (blocker) html += `<div class="pc-hint warn">${esc(blocker)}</div>`;
  if (PCS.apps === null) {
    html += `<div class="dim"><span class="spin"></span> Loading games …</div>`;
  } else if (PCS.appsError) {
    html += `<div class="pc-hint warn">${esc(PCS.appsError)}</div>`;
  } else if (!PCS.apps.length) {
    html += `<div class="pc-hint">Sunshine has no apps. Add some in Sunshine's web UI (Applications).</div>`;
  } else {
    html += `<div class="pc-apps">${PCS.apps.map((name, index) =>
      `<div class="pc-app"><span class="name" title="${esc(name)}">${esc(name)}</span>
        <button class="primary" data-play="${index}" ${blocker ? "disabled" : ""}>Play on Thor</button></div>`).join("")}</div>`;
  }
  const body = pcFill("#pc-games", html);
  body.querySelectorAll("[data-play]").forEach((button) => (button.onclick = () => pcPlay(PCS.apps[Number(button.dataset.play)], button)));
}

/** Help section "Starting games": the experimental screen picker and what Moonlight may show. */
function renderGamesHelp() {
  let html = displayPicker();
  html += `<details class="pc-dialogs"><summary>What Moonlight may show on the Thor</summary>
      <div class="pc-hint">• <b>PC not found</b>: Moonlight doesn't know this PC yet. Add it in Moonlight (see Addresses of this PC) and pair it.
• <b>PC not paired</b>: pair first (Stream, Pair Moonlight).
• <b>Computer is offline</b>: the Thor can't reach the PC. Same Wi-Fi? Tailscale on both? Firewall?
• <b>Provided App is not valid</b>: Moonlight only knows the games from the last time you opened this PC's game list in Moonlight. Open it once on the Thor, then try again.
• <b>Are you sure you want to quit the running app?</b>: another game is running on the PC. Confirm on the Thor to switch.</div>
    </details>`;
  const body = pcFill("#pc-games-help", html);
  $("#pc-display", body).onchange = (e) => { PCS.display = e.target.value; };
  $("#pc-display-detect", body).onclick = async () => {
    if (!S.connected) return toast("Thor is not connected", true);
    await detectDisplays(); // stream.js: scrcpy --list-displays
    if (STREAM.detectError) toast(`Couldn't detect the screens: ${STREAM.detectError}`, true);
    renderGamesHelp();
    $("#pc-games-help .pc-display").open = true;
  };
}

/** Starts one Sunshine app on the Thor (→ commands::pc_stream_play → thor::play). */
async function pcPlay(appName, button) {
  button.disabled = true;
  const display = PCS.display === "" ? null : Number(PCS.display);
  try {
    const result = await call("pc_stream_play", { appName, display });
    toast(result.pc_busy
      ? `Starting "${appName}" on the Thor. Another app runs on the PC; confirm on the Thor if Moonlight asks.`
      : `Starting "${appName}" on the Thor. Look at the Thor.`);
  } catch {
    // shown as a toast
  } finally {
    button.disabled = false;
  }
}

// ---------------------------------------------------------------- Step 6: stream profile, step 7: checklist links

// Static link buttons (Help, low-latency checklist); dynamic ones are wired in pcFill.
document.querySelectorAll("#help-latency [data-link]").forEach((button) =>
  (button.onclick = () => call("pc_stream_open_link", { id: button.dataset.link }).catch(() => {})));

$("#pc-profile-show").onclick = async () => {
  let answer;
  try {
    answer = await call("pc_stream_profile"); // → pcstream::stream_profile → profile::build_profile
  } catch {
    return;
  }
  await modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>Stream profile</h2></div>`));
    const body = el(`<div class="modal-body"></div>`);
    if (answer.note) body.append(el(`<p class="pc-hint warn">${esc(answer.note)}</p>`));
    const text = el(`<textarea class="pc-profile-json" readonly rows="16" spellcheck="false"></textarea>`);
    text.value = answer.json;
    body.append(text);
    body.append(el(`<p class="pc-hint">Select all and copy if you want to keep it. "Send to Thor app" hands it to the ThorBro app on the Thor.</p>`));
    box.append(body);
    const foot = el(`<div class="modal-foot"><button class="primary">Close</button></div>`);
    foot.firstChild.onclick = () => close(null);
    box.append(foot);
    setTimeout(() => { text.focus(); text.select(); }, 0);
  }, { wide: true });
};

/** Shows the result of "Send to Thor app" under the buttons; kind = "" | "warn". */
function pcProfileStatus(text, kind = "") {
  const status = $("#pc-profile-status");
  status.textContent = text;
  status.classList.toggle("warn", kind === "warn");
  status.classList.toggle("hidden", !text);
}

// "Send to Thor app" (→ commands::pc_stream_send_profile → thor::send_profile): `am start` with the
// profile as base64 extra. Android gives no answer from the app, so this can only say "sent", not
// "accepted"; the app shows the result on its own Status tab.
$("#pc-profile-send").onclick = async () => {
  const button = $("#pc-profile-send");
  button.disabled = true;
  pcProfileStatus("Sending the stream profile to the ThorBro app…");
  try {
    const result = await call("pc_stream_send_profile");
    const games = result.games === 1 ? "1 game" : `${result.games} games`;
    pcProfileStatus(
      `Stream profile sent to the ThorBro app (${games}). ThorBro can't see whether the app accepted it: ` +
      `look at the app's Status tab on the Thor. If the app refuses the profile, it says "rejected" there and keeps the previous one.` +
      (result.note ? `\n${result.note}` : ""),
      result.note ? "warn" : "");
  } catch (e) {
    pcProfileStatus(`Not sent: ${e}`, "warn");
  } finally {
    button.disabled = false;
  }
};
