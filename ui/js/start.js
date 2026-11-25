// ThorBro UI – the Start tab's games (Update 9: one block per console with emulator chips and a
// square cover grid), the location sidebar of the file manager and the keyboard shortcuts panel.
// Pinned folders (on the Start tab and in the sidebar) are drawn by pins.js, the Thor status and
// ThorBro's mood at the top of the Start tab by status.js. The games come from the catalog
// (catalog.js → commands::game_catalog), the sidebar from S.places / S.libs (files.js).
// Looks are in style.css.
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

/** Redraws everything here that shows libraries. Called when S.libs or the connection changes. */
function renderLibraryViews() {
  renderStart();
  renderSidebar();
  loadCatalog(); // catalog.js: games, covers and emulators for Start (redraws Start when done)
}

// ---------------------------------------------------------------- Start tab: games by console

/** Covers shown per console before "Show all". */
const COVER_LIMIT = 12;

/** Console ids whose block shows all covers. */
const expandedConsoles = new Set();

/** The games section of Start: one block per console, then "+ Add a console or a folder". */
function renderStart() {
  const list = $("#start-consoles");
  const data = S.connected ? CAT.data : null;
  const blocks = data ? data.consoles : [];
  list.replaceChildren(...blocks.map(consoleBlock));
  if (S.connected && data) {
    const add = el(`<button type="button" class="add-console">+ Add a console or a folder</button>`);
    add.onclick = addConsoleDialog;
    list.append(add);
  }
  let empty = "";
  if (!S.connected) empty = "Connect your Thor to see its games.";
  else if (!data && CAT.error) empty = `Couldn't read the games on your Thor: ${CAT.error}`;
  else if (!data) empty = "Looking at the games on your Thor …";
  else if (!blocks.length && !S.libs.length) empty = "No games folder found yet. Set one up in Files with “Set games folder”.";
  else if (!blocks.length) empty = "Your games folder has no games yet. Drop some above.";
  $("#start-games-empty").textContent = empty;
  $("#start-games-empty").classList.toggle("hidden", !empty);
  renderStartPins(); // pins.js
}

/** One console: heading with the game count, emulator chips, the square cover grid. */
function consoleBlock(block) {
  const title = systemTitle(block.folder || block.id, { full: block.full });
  const section = el(`<section class="console">
      <div class="console-head"><h3></h3><span class="console-count"></span></div>
      <div class="emu-row"><span class="label">emulators</span></div>
      <div class="covers"></div></section>`);
  section.dataset.console = block.id;
  section.querySelector("h3").textContent = title;
  section.querySelector(".console-count").textContent = `${block.games.length} ${block.games.length === 1 ? "game" : "games"}`;

  const row = section.querySelector(".emu-row");
  for (const id of block.emulators) row.append(emulatorChip(id));
  if (!block.emulators.length) row.append(el(`<span class="emu-none">no emulator with a profile installed</span>`));
  const add = el(`<button type="button" class="emu-chip add">+ Add emulator</button>`);
  add.onclick = () => addEmulatorDialog(block, title);
  row.append(add);

  const expanded = expandedConsoles.has(block.id);
  const shown = expanded ? block.games : block.games.slice(0, COVER_LIMIT);
  const hasEmulators = block.emulators.length > 0;
  section.querySelector(".covers").replaceChildren(...shown.map((game) => coverTile(game, hasEmulators)));
  if (block.games.length > COVER_LIMIT) {
    const more = el(`<button type="button" class="link-btn show-all"></button>`);
    more.textContent = expanded ? "Show fewer" : `Show all ${block.games.length}`;
    more.onclick = () => {
      if (expanded) expandedConsoles.delete(block.id);
      else expandedConsoles.add(block.id);
      renderStart();
    };
    section.append(more);
  }
  return section;
}

/** Chip of an installed emulator: name plus "N games · M saves" (games only if it brings its own). */
function emulatorChip(id) {
  const emulator = CAT.data.emulators.find((item) => item.id === id);
  const chip = el(`<span class="emu-chip"><span class="emu-name"></span><span class="emu-count"></span></span>`);
  chip.querySelector(".emu-name").textContent = emulator.name;
  const saves = `${emulator.saves.length} ${emulator.saves.length === 1 ? "save" : "saves"}`;
  const games = emulator.games === null ? "" : `${emulator.games} ${emulator.games === 1 ? "game" : "games"} · `;
  chip.querySelector(".emu-count").textContent = emulator.error ? "data folder not found" : games + saves;
  chip.title = emulator.error ? `${emulator.package}\n${emulator.error}` : emulator.package;
  chip.classList.toggle("broken", !!emulator.error);
  return chip;
}

/** One game: square cover (iiSU icon or initials), title, the emulators that have a save. */
function coverTile(game, hasEmulators) {
  const tile = el(game.rom ? `<button type="button" class="cover-tile"></button>` : `<div class="cover-tile"></div>`);
  tile.append(coverElement(game.title, game.art));
  const name = el(`<span class="cover-title"></span>`);
  name.textContent = game.title;
  const sub = el(`<span class="cover-sub"></span>`);
  sub.textContent = game.saves.length ? game.saves.map(emulatorName).join(" · ") : hasEmulators ? "no save yet" : "";
  tile.append(name, sub);
  tile.title = game.rom || `${game.title} (comes with ${game.saves.map(emulatorName).join(", ") || "the emulator"})`;
  if (game.rom) tile.onclick = () => openGameInFiles(game);
  return tile;
}

/** Opens the folder of a game's ROM in Files. */
function openGameInFiles(game) {
  const lib = S.libs.find((item) => item.path === game.lib);
  showTab("files");
  if (lib) selectPlace(lib.path, dirname(game.rom));
}

/** "+ Add a console or a folder": libraries come from Files ("Set games folder"). */
async function addConsoleDialog() {
  const choice = await dialog({
    title: "Add a console or a folder",
    html: true,
    body: `ThorBro shows the games in the ES-DE <b>ROMs</b> folders on your Thor, one block per console folder.<br><br>
      To add another folder: open it in Files and press <b>“Set games folder”</b>. New console folders inside a games folder show up here by themselves.`,
    buttons: [["Cancel", null, ""], ["Open Files", "files", "primary"]],
  });
  if (choice === "files") showTab("files");
}

/** "+ Add emulator": which emulators ThorBro knows for this console, and how to add your own. */
async function addEmulatorDialog(block, title) {
  const info = await loadProfiles(true);
  const installed = new Set((CAT.data ? CAT.data.emulators : []).filter((emulator) => emulator.installed).map((emulator) => emulator.id));
  const known = info.profiles.filter((profile) => profile.console === block.id);
  const rows = known.length
    ? known.map((profile) => `<li><b>${esc(profile.name)}</b> <span class="dim">${esc(profile.package)}</span> – ${installed.has(profile.id) ? "installed" : "not installed on your Thor"}${profile.source === "user" ? " (your profile)" : ""}</li>`).join("")
    : `<li class="dim">None yet.</li>`;
  const errors = info.errors.length
    ? `<p><b>Skipped profiles</b></p><ul>${info.errors.map((error) => `<li><span class="mono">${esc(error.file)}</span>: ${esc(error.error)}</li>`).join("")}</ul>`
    : "";
  const choice = await dialog({
    title: `Emulators for ${title}`,
    html: true,
    body: `ThorBro finds saves through small emulator profiles. It only ships profiles for emulators that were checked on a real Thor.
      <p><b>Profiles for ${esc(title)}</b></p><ul class="emu-list">${rows}</ul>
      <p>To add another emulator, put a <span class="mono">.json</span> profile into<br><span class="mono">${esc(info.user_dir || "~/.config/thorBro/profiles")}</span><br>
      The format is described in <span class="mono">profiles/README.md</span> in the ThorBro repository. A profile with the id of a built-in one replaces it.</p>${errors}`,
    buttons: [["Close", null, ""], ["Open profiles folder", "open", ""], ["Reload", "reload", "primary"]],
  });
  if (choice === "open") call("open_profiles_dir", {});
  if (choice === "reload") { await loadProfiles(true); loadCatalog(); }
}

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

// ---------------------------------------------------------------- Files toolbar: "?" panel

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
  if (!KEYS.wrap.contains(e.target)) setKeysOpen(false);
});
// Escape closes the open panel first (capture phase: before the file list clears the selection).
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  let target = null;
  if (!KEYS.panel.classList.contains("hidden")) { setKeysOpen(false); target = KEYS.button; }
  else return;
  if (document.activeElement !== target && target.closest("#page-files").contains(document.activeElement)) target.focus();
  e.preventDefault();
  e.stopPropagation();
}, true);

renderLibraryViews();
