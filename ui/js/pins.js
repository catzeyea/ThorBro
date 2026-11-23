// ThorBro UI – pinned Thor folders: the pin button in the file manager's path bar (Ctrl+D, see
// files.js), the "pinned" card in its sidebar and the "pinned" row on the Start tab.
// Pins are Thor folders (storage + path). Rust keeps them in settings.json in pin order, with a
// cap (commands::pins / add_pin / remove_pin, settings::MAX_PINS). Whether a pinned folder still
// exists is asked with one adb call for all of them (commands::pins_found): when the Thor
// connects, after pinning, when the Start tab opens, and when listing a pinned folder fails.
"use strict";

/** The pin icon from the design (DESIGN-UPDATE-3): outline = not pinned, filled = pinned. */
function pinIcon(size, filled) {
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="${filled ? "currentColor" : "none"}" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 3h6l-1 6 3 3v2H7v-2l3-3z"/><path d="M12 14v7"/></svg>`;
}

const PINS = {
  list: [],         // [{ storage, path }] in pin order (from Rust)
  found: new Map(), // path → true / false once checked; not checked yet counts as found
  checkSeq: 0,      // only the newest existence check is applied
};

/** The pin for this Thor folder, or null. */
function pinOf(path) {
  return PINS.list.find((pin) => pin.path === path) || null;
}

/** Shown dimmed with "not found": the Thor is not connected or the folder is gone. */
function pinMissing(pin) {
  return !S.connected || PINS.found.get(pin.path) === false;
}

/** Display name: the folder name, or the storage's name for a pinned storage root. */
function pinName(pin) {
  if (pin.path !== pin.storage) return basename(pin.path);
  const place = S.places.find((p) => !p.games && p.path === pin.storage);
  return place ? place.label : basename(pin.path);
}

function pinTitle(pin) {
  if (!S.connected) return `${pin.path} (Thor not connected)`;
  return pinMissing(pin) ? `${pin.path} (not found on the Thor)` : pin.path;
}

/** Loads the pins from settings.json (at start-up), then checks which folders exist. */
async function loadPins() {
  PINS.list = await call("pins", {}, true).catch(() => []); // → commands::pins
  renderPins();
  checkPins();
}

/** Asks the Thor which pinned folders exist and redraws. Errors keep the last known state. */
async function checkPins() {
  if (!S.connected || !PINS.list.length) { renderPins(); return; }
  const seq = ++PINS.checkSeq;
  const paths = PINS.list.map((pin) => pin.path);
  let found;
  try {
    found = await invoke("pins_found", { paths }); // → commands::pins_found → files::dirs_exist
  } catch (e) {
    invoke("log", { msg: "pins_found: " + e }).catch(() => {});
    return;
  }
  if (seq !== PINS.checkSeq) return;
  paths.forEach((path, i) => PINS.found.set(path, found[i]));
  renderPins();
}

/**
 * Called by files.js when a folder was listed: a listed pinned folder exists; a failed listing
 * of one is checked again (the error may just be adb).
 */
function notePinListing(path, ok) {
  if (!pinOf(path)) return;
  if (!ok) { checkPins(); return; }
  if (PINS.found.get(path) === true) return;
  PINS.found.set(path, true);
  renderPins();
}

/** Pins or unpins a Thor folder (default: the open one). Pin button and Ctrl+D. */
async function togglePin(path = S.cwd) {
  if (!S.connected) { toast("Thor is not connected", true); return; }
  const pinned = pinOf(path);
  const storage = pinned ? pinned.storage : storageOf(path);
  if (!storage) { toast("This folder can't be pinned", true); return; }
  try {
    // → commands::add_pin / remove_pin (settings.json). add_pin fails with a short message at the cap.
    PINS.list = await call(pinned ? "remove_pin" : "add_pin", { storage, path });
  } catch (_) {
    return; // already shown as toast by call()
  }
  if (pinned) PINS.found.delete(path);
  else if (path === S.cwd && FM.status === "ready") PINS.found.set(path, true);
  renderPins();
  if (!pinned && !PINS.found.has(path)) checkPins();
}

/** Unpins one pin (the "×" in the sidebar, right-click "Unpin"). */
async function unpin(pin) {
  try {
    PINS.list = await call("remove_pin", { storage: pin.storage, path: pin.path }); // → commands::remove_pin
  } catch (_) {
    return;
  }
  PINS.found.delete(pin.path);
  renderPins();
}

/** Opens a pinned folder in the file manager. A missing one only explains why it can't. */
function openPin(pin) {
  if (!S.connected) { toast("Thor is not connected", true); return; }
  if (pinMissing(pin) || !S.places.some((place) => place.path === pin.storage)) {
    toast(`“${pinName(pin)}” was not found on the Thor. Unpin it or check the folder.`, true);
    checkPins(); // maybe it is back
    return;
  }
  showTab("files");
  selectPlace(pin.storage, pin.path);
}

// ---------------------------------------------------------------- Rendering

/** Redraws the pin button, the sidebar card and the Start row. */
function renderPins() {
  renderPinButton();
  renderSidePins();
  renderStartPins();
}

/** The pin button next to "Refresh": shows whether the open folder is pinned. */
function renderPinButton() {
  const button = $("#pin");
  const pinned = !!pinOf(S.cwd);
  const label = pinned ? "Unpin this folder" : "Pin this folder";
  button.innerHTML = pinIcon(20, pinned);
  button.classList.toggle("on", pinned);
  button.setAttribute("aria-label", label);
  button.setAttribute("aria-pressed", String(pinned));
  button.title = `${label} (Ctrl+D)`;
  button.disabled = !S.connected || (!pinned && !storageOf(S.cwd));
}

/** Content of a pin row or chip: icon, name and, if missing, "not found". */
function pinContent(pin) {
  const parts = [el(`<span class="pin-ic">${pinIcon(16, true)}</span>`), el(`<span class="pin-name"></span>`)];
  parts[1].textContent = pinName(pin);
  if (pinMissing(pin)) parts.push(el(`<span class="pin-note">not found</span>`));
  return parts;
}

/** The "pinned" card in the file manager sidebar. Empty: only the hint line. */
function renderSidePins() {
  $("#side-pins").replaceChildren(...PINS.list.map((pin) => {
    const row = el(`<div class="side-pin"><button type="button" class="side-item pin-item"></button><button type="button" class="pin-x" title="Unpin">×</button></div>`);
    const [open, remove] = row.children;
    row.classList.toggle("missing", pinMissing(pin));
    open.append(...pinContent(pin));
    open.title = pinTitle(pin);
    open.onclick = () => openPin(pin);
    remove.setAttribute("aria-label", `Unpin ${pinName(pin)}`);
    remove.onclick = () => unpin(pin);
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      showMenu(e.clientX, e.clientY, [ // files-actions.js
        { label: "Open", run: () => openPin(pin), disabled: pinMissing(pin) },
        { label: "Unpin", run: () => unpin(pin) },
      ]);
    });
    return row;
  }));
}

/** The "pinned" row on the Start tab, one chip per pin. Hidden when there are no pins. */
function renderStartPins() {
  const row = $("#start-pins");
  row.classList.toggle("hidden", !PINS.list.length);
  $("#start-pin-chips").replaceChildren(...PINS.list.map((pin) => {
    const chip = el(`<button type="button" class="pin-chip"></button>`);
    chip.classList.toggle("missing", pinMissing(pin));
    chip.append(...pinContent(pin));
    chip.title = pinTitle(pin);
    chip.onclick = () => openPin(pin);
    return chip;
  }));
}

$("#pin").onclick = () => togglePin();
