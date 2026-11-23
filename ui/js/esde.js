// ThorBro UI – "Import ES-DE info": writes ES-DE systeminfo.txt files into the system folders of
// a games folder that ES-DE didn't create. Shows a preview first; nothing is written before the
// user confirms. The Rust side (src/esde_import.rs) checks everything again before writing.
"use strict";

/**
 * The games folder to work on: the selected folder if it is a library, else the library the open
 * folder belongs to, else the selected folder, else the open folder.
 */
function esdeTargetPath() {
  const entry = S.shown && S.shown[S.sel];
  const selectedDir = entry && entry.dir ? `${S.cwd}/${entry.name}` : null;
  if (selectedDir && libAt(selectedDir)) return selectedDir;
  const containing = S.libs
    .filter((lib) => S.cwd === lib.path || S.cwd.startsWith(lib.path + "/"))
    .sort((a, b) => b.path.length - a.path.length)[0];
  if (containing) return containing.path;
  return selectedDir || S.cwd;
}

/** "Import ES-DE info …" (right-click on empty list space inside a games folder). */
async function importEsdeInfo() {
  if (!S.connected) return toast("Thor is not connected", true);
  const path = esdeTargetPath();
  const close = busyModal(`Looking at ${path} …`);
  let plan;
  try {
    plan = await call("esde_preview", { path }); // → commands::esde_preview (writes nothing)
  } catch (_) {
    return; // shown as toast
  } finally {
    close();
  }
  if (!plan.length) return toast(`${path} has no sub folders`, true);
  const request = await esdePreviewDialog(path, plan);
  if (!request) return;
  let results;
  try {
    results = await call("esde_import", { request }); // → commands::esde_import
  } catch (_) {
    return;
  }
  await loadPlaces(false); // new systems and renamed folders show up right away
  esdeResultDialog(results);
}

/**
 * Preview: one row per folder with the proposed action. Resolves with the request for
 * esde_import ({ path, write, overwrite, rename }) or null if cancelled.
 */
function esdePreviewDialog(path, plan) {
  return modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>Import ES-DE info</h2><p class="dim">${esc(path)}</p></div>`));
    const body = el(`<div class="modal-body"></div>`);
    body.append(el(`<div><p>ThorBro writes a <code>systeminfo.txt</code> (the file ES-DE creates in every system folder)
      into each folder it recognizes. ROM files are never touched, unknown folders are left alone.</p>
      <p class="dim">Note: ES-DE finds systems by their <b>folder name</b>, not by systeminfo.txt. A folder is only
      detected by ES-DE if it has the name ES-DE expects (e.g. <code>wii</code>), so a systeminfo.txt alone is not enough.</p></div>`));
    const rows = plan.map((item) => buildEsdeRow(item));
    rows.forEach((row) => body.append(row.element));

    const existing = plan.filter((item) => item.action === "exists");
    let overwriteBox = null;
    if (existing.length) {
      const label = el(`<label class="unpack"><input type="checkbox"> Override existing systeminfo.txt files (${existing.length})</label>`);
      overwriteBox = label.querySelector("input");
      overwriteBox.onchange = () => rows.forEach((row) => row.setOverwrite(overwriteBox.checked));
      body.append(label);
    }
    box.append(body);

    const foot = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">Import</button></div>`);
    foot.children[0].onclick = () => close(null);
    foot.children[1].onclick = () => {
      const overwrite = !!(overwriteBox && overwriteBox.checked);
      const write = rows.filter((row) => row.wantsWrite(overwrite)).map((row) => row.item.folder);
      const rename = rows.filter((row) => row.wantsRename()).map((row) => row.item.folder);
      if (!write.length && !rename.length) return toast("Nothing selected");
      close({ path, write, overwrite, rename });
    };
    box.append(foot);
  }, { wide: true });
}

/** One preview row: folder, system, and the action with its checkboxes. */
function buildEsdeRow(item) {
  const element = el(`<div class="esde-row"><div class="nm"></div><div class="dim sys"></div><div class="act"></div></div>`);
  element.querySelector(".nm").textContent = item.folder;
  const origin = item.origin === "manual" ? " (set by you)" : "";
  element.querySelector(".sys").textContent = item.system ? `${item.full}${origin}` : "Unknown system";
  const act = element.querySelector(".act");

  let writeBox = null;
  let existsNote = null;
  if (item.action === "create") {
    const label = el(`<label><input type="checkbox" checked> Create systeminfo.txt</label>`);
    writeBox = label.querySelector("input");
    act.append(label);
  } else if (item.action === "exists") {
    existsNote = el(`<span class="dim">Has a systeminfo.txt – kept</span>`);
    act.append(existsNote);
  } else {
    act.append(el(`<span class="dim">Unknown folder – skipped. Set its system in the file list to include it.</span>`));
  }

  let renameBox = null;
  if (item.esde_name && item.case_only && !item.rename_conflict) {
    const label = el(`<label><input type="checkbox"> Rename to ES-DE name “${esc(item.esde_name)}”</label>`);
    renameBox = label.querySelector("input");
    act.append(label);
  } else if (item.esde_name && item.rename_conflict) {
    act.append(el(`<span class="bad">Can't rename to “${esc(item.esde_name)}”: a folder with that name already exists</span>`));
  } else if (item.esde_name) {
    act.append(el(`<span class="warn">ES-DE expects the folder name “${esc(item.esde_name)}” – rename it yourself if ES-DE should find it</span>`));
  }

  return {
    element,
    item,
    wantsWrite: (overwrite) => (writeBox ? writeBox.checked : item.action === "exists" && overwrite),
    wantsRename: () => !!(renameBox && renameBox.checked),
    setOverwrite: (on) => {
      if (existsNote) {
        existsNote.textContent = on ? "Has a systeminfo.txt – will be replaced" : "Has a systeminfo.txt – kept";
        existsNote.className = on ? "warn" : "dim";
      }
    },
  };
}

/** Shows what happened per folder. */
function esdeResultDialog(results) {
  const failed = results.filter((result) => !result.ok).length;
  const lines = results
    .map((result) => `<div class="${result.ok ? "" : "bad"}">${result.ok ? "✓" : "✗"} <b>${esc(result.folder)}</b>: ${esc(result.text)}</div>`)
    .join("");
  dialog({
    title: failed ? `Import finished – ${failed} with problems` : "Import finished",
    html: true,
    body: lines || "<p>Nothing was changed.</p>",
  });
}
