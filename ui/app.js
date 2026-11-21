// ThorBro – UI. All the real work happens in Rust; this file only displays it.
"use strict";

const T = window.__TAURI__;
window.addEventListener("error", (e) => T.core.invoke("log", { msg: `${e.message} @ ${e.filename}:${e.lineno}` }));
window.addEventListener("unhandledrejection", (e) => T.core.invoke("log", { msg: `Promise: ${e.reason}` }));
const invoke = T.core.invoke;
const $ = (s, el = document) => el.querySelector(s);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const human = (n) => {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return i ? `${n.toFixed(1)} ${u[i]}` : `${n} B`;
};
const basename = (p) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop();
const dirname = (p) => { const t = p.replace(/\/+$/, ""); const i = t.lastIndexOf("/"); return i <= 0 ? "/" : t.slice(0, i); };

const S = {
  tab: "files",
  connected: false,
  deviceMsg: "",
  places: [],
  libs: [],
  root: "/sdcard",
  cwd: "/sdcard",
  entries: [],
  sel: -1,
  busyTransfer: false,
  modalOpen: false,
  importTarget: null, // aktiver Import-Dialog nimmt Drops an
};

// ---------------------------------------------------------------- Kleinkram

function toast(text, err = false) {
  const t = document.createElement("div");
  t.className = "toast" + (err ? " err" : "");
  t.textContent = text;
  $("#toasts").appendChild(t);
  setTimeout(() => t.remove(), err ? 7000 : 4000);
}

async function call(cmd, args, quiet = false) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    if (!quiet) toast(String(e), true);
    throw e;
  }
}

/** Modal öffnen. build(box, close) füllt den Inhalt. Gibt ein Promise auf den close-Wert. */
function modal(build, { wide = false } = {}) {
  return new Promise((resolve) => {
    const wrap = $("#modal");
    const box = $("#modal-box");
    box.className = "modal" + (wide ? " wide" : "");
    box.innerHTML = "";
    wrap.classList.remove("hidden");
    S.modalOpen = true;
    let done = false;
    const close = (v) => {
      if (done) return;
      done = true;
      wrap.classList.add("hidden");
      S.modalOpen = false;
      S.importTarget = null;
      document.removeEventListener("keydown", onKey, true);
      resolve(v);
    };
    const onKey = (e) => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(null); }
    };
    document.addEventListener("keydown", onKey, true);
    build(box, close);
  });
}

function el(html) {
  const t = document.createElement("template");
  t.innerHTML = html.trim();
  return t.content.firstElementChild;
}

function dialog({ title, body = "", html = false, buttons = [["OK", "ok", "primary"]], defaultValue = null }) {
  return modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(title)}</h2></div>`));
    const b = el(`<div class="modal-body"></div>`);
    if (body) b.append(html ? el(`<div>${body}</div>`) : el(`<p>${esc(body)}</p>`));
    box.append(b);
    const f = el(`<div class="modal-foot"></div>`);
    let def;
    for (const [label, val, cls] of buttons) {
      const btn = el(`<button class="${cls || ""}">${esc(label)}</button>`);
      btn.onclick = () => close(val);
      f.append(btn);
      if (val === defaultValue || (!def && cls && cls.includes("primary"))) def = btn;
    }
    box.append(f);
    setTimeout(() => (def || f.lastChild).focus(), 0);
  });
}

function ask(title, initial = "", selectStem = false) {
  return modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(title)}</h2></div>`));
    const b = el(`<div class="modal-body"></div>`);
    const inp = el(`<input type="text">`);
    inp.value = initial;
    b.append(inp);
    box.append(b);
    const f = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">OK</button></div>`);
    f.children[0].onclick = () => close(null);
    const ok = () => close(inp.value.trim() || null);
    f.children[1].onclick = ok;
    inp.onkeydown = (e) => { if (e.key === "Enter") ok(); };
    box.append(f);
    setTimeout(() => {
      inp.focus();
      const dot = initial.lastIndexOf(".");
      inp.setSelectionRange(0, selectStem && dot > 0 ? dot : initial.length);
    }, 0);
  });
}

// ---------------------------------------------------------------- Tabs & Gerät

document.querySelectorAll(".tab").forEach((b) => (b.onclick = () => showTab(b.dataset.tab)));

function showTab(tab) {
  S.tab = tab;
  document.querySelectorAll(".tab").forEach((b) => b.classList.toggle("active", b.dataset.tab === tab));
  document.querySelectorAll(".page").forEach((p) => p.classList.toggle("active", p.id === "page-" + tab));
  if (tab === "saves") loadGames();
  if (tab === "stream") streamStatus();
}

async function pollDevice(force = false) {
  let d;
  try { d = await invoke("device"); } catch (e) { invoke("log", { msg: "device: " + e }); return; }
  const changed = d.connected !== S.connected || force;
  S.connected = d.connected;
  S.deviceMsg = d.message;
  $("#device").classList.toggle("on", d.connected);
  $("#device-text").textContent = d.connected ? `Connected: ${d.message}` : d.message;
  if (changed) {
    if (d.connected) await loadPlaces(true);
    else { S.entries = []; renderFiles("No Thor connected"); }
    if (S.tab === "saves") refreshCards();
  }
}
$("#device").onclick = () => pollDevice(true);

// ---------------------------------------------------------------- Dateien

async function loadPlaces(restore) {
  const r = await call("places", { restore });
  S.places = r.places;
  S.libs = r.libs;
  const sel = $("#place");
  sel.innerHTML = S.places.map((p, i) => `<option value="${i}">${esc(p.label)}</option>`).join("");
  let want = restore ? r.start : S.cwd;
  const match = S.places
    .filter((p) => want === p.path || want.startsWith(p.path + "/"))
    .sort((a, b) => b.path.length - a.path.length)[0];
  const root = match ? match.path : (S.places[0] || { path: "/sdcard" }).path;
  if (!match) want = root;
  selectPlace(root, want);
}

function selectPlace(root, path) {
  S.root = root;
  $("#place").value = String(S.places.findIndex((p) => p.path === root));
  go(path || root);
}

$("#place").onchange = (e) => {
  const p = S.places[+e.target.value];
  if (p) selectPlace(p.path);
};

function libAt(path) {
  return S.libs.find((l) => l.path === path);
}

async function go(path) {
  if (!(path === S.root || path.startsWith(S.root + "/"))) path = S.root;
  S.cwd = path.replace(/\/+$/, "") || "/";
  S.sel = -1;
  renderCrumbs();
  if (!S.connected) return renderFiles("No Thor connected");
  renderFiles("Loading …");
  try {
    S.entries = await call("list_dir", { path: S.cwd });
  } catch {
    S.entries = [];
  }
  renderFiles(S.entries.length ? "" : "Empty folder");
}

function renderCrumbs() {
  const place = S.places.find((p) => p.path === S.root);
  const c = $("#crumbs");
  c.innerHTML = "";
  const parts = S.cwd.slice(S.root.length).split("/").filter(Boolean);
  const mk = (label, path) => {
    const b = el(`<button>${esc(label)}</button>`);
    b.onclick = () => go(path);
    c.append(b);
  };
  mk(place ? place.label : S.root, S.root);
  let acc = S.root;
  for (const p of parts) {
    acc += "/" + p;
    c.append(el(`<span class="sep">›</span>`));
    mk(p, acc);
  }
  c.title = S.cwd;
  c.scrollLeft = c.scrollWidth;
  $("#up").disabled = S.cwd === S.root;
}

function renderFiles(placeholder) {
  const list = $("#filelist");
  list.innerHTML = "";
  const lib = libAt(S.cwd);
  const shown = S.entries.filter((e) => !(lib && e.name === "systeminfo.txt"));
  if (!shown.length) {
    list.append(el(`<div class="placeholder">${esc(placeholder || "")}</div>`));
    return;
  }
  shown.forEach((e, i) => {
    let meta = e.dir ? "" : human(e.size);
    let faded = false;
    if (lib && e.dir && lib.systems[e.name]) {
      const s = lib.systems[e.name];
      meta = `${s.full || ""}${s.full ? " · " : ""}${s.count}`;
      faded = s.count === 0;
    }
    const row = el(`<div class="row${faded ? " faded" : ""}"><span class="ic">${e.dir ? "📁" : "📄"}</span><span class="nm"></span><span class="meta"></span></div>`);
    row.querySelector(".nm").textContent = e.name;
    row.querySelector(".meta").textContent = meta;
    row.onclick = () => select(i);
    row.onmousedown = (ev) => { if (ev.button === 0) DRAG.start = { x: ev.clientX, y: ev.clientY, i }; };
    row.ondragstart = (ev) => ev.preventDefault();
    row.ondblclick = () => open(i);
    row.entry = e;
    list.append(row);
  });
  S.shown = shown;
}

function select(i) {
  S.sel = i;
  document.querySelectorAll("#filelist .row").forEach((r, j) => r.classList.toggle("sel", j === i));
  const r = $("#filelist").children[i];
  if (r) r.scrollIntoView({ block: "nearest" });
}

function selected() {
  const e = S.shown && S.shown[S.sel];
  if (!e) toast("Select an entry first");
  return e;
}

function open(i) {
  const e = S.shown[i];
  if (e && e.dir) go(`${S.cwd}/${e.name}`);
}

const goUp = () => S.cwd !== S.root && go(dirname(S.cwd));
$("#up").onclick = goUp;
$("#reload").onclick = () => go(S.cwd);

$("#btn-mkdir").onclick = async () => {
  const name = await ask("New folder");
  if (!name || name.includes("/")) return;
  await call("make_dir", { path: `${S.cwd}/${name}` });
  go(S.cwd);
};

$("#btn-rename").onclick = async () => {
  const e = selected();
  if (!e) return;
  const name = await ask("Rename", e.name, !e.dir);
  if (!name || name === e.name || name.includes("/")) return;
  await call("rename", { from: `${S.cwd}/${e.name}`, to: `${S.cwd}/${name}` });
  toast(`Renamed to ${name}`);
  go(S.cwd);
};

$("#btn-delete").onclick = async () => {
  const e = selected();
  if (!e) return;
  const r = await dialog({
    title: `Really delete “${e.name}”?`,
    body: "There is no recycle bin – this can't be undone.",
    buttons: [["Cancel", null, ""], ["Delete", "rm", "danger solid"]],
    defaultValue: null,
  });
  if (r !== "rm") return;
  await call("delete", { path: `${S.cwd}/${e.name}` });
  toast(`${e.name} deleted`);
  go(S.cwd);
};

$("#btn-pull").onclick = async () => {
  const e = selected();
  if (!e) return;
  toast(`Copying ${e.name} to the PC …`);
  const dest = await call("pull", { path: `${S.cwd}/${e.name}` });
  toast(`Copied to ${dest}`);
};

// Rausziehen auf den PC: Zeile anpacken und in den Dateimanager/auf den Desktop ziehen.
// Erst wird die Datei in den Cache geholt, dann startet der echte System-Drag.
const DRAG = { start: null, down: false, busy: false, native: false };
document.addEventListener("mousedown", (ev) => { if (ev.button === 0) DRAG.down = true; }, true);
document.addEventListener("mouseup", () => { DRAG.down = false; DRAG.start = null; }, true);
document.addEventListener("mousemove", (ev) => {
  const st = DRAG.start;
  if (!st || !DRAG.down || DRAG.busy || S.modalOpen) return;
  if (Math.hypot(ev.clientX - st.x, ev.clientY - st.y) < 8) return;
  DRAG.start = null;
  dragOut(st.i);
});

async function dragOut(i) {
  const e = S.shown && S.shown[i];
  if (!e) return;
  select(i);
  DRAG.busy = true;
  const row = $("#filelist").children[i];
  try {
    const slow = setTimeout(() => toast(`Fetching ${e.name} from the Thor, then you can drag it out …`), 500);
    const r = await call("drag_fetch", { path: `${S.cwd}/${e.name}` }).finally(() => clearTimeout(slow));
    if (row && row.entry === e) row.classList.add("ready");
    if (DRAG.down) { DRAG.native = true; await invoke("drag_start", { paths: [r.local] }); }
    else toast(`${e.name} is ready – drag it out again now`);
  } catch (_) {
    // Fehler kam schon als Toast
  } finally {
    DRAG.busy = false;
  }
}
T.event.listen("drag-done", ({ payload: dropped }) => { DRAG.down = false; DRAG.native = false; $("#drop-overlay").classList.add("hidden"); if (dropped) toast("Copied to the PC"); });
const FROM_DRAG_CACHE = /[\\/]thorbro[\\/]ziehen[\\/]/;

$("#btn-unpack").onclick = () => {
  const e = selected();
  if (!e) return;
  if (e.dir || !ARCHIVE_RE.test(e.name)) return toast("That's not an archive (zip, 7z, rar, tar …)", true);
  // In einem Game-Ordner einzelne Hüllordner im Archiv überspringen
  const inLib = S.libs.some((l) => S.cwd.startsWith(l.path + "/"));
  enqueue([job(`${S.cwd}/${e.name}`, S.cwd, true, inLib, true)]);
};

$("#filelist").addEventListener("keydown", (e) => {
  if (S.modalOpen) return;
  const n = S.shown ? S.shown.length : 0;
  if (e.key === "ArrowDown") { select(Math.min(n - 1, S.sel + 1)); e.preventDefault(); }
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

// ---------------------------------------------------------------- Senden & Einsortieren

$("#btn-send-files").onclick = async () => {
  const p = await T.dialog.open({ multiple: true, title: "Files for the Thor" });
  if (p) incoming([].concat(p));
};
$("#btn-send-dirs").onclick = async () => {
  const p = await T.dialog.open({ multiple: true, directory: true, title: "Folders for the Thor" });
  if (p) incoming([].concat(p));
};

async function incoming(paths) {
  paths = paths.filter(Boolean);
  if (!paths.length) return;
  if (!S.connected) {
    await pollDevice(true);
    if (!S.connected) return toast("Thor is not connected", true);
  }
  if (!S.libs.length) return enqueue(paths.map((p) => job(p, S.cwd)));
  const items = await call("classify", { paths, cwd: S.cwd, lib: 0, force: false });
  // Nichts Game-artiges dabei: wie früher direkt in den aktuellen Ordner
  if (!items.length) return enqueue(paths.map((p) => job(p, S.cwd)));
  sortDialog(paths, items);
}

function sortDialog(paths, items) {
  let libIdx = 0;
  modal((box, close) => {
    const n = paths.length;
    const head = el(`<div class="modal-head" style="display:flex;gap:10px;align-items:center;flex-wrap:wrap">
      <div style="flex:1;min-width:200px"><h2>Where should it go?</h2><p class="dim">${n} ${n === 1 ? "file" : "files"} – check the target, change it if needed</p></div></div>`);
    if (S.libs.length > 1) {
      const ls = el(`<select title="Game library"></select>`);
      ls.innerHTML = S.libs.map((l, i) => `<option value="${i}">${esc(l.label)} (${l.total} Games)</option>`).join("");
      ls.onchange = async () => {
        libIdx = +ls.value;
        fill(await call("classify", { paths, cwd: S.cwd, lib: libIdx, force: true }));
      };
      head.append(ls);
    }
    box.append(head);
    const body = el(`<div class="modal-body"></div>`);
    box.append(body);
    let rows = [];
    function fill(list) {
      body.innerHTML = "";
      rows = list.map((it) => {
        const why = it.guess.system ? (it.guess.sure ? "" : "unsure") : "none";
        const r = el(`<div class="sort-row">
          <span>${it.dir ? "📁" : "🎮"}</span>
          <div style="min-width:0"><div class="nm"></div><div class="why ${why}"></div></div>
          <select></select></div>`);
        r.querySelector(".nm").textContent = it.name;
        r.querySelector(".nm").title = it.path;
        const w = r.querySelector(".why");
        w.textContent = `${it.size} · ${it.guess.reason}`;
        w.title = it.guess.reason;
        const sel = r.querySelector("select");
        sel.innerHTML = it.targets.map((t, i) => `<option value="${i}">${esc(t.label)}</option>`).join("");
        sel.value = String(it.selected);
        let box = null;
        if (it.archive) {
          const u = el(`<label class="unpack"><input type="checkbox"> Extract first (copy the contents instead of the archive)</label>`);
          box = u.querySelector("input");
          box.checked = it.unpack;
          r.append(u);
        }
        body.append(r);
        return { it, sel, box };
      });
    }
    fill(items);
    const foot = el(`<div class="modal-foot">
      <button>Cancel</button><button title="${esc(S.cwd)}">All to current folder</button><button class="primary">Copy</button></div>`);
    const [cancel, here, ok] = foot.children;
    cancel.onclick = () => close(null);
    const mk = ({ it, box }, dest) => job(it.path, dest, !!(box && box.checked), !!it.guess.system);
    here.onclick = () => { close(1); enqueue(rows.map((r) => mk(r, S.cwd))); };
    ok.onclick = () => { close(1); enqueue(rows.map((r) => mk(r, r.it.targets[+r.sel.value].path))); };
    box.append(foot);
    box.addEventListener("keydown", (e) => { if (e.key === "Enter" && e.target.tagName !== "SELECT") ok.click(); });
    setTimeout(() => ok.focus(), 0);
  }, { wide: true });
}

const job = (src, dest, unpack = false, flatten = false, remote = false) => ({ src, dest, unpack, flatten, remote });
const ARCHIVE_RE = /\.(zip|7z|rar|tar|tgz|txz|tbz2|tar\.(gz|xz|bz2|zst))$/i;

function enqueue(jobs) {
  if (!jobs.length) return;
  invoke("enqueue", { jobs });
}

T.event.listen("transfer", async ({ payload: p }) => {
  $("#progress").classList.remove("hidden");
  $("#dropzone").classList.toggle("hidden", p.busy);
  $("#p-text").textContent = p.text;
  $("#p-detail").textContent = p.detail;
  const bar = $("#p-bar").parentElement;
  bar.classList.toggle("indet", p.busy && p.fraction <= 0);
  $("#p-bar").style.width = `${Math.round((p.busy && p.fraction <= 0 ? 0.3 : p.fraction) * 100)}%`;
  if (!p.busy) {
    if (p.error) toast(p.error, true);
    else toast(p.text);
    S.libs = await call("refresh_libs", {}, true).catch(() => S.libs);
    if (p.dests.some((d) => d === S.cwd || dirname(d) === S.cwd || d.startsWith(S.cwd + "/"))) go(S.cwd);
    setTimeout(() => { if (!$("#dropzone").classList.contains("hidden")) $("#progress").classList.add("hidden"); }, 6000);
  }
});

// Drag & Drop: aufs ganze Fenster
T.webview.getCurrentWebview().onDragDropEvent((ev) => {
  const t = ev.payload.type;
  const ov = $("#drop-overlay");
  if (t === "enter" || t === "over") {
    if (DRAG.native || (ev.payload.paths || []).some((p) => FROM_DRAG_CACHE.test(p))) return;
    if (S.modalOpen && !S.importTarget) return;
    ov.classList.remove("hidden");
    $("#drop-overlay div").textContent = S.importTarget ? "Drop to import" : "Drop to send";
  } else if (t === "leave") {
    ov.classList.add("hidden");
  } else if (t === "drop") {
    ov.classList.add("hidden");
    const paths = (ev.payload.paths || []).filter((p) => !FROM_DRAG_CACHE.test(p));
    if (!paths.length) return; // eigener Drag zurück ins Fenster
    if (S.importTarget) return S.importTarget(paths[0]);
    if (S.modalOpen) return;
    if (S.tab !== "files") showTab("files");
    incoming(paths);
  }
});

// ---------------------------------------------------------------- Saves

const cards = {};

async function loadGames() {
  const games = await call("games", {}, true).catch(() => ({}));
  const list = $("#game-list");
  list.innerHTML = "";
  for (const k of Object.keys(cards)) delete cards[k];
  const names = Object.keys(games).sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
  $("#games-empty").classList.toggle("hidden", names.length > 0);
  for (const name of names) {
    const c = new GameCard(name, games[name]);
    cards[name] = c;
    list.append(c.el);
  }
  refreshCards();
}

function refreshCards() {
  Object.values(cards).forEach((c) => c.refresh());
}

class GameCard {
  constructor(name, cfg) {
    this.name = name;
    this.cfg = cfg;
    this.oneSide = !(cfg.pc && cfg.thor);
    this.where = cfg.pc ? "PC" : "Thor";
    this.el = el(`<div class="card">
      <div class="card-top"><h2></h2><span class="spin hidden"></span>
        <div class="menu"><button class="icon flat" title="More">⋮</button></div></div>
      <div class="status dim"></div><div class="detail"></div>
      <div class="btns"></div></div>`);
    this.el.querySelector("h2").textContent = name;
    this.el.querySelector("h2").title = name;
    this.status = this.el.querySelector(".status");
    this.detail = this.el.querySelector(".detail");
    this.spin = this.el.querySelector(".spin");
    const btns = this.el.querySelector(".btns");
    if (this.oneSide) {
      btns.classList.add("one");
      const b = el(`<button>Import save</button>`);
      b.onclick = () => importWizard(name);
      btns.append(b);
      this.bPc = this.bThor = null;
    } else {
      this.bPc = el(`<button title="Pulls the save from the Thor to the PC">💻 Continue on PC</button>`);
      this.bThor = el(`<button title="Pushes the save from the PC to the Thor">🎮 Continue on Thor</button>`);
      this.bPc.onclick = () => startSync(this, "pc");
      this.bThor.onclick = () => startSync(this, "thor");
      btns.append(this.bPc, this.bThor);
    }
    const menuBtn = this.el.querySelector(".menu button");
    menuBtn.onclick = (e) => { e.stopPropagation(); this.menu(); };
  }

  menu() {
    const m = this.el.querySelector(".menu");
    const old = m.querySelector(".menu-pop");
    if (old) return old.remove();
    const pop = el(`<div class="menu-pop"></div>`);
    const items = [
      ["Import save …", () => importWizard(this.name)],
      ["Show folders", () => dialog({
        title: this.name,
        html: true,
        body: `<b>PC</b><div class="mono">${esc(this.cfg.pc || "—")}</div><br><b>Thor</b><div class="mono">${esc(this.cfg.thor || "—")}</div>`,
      })],
      ["Search folders again", () => addWizard(this.name)],
      ["Open backups", async () => { if (!(await call("open_backups", { name: this.name }))) toast("No backups for this game yet"); }],
      ["Remove from list", async () => {
        const r = await dialog({
          title: `Remove “${this.name}”?`,
          body: "The game is only removed from the list. Your saves on the PC and the Thor stay where they are.",
          buttons: [["Cancel", null, ""], ["Remove", "rm", "danger solid"]],
          defaultValue: null,
        });
        if (r === "rm") { await call("remove_game", { name: this.name }); loadGames(); }
      }],
    ];
    for (const [label, fn] of items) {
      const b = el(`<button>${esc(label)}</button>`);
      b.onclick = () => { pop.remove(); fn(); };
      pop.append(b);
    }
    m.append(pop);
    setTimeout(() => document.addEventListener("click", () => pop.remove(), { once: true }), 0);
  }

  busy(on) {
    this.spin.classList.toggle("hidden", !on);
    [this.bPc, this.bThor].forEach((b) => b && (b.disabled = on));
  }

  set(text, kind, detail = "") {
    this.status.className = "status " + (kind || "");
    this.status.textContent = text;
    this.detail.textContent = detail;
    [this.bPc, this.bThor].forEach((b) => b && b.classList.remove("primary"));
  }

  async refresh() {
    const needsThor = !(this.oneSide && this.where === "PC");
    if (needsThor && !S.connected) {
      return this.set("Thor not connected", "dim", "Once it's plugged in, you'll see here which side has the newer save.");
    }
    this.busy(true);
    this.set("Checking the saves …", "dim");
    let st;
    try {
      st = await call("game_status", { name: this.name, connected: S.connected }, true);
    } catch (e) {
      this.busy(false);
      return this.set("Couldn't read the saves", "warn", String(e));
    }
    this.busy(false);
    if (this.oneSide) {
      const s = this.where === "PC" ? st.pc : st.thor;
      const info = `Only set up on the ${this.where} – no sync, but import works.`;
      return s.exists && s.files
        ? this.set(`Save on the ${this.where}`, "", `${s.summary}\n${info}`)
        : this.set(`No save on the ${this.where} yet`, "dim", info);
    }
    const detail = `PC: ${st.pc.newest_text}   ·   Thor: ${st.thor.newest_text}`;
    if (!st.pc.files && !st.thor.files) this.set("No save on either side yet", "dim", detail);
    else if (st.same) this.set("✓  Both in sync", "ok", detail);
    else if (st.pc.newest === st.thor.newest) this.set("●  Different, but same age", "warn", detail);
    else if (st.thor.newest > st.pc.newest) { this.set("●  Thor is newer", "accent", detail); this.bPc.classList.add("primary"); }
    else { this.set("●  PC is newer", "accent", detail); this.bThor.classList.add("primary"); }
  }
}

async function startSync(card, target) {
  if (!S.connected) return toast("Thor is not connected", true);
  card.busy(true);
  let plan;
  try {
    plan = await call("plan_sync", { name: card.name, target });
  } catch {
    return card.busy(false);
  }
  card.busy(false);
  const src = plan.src_name, dst = plan.dst_name;
  if (plan.empty) {
    return dialog({ title: `There's no save on the ${src}`, body: `The configured folder on the ${src} is empty, so there's nothing to copy.` });
  }
  if (plan.same) {
    toast(`Already in sync – you can play on the ${dst} right away`);
    return card.refresh();
  }
  let body = `The save from the <b>${src}</b> will be copied to the <b>${dst}</b>.<br><br>
    <b>${src}:</b> ${esc(plan.src.summary)}<br><b>${dst}:</b> ${esc(plan.dst.summary)}<br><br>
    The old ${dst} save is backed up first.`;
  if (dst === "PC") body += "<br><br>Close the game on the PC first.";
  if (plan.running_pkg) body += `<br><br>⚠ <b>${esc(plan.running_pkg)}</b> is running on the Thor and will be closed. Save in-game first!`;
  let title = `Continue on the ${dst}?`;
  if (plan.dst_newer) {
    title = `Careful: the ${dst} save is newer!`;
    body = `Looks like you last played on the ${dst}. If you continue, this newer save gets overwritten (it goes into the backup though).<br><br>` + body;
  }
  const r = await dialog({
    title, html: true, body,
    buttons: [["Cancel", null, ""], [plan.dst_newer ? "Overwrite anyway" : "Let's go", "go", plan.dst_newer ? "danger solid" : "primary"]],
    defaultValue: plan.dst_newer ? null : "go",
  });
  if (r !== "go") return;
  card.busy(true);
  card.set(`Copying ${src} → ${dst} …`, "dim");
  try {
    await call("run_sync", { name: card.name, target, stopPkg: plan.running_pkg }, true);
    toast(`Done! Have fun on the ${dst} 🎮`);
  } catch (e) {
    dialog({ title: "Something went wrong", body: String(e) });
  }
  card.busy(false);
  card.refresh();
}

$("#btn-add-game").onclick = () => addWizard("");

// --- Ordner-Auswahl -------------------------------------------------------------

function folderPicker(title, cands, { noneLabel = null, entryTitle = "Or type a path yourself", entry = true } = {}) {
  return modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(title)}</h2></div>`));
    const body = el(`<div class="modal-body"></div>`);
    const filter = el(`<input type="text" placeholder="Filter …">`);
    const list = el(`<div class="pick-list"></div>`);
    body.append(filter, list);
    const render = () => {
      const q = filter.value.toLowerCase().trim();
      list.innerHTML = "";
      const shown = cands.filter((c) => !q || c.path.toLowerCase().includes(q));
      if (!shown.length) list.append(el(`<div class="pick-item dim">Nothing found${entry ? " – type the path below." : "."}</div>`));
      for (const c of shown) {
        const it = el(`<div class="pick-item"><div class="t"></div><div class="s"></div></div>`);
        it.querySelector(".t").textContent = basename(c.path) || c.path;
        it.querySelector(".s").textContent = `${c.when ? c.when + " · " : ""}${c.files} files ${c.note || ""}\n${c.path}`;
        it.onclick = () => close({ path: c.path.replace(/\/+$/, "") });
        list.append(it);
      }
    };
    filter.oninput = render;
    render();
    if (entry) {
      const own = el(`<div style="display:flex;gap:6px"><input type="text"><button>Use</button></div>`);
      const inp = own.querySelector("input");
      inp.placeholder = entryTitle;
      const take = () => inp.value.trim() && close({ path: inp.value.trim().replace(/\/+$/, "") });
      own.querySelector("button").onclick = take;
      inp.onkeydown = (e) => { if (e.key === "Enter") take(); };
      body.append(own);
    }
    box.append(body);
    const foot = el(`<div class="modal-foot"></div>`);
    if (noneLabel) {
      const n = el(`<button>${esc(noneLabel)}</button>`);
      n.onclick = () => close({ path: null });
      foot.append(n);
    }
    const c = el(`<button>Cancel</button>`);
    c.onclick = () => close(null);
    foot.append(c);
    box.append(foot);
    setTimeout(() => filter.focus(), 0);
  }, { wide: true });
}

// --- Spiel hinzufügen ------------------------------------------------------------------

/** Schritt 1 + 2: Name eingeben, dann suchen. Danach geht's in reviewGame(). */
function addWizard(initial) {
  modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${initial ? `Set up ${esc(initial)} again` : "Add game"}</h2>
      <p class="dim">I'll search the PC and the Thor for the save folders and suggest the right ones. You double-check.</p></div>`));
    const body = el(`<div class="modal-body"><input type="text" placeholder="Name, e.g. Tears of the Kingdom"></div>`);
    const inp = body.querySelector("input");
    inp.value = initial;
    box.append(body);
    const foot = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">Find folders</button></div>`);
    foot.children[0].onclick = () => close(null);
    const go = async () => {
      const game = inp.value.trim();
      if (!game) return inp.focus();
      if (!S.connected) return toast("Connect the Thor first (USB debugging on)", true);
      box.innerHTML = "";
      box.append(el(`<div class="modal-body"><div class="busy"><span class="spin"></span><b class="wiz-step">Starting …</b><span class="dim">This can take a moment.</span></div></div>`));
      const un = await T.event.listen("save-step", (e) => { const p = box.querySelector(".wiz-step"); if (p) p.textContent = e.payload; });
      let r;
      try {
        r = await call("scan_game", { name: game });
      } catch {
        un();
        return close(null);
      }
      un();
      close(1);
      reviewGame({
        game, pc: r.pc, thor: r.thor,
        choice: { pc: r.ai.pc || null, thor: r.ai.thor || null },
        none: { pc: false, thor: false },
        why: { pc: r.ai.pc ? r.ai.pc_reason : null, thor: r.ai.thor ? r.ai.thor_reason : null },
      });
    };
    foot.children[1].onclick = go;
    inp.onkeydown = (e) => { if (e.key === "Enter") go(); };
    box.append(foot);
    setTimeout(() => inp.focus(), 0);
  }, { wide: true });
}

/** Schritt 3: Vorschlag prüfen, Ordner ändern, speichern. */
function reviewGame(st) {
  modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(st.game)}: Does this look right?</h2><p class="dim">Quickly check that these folders really contain the saves.</p></div>`));
    const body = el(`<div class="modal-body"></div>`);
    for (const [side, title] of [["pc", "💻 On the PC"], ["thor", "🎮 On the Thor"]]) {
      const p = st.choice[side];
      const b = el(`<div class="side-box"><div class="head"><b>${title}</b><button>Choose another folder</button></div>
        <div class="mono"></div><div class="note ai"></div><div class="note files"></div></div>`);
      b.querySelector(".mono").textContent = p
        || (st.none[side] ? `The game isn't on the ${side === "pc" ? "PC" : "Thor"} – no sync then, but import works.` : "No folder yet – pick one yourself.");
      b.querySelector(".ai").textContent = st.why[side] ? `AI: ${st.why[side]}` : "";
      b.querySelector("button").onclick = async () => {
        close(null);
        const r = await folderPicker(side === "pc" ? "Folder on the PC" : "Folder on the Thor", st[side], {
          noneLabel: side === "pc" ? "The game isn't on the PC" : "The game isn't on the Thor",
        });
        if (r) { st.choice[side] = r.path; st.none[side] = !r.path; st.why[side] = null; }
        reviewGame(st);
      };
      if (p) {
        const f = b.querySelector(".files");
        f.textContent = "Looking inside …";
        call("peek", { side, path: p }, true).then((i) => {
          f.textContent = [i.summary, ...i.names.slice(0, 6).map((n) => "·  " + n), i.files > 6 ? `·  … and ${i.files - 6} more` : ""].filter(Boolean).join("\n");
        }).catch(() => (f.textContent = ""));
      }
      body.append(b);
    }
    const { pc, thor } = st.choice;
    if (pc && thor && basename(pc) !== basename(thor)) {
      body.append(el(`<p class="hint-warn">⚠ The folder names differ. That can be fine, but better check that both contain the same files.</p>`));
    }
    box.append(body);
    const foot = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">Save</button></div>`);
    foot.children[0].onclick = () => close(null);
    foot.children[1].disabled = !(pc || thor);
    foot.children[1].onclick = async () => {
      await call("save_game", { name: st.game, pc: pc || null, thor: thor || null });
      close(1);
      toast(`“${st.game}” is set up`);
      loadGames();
    };
    box.append(foot);
  }, { wide: true });
}

// --- Spielstand importieren --------------------------------------------------------------

function importWizard(game) {
  const cfg = (cards[game] || {}).cfg || {};
  const sides = ["pc", "thor"].filter((s) => cfg[s]);
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
    S.importTarget = (p) => p && start(p, false);
    body.querySelector('[data-k="file"]').onclick = async () => {
      const p = await T.dialog.open({ title: "Choose save", filters: [{ name: "Archives and saves", extensions: ["zip", "7z", "rar", "tar", "gz", "tgz", "xz", "bz2", "sav", "bin", "dat", "*"] }] });
      if (p) start(p, false);
    };
    body.querySelector('[data-k="dir"]').onclick = async () => {
      const p = await T.dialog.open({ title: "Folder containing the save", directory: true });
      if (p) start(p, false);
    };
    body.querySelector('[data-k="thor"]').onclick = async () => {
      if (!S.connected) return toast("Connect the Thor first (USB debugging on)", true);
      close(1);
      const wait = busyModal("Searching the Thor …");
      const cands = await call("scan_thor_sources", { name: game }).catch(() => []);
      wait();
      const r = await folderPicker("On the Thor", cands, { entryTitle: "Or type a path on the Thor" });
      if (r && r.path) runImport(game, sides, r.path, true);
    };
  });
}

/** Modal mit Spinner, gibt eine Funktion zum Schließen zurück. */
function busyModal(text) {
  let closer, setText;
  modal((box, close) => {
    box.append(el(`<div class="modal-body"><div class="busy"><span class="spin"></span><b class="wiz-step"></b></div></div>`));
    setText = (t) => (box.querySelector(".wiz-step").textContent = t);
    setText(text);
    closer = close;
  });
  const f = () => closer(null);
  f.text = (t) => setText(t);
  return f;
}

async function runImport(game, sides, source, fromThor) {
  if ((fromThor || sides.includes("thor")) && !S.connected) return toast("The Thor is not connected.", true);
  const wait = busyModal("Fetching and extracting …");
  let prep;
  try {
    prep = await call("prepare_import", { name: game, source, fromThor }, true);
  } catch (e) {
    wait();
    return dialog({ title: "Import failed", body: String(e) });
  }
  wait();
  importReview(game, sides, prep);
}

function importReview(game, sides, prep) {
  let root = { path: prep.root, label: prep.root_label, info: prep.root_info, hits: prep.hits, why: prep.why };
  let importing = false;
  modal((box, close) => {
    const render = () => {
      box.innerHTML = "";
      box.append(el(`<div class="modal-head"><h2>${esc(game)}: Does this look right?</h2></div>`));
      const body = el(`<div class="modal-body"></div>`);
      const src = el(`<div class="side-box"><div class="head"><b>From the package</b><button>Other folder</button></div>
        <div class="mono"></div><div class="note hint"></div><div class="note files mono"></div><div class="note dropped"></div></div>`);
      src.querySelector(".mono").textContent = `${root.label}  ·  ${root.info.files} files`;
      const hint = src.querySelector(".hint");
      hint.className = "note " + (root.hits ? "hint-ok" : prep.known ? "hint-warn" : "");
      hint.textContent = (root.hits ? "✓  " : prep.known ? "⚠  " : "") + root.why;
      src.querySelector(".files").textContent = root.info.names.slice(0, 10).map((n) => "·  " + n).join("\n") + (root.info.files > 10 ? `\n·  … and ${root.info.files - 10} more` : "");
      src.querySelector(".dropped").textContent = prep.dropped.length ? `Left out: ${prep.dropped.slice(0, 6).join(", ")}${prep.dropped.length > 6 ? " …" : ""}` : "";
      src.querySelector("button").onclick = async () => {
        importing = true; // Arbeitsordner nicht wegräumen, während der Picker offen ist
        close(null);
        const r = await folderPicker("Which folder is the save?", prep.dirs, { entry: false });
        if (r && r.path) {
          const c = await call("check_root", { name: game, content: prep.content, root: r.path }).catch(() => null);
          if (c) root = { path: r.path, label: c.label, info: c.info, hits: c.hits, why: c.why };
        }
        importReview2();
      };
      body.append(src);
      const tgt = el(`<div class="side-box"><b>Import to</b><span class="note">The old save is backed up first.</span></div>`);
      const checks = {};
      for (const s of sides) {
        const c = el(`<label class="check"><input type="checkbox" checked><span></span></label>`);
        c.querySelector("span").textContent = `${s === "pc" ? "💻 PC" : "🎮 Thor"} – ${(cards[game] || {}).cfg[s]}`;
        checks[s] = c.querySelector("input");
        tgt.append(c);
      }
      const merge = el(`<label class="check"><input type="checkbox"><span>Keep existing files (only overwrite what's in the import)</span></label>`);
      tgt.append(merge);
      body.append(tgt);
      if (prep.missing.length) {
        body.append(el(`<p class="note">Note: the save folder doesn't exist on the ${prep.missing.map((s) => (s === "pc" ? "PC" : "Thor")).join(" and ")} yet and will be created. If the game doesn't see the save afterwards, start it once without importing so the emulator creates the folder itself, then import again.</p>`));
      }
      box.append(body);
      const foot = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">Import</button></div>`);
      foot.children[0].onclick = () => close(null);
      const goBtn = foot.children[1];
      const check = () => (goBtn.disabled = !Object.values(checks).some((c) => c.checked));
      Object.values(checks).forEach((c) => (c.onchange = check));
      check();
      goBtn.onclick = async () => {
        const targets = sides.filter((s) => checks[s].checked);
        if (targets.includes("thor") && !S.connected) return toast("Thor is not connected", true);
        let bodyTxt = `The imported save replaces the current one on the <b>${targets.map((s) => (s === "pc" ? "PC" : "Thor")).join(" and ")}</b>.<br><br>The old save is backed up first (menu ⋮ → Open backups).`;
        if (targets.includes("pc")) bodyTxt += "<br><br>Close the game on the PC first.";
        if (targets.includes("thor")) bodyTxt += "<br>If the emulator is running on the Thor, it will be closed.";
        importing = true;
        close(null);
        const ok = await dialog({ title: "Import save?", html: true, body: bodyTxt, buttons: [["Cancel", null, ""], ["Import", "go", "primary"]], defaultValue: "go" });
        if (ok !== "go") { importing = false; return importReview2(); }
        const wait = busyModal("Starting …");
        const un = await T.event.listen("save-step", (e) => wait.text(e.payload));
        try {
          await call("do_import", { name: game, root: root.path, targets, merge: merge.querySelector("input").checked }, true);
          toast(`“${game}”: save imported 🎮`);
        } catch (e) {
          dialog({ title: "Import failed", body: String(e) });
        }
        un();
        wait();
        invoke("cleanup_import", { work: prep.work });
        if (cards[game]) cards[game].refresh();
      };
      box.append(foot);
    };
    render();
  }, { wide: true }).then(() => {
    if (!importing) invoke("cleanup_import", { work: prep.work });
  });
  function importReview2() { importReview(game, sides, { ...prep, root: root.path, root_label: root.label, root_info: root.info, hits: root.hits, why: root.why }); }
}

// ---------------------------------------------------------------- Stream

document.querySelectorAll(".stream-btn").forEach((b) => (b.onclick = async () => {
  if (!S.connected) return toast("Thor is not connected", true);
  const running = await call("stream_start", { which: b.dataset.which }).catch(() => null);
  if (running) showStreams(running);
  setTimeout(streamStatus, 2500);
}));
$("#stream-stop").onclick = async () => { await invoke("stream_stop"); streamStatus(); };

async function streamStatus() {
  showStreams(await invoke("stream_status").catch(() => []));
}

function showStreams(running) {
  $("#stream-status").textContent = running.length ? `Running: ${running.map((r) => r + " screen").join(", ")}` : "Nothing running right now.";
  $("#stream-stop").classList.toggle("hidden", !running.length);
  document.querySelectorAll(".stream-btn").forEach((b) => {
    const w = b.dataset.which;
    b.classList.toggle("running", w === "both" ? running.length === 2 : running.includes(w));
  });
}

// ---------------------------------------------------------------- Start

pollDevice(true);
setInterval(() => pollDevice(false), 3000);
setInterval(() => { if (S.tab === "stream") streamStatus(); }, 3000);
