// ThorBro UI – file manager tab, part 2: clipboard (cut / copy / paste), rename in place, new
// folder, move, delete, copy to PC, the context menu, mouse selection (click, Ctrl/Shift click,
// rubber band) and dragging rows: into a folder (move, Ctrl = copy) or out of the window to the PC.
//
// Changes are optimistic: the listing is updated first (mutateListing), then adb runs. A failed
// part is rolled back and shown as error. beginOp/endOp (files.js) hold back background refreshes
// of the affected folders until the change is done, then the open folder is listed again.
"use strict";

// ---------------------------------------------------------------- Helpers

/** The selected entries, or a hint toast if nothing is selected. */
function selectedOrHint() {
  const entries = selectedEntries();
  if (!entries.length) toast("Select an entry first");
  return entries;
}

/** "name" for one entry, "3 items" for more. */
function itemsLabel(entries) {
  return entries.length === 1 ? entries[0].name : `${entries.length} items`;
}

/** Shows the first error of a batch, with the count of the others. */
function toastErrors(errors) {
  if (!errors.length) return;
  toast(errors[0] + (errors.length > 1 ? ` (and ${errors.length - 1} more)` : ""), true);
}

/** Refreshes game counts a moment after changes inside a games folder (several changes → one refresh). */
let libRefreshTimer = 0;
function afterChange(dirs) {
  if (!dirs.some(inLibrary)) return;
  clearTimeout(libRefreshTimer);
  libRefreshTimer = setTimeout(async () => { await refreshLibs(); renderFiles(); }, 800);
}

/** Why `name` can't be used in the open folder (null if it can). `except` = the entry being renamed. */
function nameProblem(name, except = null) {
  if (name.includes("/")) return "A name can't contain “/”";
  if (name === "." || name === "..") return "That name isn't allowed";
  // Shared storage ignores case: "Wii" and "wii" clash, except when renaming "Wii" to "wii".
  const lower = name.toLowerCase();
  const clash = S.entries.some((entry) => entry.name !== except && entry.name.toLowerCase() === lower);
  return clash ? `“${name}” already exists here` : null;
}

/** "New folder", "New folder 2", … whichever is free in the open folder. */
function freeName(base) {
  for (let n = 1; ; n++) {
    const name = n === 1 ? base : `${base} ${n}`;
    if (!nameProblem(name)) return name;
  }
}

// ---------------------------------------------------------------- Clipboard

/** Ctrl+X / Ctrl+C: remembers the selected entries. Cut entries are shown dimmed until pasted. */
function setClip(mode) {
  const entries = selectedOrHint();
  if (!entries.length) return;
  const items = entries.filter((entry) => !entry.pending).map((entry) => ({ path: joinPath(S.cwd, entry.name), entry: { ...entry } }));
  FM.clip = { mode, items, paths: new Set(items.map((item) => item.path)) };
  toast(`${mode === "cut" ? "Cut" : "Copied"} ${itemsLabel(entries)} – paste with Ctrl+V`);
  renderRows();
}

/** Drops paths from the clipboard (after they were moved or deleted). */
function forgetClipPaths(paths) {
  if (!FM.clip) return;
  const gone = new Set(paths);
  FM.clip.items = FM.clip.items.filter((item) => !gone.has(item.path));
  FM.clip.paths = new Set(FM.clip.items.map((item) => item.path));
  if (!FM.clip.items.length) FM.clip = null;
}

/** Ctrl+V: pastes the clipboard into `dest`. */
function pasteInto(dest) {
  if (!FM.clip || !FM.clip.items.length) return toast("Nothing to paste");
  const { mode, items } = FM.clip;
  if (mode === "cut") FM.clip = null; // a cut is pasted once, like in Explorer / Nautilus
  transferInto(dest, items, mode === "cut");
  renderRows();
}

/**
 * Moves (move = true) or copies entries into the Thor folder `dest`.
 * - Move inside one storage: plain `mv`, instant, optimistic (moveInto).
 * - Move to another storage, or any copy: a job in the copy queue with progress and cancel
 *   (files::thor_copy_one). The list is refreshed when the queue reports it's done.
 */
function transferInto(dest, items, move) {
  const intoItself = (item) => dest === item.path || dest.startsWith(item.path + "/");
  if (items.some(intoItself)) toast("A folder can't be put into itself", true);
  items = items.filter((item) => !intoItself(item));
  if (!items.length) return;
  const destStorage = storageOf(dest);
  const instant = move ? items.filter((item) => destStorage && storageOf(item.path) === destStorage) : [];
  const queued = items.filter((item) => !instant.includes(item));
  if (instant.length) moveInto(dest, instant);
  if (queued.length) {
    // Field names as in files::Job (Rust, snake_case).
    enqueue(queued.map((item) => ({ ...job(item.path, dest), thor_copy: true, move_after: move })));
  }
}

// ---------------------------------------------------------------- Optimistic changes

/** Moves entries (all on dest's storage) into `dest` with one adb call; failed ones are rolled back. */
async function moveInto(dest, items) {
  items = items.filter((item) => dirname(item.path) !== dest); // already there
  // Known clashes are refused right away instead of after the round trip.
  const destEntries = dest === S.cwd && FM.status === "ready" ? S.entries : DIR_CACHE.get(dest);
  const taken = new Set((destEntries || []).map((entry) => entry.name.toLowerCase()));
  const clashing = items.filter((item) => taken.has(basename(item.path).toLowerCase()));
  toastErrors(clashing.map((item) => `${basename(item.path)} already exists in ${dest}`));
  items = items.filter((item) => !clashing.includes(item));
  if (!items.length) return;

  const sourceDirs = [...new Set(items.map((item) => dirname(item.path)))];
  const removeFrom = (dir, names) => mutateListing(dir, (list) => list.filter((entry) => !names.has(entry.name)));
  const namesIn = (dir, list) => new Set(list.filter((item) => dirname(item.path) === dir).map((item) => basename(item.path)));
  const addTo = (dir, list) => mutateListing(dir, (entries) => [...entries, ...list.map((item) => ({ ...item.entry, name: basename(item.path) }))]);

  for (const dir of sourceDirs) removeFrom(dir, namesIn(dir, items));
  addTo(dest, items);
  const dirs = [...sourceDirs, dest];
  beginOp(dirs);
  try {
    // → commands::move_paths → files::move_paths (one result per entry)
    const results = await invoke("move_paths", { sources: items.map((item) => item.path), dest });
    const failedPaths = new Set(results.filter((result) => result.error).map((result) => result.path));
    const failed = items.filter((item) => failedPaths.has(item.path));
    if (failed.length) {
      removeFrom(dest, new Set(failed.map((item) => basename(item.path))));
      for (const dir of sourceDirs) addTo(dir, failed.filter((item) => dirname(item.path) === dir));
      toastErrors(results.filter((result) => result.error).map((result) => result.error));
    }
    forgetClipPaths(items.filter((item) => !failedPaths.has(item.path)).map((item) => item.path));
  } catch (e) {
    removeFrom(dest, new Set(items.map((item) => basename(item.path))));
    for (const dir of sourceDirs) addTo(dir, items.filter((item) => dirname(item.path) === dir));
    toast(`Couldn't move: ${e}`, true);
  } finally {
    endOp(dirs);
    afterChange(dirs);
  }
}

/** Renames an entry of the open folder (case-only renames are handled in files::rename). */
async function renameEntry(oldName, newName) {
  const dir = S.cwd;
  const swap = (from, to) => (list) => list.map((entry) => (entry.name === from ? { ...entry, name: to } : entry));
  mutateListing(dir, swap(oldName, newName));
  if (FM.index.has(newName)) { selectOnly(newName); scrollToRow(FM.index.get(newName)); }
  beginOp([dir]);
  try {
    // → commands::rename → files::rename
    await invoke("rename", { from: joinPath(dir, oldName), to: joinPath(dir, newName) });
    if (FM.clip && FM.clip.paths.has(joinPath(dir, oldName))) forgetClipPaths([joinPath(dir, oldName)]);
  } catch (e) {
    mutateListing(dir, swap(newName, oldName));
    if (dir === S.cwd && FM.index.has(oldName)) selectOnly(oldName);
    toast(`Couldn't rename ${oldName}: ${e}`, true);
  } finally {
    endOp([dir]);
    afterChange([dir]);
  }
}

/**
 * New folder: a row appears at once and its name is edited in place. The folder is created on
 * the Thor when the name is confirmed; Escape removes the row again without touching the Thor.
 */
function newFolder() {
  if (!S.connected) return toast("Thor is not connected", true);
  if (FM.status !== "ready") return toast("The folder is still loading");
  if (FM.edit) return;
  const name = freeName("New folder");
  const entry = { name, dir: true, size: 0, mtime: Math.floor(Date.now() / 1000), pending: true };
  mutateListing(S.cwd, (list) => [...list, entry]);
  selectOnly(name);
  startRename(name, { isNew: true });
}

/** Creates the folder for a placeholder row (see newFolder) under its final name. */
async function createFolder(tempName, name) {
  const dir = S.cwd;
  if (name !== tempName) mutateListing(dir, (list) => list.map((entry) => (entry.name === tempName ? { ...entry, name } : entry)));
  if (FM.index.has(name)) selectOnly(name);
  beginOp([dir]);
  try {
    await invoke("make_dir", { path: joinPath(dir, name) }); // → commands::make_dir → files::mkdir
    mutateListing(dir, (list) => list.map((entry) => (entry.name === name ? { ...entry, pending: false } : entry)));
  } catch (e) {
    mutateListing(dir, (list) => list.filter((entry) => entry.name !== name));
    toast(`Couldn't create ${name}: ${e}`, true);
  } finally {
    endOp([dir]);
  }
}

/** Removes a placeholder row whose name editing was cancelled. */
function dropPlaceholder(name) {
  mutateListing(S.cwd, (list) => list.filter((entry) => !(entry.pending && entry.name === name)));
}

/** F2: rename the focused entry (or the only selected one). */
function renameFocused() {
  const entries = selectedEntries();
  const entry = focusedEntry() || (entries.length === 1 ? entries[0] : null);
  if (!entry) return toast("Select an entry first");
  if (!entry.pending) startRename(entry.name);
}

/**
 * Edits the name of a row in place. Enter or clicking elsewhere confirms, Escape cancels.
 * isNew: the row is a "New folder" placeholder (confirm creates it, cancel removes it).
 */
function startRename(name, { isNew = false } = {}) {
  if (FM.edit || !FM.index.has(name)) return;
  let finished = false;
  const finish = (commit, refocus) => {
    if (finished) return;
    finished = true;
    FM.edit = null;
    const value = input.value.trim();
    input.remove();
    row._content = null; // let renderRows rewrite the name cell
    renderRows();
    if (refocus) $("#filelist").focus({ preventScroll: true });
    if (commit) submitName(name, value, isNew);
    else if (isNew) dropPlaceholder(name);
  };
  FM.edit = { name, isNew, cancel: () => finish(false, false) };
  scrollToRow(FM.index.get(name));
  const row = ROWS.get(name);
  if (!row) { FM.edit = null; return; }
  const cell = row.children[1];
  const input = document.createElement("input");
  input.type = "text";
  input.className = "rename";
  input.spellcheck = false;
  input.value = name;
  cell.textContent = "";
  cell.append(input);
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (e.key === "Enter") { e.preventDefault(); finish(true, true); }
    else if (e.key === "Escape") { e.preventDefault(); finish(false, true); }
  };
  input.onblur = () => finish(true, false);
  input.onmousedown = (e) => e.stopPropagation();
  input.ondblclick = (e) => e.stopPropagation();
  input.focus();
  // Files: preselect the name without extension, like Explorer.
  const dot = name.lastIndexOf(".");
  input.setSelectionRange(0, !isNew && dot > 0 && !S.entries.find((entry) => entry.name === name)?.dir ? dot : name.length);
}

/** Checks the edited name and starts the rename / folder creation. */
function submitName(oldName, value, isNew) {
  if (!value || value === oldName) {
    if (isNew) createFolder(oldName, oldName);
    return;
  }
  const problem = nameProblem(value, oldName);
  if (problem) {
    toast(problem, true);
    setTimeout(() => startRename(oldName, { isNew }), 0); // let the user fix it
    return;
  }
  if (isNew) createFolder(oldName, value);
  else renameEntry(oldName, value);
}

/** Del: deletes the selected entries after asking (there is no recycle bin). */
async function deleteSelected() {
  const entries = selectedOrHint().filter((entry) => !entry.pending);
  if (!entries.length) return;
  const choice = await dialog({
    title: entries.length === 1 ? `Really delete “${entries[0].name}”?` : `Really delete ${entries.length} items?`,
    body: "There is no recycle bin – this can't be undone.",
    buttons: [["Cancel", null, ""], ["Delete", "rm", "danger solid"]],
    defaultValue: null,
  });
  if (choice !== "rm") return;
  const dir = S.cwd;
  const names = new Set(entries.map((entry) => entry.name));
  const paths = entries.map((entry) => joinPath(dir, entry.name));
  mutateListing(dir, (list) => list.filter((entry) => !names.has(entry.name)));
  beginOp([dir]);
  try {
    const results = await invoke("delete_paths", { paths }); // → commands::delete_paths → files::delete_paths
    const errors = results.filter((result) => result.error);
    const failedNames = new Set(errors.map((result) => basename(result.path)));
    if (errors.length) mutateListing(dir, (list) => [...list, ...entries.filter((entry) => failedNames.has(entry.name))]);
    toastErrors(errors.map((result) => result.error));
    const deleted = entries.filter((entry) => !failedNames.has(entry.name));
    if (deleted.length) toast(`${itemsLabel(deleted)} deleted`);
    forgetClipPaths(deleted.map((entry) => joinPath(dir, entry.name)));
  } catch (e) {
    mutateListing(dir, (list) => [...list, ...entries]);
    toast(`Couldn't delete: ${e}`, true);
  } finally {
    endOp([dir]);
    afterChange([dir]);
  }
}

/** "Copy to PC": copies the selected entries to ~/Downloads/thor, one after the other. */
async function pullSelected() {
  const entries = selectedOrHint();
  if (!entries.length) return;
  const dir = S.cwd;
  let dest = null;
  for (const entry of entries) {
    toast(`Copying ${entry.name} to the PC …`);
    try {
      dest = await call("pull", { path: joinPath(dir, entry.name) }); // → commands::pull → files::pull
    } catch (_) {
      return; // shown as toast; stop instead of piling up errors
    }
  }
  toast(`Copied ${itemsLabel(entries)} to ${dest}`);
}

/** "Extract": unpacks the selected archive on the Thor into the open folder (via the copy queue). */
function extractSelected() {
  const entry = focusedEntry();
  if (!entry) return toast("Select an entry first");
  if (entry.dir || !ARCHIVE_RE.test(entry.name)) return toast("That's not an archive (zip, 7z, rar, tar …)", true);
  // Inside a game folder: skip single wrapper folders in the archive.
  const inLib = S.libs.some((lib) => S.cwd.startsWith(lib.path + "/"));
  enqueue([job(joinPath(S.cwd, entry.name), S.cwd, true, inLib, true)]);
}

$("#btn-mkdir").onclick = newFolder;
$("#btn-rename").onclick = renameFocused;
$("#btn-delete").onclick = deleteSelected;
$("#btn-pull").onclick = pullSelected;
$("#btn-unpack").onclick = extractSelected;

// ---------------------------------------------------------------- Context menu

/** The open context menu element (null if none). */
const MENU = { el: null };

function closeMenu() {
  if (!MENU.el) return;
  MENU.el.remove();
  MENU.el = null;
}

/**
 * Shows a context menu at (x, y). items: { label, kbd, run, disabled, danger } or "-" for a
 * separator. Arrow keys move between items, Enter runs one, Escape closes.
 */
function showMenu(x, y, items) {
  closeMenu();
  // No separator at the start, at the end or twice in a row.
  items = items.filter((item, i, all) => item !== "-" || (i > 0 && i < all.length - 1 && all[i - 1] !== "-"));
  const menu = el(`<div class="ctx" role="menu"></div>`);
  for (const item of items) {
    if (item === "-") { menu.append(el(`<div class="ctx-sep"></div>`)); continue; }
    const button = el(`<button class="ctx-item${item.danger ? " danger" : ""}" role="menuitem"><span></span><kbd></kbd></button>`);
    button.children[0].textContent = item.label;
    button.children[1].textContent = item.kbd || "";
    button.disabled = !!item.disabled;
    button.onclick = () => { closeMenu(); item.run(); };
    menu.append(button);
  }
  menu.onkeydown = (e) => {
    e.stopPropagation();
    const buttons = [...menu.querySelectorAll("button:not(:disabled)")];
    const at = buttons.indexOf(document.activeElement);
    if (e.key === "ArrowDown") buttons[(at + 1) % buttons.length]?.focus();
    else if (e.key === "ArrowUp") buttons[(at - 1 + buttons.length) % buttons.length]?.focus();
    else if (e.key === "Escape" || e.key === "Tab") { closeMenu(); $("#filelist").focus({ preventScroll: true }); }
    else return;
    e.preventDefault();
  };
  document.body.append(menu);
  const box = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(4, Math.min(x, innerWidth - box.width - 4))}px`;
  menu.style.top = `${Math.max(4, Math.min(y, innerHeight - box.height - 4))}px`;
  MENU.el = menu;
  menu.querySelector("button:not(:disabled)")?.focus({ preventScroll: true });
}

document.addEventListener("mousedown", (e) => { if (MENU.el && !MENU.el.contains(e.target)) closeMenu(); }, true);
window.addEventListener("blur", closeMenu);
window.addEventListener("resize", closeMenu);
$("#filelist").addEventListener("scroll", closeMenu, { passive: true });

/** Menu for the selected rows. */
function rowMenuItems() {
  const entries = selectedEntries().filter((entry) => !entry.pending);
  const one = entries.length === 1 ? entries[0] : null;
  const folder = one && one.dir ? joinPath(S.cwd, one.name) : null;
  const items = [];
  if (folder) items.push({ label: "Open", kbd: "Enter", run: () => openEntry(one) }, "-");
  items.push(
    { label: "Cut", kbd: "Ctrl+X", run: () => setClip("cut"), disabled: !entries.length },
    { label: "Copy", kbd: "Ctrl+C", run: () => setClip("copy"), disabled: !entries.length },
  );
  if (folder) items.push({ label: "Paste into folder", run: () => pasteInto(folder), disabled: !FM.clip });
  else items.push({ label: "Paste", kbd: "Ctrl+V", run: () => pasteInto(S.cwd), disabled: !FM.clip });
  items.push("-", { label: "Rename …", kbd: "F2", run: renameFocused, disabled: !one });
  items.push({ label: "Copy to PC", run: pullSelected, disabled: !entries.length });
  if (one && !one.dir && ARCHIVE_RE.test(one.name)) items.push({ label: "Extract here", run: extractSelected });
  if (folder) {
    const lib = libAt(folder);
    if (!lib) items.push({ label: "Use as games folder …", run: () => setGamesFolder() });
    else if (lib.custom) items.push({ label: "Stop using as games folder …", run: () => setGamesFolder() });
  }
  items.push("-", { label: "Delete", kbd: "Del", run: deleteSelected, danger: true, disabled: !entries.length });
  return items;
}

/** Menu for the empty area of the list. */
function backgroundMenuItems() {
  return [
    { label: "New folder", kbd: "Ctrl+Shift+N", run: newFolder },
    { label: "Paste", kbd: "Ctrl+V", run: () => pasteInto(S.cwd), disabled: !FM.clip },
    "-",
    { label: "Select all", kbd: "Ctrl+A", run: selectAll, disabled: !S.shown.length },
    { label: "Refresh", kbd: "F5", run: refreshCurrent },
  ];
}

$("#filelist").addEventListener("contextmenu", (e) => {
  e.preventDefault();
  if (!S.connected || e.target.closest("input.rename")) return;
  const row = e.target.closest(".row");
  if (row) {
    if (!FM.picked.has(row._entry.name)) selectOnly(row._entry.name);
    else { FM.focus = row._entry.name; afterSelection(); }
    showMenu(e.clientX, e.clientY, rowMenuItems());
  } else {
    clearSelection();
    showMenu(e.clientX, e.clientY, backgroundMenuItems());
  }
});

/** Context menu key / Shift+F10: menu for the focused row (or the background). */
function openMenuForFocus() {
  const row = FM.focus !== null && ROWS.get(FM.focus);
  if (row && FM.picked.has(FM.focus)) {
    const box = row.getBoundingClientRect();
    showMenu(box.left + 40, box.bottom, rowMenuItems());
  } else {
    const box = $("#filelist").getBoundingClientRect();
    showMenu(box.left + 40, box.top + 20, backgroundMenuItems());
  }
}

// ---------------------------------------------------------------- Mouse: click and selection

/**
 * Mouse state:
 * start:    where the left button went down on a row ({ x, y, name }), null otherwise
 * deferred: a press on an already selected row: collapse the selection to it (or toggle it with
 *           Ctrl) on release, unless the press turned into a drag of the whole selection
 * down:     left button is held
 * busy:     a drag-out fetch is running
 * native:   an OS drag started by us is in progress
 * items:    rows being dragged inside the window ([{ path, entry }]), null otherwise
 */
const DRAG = { start: null, deferred: null, down: false, busy: false, native: false, items: null };

/** Minimum mouse movement (px) before a press on a row counts as a drag. */
const DRAG_THRESHOLD_PX = 8;

/** Matches local paths inside the drag cache (files::drag_root), so our own drags aren't re-imported. */
const FROM_DRAG_CACHE = /[\\/]thorbro[\\/]ziehen[\\/]/;

$("#filelist").addEventListener("mousedown", (e) => {
  if (e.target.closest(".mini, input.rename")) return;
  const row = e.target.closest(".row");
  $("#filelist").focus({ preventScroll: true });
  if (!row) {
    // A press on the scrollbar also lands on the list element; that must not start a band.
    const onScrollbar = e.clientX - $("#filelist").getBoundingClientRect().left >= $("#filelist").clientWidth;
    if (e.button === 0 && !FM.edit && !onScrollbar) startBand(e);
    return;
  }
  const name = row._entry.name;
  const ctrl = e.ctrlKey || e.metaKey;
  if (e.button !== 0) return; // right button: handled by "contextmenu"
  if (e.shiftKey) selectRange(name, ctrl);
  else if (ctrl && !FM.picked.has(name)) toggleSelect(name);
  else if (ctrl || FM.picked.has(name)) { DRAG.deferred = { name, toggle: ctrl }; FM.focus = name; afterSelection(); }
  else selectOnly(name);
  if (!row._entry.pending) DRAG.start = { x: e.clientX, y: e.clientY, name };
});

$("#filelist").addEventListener("dblclick", (e) => {
  if (e.target.closest(".mini, input.rename")) return;
  const row = e.target.closest(".row");
  if (row) openEntry(row._entry);
});

$("#filelist").addEventListener("click", (e) => {
  const button = e.target.closest(".mini");
  if (!button || !FM.lib) return;
  const name = button.closest(".row")._entry.name;
  pickSystem(joinPath(S.cwd, name), FM.lib.systems[name]);
});

document.addEventListener("mousedown", (e) => { if (e.button === 0) DRAG.down = true; }, true);
document.addEventListener("mouseup", (e) => {
  if (e.button !== 0) return;
  DRAG.down = false;
  if (DRAG.items) finishInternalDrag(e);
  else if (DRAG.deferred && FM.index.has(DRAG.deferred.name)) {
    if (DRAG.deferred.toggle) toggleSelect(DRAG.deferred.name);
    else selectOnly(DRAG.deferred.name);
  }
  if (BAND.active) endBand();
  DRAG.start = null;
  DRAG.deferred = null;
}, true);

document.addEventListener("mousemove", (e) => {
  if (BAND.active) return updateBand(e);
  if (DRAG.items) return moveInternalDrag(e);
  const start = DRAG.start;
  if (!start || !DRAG.down || DRAG.busy || S.modalOpen) return;
  if (Math.hypot(e.clientX - start.x, e.clientY - start.y) < DRAG_THRESHOLD_PX) return;
  DRAG.start = null;
  DRAG.deferred = null;
  beginInternalDrag(e);
});

// ---------------------------------------------------------------- Rubber band selection

/** Rubber band: start point in list content coordinates, selection before it started, last event. */
const BAND = { active: false, x0: 0, y0: 0, base: null, moved: false, last: null };

function startBand(e) {
  const list = $("#filelist");
  const box = list.getBoundingClientRect();
  BAND.active = true;
  BAND.moved = false;
  BAND.x0 = e.clientX - box.left + list.scrollLeft;
  BAND.y0 = e.clientY - box.top + list.scrollTop;
  // Ctrl: add to / toggle in the existing selection; otherwise start fresh.
  BAND.base = e.ctrlKey || e.metaKey ? new Set(FM.picked) : new Set();
  if (!(e.ctrlKey || e.metaKey)) clearSelection();
}

function updateBand(e) {
  BAND.last = e;
  const list = $("#filelist");
  const box = list.getBoundingClientRect();
  const x = Math.min(Math.max(e.clientX - box.left, 0), list.clientWidth) + list.scrollLeft;
  const y = Math.min(Math.max(e.clientY - box.top, 0), list.clientHeight) + list.scrollTop;
  if (!BAND.moved && Math.hypot(x - BAND.x0, y - BAND.y0) < 4) return;
  BAND.moved = true;
  const band = $("#band");
  const top = Math.min(BAND.y0, y);
  const bottom = Math.max(BAND.y0, y);
  band.classList.remove("hidden");
  band.style.left = `${Math.min(BAND.x0, x)}px`;
  band.style.top = `${top}px`;
  band.style.width = `${Math.abs(x - BAND.x0)}px`;
  band.style.height = `${bottom - top}px`;
  // Rows are full width and fixed height, so the covered rows follow from the band's top and bottom.
  const count = S.shown.length;
  const first = Math.floor(top / ROW_HEIGHT_PX);
  const last = Math.min(count - 1, Math.floor(Math.max(top, bottom - 1) / ROW_HEIGHT_PX));
  const picked = new Set(BAND.base);
  for (let i = first; i <= last; i++) {
    const name = S.shown[i].name;
    if (BAND.base.has(name)) picked.delete(name);
    else picked.add(name);
  }
  FM.picked = picked;
  if (first <= last) FM.focus = S.shown[y > BAND.y0 ? last : first].name;
  afterSelection();
  autoScroll(e, true);
}

function endBand() {
  BAND.active = false;
  $("#band").classList.add("hidden");
  stopAutoScroll();
}

// ---------------------------------------------------------------- Auto-scroll while dragging

/** Scrolls the list while the mouse is near its top or bottom edge during a drag or rubber band. */
const AUTOSCROLL = { frame: 0, speed: 0, last: null };
const AUTOSCROLL_EDGE_PX = 28;
const AUTOSCROLL_MAX_PX = 30;

/** outsideToo: also scroll when the mouse is above/below the list (rubber band). */
function autoScroll(e, outsideToo) {
  const box = $("#filelist").getBoundingClientRect();
  const inside = e.clientX >= box.left && e.clientX <= box.right && e.clientY >= box.top && e.clientY <= box.bottom;
  let speed = 0;
  if (inside || outsideToo) {
    if (e.clientY < box.top + AUTOSCROLL_EDGE_PX) speed = -Math.ceil((box.top + AUTOSCROLL_EDGE_PX - e.clientY) / 2);
    else if (e.clientY > box.bottom - AUTOSCROLL_EDGE_PX) speed = Math.ceil((e.clientY - box.bottom + AUTOSCROLL_EDGE_PX) / 2);
  }
  AUTOSCROLL.speed = Math.max(-AUTOSCROLL_MAX_PX, Math.min(AUTOSCROLL_MAX_PX, speed));
  AUTOSCROLL.last = e;
  if (AUTOSCROLL.speed && !AUTOSCROLL.frame) AUTOSCROLL.frame = requestAnimationFrame(stepAutoScroll);
}

function stepAutoScroll() {
  AUTOSCROLL.frame = 0;
  if (!AUTOSCROLL.speed || !(BAND.active || DRAG.items)) return;
  const list = $("#filelist");
  const before = list.scrollTop;
  list.scrollTop += AUTOSCROLL.speed;
  if (list.scrollTop === before) return; // reached the end
  renderRows();
  // Re-run the move handler: it updates the band / drop target and schedules the next step.
  if (BAND.active) updateBand(AUTOSCROLL.last);
  else moveInternalDrag(AUTOSCROLL.last);
}

function stopAutoScroll() {
  AUTOSCROLL.speed = 0;
  cancelAnimationFrame(AUTOSCROLL.frame);
  AUTOSCROLL.frame = 0;
}

// ---------------------------------------------------------------- Dragging rows inside the window
//
// Dragging selected rows onto a folder row or a breadcrumb moves them there (hold Ctrl to copy).
// Leaving the window with the mouse button held turns it into a drag out to the PC.

function beginInternalDrag(e) {
  const entries = selectedEntries().filter((entry) => !entry.pending);
  if (!entries.length) return;
  DRAG.items = entries.map((entry) => ({ path: joinPath(S.cwd, entry.name), entry: { ...entry } }));
  DRAG.label = itemsLabel(entries);
  document.body.classList.add("dragging");
  $("#drag-ghost").classList.remove("hidden");
  moveInternalDrag(e);
}

/** True if the mouse is outside the window (we still get events while the button is held). */
function outsideWindow(e) {
  return e.clientX < 0 || e.clientY < 0 || e.clientX >= innerWidth || e.clientY >= innerHeight;
}

/** The folder under the mouse that the dragged rows could go into: { path, row } or null. */
function dropTargetAt(x, y) {
  const under = document.elementFromPoint(x, y);
  if (!under) return null;
  const row = under.closest("#filelist .row");
  if (row) {
    const entry = row._entry;
    const path = joinPath(S.cwd, entry.name);
    if (!entry.dir || entry.pending || DRAG.items.some((item) => item.path === path)) return null;
    return { path, row: entry.name };
  }
  const crumb = under.closest("#crumbs button[data-path]");
  if (crumb && crumb.dataset.path !== S.cwd) return { path: crumb.dataset.path, crumb };
  return null;
}

function moveInternalDrag(e) {
  if (!e) return;
  if (outsideWindow(e)) return dragOutOfWindow();
  const copy = e.ctrlKey || e.metaKey;
  const target = dropTargetAt(e.clientX, e.clientY);
  const ghost = $("#drag-ghost");
  ghost.style.transform = `translate(${e.clientX + 14}px, ${e.clientY + 14}px)`;
  ghost.textContent = target ? `${copy ? "Copy" : "Move"} ${DRAG.label} to ${basename(target.path) || target.path}` : DRAG.label;
  ghost.classList.toggle("copy", copy);
  const rowName = target && target.row ? target.row : null;
  if (FM.dropTarget !== rowName) { FM.dropTarget = rowName; renderRows(); }
  document.querySelectorAll("#crumbs button.drop").forEach((button) => button !== (target && target.crumb) && button.classList.remove("drop"));
  if (target && target.crumb) target.crumb.classList.add("drop");
  autoScroll(e, false);
}

function endInternalDrag() {
  DRAG.items = null;
  document.body.classList.remove("dragging");
  $("#drag-ghost").classList.add("hidden");
  document.querySelectorAll("#crumbs button.drop").forEach((button) => button.classList.remove("drop"));
  stopAutoScroll();
  if (FM.dropTarget !== null) { FM.dropTarget = null; renderRows(); }
}

function finishInternalDrag(e) {
  const items = DRAG.items;
  const target = dropTargetAt(e.clientX, e.clientY);
  endInternalDrag();
  if (target) transferInto(target.path, items, !(e.ctrlKey || e.metaKey));
}

// Leaving the page area while dragging rows: hand over to the OS drag (drag out to the PC).
document.documentElement.addEventListener("mouseleave", () => { if (DRAG.items && DRAG.down) dragOutOfWindow(); });

function dragOutOfWindow() {
  const items = DRAG.items;
  endInternalDrag();
  if (items && !DRAG.busy) dragOut(items);
}

// ---------------------------------------------------------------- Drag out to the PC
//
// The files are fetched into a local cache first (drag_fetch), then a real OS drag starts
// (drag_start) if the mouse button is still held.

async function dragOut(items) {
  DRAG.busy = true;
  const label = items.length === 1 ? basename(items[0].path) : `${items.length} items`;
  // Only mention the fetch if it takes noticeably long (cached files are instant).
  const slowHint = setTimeout(() => toast(`Fetching ${label} from the Thor, then you can drag it out …`), 500);
  try {
    const locals = [];
    for (const item of items) {
      // → commands::drag_fetch → files::drag_fetch (progress via "transfer" events)
      const fetched = await call("drag_fetch", { path: item.path });
      locals.push(fetched.local);
      FM.ready.add(item.path);
    }
    renderRows();
    if (DRAG.down) {
      DRAG.native = true;
      await invoke("drag_start", { paths: locals }); // → commands::drag_start (emits "drag-done")
    } else {
      toast(`${label} is ready – drag it out again now`);
    }
  } catch (_) {
    // The error was already shown as a toast.
  } finally {
    clearTimeout(slowHint);
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
