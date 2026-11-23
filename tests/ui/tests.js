// UI tests for the file manager, the transfer strip and the stream tab, evaluated inside the page (real index.html + scripts, fake Tauri
// bridge from stub.js). Returns a JSON string: [{ name, ok, info }].
(async () => {
  const H = window.HARNESS;
  const results = [];
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const until = async (cond, ms = 3000) => { const t0 = performance.now(); while (!cond()) { if (performance.now() - t0 > ms) return false; await sleep(10); } return true; };
  const check = (name, ok, info = "") => results.push({ name, ok: !!ok, info: String(info) });
  const rowEls = () => [...document.querySelectorAll("#filelist .row")];
  const rowByName = (n) => rowEls().find((r) => r._entry && r._entry.name === n);
  const names = () => S.shown.map((e) => e.name);
  const key = (k, opts = {}) => (document.activeElement || document.body).dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...opts }));
  const mouse = (target, type, opts = {}) => {
    const box = target.getBoundingClientRect();
    target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: box.left + 60, clientY: box.top + 10, ...opts }));
  };
  const click = (name, opts = {}) => { const r = rowByName(name); mouse(r, "mousedown", opts); mouse(r, "mouseup", opts); mouse(r, "click", opts); };
  const callsOf = (cmd) => H.calls.filter((c) => c.cmd === cmd);
  const editor = () => document.querySelector("input.rename");
  const typeInto = (input, value, finalKey = "Enter") => { input.value = value; input.dispatchEvent(new KeyboardEvent("keydown", { key: finalKey, bubbles: true })); };

  try {
    // ---- initial load
    check("initial listing shown", await until(() => S.cwd === "/sdcard" && FM.status === "ready" && S.shown.length === 6), JSON.stringify(names()));
    check("folders first, natural name order", names().join(",") === "big,Download,Music,a.txt,B.iso,c.zip", names().join(","));
    check("remember_dir called for shown folder", callsOf("remember_dir").some((c) => c.args.path === "/sdcard"));
    check("transfer strip hidden while nothing runs", $("#progress").classList.contains("hidden"));

    // ---- virtualization
    go("/sdcard/big");
    check("big folder listed", await until(() => S.cwd === "/sdcard/big" && S.shown.length === 5000), S.shown.length);
    await sleep(50);
    const domRows = rowEls().length;
    check("virtualized: only visible rows in DOM", domRows > 0 && domRows < 80, `${domRows} rows in DOM for 5000 entries`);
    const list = $("#filelist");
    list.scrollTop = ROW_HEIGHT_PX * 2500;
    list.dispatchEvent(new Event("scroll"));
    await sleep(60);
    check("scrolling renders the rows at that position", rowEls().some((r) => r._entry.name.startsWith("file25")), rowEls().slice(0, 2).map((r) => r._entry.name).join(","));
    check("still few rows after scrolling", rowEls().length < 80, rowEls().length);

    // keyed diff: an identical refresh must not touch the DOM
    const before = new Map(rowEls().map((r) => [r._entry.name, r]));
    let mutations = 0;
    const mo = new MutationObserver((m) => { mutations += m.length; });
    mo.observe($("#list-sizer"), { childList: true, subtree: true, characterData: true, attributes: true });
    const n0 = callsOf("list_dir").length;
    refreshCurrent();
    await until(() => callsOf("list_dir").length > n0);
    await sleep(120);
    mo.disconnect();
    check("identical refresh keeps row elements (keyed)", rowEls().every((r) => before.get(r._entry.name) === r));
    check("identical refresh causes no DOM mutations in rows", mutations === 0, `${mutations} mutations`);

    // ---- cache + back/forward
    goBack();
    check("back to /sdcard is instant from cache (synchronous)", S.cwd === "/sdcard" && S.shown.length === 6 && FM.status === "ready", `${S.cwd} ${S.shown.length}`);
    goForward();
    check("forward to /sdcard/big instant from cache", S.cwd === "/sdcard/big" && S.shown.length === 5000);
    list.scrollTop = ROW_HEIGHT_PX * 1000; list.dispatchEvent(new Event("scroll")); await sleep(40);
    selectOnly("file1005.bin");
    go("/sdcard");
    goBack();
    await sleep(60);
    check("back restores scroll position and selection", Math.abs(list.scrollTop - ROW_HEIGHT_PX * 1000) < 2 && FM.picked.has("file1005.bin") && !!rowByName("file1005.bin"), `${list.scrollTop} ${[...FM.picked]}`);
    goUp();
    check("up selects the folder we came from", await until(() => S.cwd === "/sdcard" && FM.picked.has("big")), [...FM.picked]);

    // ---- race: older slow listing must not overwrite newer navigation
    H.latency.list_dir = (a) => (a.path === "/sdcard/Download" ? 400 : 20);
    clearDirCache();
    go("/sdcard/Download");
    await sleep(LISTING_DEBOUNCE_MS + 20); // its request is in flight now
    go("/sdcard/Music");
    await sleep(600);
    check("latest navigation wins over a slower older listing", S.cwd === "/sdcard/Music" && S.entries.length === 0 && FM.status === "ready", `${S.cwd} entries=${S.entries.map((e) => e.name)}`);
    check("slow listing still filled the cache", DIR_CACHE.has("/sdcard/Download"));
    H.latency.list_dir = undefined;

    // ---- debounce: rapid navigation lists only the final folder
    const n1 = callsOf("list_dir").length;
    go("/sdcard"); go("/sdcard/Download"); go("/sdcard"); go("/sdcard/Music"); go("/sdcard");
    await sleep(200);
    const rapid = callsOf("list_dir").slice(n1).map((c) => c.args.path);
    check("rapid navigation is debounced to one listing", rapid.length === 1 && rapid[0] === "/sdcard", rapid.join(","));

    // ---- selection
    await until(() => FM.status === "ready" && S.cwd === "/sdcard");
    list.focus();
    click("a.txt");
    check("click selects one", [...FM.picked].join() === "a.txt" && S.sel === names().indexOf("a.txt"));
    click("c.zip", { ctrlKey: true });
    check("ctrl+click adds", FM.picked.size === 2 && FM.picked.has("c.zip"));
    click("Download", { shiftKey: true });
    check("shift+click selects range from anchor", [...FM.picked].sort().join() === ["B.iso", "Download", "Music", "a.txt", "c.zip"].sort().join(), [...FM.picked]);
    key("a", { ctrlKey: true });
    check("ctrl+A selects all", FM.picked.size === 6);
    key("Escape");
    check("Escape clears selection", FM.picked.size === 0);
    click("big");
    key("ArrowDown"); key("ArrowDown", { shiftKey: true });
    check("arrow + shift+arrow", [...FM.picked].sort().join() === "Download,Music", [...FM.picked]);
    key("End");
    check("End goes to last", [...FM.picked].join() === "c.zip");
    key("m");
    check("type-ahead jumps to Music", [...FM.picked].join() === "Music");
    const menuHas = (label) => rowMenuItems().some((item) => item !== "-" && item.label === label);
    check("Rename is offered for one selection", menuHas("Rename"));
    click("a.txt", { ctrlKey: true });
    check("Rename is not offered for two, Delete is", !menuHas("Rename") && menuHas("Delete"));

    // ---- rubber band on the empty area below the rows
    key("Escape");
    const lbox = list.getBoundingClientRect();
    const yBelow = lbox.top + 6 * ROW_HEIGHT_PX + 20; // in the bottom padding
    list.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: lbox.left + 200, clientY: yBelow }));
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: lbox.left + 220, clientY: lbox.top + ROW_HEIGHT_PX + 5 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, clientX: lbox.left + 220, clientY: lbox.top + ROW_HEIGHT_PX + 5 }));
    check("rubber band selects covered rows", [...FM.picked].sort().join() === ["Download", "Music", "a.txt", "B.iso", "c.zip"].sort().join(), [...FM.picked]);

    // ---- sorting
    $('#list-head button[data-sort="size"]').click();
    check("sort by size: biggest first, folders first in A-Z", names().join() === "big,Download,Music,B.iso,c.zip,a.txt", names().join());
    $('#list-head button[data-sort="mtime"]').click();
    check("sort by date: newest first", names().slice(3).join() === "c.zip,B.iso,a.txt", names().join());
    $('#list-head button[data-sort="name"]').click();
    check("sort by name again", names().join() === "big,Download,Music,a.txt,B.iso,c.zip", names().join());

    // ---- rename in place, optimistic
    click("a.txt");
    H.latency.rename = 300;
    key("F2");
    const input = editor();
    check("F2 opens inline editor", !!input && input.value === "a.txt");
    check("editor preselects the stem", input && input.selectionStart === 0 && input.selectionEnd === 1, input && `${input.selectionStart}-${input.selectionEnd}`);
    typeInto(input, "renamed.txt");
    check("rename shows immediately (before adb answers)", names().includes("renamed.txt") && !names().includes("a.txt") && FM.picked.has("renamed.txt"));
    await sleep(450);
    check("rename confirmed by adb", H.fs.has("/sdcard/renamed.txt") && names().includes("renamed.txt"));

    H.fail.rename = "mv: Permission denied";
    click("renamed.txt"); key("F2");
    typeInto(editor(), "x.txt");
    check("failing rename shows new name first", names().includes("x.txt"));
    await sleep(450);
    check("failing rename rolls back", names().includes("renamed.txt") && !names().includes("x.txt"), names().join());
    check("error toast shown", [...document.querySelectorAll(".toast.err")].some((t) => t.textContent.includes("Permission denied")));
    H.fail.rename = undefined; H.latency.rename = undefined;

    // case-only rename passes through to Rust (the temp-name logic lives in files::rename)
    click("renamed.txt"); key("F2");
    typeInto(editor(), "Renamed.txt");
    await sleep(100);
    const lastRename = callsOf("rename").pop();
    check("case-only rename is allowed and sent to Rust", lastRename && lastRename.args.to === "/sdcard/Renamed.txt", JSON.stringify(lastRename && lastRename.args));
    const nr = callsOf("rename").length;
    click("Renamed.txt"); key("F2");
    typeInto(editor(), "b.ISO");
    await sleep(50);
    check("name clash (case-insensitive) refused without adb call", callsOf("rename").length === nr);
    const again = editor();
    check("editor reopens to fix the name", !!again);
    if (again) again.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await sleep(20);

    // ---- new folder
    list.focus();
    key("N", { ctrlKey: true, shiftKey: true });
    const nf = editor();
    check("ctrl+shift+N shows a new folder row with editor", !!nf && nf.value === "New folder" && names().includes("New folder"));
    typeInto(nf, "Games");
    check("new folder appears under its name at once", names().includes("Games") && !names().includes("New folder"));
    await until(() => H.fs.has("/sdcard/Games"));
    check("make_dir called once with the final name", callsOf("make_dir").length === 1 && callsOf("make_dir")[0].args.path === "/sdcard/Games");
    key("N", { ctrlKey: true, shiftKey: true });
    editor().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await sleep(30);
    check("Escape on new folder removes the row, no adb call", !names().includes("New folder") && callsOf("make_dir").length === 1);

    // ---- cut + paste inside one storage = move_paths, optimistic
    await sleep(150);
    click("c.zip");
    key("x", { ctrlKey: true });
    check("cut item shown dimmed", rowByName("c.zip").classList.contains("cut"));
    H.latency.move_paths = 300;
    go("/sdcard/Games");
    await until(() => S.cwd === "/sdcard/Games" && FM.status === "ready");
    key("v", { ctrlKey: true });
    check("pasted item appears immediately", names().includes("c.zip"));
    await sleep(400);
    const mv = callsOf("move_paths").pop();
    check("cut+paste same storage uses move_paths (mv)", mv && mv.args.dest === "/sdcard/Games" && mv.args.sources[0] === "/sdcard/c.zip");
    check("no copy job for a same-storage move", callsOf("enqueue").length === 0);
    check("moved on fake Thor", H.fs.has("/sdcard/Games/c.zip") && !H.fs.has("/sdcard/c.zip"));
    goBack();
    check("source folder cache already updated (no c.zip)", !names().includes("c.zip"), names().join());
    H.latency.move_paths = undefined;

    // ---- cut + paste across storages = queued copy with move_after
    await until(() => FM.status === "ready" && S.cwd === "/sdcard");
    click("B.iso");
    key("x", { ctrlKey: true });
    selectPlace("/storage/1A2B-3C4D");
    await until(() => S.cwd === "/storage/1A2B-3C4D" && FM.status === "ready");
    key("v", { ctrlKey: true });
    await until(() => callsOf("enqueue").length > 0); // paste first checks the amount (pool.js: guardedTransfer)
    const enq = callsOf("enqueue").pop();
    const j = enq && enq.args.jobs[0];
    check("cross-storage cut queues a copy job with move_after", j && j.thor_copy === true && j.move_after === true && j.src === "/sdcard/B.iso" && j.dest === "/storage/1A2B-3C4D", JSON.stringify(j));
    const enqCount = callsOf("enqueue").length;
    click("sdfile"); key("c", { ctrlKey: true }); key("v", { ctrlKey: true });
    await until(() => callsOf("enqueue").length > enqCount);
    const j2 = callsOf("enqueue").pop().args.jobs[0];
    check("copy+paste queues thor_copy without move_after", j2.thor_copy === true && j2.move_after === false && j2.dest === "/storage/1A2B-3C4D", JSON.stringify(j2));
    const n2 = callsOf("list_dir").length;
    H.emit("transfer", { busy: false, text: "Done", detail: "", fraction: 1, dests: ["/storage/1A2B-3C4D"], error: null, cancellable: false, cancelled: false });
    await sleep(150);
    check("finished transfer refreshes the open folder", callsOf("list_dir").length > n2 && S.shown.length > 0);

    // ---- delete multiple, one fails → only that one comes back
    selectPlace("/sdcard");
    await until(() => S.cwd === "/sdcard" && FM.status === "ready");
    click("Music"); click("Renamed.txt", { ctrlKey: true });
    H.fail.deleteOne = "/sdcard/Music";
    H.latency.delete_paths = 300;
    key("Delete");
    await until(() => S.modalOpen);
    check("delete asks with item count", $("#modal-box h2").textContent.includes("2 items"), $("#modal-box h2").textContent);
    [...document.querySelectorAll("#modal-box button")].find((b) => b.textContent === "Delete").click();
    await sleep(20);
    check("deleted rows vanish at once", !names().includes("Music") && !names().includes("Renamed.txt"));
    await sleep(400);
    check("failed delete is rolled back, the other stays deleted", names().includes("Music") && !names().includes("Renamed.txt"), names().join());
    H.fail.deleteOne = undefined; H.latency.delete_paths = undefined;

    // ---- context menu
    rowByName("Download").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 100, clientY: 100 }));
    const items = [...document.querySelectorAll(".ctx .ctx-item span")].map((s) => s.textContent);
    check("row context menu has the actions", ["Add to file pool", "Cut", "Copy", "Paste", "Rename", "Delete"].every((x) => items.includes(x)), items.join("|"));
    key("Escape");
    check("Escape closes the menu", !document.querySelector(".ctx"));
    list.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 100, clientY: lbox.top + 7 * ROW_HEIGHT_PX + 10 }));
    const items2 = [...document.querySelectorAll(".ctx .ctx-item span")].map((s) => s.textContent);
    check("background menu", items2.includes("New folder") && items2.includes("Add files …"), items2.join("|"));
    [...document.querySelectorAll(".ctx .ctx-item")].find((b) => b.textContent.startsWith("Add folder")).click();
    check("menu item runs and menu closes", !document.querySelector(".ctx"));

    // ---- drag rows onto a folder (move)
    await sleep(100);
    click("Music");
    const src = rowByName("Music"), dst = rowByName("Download");
    const sb = src.getBoundingClientRect(), db = dst.getBoundingClientRect();
    src.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: sb.left + 60, clientY: sb.top + 10 }));
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: sb.left + 60, clientY: sb.top + 30 }));
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: db.left + 60, clientY: db.top + 10 }));
    check("drag highlights the folder under the mouse", dst.classList.contains("drop") && !$("#drag-ghost").classList.contains("hidden"), $("#drag-ghost").textContent);
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, clientX: db.left + 60, clientY: db.top + 10 }));
    check("drop moves at once", !names().includes("Music"));
    await sleep(100);
    const mv2 = callsOf("move_paths").pop();
    check("drop uses move_paths into the folder", mv2.args.dest === "/sdcard/Download" && mv2.args.sources[0] === "/sdcard/Music", JSON.stringify(mv2.args));

    // ---- dragging rows out of the window does nothing (drag-out was removed in DESIGN-UPDATE-5)
    await sleep(100);
    click("Download");
    const d2 = rowByName("Download").getBoundingClientRect();
    const namesBeforeOut = names().join();
    rowByName("Download").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: d2.left + 60, clientY: d2.top + 10 }));
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: d2.left + 60, clientY: d2.top + 30 }));
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: -5, clientY: 100 }));
    await sleep(60);
    check("leaving the window while dragging rows starts nothing (still an in-window drag)", !!DRAG.items && !callsOf("drag_fetch").length && !callsOf("drag_start").length);
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, clientX: -5, clientY: 100 }));
    await sleep(60);
    check("…and releasing outside does nothing", !DRAG.items && names().join() === namesBeforeOut && $("#drag-ghost").classList.contains("hidden"));

    // ---- keyboard navigation
    list.focus();
    click("Download");
    key("Enter");
    check("Enter opens folder", S.cwd === "/sdcard/Download");
    key("Backspace");
    check("Backspace goes up", S.cwd === "/sdcard");
    key("ArrowLeft", { altKey: true });
    check("Alt+Left goes back", S.cwd === "/sdcard/Download");
    key("ArrowRight", { altKey: true });
    check("Alt+Right goes forward", S.cwd === "/sdcard");
    // ---- shortcuts: F5, and none of them fire while typing in a text field
    list.focus();
    const listsBefore = callsOf("list_dir").length;
    key("F5");
    check("F5 refreshes the open folder", await until(() => callsOf("list_dir").length > listsBefore));
    const fileA = S.shown.find((e) => !e.dir).name;
    click(fileA);
    key("F2");
    const typing = editor();
    const pickedWhileTyping = [...FM.picked].join();
    const clipWhileTyping = FM.clip;
    const pinCallsWhileTyping = callsOf("add_pin").length + callsOf("remove_pin").length;
    for (const [k, opts] of [["Delete", {}], ["a", { ctrlKey: true }], ["Backspace", {}], ["x", { ctrlKey: true }], ["d", { ctrlKey: true }]]) {
      typing.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...opts }));
    }
    await sleep(30);
    check("shortcuts don't fire while typing in an input (incl. Ctrl+D pin)", !!typing && !S.modalOpen && [...FM.picked].join() === pickedWhileTyping && S.cwd === "/sdcard" && FM.clip === clipWhileTyping &&
      callsOf("add_pin").length + callsOf("remove_pin").length === pinCallsWhileTyping && !PINS.list.length,
      `${S.modalOpen} ${[...FM.picked]} ${S.cwd} ${JSON.stringify(FM.clip && FM.clip.mode)}`);
    typing.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await sleep(20);

    // ---- toolbar (DESIGN-UPDATE-5): Back, Forward, Refresh, path bar with pin, New folder, "?"
    check("toolbar: Back, Forward, Refresh (drawn icons), path bar with the pin inside, New folder and ?",
      ["back", "forward", "reload", "crumbs", "pin", "btn-mkdir", "btn-keys"].every((id) => document.getElementById(id)) &&
      ["#back svg", "#forward svg", "#reload svg"].every((q) => $(q)) && $(".pathbar").contains($("#pin")) && $(".pathbar").contains($("#crumbs")) &&
      $("#btn-mkdir").textContent === "New folder");
    check("toolbar: Up, Send files/folders, Copy to PC, Delete, Pool and More are gone",
      ["up", "btn-send-files", "btn-send-dirs", "btn-pull", "btn-delete", "btn-pool", "btn-more", "more-pop", "btn-rename", "btn-esde"].every((id) => !document.getElementById(id)) && !document.querySelector(".actions"));
    const keysPanel = $("#keys-panel");
    check("shortcuts panel hidden at first", keysPanel.classList.contains("hidden"));
    $("#keys").dispatchEvent(new MouseEvent("mouseenter"));
    check("shortcuts panel opens on hover", !keysPanel.classList.contains("hidden") && $("#btn-keys").getAttribute("aria-expanded") === "true");
    $("#keys").dispatchEvent(new MouseEvent("mouseleave"));
    check("shortcuts panel closes when the mouse leaves", keysPanel.classList.contains("hidden"));
    // Headless Firefox doesn't fire focus events for .focus() in an unfocused window, so send them.
    $("#btn-keys").dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    check("shortcuts panel opens on keyboard focus", !keysPanel.classList.contains("hidden"));
    $("#btn-keys").dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: list }));
    check("shortcuts panel closes when focus leaves", keysPanel.classList.contains("hidden"));
    click(fileA);
    $("#btn-keys").click();
    $("#keys").dispatchEvent(new MouseEvent("mouseleave"));
    check("click keeps the panel open", !keysPanel.classList.contains("hidden"));
    $("#btn-keys").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    check("Escape closes the panel and leaves the selection alone", keysPanel.classList.contains("hidden") && FM.picked.has(fileA));
    const listedKeys = [...keysPanel.querySelectorAll("dt")].map((d) => d.textContent);
    check("panel lists only shortcuts the file list handles (each one is tested in this file)",
      listedKeys.join("|") === ["Add files from the PC", "Open", "Up one folder", "Back / forward", "Refresh", "Select all", "Copy", "Cut", "Paste here", "Rename", "Delete", "New folder", "Pin / unpin folder", "Menu for the selection"].join("|"),
      listedKeys.join("|"));

    // ---- transfer strip: real "transfer" events (files::Progress)
    const strip = $("#progress");
    const progress = (p) => H.emit("transfer", { busy: true, text: "", detail: "", fraction: 0, dests: [], error: null, cancellable: false, cancelled: false, index: 0, count: 0, ...p });
    check("a finished batch hides the strip by itself", await until(() => strip.classList.contains("hidden"), 7000));
    progress({ text: "Copying Celeste.nsp", detail: "1.0 GB of 1.6 GB · 40 MB/s · 1 more waiting", fraction: 0.62, cancellable: true, index: 2, count: 3 });
    check("strip shows name, file X of Y, percent, bar and Cancel",
      !strip.classList.contains("hidden") && $("#p-text").textContent === "Copying Celeste.nsp" && $("#p-meta").textContent === "file 2 of 3 · 62%" &&
      $("#p-bar").style.width === "62%" && !$("#p-cancel").classList.contains("hidden") && $("#p-mascot").src.endsWith("thorbro-busy.svg"),
      `${$("#p-meta").textContent} ${$("#p-bar").style.width}`);
    progress({ text: "Extracting x.zip", fraction: 0, cancellable: true, index: 3, count: 3 });
    check("unknown fraction -> animated bar, no percent", $("#p-bar").parentElement.classList.contains("indet") && $("#p-meta").textContent === "file 3 of 3");
    H.emit("transfer", { busy: false, text: "Done: 3 copied to /sdcard", detail: "", fraction: 1, dests: [], error: null, cancellable: false, cancelled: false, index: 0, count: 0 });
    await sleep(20);
    check("finished batch: done state, no Cancel", strip.classList.contains("done") && $("#p-cancel").classList.contains("hidden") && $("#p-mascot").src.endsWith("thorbro-done.svg"));
    hideTransfer();

    // ---- stream tab, "Mirror Thor screens": three quick-start tiles, Stop, failures
    const tile = (which) => document.querySelector(`.stream-btn[data-which="${which}"]`);
    showTab("stream");
    await sleep(50);
    check("mirror: three tiles, nothing running, Stop hidden, waiting mascot",
      document.querySelectorAll(".stream-btn").length === 3 && $("#stream-stop").classList.contains("hidden") &&
      $("#stream-status").textContent.includes("Nothing running") && $("#mirror-state-img").src.endsWith("thorbro-wait.svg"));
    check("mirror: detection cards, Stream both and per-screen options are gone, nothing detected",
      !$("#stream-detect") && !$("#stream-both") && !document.querySelector(".stream-card, [data-opt]") && !callsOf("stream_displays").length);
    tile("top").click();
    await until(() => tile("top").classList.contains("running"));
    check("mirror: Top only starts the top screen, Stop visible, busy mascot",
      callsOf("stream_start").pop()?.args.which === "top" && !$("#stream-stop").classList.contains("hidden") &&
      $("#stream-status").textContent.includes("Top screen") && $("#mirror-state-img").src.endsWith("thorbro-busy.svg"), $("#stream-status").textContent);
    H.stream.failStart[4] = "Display 4 not found";
    // Counts only this failure's toasts (older error toasts may expire meanwhile).
    const displayToasts = () => [...document.querySelectorAll(".toast.err")].filter((t) => t.textContent.includes("Display 4 not found")).length;
    const toastsBefore = displayToasts();
    tile("bottom").click();
    await until(() => !$("#stream-error").classList.contains("hidden"));
    check("mirror: a failed start is shown under the status and as a toast, the other keeps running",
      $("#stream-error").textContent === "Bottom screen: Display 4 not found" && tile("top").classList.contains("running") &&
      displayToasts() === toastsBefore + 1, $("#stream-error").textContent);
    await streamStatus();
    check("mirror: the same failure is not toasted twice", displayToasts() === toastsBefore + 1);
    delete H.stream.failStart[4];
    tile("both").click();
    await until(() => tile("both").classList.contains("running"));
    check("mirror: Both screens starts both, the failure goes away", callsOf("stream_start").pop()?.args.which === "both" && $("#stream-error").classList.contains("hidden"));
    $("#stream-stop").click();
    await until(() => $("#stream-stop").classList.contains("hidden"));
    check("mirror: Stop stops everything", callsOf("stream_stop").length === 1 && !tile("both").classList.contains("running") && $("#stream-status").textContent.includes("Nothing running"));

    // ---- stream tab, sub-tab "Stream PC games" and the Help page
    const pcText = (id) => document.querySelector(`#${id} .pc-body`).textContent;
    check("pc: mirror sub-tab is the default", $("#stream-sub-mirror").classList.contains("active") && !$("#stream-sub-pc").classList.contains("active"));
    showStreamSub("pc");
    check("pc: sub-tab switch shows the page and hides mirroring", $("#stream-sub-pc").classList.contains("active") && !$("#stream-sub-mirror").classList.contains("active"));
    check("pc: detection, Thor apps and login are checked on opening",
      await until(() => callsOf("pc_stream_detect").length && callsOf("pc_stream_thor_apps").length && callsOf("pc_stream_login_status").length && PCS.login));
    await until(() => pcText("pc-sunshine").includes("Sunshine is running"));
    const chips = () => [...document.querySelectorAll("#pc-chips .chip")].map((c) => `${c.className}:${c.textContent}`);
    check("pc: status chips show the real state",
      await until(() => chips().join("|") === "chip ok:Sunshine is running|chip :Tailscale not connected|chip bad:Moonlight not installed on the Thor"), chips().join("|"));
    document.querySelector('#pc-chips [data-help="help-thor"]').click();
    check("pc: a chip opens its Help section, the sub-tabs hide",
      $("#stream-sub-help").classList.contains("active") && $("#help-thor").open && $("#stream-nav").classList.contains("hidden"));
    $("#help-back").click();
    check("pc: Back to Stream returns to Stream PC games", $("#stream-sub-pc").classList.contains("active") && !$("#stream-nav").classList.contains("hidden"));
    check("pc: pairing box closed until Pair Moonlight is clicked", $("#pc-pair").classList.contains("hidden"));
    $("#pc-pair-toggle").click();
    check("pc: Pair Moonlight opens the pairing box", !$("#pc-pair").classList.contains("hidden") && $("#pc-pair-toggle").getAttribute("aria-expanded") === "true");
    check("pc: Sunshine status with host ID", pcText("pc-sunshine").includes("0123ABCD-4567-89EF-0123-456789ABCDEF") && pcText("pc-sunshine").includes("idle"), pcText("pc-sunshine"));
    check("pc: LAN address recommended, docker marked virtual, Tailscale missing is only a note",
      pcText("pc-addresses").includes("192.168.1.20") && pcText("pc-addresses").includes("use this") && pcText("pc-addresses").includes("docker0, virtual") &&
      pcText("pc-addresses").includes("LAN addresses only"), pcText("pc-addresses"));
    check("pc: Moonlight missing shows official links", pcText("pc-thor").includes("not installed") && !!document.querySelector('#pc-thor [data-link="moonlight_play"]'), pcText("pc-thor"));
    document.querySelector('#pc-thor [data-link="moonlight_play"]').click();
    await until(() => callsOf("pc_stream_open_link").length);
    check("pc: link buttons send only an id", callsOf("pc_stream_open_link").pop()?.args.id === "moonlight_play");
    check("pc: pairing and games wait for a login", pcText("pc-pair").includes("Save a working Sunshine login") && pcText("pc-games").includes("save a working Sunshine login"));
    check("pc: password field is a password input", $("#pc-pass").type === "password");

    $("#pc-user").value = "admin"; $("#pc-pass").value = "wrong";
    $("#pc-save-login").click();
    await until(() => callsOf("pc_stream_save_login").length === 1);
    await sleep(80);
    check("pc: wrong login is not accepted", PCS.login.state === "not_stored" && [...document.querySelectorAll(".toast.err")].some((t) => t.textContent.includes("rejected")));
    $("#pc-pass").value = "secret";
    $("#pc-save-login").click();
    check("pc: good login saved, games and pairings load",
      await until(() => PCS.login.state === "ok" && callsOf("pc_stream_apps").length && callsOf("pc_stream_pairings").length && PCS.apps), JSON.stringify(PCS.login));
    check("pc: login card shows user and Sunshine version", pcText("pc-login").includes("logged in as admin") && pcText("pc-login").includes("2026.914.233613"), pcText("pc-login"));
    check("pc: Play disabled while Moonlight is missing", [...document.querySelectorAll("#pc-games [data-play]")].every((b) => b.disabled) && pcText("pc-games").includes("Install Moonlight"));

    H.pc.thor.moonlight = { package: "com.limelight", version: "12.2" };
    await pcThorApps();
    const playButtons = () => [...document.querySelectorAll("#pc-games [data-play]")];
    check("pc: games listed with Play buttons once Moonlight is there", playButtons().length === 2 && playButtons().every((b) => !b.disabled) && pcText("pc-games").includes("Tom's \"Game\""));
    playButtons()[1].click();
    await until(() => callsOf("pc_stream_play").length);
    check("pc: Play sends the exact app name and no display by default",
      callsOf("pc_stream_play").pop()?.args.appName === "Tom's \"Game\"" && callsOf("pc_stream_play").pop()?.args.display === null, JSON.stringify(callsOf("pc_stream_play").pop()));
    // The display picker reuses the mirror tab's scrcpy detection (left empty by the tests above).
    H.stream.displays = [
      { id: 0, width: 1920, height: 1080, label: "Top screen", options: { max_size: 0, bitrate_mbps: 16, always_on_top: false } },
      { id: 4, width: 1240, height: 1080, label: "Bottom screen", options: { max_size: 0, bitrate_mbps: 16, always_on_top: false } },
    ];
    $("#pc-display-detect").click();
    check("pc: display picker filled from the screen detection",
      await until(() => document.querySelectorAll("#pc-display option").length === 3), document.querySelectorAll("#pc-display option").length);
    $("#pc-display").value = "4"; $("#pc-display").dispatchEvent(new Event("change"));
    playButtons()[0].click();
    await until(() => callsOf("pc_stream_play").length === 2);
    check("pc: experimental display id from the screen detection is passed", callsOf("pc_stream_play").pop()?.args.display === 4);
    check("pc: Moonlight dialogs explained in Help", pcText("pc-games-help").includes("PC not paired") && pcText("pc-games-help").includes("Provided App is not valid"));
    H.pc.status.sunshine.server.busy = true;
    await pcDetect();
    check("pc: PC streaming -> note and busy mascot", $("#pc-state-text").textContent.includes("PC is streaming") && $("#pc-state-img").src.endsWith("thorbro-busy.svg"));
    H.pc.status.sunshine.server.busy = false;
    await pcDetect();
    check("pc: nothing streaming -> waiting mascot", $("#pc-state-text").textContent === "Nothing is streaming right now." && $("#pc-state-img").src.endsWith("thorbro-wait.svg"));

    check("pc: no device waiting yet -> Pair disabled",
      pcText("pc-pair").includes("no device waiting") && $("#pc-pair-go").disabled, pcText("pc-pair"));
    check("pc: address advice moved to Help (Addresses of this PC)",
      pcText("pc-addresses").includes("192.168.1.20") && pcText("pc-addresses").includes("mDNS") && !pcText("pc-pair").includes("mDNS"), pcText("pc-addresses"));
    H.pc.pairings = { supported: true, pairings: [{ id: "0123456789abcdef0123456789abcdef", name: "Thor", address: "192.168.1.30" }] };
    pcPoll();
    check("pc: a waiting Moonlight appears via polling", await until(() => !$("#pc-pair-go").disabled && pcText("pc-pair").includes("192.168.1.30")));
    $("#pc-pin").value = "12a4";
    $("#pc-pair-go").click();
    await sleep(30);
    check("pc: invalid PIN rejected in the UI", !callsOf("pc_stream_submit_pin").length);
    $("#pc-pin").value = "0042";
    $("#pc-pair-go").click();
    await until(() => callsOf("pc_stream_submit_pin").length);
    const pinCall = callsOf("pc_stream_submit_pin").pop().args;
    check("pc: PIN sent with pairing id and name", pinCall.pairingId === "0123456789abcdef0123456789abcdef" && pinCall.pin === "0042" && pinCall.name === "Thor", JSON.stringify(pinCall));
    H.pc.pairings = { supported: false, pairings: [] };
    await pcLoadPairings();
    check("pc: old Sunshine -> clear message, PIN sent without pairing id",
      pcText("pc-pair").includes("older than v2026.906.222525") && !$("#pc-pair-go").disabled, pcText("pc-pair"));
    $("#pc-pin").value = "1234";
    $("#pc-pair-go").click();
    await until(() => callsOf("pc_stream_submit_pin").length === 2);
    check("pc: legacy PIN call has pairingId null", callsOf("pc_stream_submit_pin").pop().args.pairingId === null);

    $("#pc-profile-show").click();
    await until(() => !!document.querySelector(".pc-profile-json"));
    check("pc: stream profile shown read-only", document.querySelector(".pc-profile-json").readOnly && document.querySelector(".pc-profile-json").value.includes("thorbro.stream-profile"));
    document.querySelector("#modal .modal-foot button").click();
    await sleep(20);

    // "Send to Thor app": success can only say "sent", never "accepted"; the app reports "rejected" itself.
    const profileStatus = () => $("#pc-profile-status");
    check("pc: send status hidden before sending", profileStatus().classList.contains("hidden"));
    $("#pc-profile-send").click();
    check("pc: Send to Thor app disables the button while sending", $("#pc-profile-send").disabled);
    await until(() => callsOf("pc_stream_send_profile").length === 1 && !$("#pc-profile-send").disabled);
    check("pc: Send to Thor app sends no arguments (profile is built in Rust)",
      Object.keys(callsOf("pc_stream_send_profile").pop().args || {}).length === 0);
    check("pc: send success says sent, not accepted, and points to rejected on the Status tab",
      !profileStatus().classList.contains("hidden") && !profileStatus().classList.contains("warn") &&
      profileStatus().textContent.includes("sent to the ThorBro app (2 games)") && profileStatus().textContent.includes("Status tab") &&
      profileStatus().textContent.includes('"rejected"') && profileStatus().textContent.includes("can't see whether the app accepted it") &&
      !/was accepted|profile accepted/i.test(profileStatus().textContent),
      profileStatus().textContent);
    H.pc.sendProfile = { ...H.pc.sendProfile, games: 0, note: "Without the game list: no Sunshine login is stored" };
    $("#pc-profile-send").click();
    await until(() => callsOf("pc_stream_send_profile").length === 2 && !$("#pc-profile-send").disabled);
    check("pc: send without game list shows the note as warning",
      profileStatus().classList.contains("warn") && profileStatus().textContent.includes("0 games") && profileStatus().textContent.includes("Without the game list"),
      profileStatus().textContent);
    const sendErrorToasts = document.querySelectorAll(".toast.err").length;
    H.fail.pc_stream_send_profile = "The ThorBro app is not installed on the Thor. Install the ThorBro APK on the Thor first (for example with \"adb install <path to the APK>\"), then send the profile again.";
    $("#pc-profile-send").click();
    await until(() => callsOf("pc_stream_send_profile").length === 3 && !$("#pc-profile-send").disabled);
    check("pc: app not installed -> clear message with APK hint, error toast, button usable again",
      profileStatus().classList.contains("warn") && profileStatus().textContent.startsWith("Not sent:") &&
      profileStatus().textContent.includes("not installed") && profileStatus().textContent.includes("APK") &&
      !profileStatus().textContent.includes("sent to the ThorBro app") &&
      document.querySelectorAll(".toast.err").length > sendErrorToasts && !$("#pc-profile-send").disabled,
      profileStatus().textContent);
    H.fail.pc_stream_send_profile = "The stream profile is too large to send over adb (30000 characters encoded, the limit is 24576). It has 1000 games; remove apps you don't stream from Sunshine's app list and send it again.";
    $("#pc-profile-send").click();
    await until(() => callsOf("pc_stream_send_profile").length === 4 && !$("#pc-profile-send").disabled);
    check("pc: profile over the size limit -> readable message", profileStatus().textContent.includes("too large") && profileStatus().textContent.includes("24576"), profileStatus().textContent);
    delete H.fail.pc_stream_send_profile;

    H.pc.status.sunshine.server = null;
    H.pc.status.sunshine.error = "Sunshine is not answering on port 47989";
    H.pc.status.sunshine.service = { state: "not_installed", detail: "not-found" };
    H.pc.status.sunshine.executable = null;
    await pcDetect();
    check("pc: Sunshine missing -> install links, Play blocked",
      pcText("pc-sunshine").includes("Sunshine not found") && !!document.querySelector('#pc-sunshine [data-link="sunshine_getting_started"]') && playButtons().every((b) => b.disabled));
    showStreamSub("mirror");
    check("pc: back to mirroring shows the three tiles", $("#stream-sub-mirror").classList.contains("active") && document.querySelectorAll(".stream-btn").length === 3);
    showTab("files");

    // ---- pinned folders (DESIGN-UPDATE-3): pin/unpin, order, missing folders, cap
    for (const p of ["/sdcard/pinA", "/sdcard/pinB", "/sdcard/pinC"]) H.fs.set(p, { dir: true, size: 0, mtime: 1700000000 });
    const sidePins = () => [...document.querySelectorAll("#side-pins .side-pin")];
    const sidePin = (name) => sidePins().find((r) => r.querySelector(".pin-name").textContent === name);
    const sidePinNames = () => sidePins().map((r) => r.querySelector(".pin-name").textContent).join(",");
    const pinChips = () => [...document.querySelectorAll("#start-pin-chips .pin-chip")];
    const pinBtn = $("#pin");
    const openAt = async (path) => { go(path); return until(() => S.cwd === path && FM.status === "ready"); };
    const ctrlD = async (count) => { list.focus(); key("d", { ctrlKey: true }); return until(() => PINS.list.length === count); };
    const errToasts = () => [...document.querySelectorAll(".toast.err")].map((t) => t.textContent);
    showTab("files");
    check("pins: loaded once at start; none yet -> only the hint in the sidebar, no Start row",
      callsOf("pins").length === 1 && sidePins().length === 0 && !!$(".side-pins .side-hint") && $("#start-pins").classList.contains("hidden"), sidePinNames());
    await openAt("/sdcard/pinA");
    check("pins: pin button shows 'not pinned' (outline icon)",
      pinBtn.getAttribute("aria-label") === "Pin this folder" && pinBtn.getAttribute("aria-pressed") === "false" &&
      pinBtn.querySelector("svg").getAttribute("fill") === "none" && !pinBtn.disabled);
    pinBtn.click();
    await until(() => PINS.list.length === 1);
    check("pins: pin button pins the open folder as storage + path",
      JSON.stringify(callsOf("add_pin").pop()?.args) === JSON.stringify({ storage: "/sdcard", path: "/sdcard/pinA" }), JSON.stringify(callsOf("add_pin").pop()?.args));
    check("pins: pinned state = orange filled icon, label 'Unpin this folder'",
      pinBtn.getAttribute("aria-label") === "Unpin this folder" && pinBtn.getAttribute("aria-pressed") === "true" &&
      pinBtn.classList.contains("on") && pinBtn.querySelector("svg").getAttribute("fill") === "currentColor");
    check("pins: sidebar row (full path as tooltip) and Start chip appear",
      sidePinNames() === "pinA" && sidePin("pinA").querySelector(".side-item").title === "/sdcard/pinA" &&
      !$("#start-pins").classList.contains("hidden") && pinChips().length === 1 && !sidePin("pinA").classList.contains("missing"), sidePinNames());

    await openAt("/sdcard/pinC");
    await ctrlD(2);
    await openAt("/sdcard/pinB");
    await ctrlD(3);
    selectPlace("/storage/1A2B-3C4D");
    await until(() => S.cwd === "/storage/1A2B-3C4D" && FM.status === "ready");
    await ctrlD(4);
    check("pins: Ctrl+D pins the open folder; pin order is kept (not sorted)",
      sidePinNames() === "pinA,pinC,pinB,SD card (1A2B-3C4D)" && pinChips().map((c) => c.textContent).join(",") === "pinA,pinC,pinB,SD card (1A2B-3C4D)", sidePinNames());
    check("pins: stored per storage", H.pins.map((p) => p.storage).join() === "/sdcard,/sdcard,/sdcard,/storage/1A2B-3C4D", JSON.stringify(H.pins));
    await ctrlD(3);
    check("pins: Ctrl+D in a pinned folder unpins it", !pinOf("/storage/1A2B-3C4D") && pinBtn.getAttribute("aria-pressed") === "false" && sidePinNames() === "pinA,pinC,pinB", sidePinNames());

    sidePin("pinC").querySelector(".pin-x").click();
    await until(() => PINS.list.length === 2);
    check("pins: the sidebar 'x' unpins, the others keep their order", sidePinNames() === "pinA,pinB" && H.pins.map((p) => p.path).join() === "/sdcard/pinA,/sdcard/pinB", sidePinNames());
    check("pins: 'x' has an accessible name", sidePin("pinB").querySelector(".pin-x").getAttribute("aria-label") === "Unpin pinB");
    const pinRow = sidePin("pinB");
    const pinBox = pinRow.getBoundingClientRect();
    pinRow.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: pinBox.left + 20, clientY: pinBox.top + 10 }));
    const unpinItem = [...document.querySelectorAll(".ctx .ctx-item")].find((b) => b.textContent.startsWith("Unpin"));
    check("pins: right-click on a pin offers 'Unpin'", !!unpinItem);
    unpinItem && unpinItem.click();
    await until(() => PINS.list.length === 1);
    check("pins: right-click 'Unpin' unpins", sidePinNames() === "pinA" && !document.querySelector(".ctx"), sidePinNames());

    await openAt("/sdcard");
    sidePin("pinA").querySelector(".side-item").click();
    check("pins: clicking a sidebar pin opens the folder", await until(() => S.cwd === "/sdcard/pinA" && S.root === "/sdcard"), S.cwd);
    await openAt("/sdcard");
    showTab("start");
    pinChips()[0].click();
    check("pins: a Start chip opens Files at that folder", S.tab === "files" && $("#page-files").classList.contains("active") && await until(() => S.cwd === "/sdcard/pinA"), `${S.tab} ${S.cwd}`);

    await openAt("/sdcard/pinB");
    await ctrlD(2);
    PINS.list = []; renderPins();
    await loadPins();
    check("pins: reloaded from settings in the same order", sidePinNames() === "pinA,pinB", sidePinNames());

    // A pinned folder that is gone: dimmed with "not found", can't be opened, can be unpinned.
    H.fs.delete("/sdcard/pinB");
    await openAt("/sdcard");
    checkPins();
    // (A background re-list of pinB may start a newer check; wait for whichever lands.)
    await until(() => sidePin("pinB")?.classList.contains("missing"));
    const gone = sidePin("pinB");
    check("pins: missing folder is dimmed with 'not found' (sidebar and Start)",
      gone.classList.contains("missing") && gone.querySelector(".pin-note")?.textContent === "not found" &&
      !sidePin("pinA").classList.contains("missing") && pinChips()[1].classList.contains("missing") && !pinChips()[0].classList.contains("missing"),
      `${sidePinNames()} found=${JSON.stringify([...PINS.found])}`);
    const errsBefore = errToasts().length;
    gone.querySelector(".side-item").click();
    await sleep(30);
    check("pins: clicking a missing pin explains it and stays put", S.cwd === "/sdcard" && errToasts().length > errsBefore && errToasts().pop().includes("not found"), errToasts().pop());
    gone.querySelector(".pin-x").click();
    check("pins: a missing pin can be unpinned", await until(() => sidePinNames() === "pinA"), sidePinNames());
    // A pinned folder that fails to list is checked again and marked.
    await openAt("/sdcard/pinC");
    await ctrlD(2);
    H.fs.delete("/sdcard/pinC");
    go("/sdcard"); await sleep(10);
    clearDirCache();
    refreshListing("/sdcard/pinC");
    check("pins: a failed listing of a pinned folder marks it missing", await until(() => sidePin("pinC")?.classList.contains("missing")));
    await unpin(pinOf("/sdcard/pinC"));
    // Thor not connected: every pin dimmed, pin button off; nothing crashes.
    S.connected = false; renderLibraryViews();
    check("pins: Thor not connected -> pins dimmed with 'not found', pin button disabled",
      sidePins().every((r) => r.classList.contains("missing") && r.querySelector(".pin-note")) && pinChips().every((c) => c.classList.contains("missing")) && pinBtn.disabled);
    S.connected = true; renderLibraryViews();

    // Cap: 12 pins, the 13th gets a short message and is not pinned.
    H.pins = Array.from({ length: 12 }, (_, i) => ({ storage: "/sdcard", path: `/sdcard/cap${i}` }));
    await loadPins();
    await openAt("/sdcard/pinA");
    const addsBefore = callsOf("add_pin").length;
    list.focus(); key("d", { ctrlKey: true });
    await until(() => callsOf("add_pin").length > addsBefore);
    await sleep(60);
    check("pins: capped at 12 with a short message", PINS.list.length === 12 && H.pins.length === 12 && !pinOf("/sdcard/pinA") &&
      pinBtn.getAttribute("aria-pressed") === "false" && errToasts().some((t) => t.includes("up to 12")), errToasts().pop());
    H.pins = [];
    await loadPins();
    check("pins: no pins -> Start row hidden again", $("#start-pins").classList.contains("hidden") && sidePins().length === 0);

    // ======================================================================================
    // File pool, toolbar, history, context menus, drop card, large-transfer confirmation, Cancel, user-select
    // ======================================================================================
    showTab("files");
    const addFs = (path, dir, size = 0) => H.fs.set(path, { dir, size, mtime: 1700000000 });
    const mkDir = (path, files) => { addFs(path, true); for (const [name, size] of files) addFs(`${path}/${name}`, false, size); };
    const modalButton = (label) => [...document.querySelectorAll("#modal-box button")].find((b) => b.textContent === label);
    const modalTitle = () => $("#modal-box h2").textContent;
    const toastTexts = () => [...document.querySelectorAll(".toast")].map((t) => t.textContent);
    const openDir = async (path) => { go(path); await until(() => S.cwd === path && FM.status === "ready"); await refreshListing(path); return true; }; // fresh: the fixtures were just added to the fake Thor
    const mutating = () => callsOf("move_paths").length + callsOf("enqueue").length + callsOf("delete_paths").length + callsOf("rename").length;
    const rows = (...names) => names.map((n) => ({ path: joinPath(S.cwd, n), entry: { ...S.shown.find((e) => e.name === n) } }));
    // Presses on a row, moves to (x, y) and releases there: a drag & drop by mouse.
    const dragRow = async (name, x, y) => {
      const r = rowByName(name), b = r.getBoundingClientRect();
      r.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: b.left + 60, clientY: b.top + 10 }));
      document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: b.left + 60, clientY: b.top + 30 }));
      document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: x, clientY: y }));
      const ghost = $("#drag-ghost").textContent;
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, clientX: x, clientY: y }));
      return ghost;
    };
    const centerOf = (el) => { const b = el.getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2]; };

    mkDir("/sdcard/poolsrc", [["p1.bin", 100], ["p2.bin", 200]]);
    mkDir("/sdcard/poolsrc/pdir", [["q.bin", 50]]);
    mkDir("/sdcard/pooldst", []);
    mkDir("/sdcard/otherdir", [["o.bin", 7]]);
    await openDir("/sdcard/poolsrc");

    // ---- File pool in the sidebar (DESIGN-UPDATE-5): collecting never changes the Thor
    const poolCard = $("#pool");
    const shown = (node) => !node.classList.contains("hidden") && node.getClientRects().length > 0;
    const poolButtons = () => [...$("#pool-foot").querySelectorAll("button")].map((b) => b.textContent).join("|");
    check("pool: always in the sidebar, empty state (dashed border, 'wait' mascot, intro, 'Drop here', no buttons)",
      $("#side").contains(poolCard) && shown(poolCard) && poolCard.classList.contains("is-empty") && getComputedStyle(poolCard).borderTopStyle === "dashed" &&
      $("#pool-mascot").src.endsWith("assets/mascot/thorbro-wait.png") && shown($("#pool-intro")) &&
      $("#pool-intro").textContent === "Collect files and folders from the Thor here, then copy them all to your PC in one go. Drag them in from the list. The Thor stays untouched." &&
      shown($("#pool-drop")) && $("#pool-drop").textContent === "Drop here" && !shown($("#pool-foot")), `${getComputedStyle(poolCard).borderTopStyle} ${$("#pool-mascot").src}`);
    check("pool: no toolbar button, no 340 px panel, no Move here / Copy here", !$("#btn-pool") && !$("#pool-move") && !$("#pool-copy") && !$("aside.pool"));
    check("hint: nothing selected, pool empty -> how to add files (Ctrl+O, right-click)", $("#files-hint").textContent === HINTS.add && HINTS.add.includes("Ctrl+O"), $("#files-hint").textContent);
    const mutBefore = mutating();
    selectOnly("p1.bin");
    check("hint: rows selected -> how to use the file pool", $("#files-hint").textContent === HINTS.pool, $("#files-hint").textContent);
    toggleSelect("pdir");
    addSelectionToPool();
    check("pool: 'Add to file pool' collects the selection and changes nothing on the Thor",
      POOL.items.length === 2 && mutating() === mutBefore && names().includes("p1.bin") && names().includes("pdir"));
    check("pool: filled state (solid border, 'ask' mascot, count + total size, hint, exactly 'Copy to PC…' and 'Clear pool')",
      await until(() => $("#pool-count").textContent === "2 items · 150 B") && poolCard.classList.contains("is-filled") && getComputedStyle(poolCard).borderTopStyle === "solid" &&
      $("#pool-mascot").src.endsWith("assets/mascot/thorbro-ask.png") && !shown($("#pool-intro")) && !shown($("#pool-drop")) && shown($("#pool-foot")) &&
      poolButtons() === "Copy to PC…|Clear pool" && $("#pool-foot").textContent.includes("You pick the folder on your PC next. Nothing on the Thor changes."),
      `${$("#pool-count").textContent} / ${poolButtons()}`);
    check("pool: rows show names (folders with /) and sizes", [...document.querySelectorAll("#pool-items .pool-item")].map((r) => r.children[0].textContent + "=" + r.children[1].textContent).sort().join() === "p1.bin=100 B,pdir/=50 B",
      [...document.querySelectorAll("#pool-items .pool-item")].map((r) => r.textContent).join("|"));
    clearSelection();
    await sleep(30);
    check("list: pooled rows are dimmed with an orange 'in pool' tag, others aren't",
      rowByName("p1.bin").classList.contains("inpool") && rowByName("p1.bin").querySelector(".pool-tag")?.textContent === "in pool" &&
      rowByName("pdir").classList.contains("inpool") && !rowByName("p2.bin").classList.contains("inpool") && !rowByName("p2.bin").querySelector(".pool-tag"));
    check("hint: pool has items -> how to use the file pool", $("#files-hint").textContent === HINTS.pool);
    addToPool(rows("p1.bin"));
    check("pool: the same item twice is ignored", POOL.items.length === 2);
    $("#pool-clear").click();
    await sleep(20);
    check("pool: Clear pool empties it (back to the empty state, tags gone) and touches nothing",
      POOL.items.length === 0 && poolCard.classList.contains("is-empty") && !rowByName("p1.bin").classList.contains("inpool") && mutating() === mutBefore);

    // Drag rows onto the pool card: collects, never moves.
    selectOnly("p2.bin");
    const [px, py] = centerOf($("#pool-drop"));
    const ghostText = await dragRow("p2.bin", px, py);
    check("pool: dropping rows on the pool card only adds (ghost says the Thor stays untouched)",
      POOL.items.map((i) => i.name).join() === "p2.bin" && ghostText.includes("file pool") && ghostText.includes("untouched") && mutating() === mutBefore && names().includes("p2.bin"),
      `${ghostText} / ${POOL.items.map((i) => i.name)}`);
    selectOnly("p1.bin");
    const [qx, qy] = centerOf(poolCard);
    await dragRow("p1.bin", qx, qy);
    check("pool: dropping on the filled card adds too", POOL.items.map((i) => i.name).join() === "p2.bin,p1.bin" && mutating() === mutBefore);
    // Nested paths: an item inside a pooled folder is redundant, a folder swallows pooled children.
    addToPool(rows("pdir"));
    addToPool([{ path: "/sdcard/poolsrc/pdir/q.bin", entry: { name: "q.bin", dir: false, size: 50, mtime: 0 } }]);
    check("pool: items inside a pooled folder are ignored", POOL.items.filter((i) => i.path.includes("q.bin")).length === 0 && POOL.items.length === 3);
    removeFromPool("/sdcard/poolsrc/pdir");
    addToPool([{ path: "/sdcard/poolsrc/pdir/q.bin", entry: { name: "q.bin", dir: false, size: 50, mtime: 0 } }]);
    addToPool(rows("pdir"));
    check("pool: adding a folder replaces its pooled children", POOL.items.map((i) => i.name).sort().join() === "p1.bin,p2.bin,pdir", POOL.items.map((i) => i.name).join());
    document.querySelector("#pool-items .pool-item .x").click();
    check("pool: × removes just that row", POOL.items.length === 2 && mutating() === mutBefore);

    // ---- Copy to PC… (the only way from the Thor to the PC)
    clearPool();
    addToPool(rows("p1.bin", "pdir"));
    await until(() => POOL.items.every((i) => i.sizeState === "known"));
    let pickedOpts = null, picks = 0;
    T.dialog.open = async (o) => { pickedOpts = o; picks++; return null; };
    const enqBefore = callsOf("enqueue").length;
    $("#pool-save").click();
    await until(() => !POOL.busy && picks === 1);
    check("copy to PC: leaving the folder picker without a folder does nothing", callsOf("enqueue").length === enqBefore);
    T.dialog.open = async (o) => { pickedOpts = o; picks++; return "/home/user/saved"; };
    $("#pool-save").click();
    await until(() => callsOf("enqueue").length > enqBefore);
    const saveJobs = callsOf("enqueue").pop().args.jobs;
    check("copy to PC: folder picker (one folder), then pull jobs into it through the queue",
      pickedOpts.directory && !pickedOpts.multiple && saveJobs.length === 2 && saveJobs.every((j) => j.pull && j.dest === "/home/user/saved") && saveJobs[0].src === "/sdcard/poolsrc/p1.bin", JSON.stringify(saveJobs));
    check("copy to PC: the Thor is untouched and the pool stays", POOL.items.length === 2 && names().includes("p1.bin") && !callsOf("move_paths").some((c) => c.t > 0 && c.args.sources.includes("/sdcard/poolsrc/p1.bin")));
    // Large amounts ask first (size, then count); Cancel opens no folder picker.
    clearPool();
    mkDir("/sdcard/hugedir", [["h.bin", 10]]);
    H.bigSize["/sdcard/hugedir"] = 5 * 1024 ** 3;
    await openDir("/sdcard");
    addToPool(rows("hugedir"));
    await until(() => POOL.items.every((i) => i.sizeState === "known"));
    check("pool: shows the big total", $("#pool-count").textContent === "1 item · 5.0 GB", $("#pool-count").textContent);
    const picksBefore = picks, enqBig = callsOf("enqueue").length;
    $("#pool-save").click();
    check("copy to PC: a large pool asks first with count and size", await until(() => S.modalOpen) && modalTitle() === "Copy “hugedir” (5.0 GB) to your PC?", modalTitle());
    check("copy to PC: while asking, the buttons are locked", $("#pool-save").disabled && $("#pool-clear").disabled);
    modalButton("Cancel").click();
    await until(() => !POOL.busy);
    check("copy to PC: Cancel in the question opens no picker and copies nothing", picks === picksBefore && callsOf("enqueue").length === enqBig && POOL.items.length === 1);
    $("#pool-save").click();
    await until(() => S.modalOpen);
    modalButton("Choose folder…").click();
    await until(() => callsOf("enqueue").length > enqBig);
    check("copy to PC: confirming goes on to the picker and queues it", picks === picksBefore + 1 && callsOf("enqueue").pop().args.jobs[0].src === "/sdcard/hugedir");
    clearPool();
    delete H.bigSize["/sdcard/hugedir"];
    // Many small items: the count alone asks.
    mkDir("/sdcard/many", Array.from({ length: CONFIRM_ITEMS + 5 }, (_, i) => [`m${i}.txt`, 1]));
    addFs("/sdcard/many/tgt", true);
    await openDir("/sdcard/many");
    addToPool(S.shown.filter((e) => !e.dir).map((e) => ({ path: joinPath(S.cwd, e.name), entry: e })));
    $("#pool-save").click();
    check("copy to PC: many items ask first", await until(() => S.modalOpen) && modalTitle().includes(`${CONFIRM_ITEMS + 5} items`), modalTitle());
    modalButton("Cancel").click();
    await until(() => !POOL.busy);
    clearPool();
    T.dialog.open = async () => null;

    // ---- Context menus (DESIGN-UPDATE-5)
    await openDir("/sdcard/poolsrc");
    addFs("/sdcard/poolsrc/arch.zip", false, 30);
    await openDir("/sdcard/poolsrc");
    const menuSeq = () => [...document.querySelectorAll(".ctx > *")].map((c) => c.classList.contains("ctx-sep") ? "-" : c.children[0].textContent).join("|");
    const rightClickRow = (name) => { const b = rowByName(name).getBoundingClientRect(); rowByName(name).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: b.left + 100, clientY: b.top + 10 })); };
    const rightClickEmpty = () => { const b = $("#filelist").getBoundingClientRect(); $("#filelist").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: b.left + 100, clientY: b.top + (S.shown.length + 0.5) * ROW_HEIGHT_PX })); };
    FM.clip = null;
    selectOnly("p1.bin"); rightClickRow("p1.bin");
    check("row menu, one file: Add to file pool | Cut, Copy, Paste | Rename, Delete", menuSeq() === "Add to file pool|-|Cut|Copy|Paste|-|Rename|Delete", menuSeq());
    const ctxButton = (label) => [...document.querySelectorAll(".ctx .ctx-item")].find((b) => b.children[0].textContent === label);
    check("row menu: shortcuts shown, Paste disabled with an empty clipboard, Delete in danger color",
      ctxButton("Cut").children[1].textContent === "Ctrl+X" && ctxButton("Copy").children[1].textContent === "Ctrl+C" && ctxButton("Paste").children[1].textContent === "Ctrl+V" &&
      ctxButton("Rename").children[1].textContent === "F2" && ctxButton("Delete").children[1].textContent === "Del" && ctxButton("Paste").disabled && ctxButton("Delete").classList.contains("danger"));
    ctxButton("Add to file pool").click();
    check("row menu: 'Add to file pool' adds the selection", POOL.items.map((i) => i.name).join() === "p1.bin" && !document.querySelector(".ctx"));
    clearPool();
    selectOnly("p1.bin"); toggleSelect("p2.bin"); rightClickRow("p2.bin");
    check("row menu, two items: no Rename", menuSeq() === "Add to file pool|-|Cut|Copy|Paste|-|Delete", menuSeq());
    closeMenu();
    selectOnly("pdir"); rightClickRow("pdir");
    check("row menu, one folder: 'Use as games folder …' (from More) before Rename", menuSeq() === "Add to file pool|-|Cut|Copy|Paste|-|Use as games folder …|-|Rename|Delete", menuSeq());
    closeMenu();
    selectOnly("arch.zip"); rightClickRow("arch.zip");
    check("row menu, one archive: 'Extract here' (from More)", menuSeq() === "Add to file pool|-|Cut|Copy|Paste|-|Extract here|-|Rename|Delete", menuSeq());
    closeMenu();
    rightClickEmpty();
    check("empty space menu, empty clipboard: Add files …, Add folder …, New folder", menuSeq() === "Add files …|Add folder …|New folder", menuSeq());
    check("empty space menu: Ctrl+O shown for Add files", ctxButton("Add files …").children[1].textContent === "Ctrl+O");
    closeMenu();
    selectOnly("p2.bin"); key("c", { ctrlKey: true });
    rightClickEmpty();
    check("empty space menu with something copied: Paste appears", menuSeq() === "Add files …|Add folder …|Paste|New folder", menuSeq());
    closeMenu();
    FM.clip = null;
    S.libs.push({ path: "/sdcard/poolsrc", label: "Test", systems: {}, total: 0, custom: true });
    rightClickEmpty();
    check("empty space menu inside a games folder: 'Import ES-DE info …' (from More)", menuSeq() === "Add files …|Add folder …|New folder|-|Import ES-DE info …", menuSeq());
    closeMenu();
    S.libs.pop();
    // Add files … / Add folder … / Ctrl+O open the PC pickers; the files go into the open folder.
    const openCalls = [];
    T.dialog.open = async (o) => { openCalls.push(o); return openCalls.length === 1 ? ["/home/user/game.iso"] : null; };
    rightClickEmpty();
    ctxButton("Add files …").click();
    check("'Add files …' opens a multi-file picker", await until(() => openCalls.length === 1) && openCalls[0].multiple && !openCalls[0].directory);
    check("…and the picked files go into the open folder (no games folder: asks first)", await until(() => S.modalOpen) && modalTitle() === "No games folder set up");
    const enqAdd = callsOf("enqueue").length;
    modalButton("Copy here").click();
    await until(() => callsOf("enqueue").length > enqAdd);
    const addJob = callsOf("enqueue").pop().args.jobs[0];
    check("…copy job from the PC into the open folder", addJob.src === "/home/user/game.iso" && addJob.dest === "/sdcard/poolsrc" && !addJob.thor_copy && !addJob.pull, JSON.stringify(addJob));
    rightClickEmpty();
    ctxButton("Add folder …").click();
    check("'Add folder …' opens a folder picker", await until(() => openCalls.length === 2) && openCalls[1].directory);
    list.focus();
    key("o", { ctrlKey: true });
    check("Ctrl+O opens the file picker", await until(() => openCalls.length === 3) && !openCalls[2].directory && openCalls[2].multiple);
    T.dialog.open = async () => null;
    selectOnly("p1.bin");
    key("F10", { shiftKey: true });
    check("Shift+F10 opens the menu for the selection", menuSeq().startsWith("Add to file pool"), menuSeq());
    key("Escape");

    // ---- History and path bar (there is no Up button: the segments replace it)
    go("/sdcard/poolsrc/pdir");
    await until(() => S.cwd === "/sdcard/poolsrc/pdir" && FM.status === "ready");
    check("history: Forward is disabled and dimmed with nothing ahead", $("#forward").disabled && +getComputedStyle($("#forward")).opacity < 1 && !$("#back").disabled);
    check("path bar: the open folder is the highlighted last segment", $("#crumbs button.here")?.textContent === "pdir");
    [...$("#crumbs").querySelectorAll("button")].find((b) => b.dataset.path === "/sdcard/poolsrc").click();
    check("path bar: a segment opens that folder", await until(() => S.cwd === "/sdcard/poolsrc"));
    $("#back").click();
    check("history: Back returns", await until(() => S.cwd === "/sdcard/poolsrc/pdir") && !$("#forward").disabled);
    $("#forward").click();
    check("history: Forward goes ahead again", await until(() => S.cwd === "/sdcard/poolsrc") && $("#forward").disabled);

    // ---- Drag from the PC into the window: drop card on the list
    const dd = (payload) => H.dragDrop({ payload });
    await openDir("/sdcard/poolsrc/pdir");
    dd({ type: "enter", paths: ["/home/user/a.iso", "/home/user/b"], position: { x: 10, y: 10 } });
    check("drop card: list dimmed, card with the 'base' mascot, title, target folder and count",
      !$("#drop-card").classList.contains("hidden") && $("#list-wrap").classList.contains("dropping") && $("#drop-card img").src.endsWith("assets/mascot/thorbro-base.png") &&
      $(".drop-card-title").textContent === "Drop to add to the Thor" && $("#drop-card-dest").textContent === "poolsrc / pdir" && $("#drop-card-what").textContent.startsWith(" · 2 items"),
      $(".drop-card-info").textContent);
    check("drop card: total size once known", await until(() => $("#drop-card-what").textContent === " · 2 items · 4.0 KB"), $("#drop-card-what").textContent);
    dd({ type: "over", position: { x: 20, y: 20 } });
    check("drop card stays while dragging over", !$("#drop-card").classList.contains("hidden") && $("#drop-card-what").textContent.includes("2 items"));
    dd({ type: "leave" });
    check("drop card hides on leave, list no longer dimmed", $("#drop-card").classList.contains("hidden") && !$("#list-wrap").classList.contains("dropping"));
    dd({ type: "enter", paths: ["/home/user/c.bin"], position: { x: 10, y: 10 } });
    const enqDrop = callsOf("enqueue").length;
    dd({ type: "drop", paths: ["/home/user/c.bin"], position: { x: 10, y: 10 } });
    check("drop: card hides, files go into the open folder", $("#drop-card").classList.contains("hidden") && await until(() => S.modalOpen) && !!modalButton("Copy here"));
    modalButton("Copy here").click();
    await until(() => callsOf("enqueue").length > enqDrop);
    check("…copy job into the open folder", callsOf("enqueue").pop().args.jobs[0].dest === "/sdcard/poolsrc/pdir");
    await openDir("/sdcard");
    dd({ type: "enter", paths: ["/home/user/c.bin"], position: { x: 10, y: 10 } });
    check("drop card at a storage root names the storage", $("#drop-card-dest").textContent === "Internal storage", $("#drop-card-dest").textContent);
    dd({ type: "leave" });
    showTab("start");
    dd({ type: "enter", paths: ["/home/user/c.bin"], position: { x: 10, y: 10 } });
    check("other tabs keep the full-window overlay, no card", !$("#drop-overlay").classList.contains("hidden") && $("#drop-card").classList.contains("hidden"));
    dd({ type: "leave" });
    showTab("files");

    // ---- Direct drop onto a folder: small = at once, large = asks first
    await openDir("/sdcard/many");
    await sleep(60);
    const dropFiles = S.shown.filter((e) => !e.dir).map((e) => e.name);
    FM.picked = new Set(dropFiles); afterSelection(); // only the files: "tgt" itself can't be a drop target while dragged
    const tgtRect = rowByName("tgt").getBoundingClientRect();
    const [tx, ty] = [tgtRect.left + 60, tgtRect.top + 10];
    const mutDrop = mutating();
    // Press on one of the selected files, drag the whole selection onto "tgt".
    const firstFile = rowByName(dropFiles[0]), fb = firstFile.getBoundingClientRect();
    firstFile.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: fb.left + 60, clientY: fb.top + 10 }));
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: fb.left + 60, clientY: fb.top + 30 }));
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: tx, clientY: ty }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, clientX: tx, clientY: ty }));
    check("direct drop of many items asks first and moves nothing yet", await until(() => S.modalOpen) && modalTitle().includes("items") && mutating() === mutDrop, `${modalTitle()} | cwd ${S.cwd} | mut ${mutating()} vs ${mutDrop} | ${JSON.stringify(callsOf("move_paths").pop())}`);
    modalButton("Cancel").click();
    await sleep(80);
    check("…Cancel: nothing moved, items still listed", mutating() === mutDrop && dropFiles.every((n) => names().includes(n)));
    // A cross-storage move of a big folder asks, and says it can be cancelled / originals stay until verified.
    mkDir("/sdcard/bigmove", [["b.bin", 1]]);
    H.bigSize["/sdcard/bigmove"] = 3 * 1024 ** 3;
    await openDir("/sdcard");
    const started = guardedTransfer("/storage/1A2B-3C4D", rows("bigmove"), true);
    check("big cross-storage move asks and explains Cancel + verification", await until(() => S.modalOpen) && modalTitle().includes("3.0 GB") &&
      $("#modal-box").textContent.includes("only removed after the copies are verified"), $("#modal-box").textContent);
    modalButton("Move").click();
    await started;
    const bigJob = callsOf("enqueue").pop().args.jobs[0];
    check("…confirming queues a verified copy-then-delete job", bigJob.thor_copy && bigJob.move_after && bigJob.dest === "/storage/1A2B-3C4D", JSON.stringify(bigJob));
    delete H.bigSize["/sdcard/bigmove"];
    // Unknown size on a slow transfer counts as large; on an instant move it doesn't.
    H.fail.remote_sizes = "adb hiccup";
    const mutUnknown = mutating();
    const unknownAsk = guardedTransfer("/storage/1A2B-3C4D", rows("Download"), false);
    check("unknown folder size on a copy across storages asks", await until(() => S.modalOpen) && modalTitle().includes("+"), modalTitle());
    modalButton("Cancel").click();
    check("…and Cancel does nothing", (await unknownAsk) === false && mutating() === mutUnknown);
    delete H.fail.remote_sizes;
    // A plain small drop is still immediate (covered by 'drop moves at once' above) and never measured.
    const sizeCallsBefore = callsOf("remote_sizes").length;
    await guardedTransfer("/sdcard/otherdir", rows("Download"), true);
    check("small same-storage move: no confirmation and no size lookup", !S.modalOpen && callsOf("remote_sizes").length === sizeCallsBefore && callsOf("move_paths").pop().args.dest === "/sdcard/otherdir");

    // ---- Cancel in the progress strip
    progress({ text: "Moving big.iso (3.0 GB)", fraction: 0.4, cancellable: true, index: 1, count: 2 });
    check("strip: Cancel visible while a move/copy is cancellable", !$("#p-cancel").classList.contains("hidden"));
    progress({ text: "Removing the original of big.iso", fraction: 0, cancellable: false });
    check("strip: Cancel is hidden while the original is being removed (can't be interrupted)", $("#p-cancel").classList.contains("hidden"));
    progress({ text: "Moving big.iso (3.0 GB)", fraction: 0.4, cancellable: true });
    const cancelsBefore = callsOf("cancel_transfer").length;
    $("#p-cancel").click();
    check("strip: Cancel asks and promises nothing is lost", await until(() => S.modalOpen) && $("#modal-box").textContent.includes("Nothing is lost") && !!modalButton("Keep copying") && !!modalButton("Cancel everything"), $("#modal-box").textContent);
    modalButton("Keep copying").click();
    await sleep(40);
    check("strip: 'Keep copying' cancels nothing", callsOf("cancel_transfer").length === cancelsBefore);
    $("#p-cancel").click();
    await until(() => S.modalOpen);
    modalButton("Cancel this one, copy the rest").click();
    await until(() => callsOf("cancel_transfer").length === cancelsBefore + 1);
    check("strip: 'this one' keeps the queue", callsOf("cancel_transfer").pop().args.clearQueue === false);
    $("#p-cancel").click();
    await until(() => S.modalOpen);
    modalButton("Cancel everything").click();
    await until(() => callsOf("cancel_transfer").length === cancelsBefore + 2);
    check("strip: 'everything' clears the queue", callsOf("cancel_transfer").pop().args.clearQueue === true);
    H.emit("transfer", { busy: false, text: "Stopped: 0 copied to /sdcard/x, 1 cancelled", detail: "", fraction: 1, dests: [], error: null, cancellable: false, cancelled: true, index: 0, count: 0 });
    await sleep(30);
    check("strip: the final message of a cancelled batch says it was stopped", $("#p-text").textContent.startsWith("Stopped") && $("#p-cancel").classList.contains("hidden"));
    hideTransfer();

    // ---- No text selection on rows, tiles, toolbars, panels; text fields keep it
    const sel = (node) => getComputedStyle(node).userSelect;
    const probe = document.createElement("input"); probe.type = "text"; document.body.append(probe);
    const anyTile = el(`<button class="tile">x</button>`); document.body.append(anyTile);
    check("user-select: none on rows, tiles, toolbars, the pool and the strip",
      [rowEls()[0], anyTile, $(".toolbar"), $("#files-hint"), $("#pool"), $("#progress"), $("#list-head"), $("#pin"), $(".drop-card")].every((n) => sel(n) === "none"),
      [rowEls()[0], anyTile, $(".toolbar"), $("#files-hint"), $("#pool"), $("#progress"), $("#list-head"), $("#pin"), $(".drop-card")].map((n) => sel(n) + ":" + (n.className || n.id)).join());
    check("user-select: text fields can still be selected", sel(probe) === "text", sel(probe));
    probe.remove(); anyTile.remove();
    const cssText = await (await fetch("style.css")).text();
    check("CSS also has the -webkit- prefixed form (WebKitGTK)", /-webkit-user-select:\s*none/.test(cssText));
    // Pressing a row and dragging must not start a text selection.
    await openDir("/sdcard");
    const pressRow = rowEls()[0];
    const pb = pressRow.getBoundingClientRect();
    pressRow.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: pb.left + 60, clientY: pb.top + 10 }));
    const duringPress = new Event("selectstart", { bubbles: true, cancelable: true });
    pressRow.children[1].dispatchEvent(duringPress);
    check("selectstart is blocked while a row is pressed / dragged", duringPress.defaultPrevented);
    const inField = document.createElement("input"); inField.type = "text"; document.body.append(inField);
    const fieldSelect = new Event("selectstart", { bubbles: true, cancelable: true });
    inField.dispatchEvent(fieldSelect);
    check("…but not inside a text field", !fieldSelect.defaultPrevented);
    inField.remove();
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    const afterPress = new Event("selectstart", { bubbles: true, cancelable: true });
    pressRow.children[1].dispatchEvent(afterPress);
    check("…and allowed again after the press", !afterPress.defaultPrevented);

    check("no JS errors logged", !(H.logs || []).length, (H.logs || []).join(" | "));
  } catch (e) {
    check("test crashed", false, (e && e.stack) || e);
  }
  return JSON.stringify(results);
})()
