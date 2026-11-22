// ThorBro UI – saves tab: one card per game with its sync status, the card menu, and the
// "Continue on PC / Thor" sync flow.
// Adding a game lives in saves-setup.js, importing a save in saves-import.js.
"use strict";

// ---------------------------------------------------------------- Game list

/** Game name → GameCard of the currently shown cards. */
const cards = {};

/** Loads games.json and rebuilds all cards. */
async function loadGames() {
  // → commands::games → saves::load_games
  const games = await call("games", {}, true).catch(() => ({}));
  const list = $("#game-list");
  list.innerHTML = "";
  for (const name of Object.keys(cards)) delete cards[name];
  const names = Object.keys(games).sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
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

$("#btn-add-game").onclick = () => addWizard("");

// ---------------------------------------------------------------- Game card

/**
 * One game card. A game set up on both sides gets two sync buttons; a game set up on only
 * one side ("oneSide") can only import.
 */
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
    this.buildButtons(this.el.querySelector(".btns"));
    const menuButton = this.el.querySelector(".menu button");
    menuButton.onclick = (e) => { e.stopPropagation(); this.menu(); };
  }

  /** Sync buttons (both sides set up) or a single import button. */
  buildButtons(container) {
    if (this.oneSide) {
      container.classList.add("one");
      const importButton = el(`<button>Import save</button>`);
      importButton.onclick = () => importWizard(this.name);
      container.append(importButton);
      this.bPc = this.bThor = null;
    } else {
      this.bPc = el(`<button title="Pulls the save from the Thor to the PC">💻 Continue on PC</button>`);
      this.bThor = el(`<button title="Pushes the save from the PC to the Thor">🎮 Continue on Thor</button>`);
      this.bPc.onclick = () => startSync(this, "pc");
      this.bThor.onclick = () => startSync(this, "thor");
      container.append(this.bPc, this.bThor);
    }
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
    return [
      ["Import save …", () => importWizard(this.name)],
      ["Show folders", () => dialog({
        title: this.name,
        html: true,
        body: `<b>PC</b><div class="mono">${esc(this.cfg.pc || "—")}</div><br><b>Thor</b><div class="mono">${esc(this.cfg.thor || "—")}</div>`,
      })],
      ["Search folders again", () => addWizard(this.name)],
      // → commands::open_backups (false = no backup folder yet)
      ["Open backups", async () => { if (!(await call("open_backups", { name: this.name }))) toast("No backups for this game yet"); }],
      ["Remove from list", () => this.confirmRemove()],
    ];
  }

  /** Asks, then removes the game from games.json (→ commands::remove_game). */
  async confirmRemove() {
    const choice = await dialog({
      title: `Remove “${this.name}”?`,
      body: "The game is only removed from the list. Your saves on the PC and the Thor stay where they are.",
      buttons: [["Cancel", null, ""], ["Remove", "rm", "danger solid"]],
      defaultValue: null,
    });
    if (choice === "rm") { await call("remove_game", { name: this.name }); loadGames(); }
  }

  /** Shows/hides the spinner and disables the sync buttons while busy. */
  busy(on) {
    this.spin.classList.toggle("hidden", !on);
    [this.bPc, this.bThor].forEach((button) => button && (button.disabled = on));
  }

  /** Sets status line (kind = CSS class: "ok", "warn", "accent", "dim") and detail text. */
  set(text, kind, detail = "") {
    this.status.className = "status " + (kind || "");
    this.status.textContent = text;
    this.detail.textContent = detail;
    [this.bPc, this.bThor].forEach((button) => button && button.classList.remove("primary"));
  }

  /** Reads the save status from Rust and shows which side is newer. */
  async refresh() {
    const needsThor = !(this.oneSide && this.where === "PC");
    if (needsThor && !S.connected) {
      return this.set("Thor not connected", "dim", "Once it's plugged in, you'll see here which side has the newer save.");
    }
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
    if (this.oneSide) this.showOneSideStatus(status);
    else this.showSyncStatus(status);
  }

  /** Status for a game that only exists on one side. */
  showOneSideStatus(status) {
    const side = this.where === "PC" ? status.pc : status.thor;
    const info = `Only set up on the ${this.where} – no sync, but import works.`;
    if (side.exists && side.files) this.set(`Save on the ${this.where}`, "", `${side.summary}\n${info}`);
    else this.set(`No save on the ${this.where} yet`, "dim", info);
  }

  /** Status for a game on both sides; highlights the button that copies the newer save over. */
  showSyncStatus(status) {
    const detail = `PC: ${status.pc.newest_text}   ·   Thor: ${status.thor.newest_text}`;
    if (!status.pc.files && !status.thor.files) this.set("No save on either side yet", "dim", detail);
    else if (status.same) this.set("✓  Both in sync", "ok", detail);
    else if (status.pc.newest === status.thor.newest) this.set("●  Different, but same age", "warn", detail);
    else if (status.thor.newest > status.pc.newest) { this.set("●  Thor is newer", "accent", detail); this.bPc.classList.add("primary"); }
    else { this.set("●  PC is newer", "accent", detail); this.bThor.classList.add("primary"); }
  }
}

// ---------------------------------------------------------------- Sync

/** "Continue on PC/Thor": plan, confirm, sync. target = "pc" or "thor". */
async function startSync(card, target) {
  if (!S.connected) return toast("Thor is not connected", true);
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
  } catch (e) {
    dialog({ title: "Something went wrong", body: String(e) });
  }
  card.busy(false);
  card.refresh();
}
