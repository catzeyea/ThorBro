// ThorBro UI – tabs and device detection.
// pollDevice() runs every 3 s (started in main.js), updates the header (status pill and the
// ThorBro next to it, whose mood comes from status.js) and reloads the file manager, the save
// cards and the Start page status when the Thor is plugged in or out.
"use strict";

// ---------------------------------------------------------------- Tabs

document.querySelectorAll(".tab").forEach((button) => (button.onclick = () => showTab(button.dataset.tab)));

/** Switches to "start", "files", "saves" or "stream". */
function showTab(tab) {
  S.tab = tab;
  document.querySelectorAll(".tab").forEach((button) => button.classList.toggle("active", button.dataset.tab === tab));
  document.querySelectorAll(".page").forEach((page) => page.classList.toggle("active", page.id === "page-" + tab));
  if (tab === "start") { renderStart(); checkPins(); statusTick(); } // pins.js: pinned folders may have changed; status.js
  if (tab === "saves") loadGames();
  if (tab === "stream") streamStatus();
}

// ---------------------------------------------------------------- Device status

/** The poll that is running right now (or null). */
let devicePoll = null;

/**
 * Asks Rust whether a Thor is connected and updates the badge in the top bar.
 * On a change (or with force), the file manager and save cards are refreshed.
 * While a poll is still running, a timer tick doesn't start another one but gets the running one;
 * a forced poll (click on the pill, a drop) runs right after it.
 */
function pollDevice(force = false) {
  if (devicePoll) return force ? devicePoll.then(() => pollDevice(true)) : devicePoll;
  devicePoll = pollDeviceOnce(force).finally(() => { devicePoll = null; });
  return devicePoll;
}

async function pollDeviceOnce(force) {
  let device;
  // → commands::device → adb::device_status (runs `adb devices`)
  try { device = await invoke("device"); } catch (e) { invoke("log", { msg: "device: " + e }); return; }
  const changed = device.connected !== S.connected || force;
  S.connected = device.connected;
  S.deviceMsg = device.message;
  renderDeviceStatus();
  if (changed) renderPool(); // pool.js: its actions need a connected Thor
  if (changed) {
    statusConnectionChanged(); // status.js: new readings for the Start page and the mood
    if (device.connected) { await loadPlaces(true); checkPins(); } // checkPins: pins.js
    else { S.entries = []; FM.status = "offline"; clearDirCache(); renderFiles(); updateLibHint(); }
    if (S.tab === "saves") refreshCards();
    if (S.tab === "stream" && PCS.sub !== "mirror") pcThorApps();
  }
}
$("#device").onclick = () => pollDevice(true);

/**
 * Shows S.connected / S.deviceMsg in the header: the pill ("Connected: <model>" or "Not connected",
 * the reason in its tooltip) and the ThorBro next to it, in the current mood (status.js). Runs on
 * every poll, so it switches live.
 */
function renderDeviceStatus() {
  const on = S.connected;
  $("#status").classList.toggle("on", on);
  $("#device").classList.toggle("on", on);
  $("#device-text").textContent = on ? `Connected: ${S.deviceMsg}` : "Not connected";
  // The pill text may be cut off with "…" in small windows, so the tooltip has it in full.
  $("#device").title = on ? `Connected: ${S.deviceMsg}\nClick to reconnect.` : `${S.deviceMsg}\nClick to check again.`;
  renderHeaderMood(); // status.js
}
