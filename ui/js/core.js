// ThorBro UI – core helpers and shared state.
//
// All real work happens in Rust (src/commands.rs); the UI only displays it. The JS is split
// into plain <script> files (no bundler, no modules). They share one global scope and are
// loaded in this order by index.html:
//   core.js → dialogs.js → device.js → files.js → files-actions.js → pool.js → esde.js → transfer.js → saves.js →
//   saves-setup.js → saves-import.js → stream.js → pcstream.js → pins.js → status.js → start.js → main.js
// Code that runs immediately at load time may only use names from the same or earlier files.
// Inside functions, any name works, because those run later.
"use strict";

// ---------------------------------------------------------------- Tauri bridge

/** Tauri's JS API (available because tauri.conf.json sets "withGlobalTauri": true). */
const T = window.__TAURI__;

/**
 * Calls a Rust command, e.g. invoke("list_dir", { path }) → `commands::list_dir` in
 * src/commands.rs. Argument names are camelCase here and snake_case in Rust.
 */
const invoke = T.core.invoke;

// Uncaught JS errors are forwarded to Rust (→ commands::log) and end up on stderr.
window.addEventListener("error", (e) => invoke("log", { msg: `${e.message} @ ${e.filename}:${e.lineno}` }));
window.addEventListener("unhandledrejection", (e) => invoke("log", { msg: `Promise: ${e.reason}` }));

// ---------------------------------------------------------------- Small helpers

/** querySelector shortcut. */
const $ = (selector, root = document) => root.querySelector(selector);

const HTML_ESCAPES = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

/** Escapes text for use inside HTML strings. */
const esc = (text) => String(text ?? "").replace(/[&<>"']/g, (c) => HTML_ESCAPES[c]);

/** Byte count for humans, same format as util::human in Rust. */
const human = (bytes) => {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let unitIndex = 0;
  while (bytes >= 1024 && unitIndex < units.length - 1) { bytes /= 1024; unitIndex++; }
  return unitIndex ? `${bytes.toFixed(1)} ${units[unitIndex]}` : `${bytes} B`;
};

/** Last path component (works with / and \). */
const basename = (path) => path.replace(/[\\/]+$/, "").split(/[\\/]/).pop();

/** Parent folder of a Thor path ("/a/b" → "/a", "/a" → "/"). */
const dirname = (path) => {
  const trimmed = path.replace(/\/+$/, "");
  const slash = trimmed.lastIndexOf("/");
  return slash <= 0 ? "/" : trimmed.slice(0, slash);
};

/** Thor path of `name` inside the folder `dir` ("/" + "a" → "/a", "/x" + "a" → "/x/a"). */
const joinPath = (dir, name) => (dir === "/" ? `/${name}` : `${dir}/${name}`);

/** Creates one DOM element from an HTML string. */
function el(html) {
  const template = document.createElement("template");
  template.innerHTML = html.trim();
  return template.content.firstElementChild;
}

// ---------------------------------------------------------------- Shared state

/** UI state shared by all parts of the app. */
const S = {
  tab: "start",     // every launch opens Start (not remembered, see main.js)
  connected: false,
  deviceMsg: "",
  places: [],       // location drop-down entries (from commands::places)
  libs: [],         // game libraries on the Thor
  root: "/sdcard",  // root of the selected location
  cwd: "/sdcard",   // current folder in the file manager
  entries: [],      // folder listing (from commands::list_dir)
  shown: [],        // entries as displayed (filtered and sorted, see files.js deriveShown)
  sel: -1,          // index in `shown` of the focused row if it is selected, else -1 (more in files.js FM)
  busyTransfer: false,
  modalOpen: false,
  importTarget: null, // set while the import dialog is open: it receives dropped files
};

// ---------------------------------------------------------------- Toasts & error handling

/** Shows a short message in the corner. Errors stay longer. */
function toast(text, err = false) {
  const toastEl = document.createElement("div");
  toastEl.className = "toast" + (err ? " err" : "");
  toastEl.textContent = text;
  $("#toasts").appendChild(toastEl);
  setTimeout(() => toastEl.remove(), err ? 7000 : 4000);
}

/**
 * invoke() with error toast. The Rust `Err(String)` arrives here as exception; it is shown
 * (unless `quiet`) and re-thrown so the caller can react too.
 */
async function call(cmd, args, quiet = false) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    if (!quiet) toast(String(e), true);
    throw e;
  }
}
