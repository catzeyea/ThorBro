// ThorBro UI – file manager tab: locations, folder listing, file actions, keyboard shortcuts
// and dragging entries out of the window onto the PC.
// Sending files *to* the Thor (drop, "Send files …") lives in transfer.js.
"use strict";

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
  const select = $("#place");
  select.innerHTML = S.places.map((place, i) => `<option value="${i}">${esc(place.label)}</option>`).join("");
  let wanted = restore ? result.start : S.cwd;
  // The location that contains the wanted folder (longest match wins).
  const match = S.places
    .filter((place) => wanted === place.path || wanted.startsWith(place.path + "/"))
    .sort((a, b) => b.path.length - a.path.length)[0];
  const root = match ? match.path : (S.places[0] || { path: "/sdcard" }).path;
  if (!match) wanted = root;
  selectPlace(root, wanted);
}

/** Switches to the location `root` and opens `path` (default: the root). */
function selectPlace(root, path) {
  S.root = root;
  $("#place").value = String(S.places.findIndex((place) => place.path === root));
  go(path || root);
}

$("#place").onchange = (e) => {
  const place = S.places[+e.target.value];
  if (place) selectPlace(place.path);
};

/** The game library whose root is `path`, if any. */
function libAt(path) {
  return S.libs.find((lib) => lib.path === path);
}

// ---------------------------------------------------------------- Folder listing

/** Opens a folder (kept inside the current location) and lists it. */
async function go(path) {
  if (!(path === S.root || path.startsWith(S.root + "/"))) path = S.root;
  S.cwd = path.replace(/\/+$/, "") || "/";
  S.sel = -1;
  renderCrumbs();
  if (!S.connected) return renderFiles("No Thor connected");
  renderFiles("Loading …");
  try {
    // → commands::list_dir → files::listdir (also remembers the folder for next start)
    S.entries = await call("list_dir", { path: S.cwd });
  } catch {
    S.entries = [];
  }
  renderFiles(S.entries.length ? "" : "Empty folder");
}

/** Breadcrumb buttons for the current path. */
function renderCrumbs() {
  const place = S.places.find((p) => p.path === S.root);
  const crumbs = $("#crumbs");
  crumbs.innerHTML = "";
  const parts = S.cwd.slice(S.root.length).split("/").filter(Boolean);
  const addCrumb = (label, path) => {
    const button = el(`<button>${esc(label)}</button>`);
    button.onclick = () => go(path);
    crumbs.append(button);
  };
  addCrumb(place ? place.label : S.root, S.root);
  let pathSoFar = S.root;
  for (const part of parts) {
    pathSoFar += "/" + part;
    crumbs.append(el(`<span class="sep">›</span>`));
    addCrumb(part, pathSoFar);
  }
  crumbs.title = S.cwd;
  crumbs.scrollLeft = crumbs.scrollWidth;
  $("#up").disabled = S.cwd === S.root;
}

/**
 * Draws the file list. In a library root, system folders show their full name and game
 * count (empty systems are faded) and systeminfo.txt is hidden.
 */
function renderFiles(placeholder) {
  const list = $("#filelist");
  list.innerHTML = "";
  const lib = libAt(S.cwd);
  const shown = S.entries.filter((entry) => !(lib && entry.name === "systeminfo.txt"));
  if (!shown.length) {
    list.append(el(`<div class="placeholder">${esc(placeholder || "")}</div>`));
    return;
  }
  shown.forEach((entry, i) => list.append(buildFileRow(entry, i, lib)));
  S.shown = shown;
}

/** One row of the file list, with click/double-click/drag handlers. */
function buildFileRow(entry, index, lib) {
  let meta = entry.dir ? "" : human(entry.size);
  let faded = false;
  if (lib && entry.dir && lib.systems[entry.name]) {
    const system = lib.systems[entry.name];
    meta = `${system.full || ""}${system.full ? " · " : ""}${system.count}`;
    faded = system.count === 0;
  }
  const row = el(`<div class="row${faded ? " faded" : ""}"><span class="ic">${entry.dir ? "📁" : "📄"}</span><span class="nm"></span><span class="meta"></span></div>`);
  row.querySelector(".nm").textContent = entry.name;
  row.querySelector(".meta").textContent = meta;
  row.onclick = () => select(index);
  // Remember where the mouse went down; moving far enough starts a drag-out (see below).
  row.onmousedown = (ev) => { if (ev.button === 0) DRAG.start = { x: ev.clientX, y: ev.clientY, i: index }; };
  row.ondragstart = (ev) => ev.preventDefault();
  row.ondblclick = () => open(index);
  row.entry = entry;
  return row;
}

/** Selects row i and scrolls it into view. */
function select(i) {
  S.sel = i;
  document.querySelectorAll("#filelist .row").forEach((row, j) => row.classList.toggle("sel", j === i));
  const row = $("#filelist").children[i];
  if (row) row.scrollIntoView({ block: "nearest" });
}

/** The selected entry, or a hint toast if nothing is selected. */
function selected() {
  const entry = S.shown && S.shown[S.sel];
  if (!entry) toast("Select an entry first");
  return entry;
}

/** Opens row i if it is a folder. */
function open(i) {
  const entry = S.shown[i];
  if (entry && entry.dir) go(`${S.cwd}/${entry.name}`);
}

const goUp = () => S.cwd !== S.root && go(dirname(S.cwd));
$("#up").onclick = goUp;
$("#reload").onclick = () => go(S.cwd);

// ---------------------------------------------------------------- File actions

$("#btn-mkdir").onclick = async () => {
  const name = await ask("New folder");
  if (!name || name.includes("/")) return;
  await call("make_dir", { path: `${S.cwd}/${name}` }); // → commands::make_dir → files::mkdir
  go(S.cwd);
};

$("#btn-rename").onclick = async () => {
  const entry = selected();
  if (!entry) return;
  const name = await ask("Rename", entry.name, !entry.dir);
  if (!name || name === entry.name || name.includes("/")) return;
  await call("rename", { from: `${S.cwd}/${entry.name}`, to: `${S.cwd}/${name}` }); // → commands::rename → files::rename
  toast(`Renamed to ${name}`);
  go(S.cwd);
};

$("#btn-delete").onclick = async () => {
  const entry = selected();
  if (!entry) return;
  const choice = await dialog({
    title: `Really delete “${entry.name}”?`,
    body: "There is no recycle bin – this can't be undone.",
    buttons: [["Cancel", null, ""], ["Delete", "rm", "danger solid"]],
    defaultValue: null,
  });
  if (choice !== "rm") return;
  await call("delete", { path: `${S.cwd}/${entry.name}` }); // → commands::delete → files::delete
  toast(`${entry.name} deleted`);
  go(S.cwd);
};

$("#btn-pull").onclick = async () => {
  const entry = selected();
  if (!entry) return;
  toast(`Copying ${entry.name} to the PC …`);
  const dest = await call("pull", { path: `${S.cwd}/${entry.name}` }); // → commands::pull → files::pull
  toast(`Copied to ${dest}`);
};

$("#btn-unpack").onclick = () => {
  const entry = selected();
  if (!entry) return;
  if (entry.dir || !ARCHIVE_RE.test(entry.name)) return toast("That's not an archive (zip, 7z, rar, tar …)", true);
  // Inside a game folder: skip single wrapper folders in the archive.
  const inLib = S.libs.some((lib) => S.cwd.startsWith(lib.path + "/"));
  enqueue([job(`${S.cwd}/${entry.name}`, S.cwd, true, inLib, true)]);
};

// ---------------------------------------------------------------- Keyboard

$("#filelist").addEventListener("keydown", (e) => {
  if (S.modalOpen) return;
  const count = S.shown ? S.shown.length : 0;
  if (e.key === "ArrowDown") { select(Math.min(count - 1, S.sel + 1)); e.preventDefault(); }
  else if (e.key === "ArrowUp") { select(Math.max(0, S.sel - 1)); e.preventDefault(); }
  else if (e.key === "Enter") open(S.sel);
});

document.addEventListener("keydown", (e) => {
  if (S.modalOpen || S.tab !== "files" || e.target.tagName === "INPUT" || e.target.tagName === "SELECT") return;
  if (e.key === "F2") $("#btn-rename").click();
  else if (e.key === "Delete") $("#btn-delete").click();
  else if (e.key === "Backspace") goUp();
  else if (e.key === "F5") { e.preventDefault(); go(S.cwd); }
  else return;
  e.preventDefault();
});

// ---------------------------------------------------------------- Drag out to the PC
//
// Grab a row and drag it onto the desktop / into a file manager. The file is fetched into a
// local cache first (drag_fetch), then a real OS drag starts (drag_start).

/** Minimum mouse movement (px) before a press on a row counts as a drag. */
const DRAG_THRESHOLD_PX = 8;

/** Matches local paths inside the drag cache (files::drag_root), so our own drags aren't re-imported. */
const FROM_DRAG_CACHE = /[\\/]thorbro[\\/]ziehen[\\/]/;

/**
 * start:  where the mouse went down on a row ({x, y, i}), null otherwise
 * down:   left mouse button is held
 * busy:   a fetch is running
 * native: an OS drag started by us is in progress
 */
const DRAG = { start: null, down: false, busy: false, native: false };

document.addEventListener("mousedown", (ev) => { if (ev.button === 0) DRAG.down = true; }, true);
document.addEventListener("mouseup", () => { DRAG.down = false; DRAG.start = null; }, true);
document.addEventListener("mousemove", (ev) => {
  const start = DRAG.start;
  if (!start || !DRAG.down || DRAG.busy || S.modalOpen) return;
  if (Math.hypot(ev.clientX - start.x, ev.clientY - start.y) < DRAG_THRESHOLD_PX) return;
  DRAG.start = null;
  dragOut(start.i);
});

/** Fetches row i into the cache and starts the OS drag if the button is still held. */
async function dragOut(i) {
  const entry = S.shown && S.shown[i];
  if (!entry) return;
  select(i);
  DRAG.busy = true;
  const row = $("#filelist").children[i];
  try {
    // Only mention the fetch if it takes noticeably long (cached files are instant).
    const slowHint = setTimeout(() => toast(`Fetching ${entry.name} from the Thor, then you can drag it out …`), 500);
    // → commands::drag_fetch → files::drag_fetch (progress via "transfer" events)
    const fetched = await call("drag_fetch", { path: `${S.cwd}/${entry.name}` }).finally(() => clearTimeout(slowHint));
    if (row && row.entry === entry) row.classList.add("ready");
    if (DRAG.down) {
      DRAG.native = true;
      await invoke("drag_start", { paths: [fetched.local] }); // → commands::drag_start (emits "drag-done")
    } else {
      toast(`${entry.name} is ready – drag it out again now`);
    }
  } catch (_) {
    // The error was already shown as a toast.
  } finally {
    DRAG.busy = false;
  }
}

// Emitted by commands::start_os_drag when the OS drag ends; payload = was it dropped somewhere.
T.event.listen("drag-done", ({ payload: dropped }) => {
  DRAG.down = false;
  DRAG.native = false;
  $("#drop-overlay").classList.add("hidden");
  if (dropped) toast("Copied to the PC");
});
