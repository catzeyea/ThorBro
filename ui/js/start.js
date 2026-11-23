// ThorBro UI – the Start tab (drop zone, games folder, library tiles), the location sidebar of the
// file manager, its "More" menu and the keyboard shortcuts panel. Pinned folders (on the Start tab
// and in the sidebar) are drawn by pins.js.
// Nothing here loads data or calls Rust: everything comes from what loadPlaces() / refreshLibs()
// (files.js) already put into S.places and S.libs, and every click goes through the existing
// navigation (showTab, selectPlace) or presses an existing button. Looks are in style.css.
"use strict";

// ---------------------------------------------------------------- Library data for display

/** Maker names left out of tile and sidebar names ("Nintendo GameCube" → "GameCube"). */
const SYSTEM_MAKER_RE = /^(Nintendo|Sony|Sega|Microsoft|Atari|NEC|SNK|Bandai|Coleco|Mattel|Commodore|Panasonic|Philips) +/;

/**
 * Short display name of a system folder: the known system's name without "(…)" and without the
 * maker (kept when only a number would remain, as in "Nintendo 64"); else the folder name.
 */
function systemTitle(folder, system) {
  const full = (system.full || "").replace(/\s*\(.*\)\s*$/, "").trim();
  const short = full.replace(SYSTEM_MAKER_RE, "");
  return (/[a-z]/i.test(short) ? short : full) || folder;
}

/**
 * The non-empty system folders of all libraries, in library order and A–Z per library:
 * [{ lib, folder, path, title, count }].
 */
function librarySystems() {
  return S.libs.flatMap((lib) => Object.entries(lib.systems)
    .filter(([, system]) => system.count > 0)
    .map(([folder, system]) => ({ lib, folder, path: joinPath(lib.path, folder), title: systemTitle(folder, system), count: system.count }))
    .sort((a, b) => NAME_COLLATOR.compare(a.title, b.title)));
}

/** A Thor path as the user knows it: "Internal storage / ROMs" for "/sdcard/ROMs". */
function friendlyPath(path) {
  const storage = S.places
    .filter((place) => !place.games && (path === place.path || path.startsWith(place.path + "/")))
    .sort((a, b) => b.path.length - a.path.length)[0];
  if (!storage) return path;
  const rest = path.slice(storage.path.length).split("/").filter(Boolean);
  return [storage.label, ...rest].join(" / ");
}

/** Opens a library folder in the file manager (same as picking it in the location drop-down). */
function openLibraryFolder(lib, path) {
  showTab("files");
  selectPlace(lib.path, path);
}

/** Redraws everything here that shows libraries. Called when S.libs or the connection changes. */
function renderLibraryViews() {
  renderStart();
  renderSidebar();
}

// ---------------------------------------------------------------- Start tab

function renderStart() {
  const lib = S.libs[0];
  $("#start-folder").textContent = !S.connected ? "Thor not connected" : lib ? friendlyPath(lib.path) : "Not set up yet";
  const tiles = $("#start-tiles");
  const systems = S.connected ? librarySystems() : [];
  const several = S.libs.length > 1;
  tiles.replaceChildren(...systems.map((item) => {
    const tile = el(`<button type="button" class="tile"><span class="tile-name"></span><span class="tile-folder"></span><span class="tile-count"></span></button>`);
    tile.querySelector(".tile-name").textContent = item.title;
    tile.querySelector(".tile-folder").textContent = several ? `${item.lib.label} / ${item.folder}/` : `${item.folder}/`;
    tile.querySelector(".tile-count").textContent = `${item.count} ${item.count === 1 ? "game" : "games"}`;
    tile.title = item.path;
    tile.onclick = () => openLibraryFolder(item.lib, item.path);
    return tile;
  }));
  let empty = "";
  if (!S.connected) empty = "Connect your Thor to see its libraries.";
  else if (!S.libs.length) empty = "No games folder found yet. Set one up in Files with “Set games folder”.";
  else if (!systems.length) empty = "Your games folder has no games yet. Drop some above.";
  $("#start-tiles-empty").textContent = empty;
  $("#start-tiles-empty").classList.toggle("hidden", !empty);
  renderStartPins(); // pins.js
}

// "Choose files" is the file manager's "Send files …": same dialog, same sorting, and the
// progress bar is on the Files tab, so that is where it happens.
$("#start-choose").onclick = () => {
  showTab("files");
  $("#btn-send-files").click();
};

// ---------------------------------------------------------------- Files sidebar

function sideButton(label, count, selected, onClick, title) {
  const button = el(`<button type="button" class="side-item"><span class="side-name"></span></button>`);
  button.querySelector(".side-name").textContent = label;
  if (count !== null) button.append(el(`<span class="side-count">${count}</span>`));
  button.classList.toggle("sel", selected);
  if (selected) button.setAttribute("aria-current", "true");
  button.title = title;
  button.onclick = onClick;
  return button;
}

/**
 * Storages and libraries next to the file list. A library shows its root ("All games", or its
 * name when there are several) and its non-empty system folders; the most specific entry that
 * contains the open folder is highlighted.
 */
function renderSidebar() {
  const connected = S.connected;
  const storages = connected ? S.places.filter((place) => !place.games) : [];
  const openStorage = connected ? storageOf(S.cwd) : null;
  $("#side-storage").replaceChildren(...storages.map((place) =>
    sideButton(place.label, null, place.path === openStorage, () => selectPlace(place.path), place.path)));

  const entries = [];
  if (connected) {
    const systems = librarySystems();
    for (const lib of S.libs) {
      entries.push({ lib, path: lib.path, label: S.libs.length > 1 ? lib.label : "All games", count: lib.total });
      for (const item of systems) if (item.lib === lib) entries.push({ lib, path: item.path, label: item.title, count: item.count });
    }
  }
  const containing = entries.filter((entry) => S.cwd === entry.path || S.cwd.startsWith(entry.path + "/"));
  const current = containing.sort((a, b) => b.path.length - a.path.length)[0];
  $("#side-libs").replaceChildren(...entries.map((entry) =>
    sideButton(entry.label, entry.count, entry === current, () => selectPlace(entry.lib.path, entry.path), entry.path)));
  $("#side-libs").closest(".side-card").classList.toggle("hidden", !entries.length);
  renderSidePins(); // pins.js
  renderPinButton();
}

// ---------------------------------------------------------------- Files toolbar: "More" menu and "?" panel

const MORE = { button: $("#btn-more"), pop: $("#more-pop") };

function setMoreOpen(open) {
  MORE.pop.classList.toggle("hidden", !open);
  MORE.button.setAttribute("aria-expanded", String(open));
}

MORE.button.onclick = () => setMoreOpen(MORE.pop.classList.contains("hidden"));
// The items keep their own click handlers (files-actions.js, esde.js); the menu just closes after.
MORE.pop.addEventListener("click", (e) => { if (e.target.closest("button")) setMoreOpen(false); });

/**
 * Keyboard shortcuts panel (the list itself is in index.html; it names only keys files.js handles).
 * Hover or keyboard focus shows it while it lasts; a click pins it open until the next click,
 * Escape or a click elsewhere.
 */
const KEYS = { wrap: $("#keys"), button: $("#btn-keys"), panel: $("#keys-panel"), pinned: false };

function setKeysOpen(open) {
  if (!open) KEYS.pinned = false;
  KEYS.panel.classList.toggle("hidden", !open);
  KEYS.button.setAttribute("aria-expanded", String(open));
}

KEYS.wrap.addEventListener("mouseenter", () => setKeysOpen(true));
KEYS.wrap.addEventListener("mouseleave", () => { if (!KEYS.pinned && !KEYS.wrap.contains(document.activeElement)) setKeysOpen(false); });
KEYS.wrap.addEventListener("focusin", () => setKeysOpen(true));
KEYS.wrap.addEventListener("focusout", (e) => { if (!KEYS.pinned && !KEYS.wrap.contains(e.relatedTarget)) setKeysOpen(false); });
KEYS.button.onclick = () => {
  if (KEYS.pinned) setKeysOpen(false);
  else { setKeysOpen(true); KEYS.pinned = true; }
};

document.addEventListener("mousedown", (e) => {
  if (!$("#more").contains(e.target)) setMoreOpen(false);
  if (!KEYS.wrap.contains(e.target)) setKeysOpen(false);
});
// Escape closes an open menu or panel first (capture phase: before the file list clears the selection).
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  let target = null;
  if (!MORE.pop.classList.contains("hidden")) { setMoreOpen(false); target = MORE.button; }
  else if (!KEYS.panel.classList.contains("hidden")) { setKeysOpen(false); target = KEYS.button; }
  else return;
  if (document.activeElement !== target && target.closest("#page-files").contains(document.activeElement)) target.focus();
  e.preventDefault();
  e.stopPropagation();
}, true);

renderLibraryViews();
