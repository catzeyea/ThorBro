// ThorBro UI – modal dialogs: generic modal, message dialog, text input, spinner, folder picker.
// Used by files.js, transfer.js and the saves-*.js files. Only one modal is open at a time.
"use strict";

// ---------------------------------------------------------------- Generic modal

/**
 * Opens the modal. build(box, close) fills its content; calling close(value) closes it.
 * Returns a Promise that resolves with the close value (null for Escape).
 */
function modal(build, { wide = false } = {}) {
  return new Promise((resolve) => {
    const wrap = $("#modal");
    // Every modal gets a fresh, empty box element. Replacing the element (instead of only
    // clearing its content) also drops event listeners an earlier dialog attached to the box,
    // e.g. the Enter handler of the sort dialog, which would otherwise fire again later.
    const oldBox = $("#modal-box");
    const box = oldBox.cloneNode(false);
    oldBox.replaceWith(box);
    box.className = "modal" + (wide ? " wide" : "");
    wrap.classList.remove("hidden");
    S.modalOpen = true;
    let done = false;
    const close = (value) => {
      if (done) return;
      done = true;
      wrap.classList.add("hidden");
      S.modalOpen = false;
      S.importTarget = null;
      document.removeEventListener("keydown", onKey, true);
      resolve(value);
    };
    const onKey = (e) => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(null); }
    };
    document.addEventListener("keydown", onKey, true);
    build(box, close);
  });
}

// ---------------------------------------------------------------- Message dialog

/**
 * Message box with buttons. buttons = [[label, value, cssClass], ...]; resolves with the
 * value of the clicked button. `html: true` inserts `body` as HTML (caller must escape).
 */
function dialog({ title, body = "", html = false, buttons = [["OK", "ok", "primary"]], defaultValue = null }) {
  return modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(title)}</h2></div>`));
    const bodyEl = el(`<div class="modal-body"></div>`);
    if (body) bodyEl.append(html ? el(`<div>${body}</div>`) : el(`<p>${esc(body)}</p>`));
    box.append(bodyEl);
    const foot = el(`<div class="modal-foot"></div>`);
    // Focused button: the one with defaultValue, else the first primary one, else the last.
    let focusButton;
    for (const [label, value, cssClass] of buttons) {
      const button = el(`<button class="${cssClass || ""}">${esc(label)}</button>`);
      button.onclick = () => close(value);
      foot.append(button);
      if (value === defaultValue || (!focusButton && cssClass && cssClass.includes("primary"))) focusButton = button;
    }
    box.append(foot);
    setTimeout(() => (focusButton || foot.lastChild).focus(), 0);
  });
}

// ---------------------------------------------------------------- Text input

/**
 * Asks for a line of text. Resolves with the trimmed text, or null if cancelled/empty.
 * selectStem: preselect the name without extension (for renaming files).
 */
function ask(title, initial = "", selectStem = false) {
  return modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(title)}</h2></div>`));
    const body = el(`<div class="modal-body"></div>`);
    const input = el(`<input type="text">`);
    input.value = initial;
    body.append(input);
    box.append(body);
    const foot = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">OK</button></div>`);
    foot.children[0].onclick = () => close(null);
    const confirm = () => close(input.value.trim() || null);
    foot.children[1].onclick = confirm;
    input.onkeydown = (e) => { if (e.key === "Enter") confirm(); };
    box.append(foot);
    setTimeout(() => {
      input.focus();
      const dot = initial.lastIndexOf(".");
      input.setSelectionRange(0, selectStem && dot > 0 ? dot : initial.length);
    }, 0);
  });
}

// ---------------------------------------------------------------- Spinner

/**
 * Modal with a spinner. Returns a function that closes it; its .text(t) method changes
 * the text (used for "save-step" progress events).
 */
function busyModal(text) {
  let closeModal, setText;
  modal((box, close) => {
    box.append(el(`<div class="modal-body"><div class="busy"><span class="spin"></span><b class="wiz-step"></b></div></div>`));
    setText = (t) => (box.querySelector(".wiz-step").textContent = t);
    setText(text);
    closeModal = close;
  });
  const closer = () => closeModal(null);
  closer.text = (t) => setText(t);
  return closer;
}

// ---------------------------------------------------------------- Folder picker

/**
 * Lets the user pick one of `cands` (Cand objects from Rust: path, when, files, note),
 * with a filter field and optionally a free-text path entry.
 * Resolves with { path } (path null = the "none" button), or null if cancelled.
 */
function folderPicker(title, cands, { noneLabel = null, entryTitle = "Or type a path yourself", entry = true } = {}) {
  return modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(title)}</h2></div>`));
    const body = el(`<div class="modal-body"></div>`);
    const filter = el(`<input type="text" placeholder="Filter …">`);
    const list = el(`<div class="pick-list"></div>`);
    body.append(filter, list);
    const pick = (path) => close({ path: path.replace(/\/+$/, "") });
    filter.oninput = () => renderPickList(list, cands, filter.value, entry, pick);
    renderPickList(list, cands, filter.value, entry, pick);
    if (entry) body.append(buildPathEntry(entryTitle, pick));
    box.append(body);
    box.append(buildPickerFoot(noneLabel, close));
    setTimeout(() => filter.focus(), 0);
  }, { wide: true });
}

/** Fills the picker list with all candidates whose path contains the filter text. */
function renderPickList(list, cands, filterText, entry, pick) {
  const query = filterText.toLowerCase().trim();
  list.innerHTML = "";
  const shown = cands.filter((cand) => !query || cand.path.toLowerCase().includes(query));
  if (!shown.length) list.append(el(`<div class="pick-item dim">Nothing found${entry ? " – type the path below." : "."}</div>`));
  for (const cand of shown) {
    const item = el(`<div class="pick-item"><div class="t"></div><div class="s"></div></div>`);
    item.querySelector(".t").textContent = basename(cand.path) || cand.path;
    item.querySelector(".s").textContent = `${cand.when ? cand.when + " · " : ""}${cand.files} files ${cand.note || ""}\n${cand.path}`;
    item.onclick = () => pick(cand.path);
    list.append(item);
  }
}

/** Text field + "Use" button for typing a path by hand. */
function buildPathEntry(placeholder, pick) {
  const row = el(`<div style="display:flex;gap:6px"><input type="text"><button>Use</button></div>`);
  const input = row.querySelector("input");
  input.placeholder = placeholder;
  const take = () => input.value.trim() && pick(input.value.trim());
  row.querySelector("button").onclick = take;
  input.onkeydown = (e) => { if (e.key === "Enter") take(); };
  return row;
}

/** Footer with the optional "none" button and Cancel. */
function buildPickerFoot(noneLabel, close) {
  const foot = el(`<div class="modal-foot"></div>`);
  if (noneLabel) {
    const noneButton = el(`<button>${esc(noneLabel)}</button>`);
    noneButton.onclick = () => close({ path: null });
    foot.append(noneButton);
  }
  const cancel = el(`<button>Cancel</button>`);
  cancel.onclick = () => close(null);
  foot.append(cancel);
  return foot;
}
