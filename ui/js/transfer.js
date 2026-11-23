// ThorBro UI – sending files to the Thor: file dialogs, drag & drop onto the window, the
// "Where should it go?" sort dialog, the copy queue and its progress bar.
// The drop handler also feeds the import dialog (saves-import.js) via S.importTarget.
"use strict";

// ---------------------------------------------------------------- Copy jobs

/** A copy job as expected by files::Job in Rust. */
const job = (src, dest, unpack = false, flatten = false, remote = false) => ({ src, dest, unpack, flatten, remote });

/** Archive file names (same list as unpack::EXTS in Rust). */
const ARCHIVE_RE = /\.(zip|7z|rar|tar|tgz|txz|tbz2|tar\.(gz|xz|bz2|zst))$/i;

/** Hands jobs to the Rust copy queue. Progress comes back as "transfer" events. */
function enqueue(jobs) {
  if (!jobs.length) return;
  invoke("enqueue", { jobs }); // → commands::enqueue → files::enqueue
}

// ---------------------------------------------------------------- Sending

$("#btn-send-files").onclick = async () => {
  const picked = await T.dialog.open({ multiple: true, title: "Files for the Thor" });
  if (picked) incoming([].concat(picked));
};
$("#btn-send-dirs").onclick = async () => {
  const picked = await T.dialog.open({ multiple: true, directory: true, title: "Folders for the Thor" });
  if (picked) incoming([].concat(picked));
};

/**
 * Local paths that should go to the Thor. Games are sorted via the dialog, anything
 * else goes straight into the current folder.
 */
async function incoming(paths) {
  paths = paths.filter(Boolean);
  if (!paths.length) return;
  if (!S.connected) {
    await pollDevice(true);
    if (!S.connected) return toast("Thor is not connected", true);
  }
  if (!S.libs.length) return askCopyWithoutLibrary(paths);
  // → commands::classify → library::classify per file (empty list = no game among them)
  const items = await call("classify", { paths, cwd: S.cwd, lib: 0, force: false });
  // Nothing game-like: copy straight into the current folder, as before.
  if (!items.length) return enqueue(paths.map((path) => job(path, S.cwd)));
  sortDialog(paths, items);
}

/**
 * Without a games folder nothing can be sorted. Instead of silently copying into the current
 * folder, ask first and offer to set up the games folder.
 */
async function askCopyWithoutLibrary(paths) {
  const count = paths.length;
  const what = count === 1 ? "this file" : `these ${count} files`;
  const choice = await dialog({
    title: "No games folder set up",
    html: true,
    body: `<p>ThorBro can't sort games without a games folder, so ${what} would go into the open folder:</p>
      <p><b>${esc(S.cwd)}</b></p>
      <p class="dim">To set up your games folder, cancel, open or select it in the list (e.g. /sdcard/lisu/Roms)
      and press “Set games folder”. Or use the open folder as games folder right away.</p>`,
    buttons: [["Cancel", null, ""], ["Use open folder as games folder", "setup", ""], ["Copy here", "copy", "primary"]],
  });
  if (choice === "copy") enqueue(paths.map((path) => job(path, S.cwd)));
  if (choice === "setup" && await setGamesFolder(true)) incoming(paths);
}

// ---------------------------------------------------------------- Sort dialog

/**
 * "Where should it go?": one row per dropped file with a target drop-down (preselected by
 * Rust) and an optional "extract first" checkbox.
 * items: Incoming objects from commands::classify.
 */
function sortDialog(paths, items) {
  modal((box, close) => {
    const body = el(`<div class="modal-body"></div>`);
    let rows = [];
    const fill = (list) => {
      body.innerHTML = "";
      rows = list.map((item) => {
        const row = buildSortRow(item);
        body.append(row.element);
        return row;
      });
    };
    box.append(buildSortHead(paths, fill));
    box.append(body);
    fill(items);
    box.append(buildSortFoot(box, close, () => rows));
  }, { wide: true });
}

/** Title plus, with several libraries, a library drop-down that re-classifies the files. */
function buildSortHead(paths, fill) {
  const count = paths.length;
  const head = el(`<div class="modal-head" style="display:flex;gap:10px;align-items:center;flex-wrap:wrap">
      <div style="flex:1;min-width:200px"><h2>Where should it go?</h2><p class="dim">${count} ${count === 1 ? "file" : "files"} – check the target, change it if needed</p></div></div>`);
  if (S.libs.length > 1) {
    const librarySelect = el(`<select title="Game library"></select>`);
    librarySelect.innerHTML = S.libs.map((lib, i) => `<option value="${i}">${esc(lib.label)} (${lib.total} Games)</option>`).join("");
    librarySelect.onchange = async () => {
      // → commands::classify again, for the chosen library (force: always return all rows)
      fill(await call("classify", { paths, cwd: S.cwd, lib: +librarySelect.value, force: true }));
    };
    head.append(librarySelect);
  }
  return head;
}

/** One row of the sort dialog. Returns { element, it, sel, box } (box = unpack checkbox or null). */
function buildSortRow(item) {
  const whyClass = item.guess.system ? (item.guess.sure ? "" : "unsure") : "none";
  const row = el(`<div class="sort-row">
          <span>${item.dir ? "📁" : "🎮"}</span>
          <div style="min-width:0"><div class="nm"></div><div class="why ${whyClass}"></div></div>
          <select></select></div>`);
  row.querySelector(".nm").textContent = item.name;
  row.querySelector(".nm").title = item.path;
  const why = row.querySelector(".why");
  why.textContent = `${item.size} · ${item.guess.reason}`;
  why.title = item.guess.reason;
  const targetSelect = row.querySelector("select");
  targetSelect.innerHTML = item.targets.map((target, i) => `<option value="${i}">${esc(target.label)}</option>`).join("");
  targetSelect.value = String(item.selected);
  let unpackBox = null;
  if (item.archive) {
    const label = el(`<label class="unpack"><input type="checkbox"> Extract first (copy the contents instead of the archive)</label>`);
    unpackBox = label.querySelector("input");
    unpackBox.checked = item.unpack;
    row.append(label);
  }
  return { element: row, it: item, sel: targetSelect, box: unpackBox };
}

/**
 * Cancel / "All to current folder" / Copy. getRows() returns the current rows.
 * Enter anywhere in the modal box (except in a drop-down) presses Copy.
 */
function buildSortFoot(box, close, getRows) {
  const foot = el(`<div class="modal-foot">
      <button>Cancel</button><button title="${esc(S.cwd)}">All to current folder</button><button class="primary">Copy</button></div>`);
  const [cancel, allHere, copy] = foot.children;
  cancel.onclick = () => close(null);
  // Recognized games are flattened (single wrapper folders in archives are skipped).
  const makeJob = ({ it, box }, dest) => job(it.path, dest, !!(box && box.checked), !!it.guess.system);
  allHere.onclick = () => { close(1); enqueue(getRows().map((row) => makeJob(row, S.cwd))); };
  copy.onclick = () => { close(1); enqueue(getRows().map((row) => makeJob(row, row.it.targets[+row.sel.value].path))); };
  box.addEventListener("keydown", (e) => { if (e.key === "Enter" && e.target.tagName !== "SELECT") copy.click(); });
  setTimeout(() => copy.focus(), 0);
  return foot;
}

// ---------------------------------------------------------------- Progress strip
//
// The strip at the bottom of the Files view shows the "transfer" events Rust emits while copying
// (files.rs: copy queue, drag-out fetch, "Copy to PC"): mascot, what is being copied, "file X of Y",
// percent, the bar and Cancel. A finished batch stays visible for a moment, then the strip hides.

/** How long the "done" state of the strip stays visible (ms). */
const TRANSFER_DONE_MS = 6000;

/** Strip state. `pull`: { index, count } while "Copy to PC" runs (its events carry no position). */
const STRIP = { pull: null, hideTimer: 0 };

/** Shows one progress event (files::Progress) on the strip. */
function showTransfer(progress) {
  clearTimeout(STRIP.hideTimer);
  const strip = $("#progress");
  strip.classList.remove("hidden");
  const failed = !progress.busy && !!progress.error;
  strip.classList.toggle("done", !progress.busy && !failed);
  strip.classList.toggle("failed", failed);
  $("#p-mascot").src = `assets/mascot/thorbro-${progress.busy ? "busy" : failed ? "ask" : "done"}.svg`;
  $("#p-text").textContent = progress.text;
  $("#p-detail").textContent = failed ? progress.error : progress.detail;

  // "file X of Y": the copy queue sends its position; "Copy to PC" is counted in pullSelected.
  const position = progress.count > 1 ? progress : (STRIP.pull && !progress.cancellable ? STRIP.pull : null);
  const meta = [];
  if (progress.busy && position && position.count > 1) meta.push(`file ${position.index} of ${position.count}`);
  // fraction 0 while busy = unknown → animated bar at 30 %, no percent.
  const unknown = progress.busy && progress.fraction <= 0;
  if (progress.busy && !unknown) meta.push(`${Math.round(progress.fraction * 100)}%`);
  $("#p-meta").textContent = meta.join(" · ");
  $("#p-bar").parentElement.classList.toggle("indet", unknown);
  $("#p-bar").style.width = `${Math.round((unknown ? 0.3 : progress.fraction) * 100)}%`;
  $("#p-cancel").classList.toggle("hidden", !(progress.busy && progress.cancellable));
  if (!progress.busy) STRIP.hideTimer = setTimeout(hideTransfer, TRANSFER_DONE_MS);
}

function hideTransfer() {
  clearTimeout(STRIP.hideTimer);
  $("#progress").classList.add("hidden");
}

/**
 * "Copy to PC" (files-actions.js) has no final event: it calls pullProgress() before each entry
 * and pullFinished(text) or pullFinished(null) afterwards (null = failed, the error is a toast).
 */
function pullProgress(index, count) {
  STRIP.pull = { index, count };
}
function pullFinished(text) {
  STRIP.pull = null;
  if (text) showTransfer({ busy: false, text, detail: "", fraction: 1, error: null, cancellable: false, count: 0 });
  else hideTransfer();
}

// Emitted by files.rs (emit_progress) while copying / extracting / fetching.
T.event.listen("transfer", async ({ payload: progress }) => {
  showTransfer(progress);
  if (!progress.busy) await onTransferFinished(progress);
});

/**
 * After a batch: toast, refresh game counts, refresh the open folder in the background (keyed
 * diff, so nothing flickers). A copy job may also have removed its source (move across storages),
 * so the open folder is refreshed after every copy batch, not only when it was the target.
 */
async function onTransferFinished(progress) {
  if (progress.error) toast(progress.error, true);
  else toast(progress.text);
  if (progress.dests.length) {
    for (const dest of progress.dests) DIR_CACHE.delete(dest);
    refreshCurrent();
  }
  await refreshLibs(); // game counts changed
  if (progress.dests.length) renderFiles();
}

/** Cancel button next to the progress bar (only shown for copy jobs, see Progress.cancellable). */
$("#p-cancel").onclick = async () => {
  const choice = await dialog({
    title: "Cancel the transfer?",
    body: "The copy that is running is stopped and its partial copy on the Thor removed. " +
      "If an archive is being fetched or extracted right now, it stops before uploading.",
    buttons: [["Keep copying", null, ""], ["Cancel this one, copy the rest", "one", ""], ["Cancel everything", "all", "danger solid"]],
    defaultValue: null,
  });
  if (!choice) return;
  // → commands::cancel_transfer → files::cancel (false = nothing was running anymore)
  if (await call("cancel_transfer", { clearQueue: choice === "all" })) {
    $("#p-detail").textContent = "Cancelling …";
    $("#p-cancel").classList.add("hidden");
  }
};

// ---------------------------------------------------------------- Drag & drop onto the window

T.webview.getCurrentWebview().onDragDropEvent((ev) => {
  const type = ev.payload.type;
  const overlay = $("#drop-overlay");
  if (type === "enter" || type === "over") {
    // Ignore our own drag-out passing over the window.
    if (DRAG.native || (ev.payload.paths || []).some((path) => FROM_DRAG_CACHE.test(path))) return;
    if (S.modalOpen && !S.importTarget) return;
    overlay.classList.remove("hidden");
    $("#drop-overlay div").textContent = S.importTarget ? "Drop to import" : "Drop to send";
  } else if (type === "leave") {
    overlay.classList.add("hidden");
  } else if (type === "drop") {
    overlay.classList.add("hidden");
    const paths = (ev.payload.paths || []).filter((path) => !FROM_DRAG_CACHE.test(path));
    if (!paths.length) return; // our own drag came back into the window
    if (S.importTarget) return S.importTarget(paths[0]);
    if (S.modalOpen) return;
    if (S.tab !== "files") showTab("files");
    incoming(paths);
  }
});
