// ThorBro UI – adding a game (or searching its folders again):
//   addWizard  → enter name, search PC + Thor, AI suggestion   (commands::scan_game)
//   reviewGame → check / change the two folders, save          (commands::peek, save_game)
"use strict";

// ---------------------------------------------------------------- Step 1 + 2: name and search

/** Asks for the game name, then searches. `initial` = name when re-running for an existing game. */
function addWizard(initial) {
  modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${initial ? `Set up ${esc(initial)} again` : "Add game"}</h2>
      <p class="dim">I'll search the PC and the Thor for the save folders and suggest the right ones. You double-check.</p></div>`));
    const body = el(`<div class="modal-body"><input type="text" placeholder="Name, e.g. Tears of the Kingdom"></div>`);
    const input = body.querySelector("input");
    input.value = initial;
    box.append(body);
    const foot = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">Find folders</button></div>`);
    foot.children[0].onclick = () => close(null);
    const startSearch = () => searchGameFolders(box, close, input);
    foot.children[1].onclick = startSearch;
    input.onkeydown = (e) => { if (e.key === "Enter") startSearch(); };
    box.append(foot);
    setTimeout(() => input.focus(), 0);
  }, { wide: true });
}

/** Shows a spinner in the same modal, runs the scan and continues with reviewGame(). */
async function searchGameFolders(box, close, input) {
  const game = input.value.trim();
  if (!game) return input.focus();
  if (!S.connected) return toast("Connect the Thor first (USB debugging on)", true);
  box.innerHTML = "";
  box.append(el(`<div class="modal-body"><div class="busy"><span class="spin"></span><b class="wiz-step">Starting …</b><span class="dim">This can take a moment.</span></div></div>`));
  // Progress texts from Rust ("Searching the PC …"), emitted as "save-step" events.
  const unlisten = await T.event.listen("save-step", (e) => {
    const stepText = box.querySelector(".wiz-step");
    if (stepText) stepText.textContent = e.payload;
  });
  let found;
  try {
    // → commands::scan_game → saves::scan_pc, saves::scan_thor, saves::ai_pick
    found = await call("scan_game", { name: game });
  } catch {
    unlisten();
    return close(null);
  }
  unlisten();
  close(1);
  reviewGame({
    game, pc: found.pc, thor: found.thor,
    choice: { pc: found.ai.pc || null, thor: found.ai.thor || null },
    none: { pc: false, thor: false },
    why: { pc: found.ai.pc ? found.ai.pc_reason : null, thor: found.ai.thor ? found.ai.thor_reason : null },
  });
}

// ---------------------------------------------------------------- Step 3: review and save

/**
 * Shows the chosen folders for both sides. "Choose another folder" closes this dialog,
 * opens the picker and then reopens reviewGame with the updated state `st`:
 *   st.choice[side] – chosen path or null, st.none[side] – "game isn't on this side",
 *   st.why[side]    – AI reason, st.pc / st.thor – all candidates.
 */
function reviewGame(st) {
  modal((box, close) => {
    box.append(el(`<div class="modal-head"><h2>${esc(st.game)}: Does this look right?</h2><p class="dim">Quickly check that these folders really contain the saves.</p></div>`));
    const body = el(`<div class="modal-body"></div>`);
    body.append(buildSideBox(st, "pc", "💻 On the PC", close));
    body.append(buildSideBox(st, "thor", "🎮 On the Thor", close));
    const { pc, thor } = st.choice;
    if (pc && thor && basename(pc) !== basename(thor)) {
      body.append(el(`<p class="hint-warn">⚠ The folder names differ. That can be fine, but better check that both contain the same files.</p>`));
    }
    box.append(body);
    const foot = el(`<div class="modal-foot"><button>Cancel</button><button class="primary">Save</button></div>`);
    foot.children[0].onclick = () => close(null);
    foot.children[1].disabled = !(pc || thor);
    foot.children[1].onclick = async () => {
      // → commands::save_game → saves::save_games (writes games.json)
      await call("save_game", { name: st.game, pc: pc || null, thor: thor || null });
      close(1);
      toast(`“${st.game}” is set up`);
      loadGames();
    };
    box.append(foot);
  }, { wide: true });
}

/** The box for one side: chosen folder, AI reason, a peek at the files, change button. */
function buildSideBox(st, side, title, close) {
  const sideName = side === "pc" ? "PC" : "Thor";
  const path = st.choice[side];
  const sideBox = el(`<div class="side-box"><div class="head"><b>${title}</b><button>Choose another folder</button></div>
        <div class="mono"></div><div class="note ai"></div><div class="note files"></div></div>`);
  sideBox.querySelector(".mono").textContent = path
    || (st.none[side] ? `The game isn't on the ${sideName} – no sync then, but import works.` : "No folder yet – pick one yourself.");
  sideBox.querySelector(".ai").textContent = st.why[side] ? `AI: ${st.why[side]}` : "";
  sideBox.querySelector("button").onclick = async () => {
    close(null);
    const picked = await folderPicker(`Folder on the ${sideName}`, st[side], {
      noneLabel: `The game isn't on the ${sideName}`,
    });
    if (picked) { st.choice[side] = picked.path; st.none[side] = !picked.path; st.why[side] = null; }
    reviewGame(st);
  };
  if (path) showFolderPeek(sideBox.querySelector(".files"), side, path);
  return sideBox;
}

/** Fills `target` with a summary and the first file names of a folder (→ commands::peek). */
function showFolderPeek(target, side, path) {
  target.textContent = "Looking inside …";
  call("peek", { side, path }, true).then((info) => {
    const lines = [info.summary, ...info.names.slice(0, 6).map((name) => "·  " + name), info.files > 6 ? `·  … and ${info.files - 6} more` : ""];
    target.textContent = lines.filter(Boolean).join("\n");
  }).catch(() => (target.textContent = ""));
}
