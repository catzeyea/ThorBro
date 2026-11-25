// ThorBro UI – "+ Add a game save" (Update 9), two steps:
//   1. search a game → the installed emulators of its console, the ones with a save first
//      (found by title ID in the catalog, catalog.js); the others are greyed out.
//   2. exactly one save folder (path, file count, last change) → "Add this save"
//      (→ commands::add_save). "Wrong folder? Browse" opens the folder picker with the folders
//      of the emulator's data folder (→ commands::browse_emulator).
// "The game isn't on the Thor" and "search all folders" go to the older flow in saves-setup.js.
"use strict";

/** Most games listed for one search. */
const SEARCH_LIMIT = 8;

/** Lowercase words of a text, punctuation removed ("Zelda: BotW" → ["zelda", "botw"]). */
function searchWords(text) {
  return String(text).toLowerCase().replace(/[^a-z0-9]+/g, " ").trim().split(" ").filter(Boolean);
}

/**
 * Games of the catalog that match `query`: every word must appear in the title (or the query is
 * a part of the title ID). Titles that start with the query come first.
 */
function searchGames(games, query) {
  const words = searchWords(query);
  if (!words.length) return [];
  const compact = words.join("");
  const hits = games.filter((game) => {
    const title = searchWords(game.title).join(" ");
    return words.every((word) => title.includes(word)) || game.title_id.toLowerCase().includes(compact);
  });
  const starts = (game) => (searchWords(game.title).join(" ").startsWith(words.join(" ")) ? 0 : 1);
  hits.sort((a, b) => starts(a) - starts(b) || a.title.localeCompare(b.title));
  return hits.slice(0, SEARCH_LIMIT);
}

/** The key of an existing entry for this emulator and title ID, or null. */
function managedSave(emulatorId, titleId) {
  const hit = Object.entries(cards).find(([, card]) => card.cfg.emulator === emulatorId && card.cfg.title_id === titleId);
  return hit ? hit[1] : null;
}

/** A Thor path for display: internal storage paths without "/sdcard/" (as on the boards). */
function thorPathLabel(path) {
  return path.startsWith("/sdcard/") ? path.slice("/sdcard/".length) : path;
}

/** "27 files · changed today 08:30". */
function saveInfoText(files, newest) {
  return `${files} ${files === 1 ? "file" : "files"} · changed ${friendlyWhen(newest)}`;
}

/** Opens step 1. `query` prefills the search. */
async function addSaveWizard(query = "") {
  if (!S.connected) return toast("Connect the Thor first (USB debugging on)", true);
  await loadProfiles();
  // Always read the Thor again (a save may be new since the last catalog); the old catalog is
  // shown meanwhile.
  const st = { query, game: null, loaded: false };
  st.load = loadCatalog().finally(() => { st.loaded = true; });
  addSaveStep1(st);
}

// ---------------------------------------------------------------- Step 1: game and emulator

function addSaveStep1(st) {
  modal((box, close) => {
    box.append(el(`<div class="modal-head step-head"><h2>Add a game save</h2><span class="step">Step 1 of 2</span></div>`));
    const body = el(`<div class="modal-body add-save"><input type="text" class="search" placeholder="Search a game, e.g. breath of the wild"><div class="results"></div>
      <p class="dim small">Using an emulator ThorBro has no profile for? <button type="button" class="link-btn all-folders">Search all folders instead</button></p></div>`);
    const input = body.querySelector("input");
    const results = body.querySelector(".results");
    input.value = st.query;
    const toOldFlow = () => { close(null); addWizard(st.query.trim()); }; // saves-setup.js
    body.querySelector(".all-folders").onclick = toOldFlow;
    box.append(body);
    const foot = el(`<div class="modal-foot"><button>The game isn't on the Thor</button><button>Cancel</button></div>`);
    foot.children[0].title = "Set up the save on the PC only (search all folders)";
    foot.children[0].onclick = toOldFlow;
    foot.children[1].onclick = () => close(null);
    box.append(foot);
    const render = () => renderStep1(st, results, close, render);
    input.oninput = () => { st.query = input.value; st.game = null; render(); };
    render();
    if (!st.loaded) st.load.then(() => { if (results.isConnected) render(); });
    setTimeout(() => input.focus(), 0);
  });
}

/** Fills the result area of step 1. */
function renderStep1(st, results, close, render) {
  results.replaceChildren();
  if (!CAT.data) {
    if (st.loaded) {
      const error = el(`<p class="hint-warn"></p>`);
      error.textContent = `Couldn't read the games on your Thor${CAT.error ? `: ${CAT.error}` : "."}`;
      results.append(error);
    } else {
      results.append(el(`<div class="busy"><span class="spin"></span><span>Looking at your Thor …</span></div>`));
    }
    return;
  }
  if (!searchWords(st.query).length) {
    results.append(el(`<p class="dim">Type a few letters of the game's name (or its title ID).</p>`));
    return;
  }
  const matches = searchGames(CAT.data.search, st.query);
  if (!matches.length) {
    const none = el(`<p class="dim"></p>`);
    none.textContent = `No game on your Thor matches “${st.query.trim()}”.`;
    results.append(none);
    return;
  }
  if (!st.game && matches.length === 1) st.game = matches[0];
  if (!st.game) {
    results.append(el(`<span class="label">pick the game</span>`));
    for (const game of matches) {
      const row = el(`<button type="button" class="pick-row"><span class="t"></span><span class="s"></span></button>`);
      row.querySelector(".t").textContent = game.title;
      row.querySelector(".s").textContent = systemTitle(game.console, { full: game.console_name });
      row.onclick = () => { st.game = game; render(); };
      results.append(row);
    }
    return;
  }
  const game = st.game;
  const head = el(`<div class="picked-game"><span class="label">game</span><div><b class="t"></b> <span class="dim s"></span></div></div>`);
  head.querySelector(".t").textContent = game.title;
  head.querySelector(".s").textContent = `· ${systemTitle(game.console, { full: game.console_name })}`;
  if (matches.length > 1) {
    const other = el(`<button type="button" class="link-btn">another game</button>`);
    other.onclick = () => { st.game = null; render(); };
    head.querySelector("div").append(" ", other);
  }
  results.append(head);

  const emulators = CAT.data.emulators.filter((emulator) => emulator.installed && emulator.console === game.console);
  if (!emulators.length) {
    results.append(el(`<p class="dim">None of the emulators ThorBro has a profile for is installed for this console.</p>`));
    return;
  }
  results.append(el(`<span class="label">pick the emulator that has your save</span>`));
  const withSave = (emulator) => emulator.saves.find((save) => save.title_id === game.title_id) || null;
  emulators.sort((a, b) => Number(!!withSave(b)) - Number(!!withSave(a)));
  for (const emulator of emulators) {
    const save = withSave(emulator);
    const existing = save && managedSave(emulator.id, game.title_id);
    const row = el(`<button type="button" class="emu-row-pick"><span class="emu-text"><b></b><span class="s"></span></span><span class="state"></span></button>`);
    row.querySelector("b").textContent = emulator.name;
    if (emulator.error) {
      row.querySelector(".s").textContent = emulator.error;
      row.querySelector(".state").textContent = "Not readable";
    } else if (!save) {
      row.querySelector(".s").textContent = "installed on your Thor";
      row.querySelector(".state").textContent = "No save for this game";
    } else if (existing) {
      row.querySelector(".s").textContent = saveInfoText(save.files, save.newest);
      row.querySelector(".state").textContent = "Already in your saves";
    } else {
      row.querySelector(".s").textContent = saveInfoText(save.files, save.newest);
      row.querySelector(".state").textContent = "Save found";
      row.classList.add("found");
    }
    row.disabled = !save || !!existing || !!emulator.error;
    row.onclick = () => { close(1); addSaveStep2(st, emulator, { path: save.path, files: save.files, newest: save.newest }, false); };
    results.append(row);
  }
}

// ---------------------------------------------------------------- Step 2: the one folder

/** Shows the one save folder of `emulator` for the picked game. `picked` = chosen by hand. */
function addSaveStep2(st, emulator, save, picked) {
  const game = st.game;
  modal((box, close) => {
    box.append(el(`<div class="modal-head step-head"><h2>Is this your save?</h2><span class="step">Step 2 of 2</span></div>`));
    const body = el(`<div class="modal-body add-save">
        <div class="save-game"><b class="t"></b><span class="emu-chip"></span></div>
        <div class="save-folder"><span class="label">save folder on the Thor</span><b class="path"></b><span class="dim info"></span></div>
        <p class="why"></p></div>`);
    body.querySelector(".t").textContent = game.title;
    body.querySelector(".emu-chip").textContent = emulator.name;
    body.querySelector(".path").textContent = thorPathLabel(save.path);
    body.querySelector(".path").title = save.path;
    body.querySelector(".info").textContent = saveInfoText(save.files, save.newest);
    body.querySelector(".why").textContent = picked
      ? "You picked this folder yourself. Nothing is copied yet. Adding it lets you move this save between PC and Thor and keeps a backup before every copy."
      : "ThorBro found it by the game's title ID. Nothing is copied yet. Adding it lets you move this save between PC and Thor and keeps a backup before every copy.";
    if (!emulator.write && emulator.write_note) {
      const warn = el(`<p class="hint-warn"></p>`);
      warn.textContent = emulator.write_note;
      body.append(warn);
    }
    box.append(body);
    const foot = el(`<div class="modal-foot"><button type="button" class="link-btn browse">Wrong folder? Browse</button><span class="foot-gap"></span><button>Back</button><button class="primary">Add this save</button></div>`);
    foot.querySelector(".browse").onclick = async () => {
      close(null);
      const chosen = await browseEmulatorFolder(emulator, game.title);
      if (chosen) addSaveStep2(st, emulator, chosen, true);
      else addSaveStep2(st, emulator, save, picked);
    };
    foot.children[2].onclick = () => { close(null); addSaveStep1(st); };
    foot.children[3].onclick = async () => {
      try {
        // → commands::add_save (games.json: one entry per game and emulator)
        await call("add_save", { title: game.title, emulator: emulator.id, titleId: game.title_id, thor: save.path });
      } catch {
        return;
      }
      close(1);
      toast(`“${game.title}” (${emulator.name}) is in your saves`);
      loadGames();
    };
    box.append(foot);
    setTimeout(() => foot.children[3].focus(), 0);
  });
}

/**
 * The old folder picker, filled with the folders of the emulator's data folder (save folders
 * first). Resolves with { path, files, newest } of the chosen folder, or null.
 */
async function browseEmulatorFolder(emulator, title) {
  const closeBusy = busyModal(`Reading ${emulator.name}'s folders …`);
  let cands;
  try {
    // → commands::browse_emulator → emulators::thor::browse_dirs
    cands = await call("browse_emulator", { emulator: emulator.id });
  } catch {
    closeBusy();
    return null;
  }
  closeBusy();
  const picked = await folderPicker(`${title}: folder on the Thor (${emulator.name})`, cands, { entryTitle: "Or type a Thor path" });
  if (!picked || !picked.path) return null;
  // → commands::peek → saves::snapshot (file count and newest change)
  const info = await call("peek", { side: "thor", path: picked.path }, true).catch(() => ({ files: 0, newest: 0 }));
  return { path: picked.path, files: info.files, newest: info.newest };
}

/** Card menu "Change the Thor folder": the same picker, then stores the folder. */
async function changeThorFolder(card) {
  const emulator = (CAT.data && CAT.data.emulators.find((item) => item.id === card.cfg.emulator)) || profileById(card.cfg.emulator);
  const chosen = await browseEmulatorFolder(emulator, card.title);
  if (!chosen) return;
  // → commands::set_save_folder (games.json)
  await call("set_save_folder", { name: card.name, side: "thor", path: chosen.path });
  toast(`Thor folder of “${card.title}” changed`);
  loadGames();
}
