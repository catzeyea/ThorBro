// ThorBro UI – tabs and device detection.
// pollDevice() runs every 3 s (started in main.js) and reloads the file manager and the
// save cards when the Thor is plugged in or out.
"use strict";

// ---------------------------------------------------------------- Tabs

document.querySelectorAll(".tab").forEach((button) => (button.onclick = () => showTab(button.dataset.tab)));

/** Switches to "files", "saves" or "stream". */
function showTab(tab) {
  S.tab = tab;
  document.querySelectorAll(".tab").forEach((button) => button.classList.toggle("active", button.dataset.tab === tab));
  document.querySelectorAll(".page").forEach((page) => page.classList.toggle("active", page.id === "page-" + tab));
  if (tab === "saves") loadGames();
  if (tab === "stream") streamStatus();
}

// ---------------------------------------------------------------- Device status

/**
 * Asks Rust whether a Thor is connected and updates the badge in the top bar.
 * On a change (or with force), the file manager and save cards are refreshed.
 */
async function pollDevice(force = false) {
  let device;
  // → commands::device → adb::device_status (runs `adb devices`)
  try { device = await invoke("device"); } catch (e) { invoke("log", { msg: "device: " + e }); return; }
  const changed = device.connected !== S.connected || force;
  S.connected = device.connected;
  S.deviceMsg = device.message;
  $("#device").classList.toggle("on", device.connected);
  $("#device-text").textContent = device.connected ? `Connected: ${device.message}` : device.message;
  if (changed) {
    if (device.connected) await loadPlaces(true);
    else { S.entries = []; FM.status = "offline"; clearDirCache(); renderFiles(); updateLibHint(); }
    if (S.tab === "saves") refreshCards();
    if (S.tab === "stream" && PCS.sub === "pc") pcThorApps();
  }
}
$("#device").onclick = () => pollDevice(true);
