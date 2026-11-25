// ThorBro UI – saves tab: one card per managed save (one per game and emulator) with its sync
// status, the card menu, and the "Continue on PC / Thor" sync flow.
// Adding a save lives in saves-add.js (by title ID, Update 9) and saves-setup.js (searching all
// folders), importing a save in saves-import.js.
"use strict";

// ---------------------------------------------------------------- Game list

/** Entry key → GameCard of the currently shown cards. */
const cards = {};

/** Loads games.json and rebuilds all cards. */
async function loadGames() {
  await loadProfiles(); // catalog.js: emulator names and which ones may be written to
  // → commands::games → saves::migrate_games + saves::load_games
  const games = await call("games", {}, true).catch(() => ({}));
  const list = $("#game-list");
  list.innerHTML = "";
  for (const name of Object.keys(cards)) delete cards[name];
  const title = (name) => games[name].title || name;
  const names = Object.keys(games).sort((a, b) => title(a).localeCompare(title(b), undefined, { sensitivity: "base" }) || a.localeCompare(b));
  $("#games-empty").classList.toggle("hidden", names.length > 0);
  for (const name of names) {
    const card = new GameCard(name, games[name]);
    cards[name] = card;
    list.append(card.el);
  }
  refreshCards();
}

/** Re-reads the save status of every card. */
function refreshCards() {
  Object.values(cards).forEach((card) => card.refresh());
}

$("#btn-add-game").onclick = () => addSaveWizard(""); // saves-add.js

// ---------------------------------------------------------------- Game card

/**
 * One managed save. Entries with a Thor folder get both sync buttons ("Continue on PC" asks for
 * the PC folder the first time); an old entry that only has a PC folder can only import.
 * The emulator chip comes from the profile; a profile with `write: false` (Peanuts) disables
 * "Continue on Thor" and says why.
 */
class GameCard {
  constructor(name, cfg) {
    this.name = name;
    this.cfg = cfg;
    this.title = cfg.title || name;
    const profile = cfg.emulator ? profileById(cfg.emulator) : null;
    this.blocked = profile && !profile.write ? profile.write_note : null;
    this.pcOnly = !!cfg.pc && !cfg.thor;
    this.el = el(`<div class="card">
      <div class="card-top"><h2></h2><span class="spin hidden"></span><span class="emu-chip card-emu hidden"></span>
        <div class="menu"><button class="icon flat" title="More">⋮</button></div></div>
      <div class="save-state dim"></div><div class="detail"></div><p class="card-note hidden"></p>
      <div class="btns"></div></div>`);
    this.el.querySelector("h2").textContent = this.title;
    this.el.querySelector("h2").title = this.title;
    if (cfg.emulator) {
      const chip = this.el.querySelector(".card-emu");
      chip.textContent = profile ? profile.name : cfg.emulator;
      chip.title = profile ? profile.package : `No profile “${cfg.emulator}” found`;
      chip.classList.remove("hidden");
    }
    if (this.blocked) {
      const note = this.el.querySelector(".card-note");
      note.textContent = this.blocked;
      note.classList.remove("hidden");
    }
    this.status = this.el.querySelector(".save-state");
    this.detail = this.el.querySelector(".detail");
    this.spin = this.el.querySelector(".spin");
    this.buildButtons(this.el.querySelector(".btns"));
    const menuButton = this.el.querySelector(".menu button");
    menuButton.onclick = (e) => { e.stopPropagation(); this.menu(); };
  }

  /** Sync buttons, or a single import button for an entry without a Thor folder. */
  buildButtons(container) {
    if (this.pcOnly) {
      container.classList.add("one");
      const importButton = el(`<button>Import save</button>`);
      importButton.onclick = () => importWizard(this.name);
      container.append(importButton);
      this.bPc = this.bThor = null;
      return;
    }
    this.bPc = el(`<button title="Pulls the save from the Thor to the PC">Continue on PC</button>`);
    this.bThor = el(`<button title="Pushes the save from the PC to the Thor">Continue on Thor</button>`);
    this.bPc.onclick = () => startSync(this, "pc");
    this.bThor.onclick = () => startSync(this, "thor");
    if (this.blocked) this.bThor.title = this.blocked;
    container.append(this.bPc, this.bThor);
    this.lockThor();
  }

  /** Keeps "Continue on Thor" disabled for emulators ThorBro may not write to. */
  lockThor() {
    if (this.bThor && this.blocked) this.bThor.disabled = true;
  }

  /** Opens / closes the ⋮ menu. */
  menu() {
    const menuEl = this.el.querySelector(".menu");
    const oldPopup = menuEl.querySelector(".menu-pop");
    if (oldPopup) return oldPopup.remove();
    const popup = el(`<div class="menu-pop"></div>`);
    for (const [label, action] of this.menuItems()) {
      const button = el(`<button>${esc(label)}</button>`);
      button.onclick = () => { popup.remove(); action(); };
      popup.append(button);
    }
    menuEl.append(popup);
    // Any click elsewhere closes the menu (registered after the current click is done).
    setTimeout(() => document.addEventListener("click", () => popup.remove(), { once: true }), 0);
  }

  /** [label, action] pairs of the ⋮ menu. */
  menuItems() {
    const emulator = this.cfg.emulator ? `<b>Emulator</b><div>${esc(emulatorName(this.cfg.emulator))}${this.cfg.title_id ? ` · title ID ${esc(this.cfg.title_id)}` : ""}</div><br>` : "";
    const items = [
      ["Import save …", () => importWizard(this.name)],
      ["Show folders", () => dialog({
        title: this.title,
        html: true,
        body: `${emulator}<b>PC</b><div class="mono">${esc(this.cfg.pc || "—")}</div><br><b>Thor</b><div class="mono">${esc(this.cfg.thor || "—")}</div>`,
      })],
    ];
    if (this.cfg.emulator && profileById(this.cfg.emulator)) {
      items.push(["Change the PC folder", async () => { if (await choosePcFolder(this)) loadGames(); }]);
      items.push(["Change the Thor folder", () => changeThorFolder(this)]); // saves-add.js
    } else {
      items.push(["Search folders again", () => addWizard(this.name)]);
    }
    // → commands::open_backups (false = no backup folder yet)
    items.push(["Open backups", async () => { if (!(await call("open_backups", { name: this.name }))) toast("No backups for this game yet"); }]);
    items.push(["Remove from list", () => this.confirmRemove()]);
    return items;
  }

  /** Asks, then removes the game from games.json (→ commands::remove_game). */
  async confirmRemove() {
    const choice = await dialog({
      title: `Remove “${this.title}”?`,
      body: "The save is only removed from the list. Your saves on the PC and the Thor stay where they are.",
      buttons: [["Cancel", null, ""], ["Remove", "rm", "danger solid"]],
      defaultValue: null,
    });
    if (choice === "rm") { await call("remove_game", { name: this.name }); loadGames(); }
  }

  /** Shows/hides the spinner and disables the sync buttons while busy. */
  busy(on) {
    this.spin.classList.toggle("hidden", !on);
    [this.bPc, this.bThor].forEach((button) => button && (button.disabled = on));
    this.lockThor();
  }

  /**
   * Sets the status line (kind = CSS class: "ok", "accent" = one side newer, "only" = only on one
   * side, "warn", "dim") and the detail line (default: when it was last synced).
   */
  set(text, kind, detail = this.lastSynced(), tooltip = "") {
    this.status.className = "save-state " + (kind || "");
    this.status.textContent = text;
    this.status.title = tooltip;
    this.detail.textContent = detail;
    [this.bPc, this.bThor].forEach((button) => button && button.classList.remove("primary"));
  }

  /** "last synced today 08:30" or "never synced". */
  lastSynced() {
    return this.cfg.last_sync ? `last synced ${friendlyWhen(this.cfg.last_sync)}` : "never synced";
  }

  /** Reads the save status from Rust and shows which side is newer. */
  async refresh() {
    if (!S.connected && !this.pcOnly) return this.set("Thor not connected", "dim", "Once it's plugged in, you'll see which side has the newer save.");
    this.busy(true);
    this.set("Checking the saves …", "dim");
    let status;
    try {
      // → commands::game_status → saves::status (snapshots of both sides)
      status = await call("game_status", { name: this.name, connected: S.connected }, true);
    } catch (e) {
      this.busy(false);
      return this.set("Couldn't read the saves", "warn", String(e));
    }
    this.busy(false);
    if (this.pcOnly) this.showPcOnlyStatus(status);
    else this.showSyncStatus(status);
  }

  /** An old entry that only has a PC folder. */
  showPcOnlyStatus(status) {
    const info = "Only set up on the PC – no sync, but import works.";
    if (status.pc.exists && status.pc.files) this.set("Save on the PC", "", `${status.pc.summary}\n${info}`);
    else this.set("No save on the PC yet", "dim", info);
  }

  /** Which side is newer; highlights the button that copies the newer save over. */
  showSyncStatus(status) {
    const tooltip = `PC: ${this.cfg.pc ? status.pc.newest_text : "no folder yet"}  ·  Thor: ${status.thor.newest_text}`;
    const pcFiles = status.pc.files, thorFiles = status.thor.files;
    const preferPc = () => this.bPc.classList.add("primary");
    const preferThor = () => { if (!this.blocked) this.bThor.classList.add("primary"); };
    if (!pcFiles && !thorFiles) this.set("No save on either side yet", "dim", this.lastSynced(), tooltip);
    else if (!pcFiles) { this.set("Only on Thor", "only", this.lastSynced(), tooltip); preferPc(); }
    else if (!thorFiles) { this.set("Only on PC", "only", this.lastSynced(), tooltip); preferThor(); }
    else if (status.same) this.set("In sync", "ok", this.lastSynced(), tooltip);
    else if (status.pc.newest === status.thor.newest) this.set("Different, but same age", "warn", this.lastSynced(), tooltip);
    else if (status.thor.newest > status.pc.newest) { this.set("Thor save is newer", "accent", this.lastSynced(), tooltip); preferPc(); }
    else { this.set("PC save is newer", "accent", this.lastSynced(), tooltip); preferThor(); }
  }
}

/**
 * Asks where the save lives on the PC (search by name, folders named like the title ID first)
 * and stores it. Resolves with true if a folder was set.
 */
async function choosePcFolder(card) {
  const closeBusy = busyModal("Looking for this save on the PC …");
  let cands;
  try {
    // → commands::scan_pc_for → saves::scan_pc (+ title ID matches first)
    cands = await call("scan_pc_for", { name: card.name });
  } catch {
    closeBusy();
    return false;
  }
  closeBusy();
  const picked = await folderPicker(`Where is the save of ${card.title} on the PC?`, cands, { entryTitle: "Or type the PC folder (it may not exist yet)" });
  if (!picked || !picked.path) return false;
  // → commands::set_save_folder (games.json)
  await call("set_save_folder", { name: card.name, side: "pc", path: picked.path });
  card.cfg.pc = picked.path;
  return true;
}

// ---------------------------------------------------------------- Sync

/** "Continue on PC/Thor": plan, confirm, sync. target = "pc" or "thor". */
async function startSync(card, target) {
  if (!S.connected) return toast("Thor is not connected", true);
  if (target === "thor" && card.blocked) return dialog({ title: "Can't write to this emulator yet", body: card.blocked });
  if (!card.cfg.pc && !(await choosePcFolder(card))) return;
  card.busy(true);
  let plan;
  try {
    // → commands::plan_sync → saves::plan_sync (what would happen, nothing is copied yet)
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
  if (plan.blocked) return dialog({ title: "Can't write to this emulator yet", body: plan.blocked });
  const { title, body } = syncConfirmText(plan);
  const choice = await dialog({
    title, html: true, body,
    buttons: [["Cancel", null, ""], [plan.dst_newer ? "Overwrite anyway" : "Let's go", "go", plan.dst_newer ? "danger solid" : "primary"]],
    defaultValue: plan.dst_newer ? null : "go",
  });
  if (choice !== "go") return;
  await runSync(card, target, plan);
}

/** Title and HTML body of the sync confirmation, with a warning if the target is newer. */
function syncConfirmText(plan) {
  const src = plan.src_name, dst = plan.dst_name;
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
  return { title, body };
}

/** Performs the confirmed sync and refreshes the card. */
async function runSync(card, target, plan) {
  card.busy(true);
  card.set(`Copying ${plan.src_name} → ${plan.dst_name} …`, "dim");
  try {
    // → commands::run_sync → saves::run_sync (backup, copy, verify)
    await call("run_sync", { name: card.name, target, stopPkg: plan.running_pkg }, true);
    toast(`Done! Have fun on the ${plan.dst_name} 🎮`);
    card.cfg.last_sync = Date.now() / 1000;
  } catch (e) {
    dialog({ title: "Something went wrong", body: String(e) });
  }
  card.busy(false);
  card.refresh();
}
