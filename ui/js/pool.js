// ThorBro UI – the file pool in the sidebar, the "are you sure?" check for large transfers, and
// copying Thor files to the PC.
//
// The file pool is the only way to get files from the Thor to the PC. Rows dragged into it (or
// added with right-click "Add to file pool") are only collected as references (path + the listing
// entry), from any number of folders; nothing on the Thor changes. "Copy to PC…" asks for a PC
// folder and copies everything there through the copy queue (progress, Cancel, never overwrites,
// partial copies removed: files.rs pull_one). The pool lives in memory for this session only.
//
// guardedTransfer is the shared "large amount? ask first" step for moves and copies *on* the Thor
// (direct drops onto a folder, paste). Those use transferInto (files-actions.js): moves inside one
// storage are one `mv`, everything else goes through the copy queue, which never deletes a source
// before its copy is verified (files.rs: remove_source_after_move).
"use strict";

// ---------------------------------------------------------------- Tuning

/** Moving or copying this much (or more) at once asks first. */
const CONFIRM_BYTES = 1024 ** 3;
/** …or this many items (a folder counts as one item). */
const CONFIRM_ITEMS = 20;

// ---------------------------------------------------------------- State

/**
 * items: [{ path, name, dir, size, sizeState, entry }] in the order they were added.
 *   size:      bytes (files: from the listing; folders: asked from the Thor with `du`)
 *   sizeState: "known" | "pending" (being asked) | "unknown" (the Thor couldn't say)
 * busy: "Copy to PC…" is being prepared (size check, confirmation, folder picker).
 */
const POOL = { items: [], busy: false };

/** True if `path` is `folder` or lies inside it. */
const isWithin = (path, folder) => path === folder || path.startsWith(folder + "/");

/** True if the Thor path is in the pool, itself or through a pooled folder (the list tags it "in pool"). */
function inPool(path) {
  return POOL.items.some((item) => isWithin(path, item.path));
}

/**
 * Adds rows ({ path, entry }) to the pool. A path that is already in (or inside a folder that is
 * in) the pool is ignored; adding a folder replaces pooled items inside it. Returns how many were added.
 */
function addToPool(items) {
  let added = 0;
  for (const { path, entry } of items) {
    if (entry.pending) continue;
    if (POOL.items.some((pooled) => isWithin(path, pooled.path))) continue;
    POOL.items = POOL.items.filter((pooled) => !isWithin(pooled.path, path));
    const known = !entry.dir;
    POOL.items.push({ path, name: basename(path), dir: entry.dir, size: known ? entry.size : 0, sizeState: known ? "known" : "pending", entry: { ...entry } });
    added++;
  }
  if (added) { renderPool(); measurePoolFolders(); }
  return added;
}

/** Adds the selected rows (right-click "Add to file pool"). */
function addSelectionToPool() {
  const items = selectedEntries().filter((entry) => !entry.pending).map((entry) => ({ path: joinPath(S.cwd, entry.name), entry }));
  if (!items.length) return toast("Select an entry first");
  reportPoolAdd(addToPool(items), items.length);
}

/** Tells what adding to the pool did (also covers "already in the pool"). */
function reportPoolAdd(added, offered) {
  if (added) toast(`Added ${added === 1 ? "1 item" : `${added} items`} to the file pool – nothing on the Thor changed`);
  else toast(offered === 1 ? "That's already in the file pool" : "Those are already in the file pool");
}

function removeFromPool(path) {
  POOL.items = POOL.items.filter((item) => item.path !== path);
  renderPool();
}

function clearPool() {
  POOL.items = [];
  renderPool();
}

/** The pool as `{ path, entry }` rows. */
const poolRows = () => POOL.items.map((item) => ({ path: item.path, entry: { ...item.entry } }));

/** Asks the Thor for the sizes of pooled folders (one call for all that are still pending). */
async function measurePoolFolders() {
  const pending = POOL.items.filter((item) => item.sizeState === "pending");
  if (!pending.length) return;
  let sizes;
  try {
    sizes = await invoke("remote_sizes", { paths: pending.map((item) => item.path) }); // → commands::remote_sizes
  } catch (_) {
    sizes = pending.map(() => null);
  }
  pending.forEach((item, i) => {
    if (!POOL.items.includes(item)) return; // removed meanwhile
    item.size = sizes[i] ?? 0;
    item.sizeState = sizes[i] == null ? "unknown" : "known";
  });
  renderPool();
}

/** Drops pooled items that no longer exist on the Thor (after a move, a delete or a rename). */
async function prunePool() {
  if (!POOL.items.length || !S.connected) return;
  const asked = POOL.items.map((item) => item.path);
  let found;
  try {
    found = await invoke("paths_exist", { paths: asked }); // → commands::paths_exist
  } catch (_) {
    return; // can't ask (unplugged?): keep everything rather than guess
  }
  const gone = new Set(asked.filter((path, i) => !found[i]));
  if (!gone.size) return;
  POOL.items = POOL.items.filter((item) => !gone.has(item.path));
  renderPool();
}

// ---------------------------------------------------------------- Sizes and confirmation

/**
 * How much `rows` ({ path, entry }) are: { bytes, unknown } where `unknown` counts folders the
 * Thor couldn't measure. Files use their listing size; folders are asked fresh (`du`, one call).
 */
async function measureRows(rows) {
  let bytes = 0;
  const folders = [];
  for (const { path, entry } of rows) {
    if (entry.dir) folders.push(path);
    else bytes += entry.size || 0;
  }
  let unknown = 0;
  if (folders.length) {
    let sizes;
    try {
      sizes = await invoke("remote_sizes", { paths: folders });
    } catch (_) {
      sizes = folders.map(() => null);
    }
    for (const size of sizes) { if (size == null) unknown++; else bytes += size; }
  }
  return { bytes, unknown };
}

/** "21.2 GB", "21.2 GB+" when some folders couldn't be measured. */
const sizeText = ({ bytes, unknown }) => human(bytes) + (unknown ? "+" : "");

/**
 * Should this much be confirmed first? `slow` = it goes through the copy queue (another storage,
 * a copy, or to the PC), where an unmeasurable folder could be huge, so it counts as large.
 */
function isLargeTransfer(count, { bytes, unknown }, slow) {
  return count >= CONFIRM_ITEMS || bytes >= CONFIRM_BYTES || (slow && unknown > 0);
}

/** Splits items into those moved with one instant `mv` and those that need the copy queue. */
function splitTransfer(dest, items, move) {
  const destStorage = storageOf(dest);
  const instant = move ? items.filter((item) => destStorage && storageOf(item.path) === destStorage) : [];
  return { instant, queued: items.filter((item) => !instant.includes(item)) };
}

/**
 * Moves / copies `items` into the Thor folder `dest` after asking if it is a large amount.
 * Returns true if it was started (false: cancelled).
 *
 * Only transfers that need the copy queue are measured – an instant `mv` of a big folder inside
 * one storage is harmless, and measuring a folder first would delay it by an adb round trip –
 * otherwise only the item count is checked.
 */
async function guardedTransfer(dest, items, move) {
  const { queued } = splitTransfer(dest, items, move);
  const slow = queued.length > 0;
  const size = slow ? await measureRows(items) : { bytes: 0, unknown: 0 };
  if (isLargeTransfer(items.length, size, slow)) {
    const verb = move ? "Move" : "Copy";
    const what = items.length === 1 ? `“${basename(items[0].path)}”` : `${items.length} items`;
    const body = slow
      ? "This can take a while. You can stop it with Cancel in the progress bar. " +
        (move ? "The originals are only removed after the copies are verified." : "The originals are not touched.")
      : "This happens at once, there is no undo. If it was the wrong folder, move the items back by hand.";
    const choice = await dialog({
      title: `${verb} ${what} (${sizeText(size)}) to “${basename(dest) || dest}”?`,
      html: true,
      body: `<p><b>${esc(dest)}</b></p><p class="dim">${esc(body)}</p>`,
      buttons: [["Cancel", null, ""], [verb, "ok", "primary"]],
      defaultValue: null,
    });
    if (choice !== "ok") return false;
  }
  await transferInto(dest, items, move);
  return true;
}

// ---------------------------------------------------------------- Copy to PC

/**
 * Asks for a folder on this PC and copies `rows` ({ path }) into it through the copy queue
 * (progress, Cancel, "file X of Y"). Existing files there are never overwritten and the Thor is
 * only read. Returns true if a folder was chosen.
 */
async function saveToPc(rows) {
  if (!rows.length) return false;
  const picked = await T.dialog.open({ directory: true, multiple: false, title: "Copy to PC – choose a folder" });
  const dest = [].concat(picked || [])[0];
  if (!dest) return false;
  enqueue(rows.map((row) => ({ ...job(row.path, dest), pull: true })));
  return true;
}

/** "Copy to PC…": a large pool is confirmed first (count + size), then the folder picker. */
async function copyPoolToPc() {
  if (!S.connected) return toast("Thor is not connected", true);
  await poolAction(async () => {
    const rows = poolRows();
    const size = await measureRows(rows);
    if (isLargeTransfer(rows.length, size, true)) {
      const what = rows.length === 1 ? `“${basename(rows[0].path)}”` : `${rows.length} items`;
      const choice = await dialog({
        title: `Copy ${what} (${sizeText(size)}) to your PC?`,
        body: "This can take a while and needs that much free space on the PC. You can stop it with Cancel in the progress bar. The Thor is not changed.",
        buttons: [["Cancel", null, ""], ["Choose folder…", "ok", "primary"]],
        defaultValue: null,
      });
      if (choice !== "ok") return;
    }
    await saveToPc(rows);
  });
}

/** Runs a pool action with the buttons locked, so a second click can't start it twice. */
async function poolAction(action) {
  if (POOL.busy || !POOL.items.length) return;
  POOL.busy = true;
  renderPool();
  try {
    await action();
  } finally {
    POOL.busy = false;
    renderPool();
  }
}

// ---------------------------------------------------------------- Sidebar card

function poolTotals() {
  const bytes = POOL.items.reduce((sum, item) => sum + item.size, 0);
  const pending = POOL.items.filter((item) => item.sizeState === "pending").length;
  const unknown = POOL.items.filter((item) => item.sizeState === "unknown").length;
  return { bytes, pending, unknown };
}

function itemSizeText(item) {
  if (!item.dir) return human(item.size);
  return item.sizeState === "pending" ? "…" : item.sizeState === "unknown" ? "?" : human(item.size);
}

/**
 * Redraws the pool card: empty (dashed border, mascot "wait", intro text, "Drop here") or filled
 * (solid border, mascot "ask", count + size, item rows, "Copy to PC…" and "Clear pool"). Also
 * refreshes the "in pool" tags in the list and the hint line, which depend on the pool.
 */
function renderPool() {
  const count = POOL.items.length;
  const card = $("#pool");
  card.classList.toggle("is-empty", !count);
  card.classList.toggle("is-filled", count > 0);
  $("#pool-mascot").src = `assets/mascot/thorbro-${count ? "ask" : "wait"}.png`;
  $("#pool-intro").classList.toggle("hidden", count > 0);
  $("#pool-drop").classList.toggle("hidden", count > 0);
  $("#pool-foot").classList.toggle("hidden", !count);
  const totals = poolTotals();
  $("#pool-count").textContent = count
    ? `${count} ${count === 1 ? "item" : "items"} · ${human(totals.bytes)}${totals.pending ? " …" : totals.unknown ? "+" : ""}`
    : "";
  const list = $("#pool-items");
  list.textContent = "";
  for (const item of POOL.items) {
    const row = el(`<div class="pool-item"><span class="nm"></span><span class="sz"></span><button type="button" class="x" aria-label="Remove from pool">×</button></div>`);
    row.children[0].textContent = item.dir ? `${item.name}/` : item.name;
    row.children[0].title = item.path;
    row.children[1].textContent = itemSizeText(item);
    row.children[2].onclick = () => removeFromPool(item.path);
    list.append(row);
  }
  $("#pool-save").disabled = !count || !S.connected || POOL.busy;
  $("#pool-clear").disabled = !count || POOL.busy;
  renderRows(); // files.js: "in pool" tags
  updateHint(); // files.js
}

$("#pool-save").onclick = copyPoolToPc;
$("#pool-clear").onclick = clearPool;
