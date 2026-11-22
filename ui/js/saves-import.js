// ThorBro UI – importing a save package (e.g. downloaded from the internet):
//   importWizard → choose source (PC archive/folder, drop, or search the Thor)
//   runImport    → fetch + extract + clean in Rust                 (commands::prepare_import)
//   importReview → check the suggested folder, choose targets, go  (commands::check_root, do_import)
// The work folder in Rust is removed afterwards with commands::cleanup_import.
"use strict";

// ---------------------------------------------------------------- Choose source

/** Asks where the save package comes from. Files dropped on the window are accepted too. */
function importWizard(game) {
  const cfg = (cards[game] || {}).cfg || {};
  const sides = ["pc", "thor"].filter((side) => cfg[side]);
  modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(game)}: Import save</h2>
      <p class="dim">Drag an archive (zip, 7z, rar) or a folder into the window, or pick one. Readmes, images and ROMs are filtered out, and I'll find the folder in the package that matches your save.</p></div>`));
    const body = el(`<div class="modal-body big-choice">
      <button data-k="file">📦 Archive or file on the PC<small>zip, 7z, rar, tar or a single save file</small></button>
      <button data-k="dir">📁 Folder on the PC<small>e.g. an already extracted save</small></button>
      <button data-k="thor">🎮 Search on the Thor<small>Download folder and SD card</small></button></div>`);
    box.append(body);
    const foot = el(`<div class="modal-foot"><button>Cancel</button></div>`);
    foot.children[0].onclick = () => close(null);
    box.append(foot);

    const start = (source, fromThor) => { close(1); runImport(game, sides, source, fromThor); };
    // While this dialog is open, the drop handler (transfer.js) hands dropped paths to us.
    S.importTarget = (path) => path && start(path, false);
    body.querySelector('[data-k="file"]').onclick = async () => {
      const path = await T.dialog.open({ title: "Choose save", filters: [{ name: "Archives and saves", extensions: ["zip", "7z", "rar", "tar", "gz", "tgz", "xz", "bz2", "sav", "bin", "dat", "*"] }] });
      if (path) start(path, false);
    };
    body.querySelector('[data-k="dir"]').onclick = async () => {
      const path = await T.dialog.open({ title: "Folder containing the save", directory: true });
      if (path) start(path, false);
    };
    body.querySelector('[data-k="thor"]').onclick = () => pickThorSource(game, sides, close);
  });
}

/** Searches the Thor for save packages and lets the user pick one. */
async function pickThorSource(game, sides, close) {
  if (!S.connected) return toast("Connect the Thor first (USB debugging on)", true);
  close(1);
  const closeSpinner = busyModal("Searching the Thor …");
  // → commands::scan_thor_sources → saves::scan_thor_sources
  const cands = await call("scan_thor_sources", { name: game }).catch(() => []);
  closeSpinner();
  const picked = await folderPicker("On the Thor", cands, { entryTitle: "Or type a path on the Thor" });
  if (picked && picked.path) runImport(game, sides, picked.path, true);
}

// ---------------------------------------------------------------- Prepare

/** Lets Rust fetch, extract and clean the package, then shows the review. */
async function runImport(game, sides, source, fromThor) {
  if ((fromThor || sides.includes("thor")) && !S.connected) return toast("The Thor is not connected.", true);
  const closeSpinner = busyModal("Fetching and extracting …");
  let prep;
  try {
    // → commands::prepare_import → saves::prepare_import (returns a `Prepared` object)
    prep = await call("prepare_import", { name: game, source, fromThor }, true);
  } catch (e) {
    closeSpinner();
    return dialog({ title: "Import failed", body: String(e) });
  }
  closeSpinner();
  importReview(game, sides, prep);
}

// ---------------------------------------------------------------- Review and import

/**
 * Review dialog: which folder of the package is the save, which sides to write, merge or not.
 *
 * Sub-dialogs (folder picker, confirmation) close this modal and reopen it afterwards with
 * the current choice. `keepWorkFolder` stops the work folder from being deleted when the modal
 * closes for such a sub-dialog; otherwise closing (Cancel/Escape) cleans up.
 */
function importReview(game, sides, prep) {
  let root = { path: prep.root, label: prep.root_label, info: prep.root_info, hits: prep.hits, why: prep.why };
  let keepWorkFolder = false;
  const reopen = () => importReview(game, sides, { ...prep, root: root.path, root_label: root.label, root_info: root.info, hits: root.hits, why: root.why });

  modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(game)}: Does this look right?</h2></div>`));
    const body = el(`<div class="modal-body"></div>`);
    body.append(buildPackageBox(prep, root, async () => {
      keepWorkFolder = true; // don't delete the work folder while the picker is open
      close(null);
      root = await pickOtherRoot(game, prep, root);
      reopen();
    }));
    const targets = buildImportTargets(game, sides);
    body.append(targets.element);
    if (prep.missing.length) body.append(missingFolderNote(prep.missing));
    box.append(body);

    const foot = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">Import</button></div>`);
    foot.children[0].onclick = () => close(null);
    const importButton = foot.children[1];
    const updateButton = () => (importButton.disabled = !Object.values(targets.checks).some((check) => check.checked));
    Object.values(targets.checks).forEach((check) => (check.onchange = updateButton));
    updateButton();
    importButton.onclick = async () => {
      const chosen = sides.filter((side) => targets.checks[side].checked);
      if (chosen.includes("thor") && !S.connected) return toast("Thor is not connected", true);
      keepWorkFolder = true;
      close(null);
      const confirmed = await confirmImport(chosen);
      if (!confirmed) { keepWorkFolder = false; return reopen(); }
      await doImport(game, prep, root, chosen, targets.merge.querySelector("input").checked);
    };
    box.append(foot);
  }, { wide: true }).then(() => {
    // → commands::cleanup_import (deletes the work folder of prepare_import)
    if (!keepWorkFolder) invoke("cleanup_import", { work: prep.work });
  });
}

/** "From the package": chosen folder, match hint, file preview, filtered files, change button. */
function buildPackageBox(prep, root, onChooseOther) {
  const box = el(`<div class="side-box"><div class="head"><b>From the package</b><button>Other folder</button></div>
        <div class="mono"></div><div class="note hint"></div><div class="note files mono"></div><div class="note dropped"></div></div>`);
  box.querySelector(".mono").textContent = `${root.label}  ·  ${root.info.files} files`;
  const hint = box.querySelector(".hint");
  // Green with matches, yellow if there was something to compare against but no match.
  hint.className = "note " + (root.hits ? "hint-ok" : prep.known ? "hint-warn" : "");
  hint.textContent = (root.hits ? "✓  " : prep.known ? "⚠  " : "") + root.why;
  const preview = root.info.names.slice(0, 10).map((name) => "·  " + name).join("\n");
  box.querySelector(".files").textContent = preview + (root.info.files > 10 ? `\n·  … and ${root.info.files - 10} more` : "");
  box.querySelector(".dropped").textContent = prep.dropped.length
    ? `Left out: ${prep.dropped.slice(0, 6).join(", ")}${prep.dropped.length > 6 ? " …" : ""}`
    : "";
  box.querySelector("button").onclick = onChooseOther;
  return box;
}

/** Lets the user pick another package folder and rates it. Returns the (possibly new) root. */
async function pickOtherRoot(game, prep, root) {
  const picked = await folderPicker("Which folder is the save?", prep.dirs, { entry: false });
  if (picked && picked.path) {
    // → commands::check_root → saves::check_root
    const check = await call("check_root", { name: game, content: prep.content, root: picked.path }).catch(() => null);
    if (check) return { path: picked.path, label: check.label, info: check.info, hits: check.hits, why: check.why };
  }
  return root;
}

/** "Import to": one checkbox per configured side plus the merge option. */
function buildImportTargets(game, sides) {
  const element = el(`<div class="side-box"><b>Import to</b><span class="note">The old save is backed up first.</span></div>`);
  const checks = {};
  for (const side of sides) {
    const label = el(`<label class="check"><input type="checkbox" checked><span></span></label>`);
    label.querySelector("span").textContent = `${side === "pc" ? "💻 PC" : "🎮 Thor"} – ${(cards[game] || {}).cfg[side]}`;
    checks[side] = label.querySelector("input");
    element.append(label);
  }
  const merge = el(`<label class="check"><input type="checkbox"><span>Keep existing files (only overwrite what's in the import)</span></label>`);
  element.append(merge);
  return { element, checks, merge };
}

/** Hint that the save folder doesn't exist yet on some side. */
function missingFolderNote(missing) {
  const where = missing.map((side) => (side === "pc" ? "PC" : "Thor")).join(" and ");
  return el(`<p class="note">Note: the save folder doesn't exist on the ${where} yet and will be created. If the game doesn't see the save afterwards, start it once without importing so the emulator creates the folder itself, then import again.</p>`);
}

/** Final "Import save?" question. Resolves true if confirmed. */
async function confirmImport(chosen) {
  const where = chosen.map((side) => (side === "pc" ? "PC" : "Thor")).join(" and ");
  let body = `The imported save replaces the current one on the <b>${where}</b>.<br><br>The old save is backed up first (menu ⋮ → Open backups).`;
  if (chosen.includes("pc")) body += "<br><br>Close the game on the PC first.";
  if (chosen.includes("thor")) body += "<br>If the emulator is running on the Thor, it will be closed.";
  const choice = await dialog({ title: "Import save?", html: true, body, buttons: [["Cancel", null, ""], ["Import", "go", "primary"]], defaultValue: "go" });
  return choice === "go";
}

/** Runs the import with a spinner showing the "save-step" progress, then cleans up. */
async function doImport(game, prep, root, chosen, merge) {
  const spinner = busyModal("Starting …");
  const unlisten = await T.event.listen("save-step", (e) => spinner.text(e.payload));
  try {
    // → commands::do_import → saves::import_save (backup, write, verify per side)
    await call("do_import", { name: game, root: root.path, targets: chosen, merge }, true);
    toast(`“${game}”: save imported 🎮`);
  } catch (e) {
    dialog({ title: "Import failed", body: String(e) });
  }
  unlisten();
  spinner();
  invoke("cleanup_import", { work: prep.work }); // → commands::cleanup_import
  if (cards[game]) cards[game].refresh();
}
