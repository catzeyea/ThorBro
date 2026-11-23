// ThorBro UI – file manager tab, part 1: locations, games folder setup, the folder listing with
// its cache, the virtualized list, selection, history and keyboard shortcuts.
// Part 2 (files-actions.js): clipboard, rename / new folder / move / delete, context menu and
// dragging rows. Sending files *to* the Thor (drop, "Send files …") lives in transfer.js.
//
// How the list stays fast although every listing is an `adb shell` call:
// - Listings are cached per path. Opening a folder shows the cached listing at once and refreshes
//   it in the background; the newest request per folder wins, older answers are dropped.
// - Only the visible rows exist in the DOM (fixed row height). Rows are keyed by file name and
//   only rewritten when their data changed, so a refresh that changes nothing touches nothing.
// - Changes (rename, move, delete, new folder) are applied to the listing first and confirmed by
//   adb afterwards; see files-actions.js.
"use strict";

// ---------------------------------------------------------------- Tuning

/** Height of one list row in px. Must be fixed for the virtualized list (see --row-h in CSS). */
const ROW_HEIGHT_PX = 36;
/** Rows rendered above and below the visible area, so fast scrolling doesn't show gaps. */
const OVERSCAN_ROWS = 8;
/** Empty space below the last row: room to start a rubber band or right-click the background. */
const LIST_BOTTOM_PAD_PX = 48;
/** Rapid navigation (holding Backspace, quick double-clicks) only lists the folder you end up in. */
const LISTING_DEBOUNCE_MS = 60;
/** The loading bar only appears if a listing takes longer than this (no flicker for fast ones). */
const LOADING_HINT_DELAY_MS = 150;
/** Folders kept in the listing cache (least recently used ones are dropped first). */
const DIR_CACHE_LIMIT = 300;
/** Typing letters jumps to the first matching name; the typed text resets after this pause. */
const TYPEAHEAD_RESET_MS = 900;
/** Most folders kept in the back history. */
const HISTORY_LIMIT = 100;

/** Shared storage on Android ignores case, so name clashes are checked case-insensitively. */
const NAME_COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
const DATE_FORMAT = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

// ---------------------------------------------------------------- File manager state

/**
 * State of the file manager beyond the shared `S` (core.js). `S.cwd`, `S.entries`, `S.shown` and
 * `S.sel` keep their old meaning for the other files: `S.sel` is the index of the focused row in
 * `S.shown` if that row is selected, else -1.
 */
const FM = {
  picked: new Set(),   // names of the selected entries in the open folder
  anchor: null,        // name where a Shift range starts
  focus: null,         // name of the keyboard cursor row
  index: new Map(),    // name → index in S.shown
  lib: null,           // library whose root is the open folder (or null)
  sort: { key: "name", dir: 1 },
  back: [],            // history: [{ path, root, scroll, focus }]
  fwd: [],
  cwdRoot: null,       // location (S.root) the open folder belongs to, for the history
  status: "loading",   // open folder: "loading" | "ready" | "error" | "offline"
  pendingFocus: null,  // name to select once it shows up (the folder we came from after "Up")
  edit: null,          // inline rename in progress: { name, isNew, cancel() }
  clip: null,          // clipboard: { mode: "cut" | "copy", items: [{ path, entry }], paths: Set }
  dropTarget: null,    // folder row highlighted while dragging rows inside the window
  ready: new Set(),    // Thor paths already fetched for dragging out (row shows a small marker)
};

try {
  const saved = JSON.parse(localStorage.getItem("fileSort") || "null");
  if (saved && ["name", "size", "mtime"].includes(saved.key) && (saved.dir === 1 || saved.dir === -1)) FM.sort = saved;
} catch (_) { /* default sort */ }

// ---------------------------------------------------------------- Locations

/**
 * Loads storages and game libraries into the location drop-down.
 * restore: open the last folder from the previous session.
 */
async function loadPlaces(restore) {
  // → commands::places → adb::storages + library::find_libraries
  const result = await call("places", { restore });
  S.places = result.places;
  S.libs = result.libs;
  updateLibHint();
  const select = $("#place");
  select.innerHTML = S.places.map((place, i) => `<option value="${i}">${esc(place.label)}</option>`).join("");
  let wanted = restore ? result.start : S.cwd;
  // The location that contains the wanted folder (longest match wins).
  const match = S.places
    .filter((place) => wanted === place.path || wanted.startsWith(place.path + "/"))
    .sort((a, b) => b.path.length - a.path.length)[0];
  const root = match ? match.path : (S.places[0] || { path: "/sdcard" }).path;
  if (!match) wanted = root;
  selectPlace(root, wanted, !restore);
}

/** Switches to the location `root` and opens `path` (default: the root). */
function selectPlace(root, path, history = true) {
  S.root = root;
  $("#place").value = String(S.places.findIndex((place) => place.path === root));
  go(path || root, { history });
}

$("#place").onchange = (e) => {
  const place = S.places[+e.target.value];
  if (place) selectPlace(place.path);
};

/** The game library whose root is `path`, if any. */
function libAt(path) {
  return S.libs.find((lib) => lib.path === path);
}

/** True if `path` is a library root or inside one. */
function inLibrary(path) {
  return S.libs.some((lib) => path === lib.path || path.startsWith(lib.path + "/"));
}

/** The storage (internal / SD card) a Thor path is on, or null. Moves inside one storage are a plain `mv`. */
function storageOf(path) {
  const match = S.places
    .filter((place) => !place.games && (path === place.path || path.startsWith(place.path + "/")))
    .sort((a, b) => b.path.length - a.path.length)[0];
  return match ? match.path : null;
}

/** Shows the "No games folder found" hint while there is no library. */
function updateLibHint() {
  $("#no-lib").classList.toggle("hidden", !S.connected || S.libs.length > 0);
  renderLibraryViews(); // Start tiles and sidebar (start.js)
}

/** Re-reads the libraries (game counts, systems) and redraws the list. */
async function refreshLibs() {
  // → commands::refresh_libs
  S.libs = await call("refresh_libs", {}, true).catch(() => S.libs);
  updateLibHint();
}

// ---------------------------------------------------------------- Games folder setup

/**
 * "Set games folder": uses the selected folder (or the open one if no folder is selected) as
 * game library. On a folder that already is a user-added library it offers to remove it.
 * openFolderOnly: ignore the selection and always offer the open folder.
 * Resolves with true if a library was added.
 */
async function setGamesFolder(openFolderOnly = false) {
  if (!S.connected) { toast("Thor is not connected", true); return false; }
  const entry = !openFolderOnly && S.shown && S.shown[S.sel];
  const path = entry && entry.dir ? joinPath(S.cwd, entry.name) : S.cwd;
  const existing = libAt(path);
  if (existing && !existing.custom) { toast("This folder is already used as games folder"); return false; }
  if (existing) {
    const choice = await dialog({
      title: "Stop using this games folder?",
      body: `“${path}” will no longer be used as games folder. The folder and its files stay untouched.`,
      buttons: [["Cancel", null, ""], ["Remove", "rm", "danger solid"]],
    });
    if (choice !== "rm") return false;
    await call("remove_library", { path }); // → commands::remove_library
    toast("Games folder removed");
    await loadPlaces(false);
    return false;
  }
  const choice = await dialog({
    title: "Use this as games folder?",
    html: true,
    body: `<p><b>${esc(path)}</b></p><p class="dim">It should contain one folder per system (e.g. snes, psx, Wii).
      Not the right one? Cancel, open or select the right folder in the list and press “Set games folder” again.</p>`,
    buttons: [["Cancel", null, ""], ["Use this folder", "ok", "primary"]],
  });
  if (choice !== "ok") return false;
  await call("add_library", { path }); // → commands::add_library (stored in settings.json)
  toast(`Games folder set: ${path}`);
  await loadPlaces(false);
  return true;
}

$("#btn-set-lib").onclick = () => setGamesFolder();
$("#btn-lib-hint").onclick = () => setGamesFolder();

// ---------------------------------------------------------------- System of a folder

/** Known systems for the picker (from commands::known_systems), loaded on first use. */
let KNOWN_SYSTEMS = null;

/** What the file list shows next to a system folder's name, by where the info comes from. */
const ORIGIN_NOTE = { name: " (from folder name)", manual: " (set by you)" };

/**
 * Lets the user pick the system of a library folder ThorBro can't identify (no systeminfo.txt
 * and an unknown name), or correct a guess. Stored per folder in settings.json.
 */
async function pickSystem(path, system) {
  KNOWN_SYSTEMS = KNOWN_SYSTEMS || await call("known_systems", {});
  const choice = await modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>Which system is “${esc(basename(path))}”?</h2></div>`));
    const body = el(`<div class="modal-body"><p class="dim">ThorBro uses this to sort dropped games into this folder.</p></div>`);
    const select = el(`<select></select>`);
    select.innerHTML = `<option value="">Automatic (from the folder name)</option>` +
      KNOWN_SYSTEMS.map((known) => `<option value="${esc(known.id)}">${esc(known.full)} (${esc(known.id)})</option>`).join("");
    select.value = system.origin === "manual" ? system.id : "";
    body.append(select);
    box.append(body);
    const foot = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">Save</button></div>`);
    foot.children[0].onclick = () => close(null);
    foot.children[1].onclick = () => close({ system: select.value || null });
    box.append(foot);
    setTimeout(() => select.focus(), 0);
  });
  if (!choice) return;
  await call("set_folder_system", { path, system: choice.system }); // → commands::set_folder_system
  await refreshLibs();
  renderFiles();
}

// ---------------------------------------------------------------- Listing cache

/** path → entries (from commands::list_dir), in least-recently-used order. */
const DIR_CACHE = new Map();

function cacheGet(path) {
  const entries = DIR_CACHE.get(path);
  if (entries) { DIR_CACHE.delete(path); DIR_CACHE.set(path, entries); }
  return entries;
}

function cachePut(path, entries) {
  DIR_CACHE.delete(path);
  DIR_CACHE.set(path, entries);
  if (DIR_CACHE.size > DIR_CACHE_LIMIT) DIR_CACHE.delete(DIR_CACHE.keys().next().value);
}

/** Forgets all cached listings (e.g. when the Thor is unplugged). */
function clearDirCache() {
  DIR_CACHE.clear();
}

/**
 * Bookkeeping for listings:
 * debounce:    timer of the next scheduled listing of the open folder
 * loadingHint: timer that shows the loading bar
 * seq/latest:  request counter and the newest request per path (older answers are dropped)
 * ops:         path → number of changes in flight (listings arriving meanwhile are dropped,
 *              the folder is listed again when the last change is done)
 */
const LISTING = { debounce: 0, loadingHint: 0, seq: 0, latest: new Map(), ops: new Map() };

/**
 * Applies `change` to the listing of `dir` (the open folder and/or its cache entry) and redraws.
 * Used for optimistic updates; does nothing if the folder was never listed.
 */
function mutateListing(dir, change) {
  const base = dir === S.cwd && FM.status === "ready" ? S.entries : DIR_CACHE.get(dir);
  if (!base) return;
  const next = change(base.slice());
  cachePut(dir, next);
  if (dir === S.cwd) { S.entries = next; renderFiles(); }
}

/** Marks the start of a change in these folders (see LISTING.ops). */
function beginOp(dirs) {
  for (const dir of new Set(dirs)) LISTING.ops.set(dir, (LISTING.ops.get(dir) || 0) + 1);
}

/** Marks the end of a change. The open folder is then listed again to confirm the result. */
function endOp(dirs) {
  for (const dir of new Set(dirs)) {
    const left = (LISTING.ops.get(dir) || 1) - 1;
    if (left > 0) { LISTING.ops.set(dir, left); continue; }
    LISTING.ops.delete(dir);
    if (dir === S.cwd) scheduleListing(dir);
  }
}

// ---------------------------------------------------------------- Navigation

/**
 * Opens a folder (kept inside the current location). Shows the cached listing right away and
 * refreshes it in the background.
 * history: add the folder we leave to the back history.
 * focus:   name to select once the listing is there.
 */
function go(path, { history = true, focus = null } = {}) {
  if (!(path === S.root || path.startsWith(S.root + "/"))) path = S.root;
  path = path.replace(/\/+$/, "") || "/";
  const changed = path !== S.cwd;
  if (changed) {
    if (FM.edit) FM.edit.cancel();
    if (history && FM.cwdRoot) {
      FM.back.push(historyEntry());
      if (FM.back.length > HISTORY_LIMIT) FM.back.shift();
      FM.fwd.length = 0;
    }
    S.cwd = path;
    resetSelection();
    $("#filelist").scrollTop = 0;
  }
  FM.cwdRoot = S.root;
  if (focus) FM.pendingFocus = focus;
  renderCrumbs();
  renderSidebar(); // highlight of the open location (start.js)
  updateNavButtons();
  if (!S.connected) {
    S.entries = [];
    FM.status = "offline";
    return renderFiles();
  }
  const cached = cacheGet(path);
  if (cached) {
    S.entries = cached;
    FM.status = "ready";
  } else if (changed || FM.status !== "ready") {
    S.entries = [];
    FM.status = "loading";
  }
  renderFiles();
  scheduleListing(path);
}

/** Lists `path` soon; calls within LISTING_DEBOUNCE_MS collapse into one. */
function scheduleListing(path) {
  clearTimeout(LISTING.debounce);
  LISTING.debounce = setTimeout(() => refreshListing(path), LISTING_DEBOUNCE_MS);
}

/** Re-lists the open folder in the background (F5, after a transfer, window focus). */
function refreshCurrent() {
  if (S.connected) refreshListing(S.cwd);
}

/**
 * Lists `path` via adb and, if it is still the open folder and no newer listing or change is
 * pending, shows the result (keyed diff, see renderRows). Every answer also fills the cache.
 */
async function refreshListing(path) {
  if (!S.connected || LISTING.ops.get(path)) return; // a running change re-lists when it's done
  const seq = ++LISTING.seq;
  LISTING.latest.set(path, seq);
  if (path === S.cwd) startLoadingHint();
  let entries;
  try {
    entries = await invoke("list_dir", { path }); // → commands::list_dir → files::listdir
  } catch (e) {
    if (LISTING.latest.get(path) !== seq) return;
    DIR_CACHE.delete(path);
    notePinListing(path, false); // pins.js: a pinned folder may be gone
    if (path !== S.cwd) return;
    stopLoadingHint();
    S.entries = [];
    FM.status = "error";
    renderFiles();
    toast(String(e), true);
    return;
  }
  if (LISTING.latest.get(path) !== seq) return; // a newer listing of this folder is on its way
  if (LISTING.ops.get(path)) { if (path === S.cwd) stopLoadingHint(); return; } // changed meanwhile
  cachePut(path, entries);
  notePinListing(path, true); // pins.js
  if (path !== S.cwd) return;
  stopLoadingHint();
  S.entries = entries;
  FM.status = "ready";
  renderFiles();
  invoke("remember_dir", { path }).catch(() => {}); // → commands::remember_dir (opened on next start)
}

function startLoadingHint() {
  clearTimeout(LISTING.loadingHint);
  LISTING.loadingHint = setTimeout(() => $("#list-wrap").classList.add("loading"), LOADING_HINT_DELAY_MS);
}

function stopLoadingHint() {
  clearTimeout(LISTING.loadingHint);
  $("#list-wrap").classList.remove("loading");
}

/** The open folder as history entry, with scroll position and focused row to restore later. */
function historyEntry() {
  return { path: S.cwd, root: FM.cwdRoot, scroll: $("#filelist").scrollTop, focus: FM.focus };
}

/** Goes to a history entry, switching the location if needed; restores scroll and focus. */
function navigateTo({ path, root, scroll, focus }) {
  if (root !== S.root && S.places.some((place) => place.path === root)) {
    S.root = root;
    $("#place").value = String(S.places.findIndex((place) => place.path === root));
  }
  go(path, { history: false, focus });
  // With a cached listing the rows are already there; otherwise the list is still empty and the
  // browser clamps this to 0.
  $("#filelist").scrollTop = scroll || 0;
  requestRender();
}

function goBack() {
  const previous = FM.back.pop();
  if (!previous) return;
  FM.fwd.push(historyEntry());
  navigateTo(previous);
}

function goForward() {
  const next = FM.fwd.pop();
  if (!next) return;
  FM.back.push(historyEntry());
  navigateTo(next);
}

/** One folder up; the folder we came from gets selected. */
function goUp() {
  if (S.cwd === S.root) return;
  go(dirname(S.cwd), { focus: basename(S.cwd) });
}

/** Opens a folder entry of the open folder. Files are not opened (there is nothing to open them with). */
function openEntry(entry) {
  if (entry && entry.dir && !entry.pending) go(joinPath(S.cwd, entry.name));
}

$("#back").onclick = goBack;
$("#forward").onclick = goForward;
$("#up").onclick = goUp;
$("#reload").onclick = refreshCurrent;

function updateNavButtons() {
  $("#back").disabled = !FM.back.length;
  $("#forward").disabled = !FM.fwd.length;
  $("#up").disabled = S.cwd === S.root;
}

/** Breadcrumb buttons for the current path. Each crumb is also a drop target (see files-actions.js). */
function renderCrumbs() {
  const place = S.places.find((p) => p.path === S.root);
  const crumbs = $("#crumbs");
  const parts = S.cwd.slice(S.root.length).split("/").filter(Boolean);
  const html = [`<button data-path="${esc(S.root)}">${esc(place ? place.label : S.root)}</button>`];
  let pathSoFar = S.root;
  for (const part of parts) {
    pathSoFar += "/" + part;
    html.push(`<span class="sep">/</span><button data-path="${esc(pathSoFar)}">${esc(part)}</button>`);
  }
  crumbs.innerHTML = html.join("");
  crumbs.title = S.cwd;
  // Reading scrollWidth forces a layout; doing it in the next frame batches it with the paint.
  requestAnimationFrame(() => { crumbs.scrollLeft = crumbs.scrollWidth; });
}

$("#crumbs").onclick = (e) => {
  const button = e.target.closest("button[data-path]");
  if (button) go(button.dataset.path);
};

// "Back" / "Forward" mouse buttons.
$("#page-files").addEventListener("mouseup", (e) => {
  if (e.button === 3) { e.preventDefault(); goBack(); }
  if (e.button === 4) { e.preventDefault(); goForward(); }
});

// Coming back to the window: the folder may have changed on the Thor meanwhile.
window.addEventListener("focus", () => { if (S.tab === "files") refreshCurrent(); });

// ---------------------------------------------------------------- Sorting

/** Folders first, then by the sort column; ties by name (natural order: 2 before 10). */
function compareEntries(a, b) {
  if (a.dir !== b.dir) return a.dir ? -1 : 1;
  // Folders have no meaningful size: by size, they stay in A–Z order.
  if (FM.sort.key === "size" && a.dir) return NAME_COLLATOR.compare(a.name, b.name);
  let result = 0;
  if (FM.sort.key === "size") result = a.size - b.size;
  else if (FM.sort.key === "mtime") result = (a.mtime || 0) - (b.mtime || 0);
  if (!result) result = NAME_COLLATOR.compare(a.name, b.name);
  return result * FM.sort.dir;
}

$("#list-head").onclick = (e) => {
  const button = e.target.closest("button[data-sort]");
  if (!button) return;
  const key = button.dataset.sort;
  // Size and date start with the biggest / newest first, names with A.
  FM.sort = FM.sort.key === key ? { key, dir: -FM.sort.dir } : { key, dir: key === "name" ? 1 : -1 };
  try { localStorage.setItem("fileSort", JSON.stringify(FM.sort)); } catch (_) { /* not remembered */ }
  renderFiles();
  if (FM.focus !== null && FM.index.has(FM.focus)) scrollToRow(FM.index.get(FM.focus));
};

function updateSortHeader() {
  document.querySelectorAll("#list-head button[data-sort]").forEach((button) => {
    const active = button.dataset.sort === FM.sort.key;
    button.classList.toggle("asc", active && FM.sort.dir === 1);
    button.classList.toggle("desc", active && FM.sort.dir === -1);
  });
}

// ---------------------------------------------------------------- Rendering

/** Filters (library root: hide systeminfo.txt and, optionally, empty systems) and sorts the listing. */
function deriveShown() {
  const lib = libAt(S.cwd) || null;
  const hideEmpty = lib && $("#hide-empty").checked;
  const isEmptySystem = (entry) => entry.dir && lib.systems[entry.name] && lib.systems[entry.name].count === 0;
  const shown = S.entries.filter((entry) => !(lib && entry.name === "systeminfo.txt") && !(hideEmpty && isEmptySystem(entry)));
  shown.sort(compareEntries);
  S.shown = shown;
  FM.lib = lib;
  FM.index = new Map(shown.map((entry, i) => [entry.name, i]));
  // Entries that are gone can't stay selected.
  for (const name of FM.picked) if (!FM.index.has(name)) FM.picked.delete(name);
  if (FM.focus !== null && !FM.index.has(FM.focus)) FM.focus = null;
  if (FM.anchor !== null && !FM.index.has(FM.anchor)) FM.anchor = null;
  syncSel();
  return { lib, hideEmpty };
}

/** The text shown in an empty list. */
function listMessage(hideEmpty) {
  if (S.shown.length) return "";
  if (FM.status === "offline") return "No Thor connected";
  if (FM.status === "error") return "Couldn't open this folder";
  if (FM.status === "loading") return "";
  if (hideEmpty && S.entries.length) return "All system folders are empty (hidden by “Hide empty”)";
  return "Empty folder";
}

/**
 * Redraws the list from S.entries. Cheap: only visible rows are touched, and only if their data
 * changed. `placeholder` overrides the empty-list text (kept for older callers).
 */
function renderFiles(placeholder) {
  const { lib, hideEmpty } = deriveShown();
  if (FM.pendingFocus && FM.index.has(FM.pendingFocus)) {
    const name = FM.pendingFocus;
    FM.pendingFocus = null;
    FM.picked = new Set([name]);
    FM.anchor = FM.focus = name;
    syncSel();
    requestAnimationFrame(() => FM.index.has(name) && scrollToRow(FM.index.get(name)));
  }
  $("#hide-empty-wrap").classList.toggle("hidden", !lib);
  $("#list-sizer").style.height = `${S.shown.length * ROW_HEIGHT_PX + LIST_BOTTOM_PAD_PX}px`;
  const message = (!S.shown.length && placeholder) || listMessage(hideEmpty);
  const messageEl = $("#list-msg");
  messageEl.textContent = message;
  messageEl.classList.toggle("hidden", !message);
  updateSortHeader();
  updateActionButtons();
  renderRows();
}

/** Row elements in the DOM by entry name, and detached rows ready for reuse. */
const ROWS = new Map();
const ROW_POOL = [];
let renderQueued = false;

/** Renders on the next frame (scroll, resize); several calls in one frame render once. */
function requestRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => { renderQueued = false; renderRows(); });
}

/** Puts the rows for the visible part of the list into the DOM and updates them. */
function renderRows() {
  const list = $("#filelist");
  const sizer = $("#list-sizer");
  const count = S.shown.length;
  const top = list.scrollTop;
  const height = list.clientHeight || 600;
  const first = Math.max(0, Math.floor(top / ROW_HEIGHT_PX) - OVERSCAN_ROWS);
  const last = Math.min(count, Math.ceil((top + height) / ROW_HEIGHT_PX) + OVERSCAN_ROWS);
  const wanted = new Set();
  for (let i = first; i < last; i++) wanted.add(S.shown[i].name);
  // The row being renamed stays, even when scrolled away, so its text field survives.
  if (FM.edit && FM.index.has(FM.edit.name)) wanted.add(FM.edit.name);
  for (const [name, row] of ROWS) {
    if (wanted.has(name)) continue;
    row.remove();
    ROWS.delete(name);
    ROW_POOL.push(row);
  }
  for (const name of wanted) {
    let row = ROWS.get(name);
    if (!row) {
      row = ROW_POOL.pop() || createRow();
      row._content = null; // force a full update for a reused row
      ROWS.set(name, row);
      sizer.append(row);
    }
    const index = FM.index.get(name);
    updateRow(row, S.shown[index], index);
  }
}

function createRow() {
  const row = document.createElement("div");
  row.className = "row";
  row.innerHTML = `<span class="ic"></span><span class="nm"></span><span class="info"></span><span class="sz"></span><span class="dt"></span>`;
  return row;
}

/** Formats a modification time (seconds since 1970) for the list. */
function formatTime(seconds) {
  return seconds > 0 ? DATE_FORMAT.format(new Date(seconds * 1000)) : "";
}

/** Brings one row up to date. Text is only rewritten when the entry's data changed. */
function updateRow(row, entry, index) {
  if (row._y !== index) {
    row.style.transform = `translateY(${index * ROW_HEIGHT_PX}px)`;
    row._y = index;
  }
  row._entry = entry;
  const path = joinPath(S.cwd, entry.name);
  const system = FM.lib && entry.dir ? FM.lib.systems[entry.name] : null;
  row.classList.toggle("dir", entry.dir);
  row.classList.toggle("sel", FM.picked.has(entry.name));
  row.classList.toggle("focus", FM.focus === entry.name);
  row.classList.toggle("cut", !!(FM.clip && FM.clip.mode === "cut" && FM.clip.paths.has(path)));
  row.classList.toggle("drop", FM.dropTarget === entry.name);
  row.classList.toggle("pending", !!entry.pending);
  row.classList.toggle("ready", FM.ready.has(path));
  row.classList.toggle("faded", !!(system && system.count === 0));
  if (FM.edit && FM.edit.name === entry.name) return; // the text field owns the name cell
  const systemKey = system ? `${system.id}|${system.full}|${system.origin}|${system.count}` : "";
  const content = `${entry.name}\u0000${entry.dir}\u0000${entry.size}\u0000${entry.mtime}\u0000${systemKey}`;
  if (row._content === content) return;
  row._content = content;
  const [, name, info, size, date] = row.children;
  name.textContent = entry.name;
  name.title = entry.name;
  size.textContent = entry.dir ? "" : human(entry.size);
  date.textContent = formatTime(entry.mtime);
  info.textContent = "";
  if (!system) return;
  const label = system.full ? system.full + (ORIGIN_NOTE[system.origin] || "") : (system.id ? "" : "Unknown system");
  info.append(`${label}${label ? " · " : ""}${system.count}`);
  // Folders without systeminfo.txt: let the user set or correct the system.
  if (system.origin !== "systeminfo") {
    const button = document.createElement("button");
    button.className = "mini";
    button.textContent = system.id ? "Change system" : "Set system …";
    info.append(button);
  }
}

/** The list has a scrollbar, the header doesn't: pad the header so the columns line up (--row-pad in CSS). */
function alignHeader() {
  const list = $("#filelist");
  $("#list-head").style.paddingRight = `calc(var(--row-pad) + ${list.offsetWidth - list.clientWidth}px)`;
}

$("#filelist").addEventListener("scroll", requestRender, { passive: true });
const listResizeObserver = new ResizeObserver(() => { alignHeader(); requestRender(); });
listResizeObserver.observe($("#filelist"));
listResizeObserver.observe($("#list-sizer")); // the scrollbar comes and goes with the row count

// "Hide empty" in a games folder: remembered per PC. localStorage may be unavailable, hence try.
try { $("#hide-empty").checked = localStorage.getItem("hideEmptySystems") === "1"; } catch (_) { /* default: off */ }
$("#hide-empty").onchange = () => {
  try { localStorage.setItem("hideEmptySystems", $("#hide-empty").checked ? "1" : "0"); } catch (_) { /* not remembered */ }
  renderFiles();
};

// ---------------------------------------------------------------- Selection

/** Keeps S.sel (used by esde.js and setGamesFolder) in line with the focus. */
function syncSel() {
  S.sel = FM.focus !== null && FM.picked.has(FM.focus) ? FM.index.get(FM.focus) : -1;
}

/** Clears selection, focus and anchor without redrawing (used when the folder changes). */
function resetSelection() {
  FM.picked = new Set();
  FM.anchor = FM.focus = FM.pendingFocus = null;
  S.sel = -1;
}

function afterSelection() {
  syncSel();
  updateActionButtons();
  renderRows();
}

/** The selected entries in display order. */
function selectedEntries() {
  return S.shown.filter((entry) => FM.picked.has(entry.name));
}

/** The focused entry if it is selected, else null. */
function focusedEntry() {
  return S.sel >= 0 ? S.shown[S.sel] : null;
}

function selectOnly(name) {
  FM.picked = new Set(name === null ? [] : [name]);
  FM.anchor = FM.focus = name;
  afterSelection();
}

function toggleSelect(name) {
  if (FM.picked.has(name)) FM.picked.delete(name);
  else FM.picked.add(name);
  FM.anchor = FM.focus = name;
  afterSelection();
}

/** Selects everything between the anchor and `name`; `additive` keeps the current selection. */
function selectRange(name, additive) {
  const from = FM.anchor !== null && FM.index.has(FM.anchor) ? FM.index.get(FM.anchor) : FM.index.get(name);
  const to = FM.index.get(name);
  const picked = additive ? new Set(FM.picked) : new Set();
  for (let i = Math.min(from, to); i <= Math.max(from, to); i++) picked.add(S.shown[i].name);
  FM.picked = picked;
  if (FM.anchor === null) FM.anchor = name;
  FM.focus = name;
  afterSelection();
}

function selectAll() {
  FM.picked = new Set(S.shown.map((entry) => entry.name));
  if (FM.focus === null && S.shown.length) FM.focus = S.shown[0].name;
  afterSelection();
}

function clearSelection() {
  FM.picked = new Set();
  afterSelection();
}

/** Scrolls just enough to make row `index` fully visible. */
function scrollToRow(index) {
  if (index === undefined || index < 0) return;
  const list = $("#filelist");
  const top = index * ROW_HEIGHT_PX;
  if (top < list.scrollTop) list.scrollTop = top;
  else if (top + ROW_HEIGHT_PX > list.scrollTop + list.clientHeight) list.scrollTop = top + ROW_HEIGHT_PX - list.clientHeight;
  renderRows();
}

/** Arrow keys etc.: Shift extends the selection, Ctrl only moves the focus. */
function moveFocus(target, e) {
  const count = S.shown.length;
  if (!count) return;
  const current = FM.focus !== null && FM.index.has(FM.focus) ? FM.index.get(FM.focus) : -1;
  const page = Math.max(1, Math.floor($("#filelist").clientHeight / ROW_HEIGHT_PX) - 1);
  const steps = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page };
  let next;
  if (target === "Home") next = 0;
  else if (target === "End") next = count - 1;
  else if (current < 0) next = steps[target] > 0 ? 0 : count - 1;
  else next = Math.min(count - 1, Math.max(0, current + steps[target]));
  const name = S.shown[next].name;
  if (e.shiftKey) {
    if (FM.anchor === null) FM.anchor = current >= 0 ? S.shown[current].name : name;
    selectRange(name, e.ctrlKey || e.metaKey);
  } else if (e.ctrlKey || e.metaKey) {
    FM.focus = name;
    afterSelection();
  } else {
    selectOnly(name);
  }
  scrollToRow(next);
}

/** Typing a name jumps to the first entry starting with the typed text. */
const TYPEAHEAD = { text: "", timer: 0 };
function typeAhead(char) {
  clearTimeout(TYPEAHEAD.timer);
  TYPEAHEAD.timer = setTimeout(() => { TYPEAHEAD.text = ""; }, TYPEAHEAD_RESET_MS);
  TYPEAHEAD.text += char.toLowerCase();
  const count = S.shown.length;
  const current = FM.focus !== null && FM.index.has(FM.focus) ? FM.index.get(FM.focus) : -1;
  // A single letter cycles through the matches, a longer text refines the current one.
  const start = TYPEAHEAD.text.length === 1 ? current + 1 : Math.max(0, current);
  for (let k = 0; k < count; k++) {
    const i = (start + k) % count;
    if (S.shown[i].name.toLowerCase().startsWith(TYPEAHEAD.text)) {
      selectOnly(S.shown[i].name);
      scrollToRow(i);
      return;
    }
  }
}

// ---------------------------------------------------------------- Action buttons

/** Enables the action buttons that make sense for the current selection. */
function updateActionButtons() {
  const selected = FM.picked.size;
  const focused = focusedEntry();
  $("#btn-rename").disabled = selected !== 1;
  $("#btn-delete").disabled = !selected;
  $("#btn-pull").disabled = !selected;
  $("#btn-unpack").disabled = !(selected === 1 && focused && !focused.dir && ARCHIVE_RE.test(focused.name));
}

// ---------------------------------------------------------------- Keyboard

const FOCUS_KEYS = new Set(["ArrowDown", "ArrowUp", "PageDown", "PageUp", "Home", "End"]);

document.addEventListener("keydown", (e) => {
  if (S.modalOpen || S.tab !== "files") return;
  if (DRAG.items && e.key === "Escape") { endInternalDrag(); e.preventDefault(); return; }
  if (MENU.el) { if (e.key === "Escape") { closeMenu(); e.preventDefault(); } return; }
  const tag = e.target.tagName;
  if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
  const ctrl = e.ctrlKey || e.metaKey;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (e.altKey) {
    if (key === "ArrowLeft") goBack();
    else if (key === "ArrowRight") goForward();
    else if (key === "ArrowUp") goUp();
    else return;
  } else if (ctrl && e.shiftKey && key === "n") newFolder();
  else if (ctrl && key === "d") togglePin(); // pins.js
  else if (ctrl && key === "a") selectAll();
  else if (ctrl && key === "x") setClip("cut");
  else if (ctrl && key === "c") setClip("copy");
  else if (ctrl && key === "v") pasteInto(S.cwd);
  else if (ctrl && key === "r") refreshCurrent();
  else if (key === "F5") refreshCurrent();
  else if (key === "F2") renameFocused();
  else if (key === "Delete") deleteSelected();
  else if (key === "Backspace") goUp();
  else if (key === "Escape") clearSelection();
  else if (key === "ContextMenu" || (e.shiftKey && key === "F10")) openMenuForFocus();
  else if (tag === "BUTTON" && (key === "Enter" || key === " ")) return; // let the button act
  else if (key === "Enter") openEntry(focusedEntry());
  else if (ctrl && key === " ") { if (FM.focus !== null) toggleSelect(FM.focus); }
  else if (FOCUS_KEYS.has(key)) moveFocus(key, e);
  else if (e.key.length === 1 && !ctrl && e.key !== " ") typeAhead(e.key);
  else return;
  e.preventDefault();
});
