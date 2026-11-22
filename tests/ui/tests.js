// UI tests for the file manager and the stream tab, evaluated inside the page (real index.html + scripts, fake Tauri
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

    // ---- virtualization
    go("/sdcard/big");
    check("big folder listed", await until(() => S.cwd === "/sdcard/big" && S.shown.length === 5000), S.shown.length);
    await sleep(50);
    const domRows = rowEls().length;
    check("virtualized: only visible rows in DOM", domRows > 0 && domRows < 80, `${domRows} rows in DOM for 5000 entries`);
    const list = $("#filelist");
    list.scrollTop = 30 * 2500;
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
    list.scrollTop = 30 * 1000; list.dispatchEvent(new Event("scroll")); await sleep(40);
    selectOnly("file1005.bin");
    go("/sdcard");
    goBack();
    await sleep(60);
    check("back restores scroll position and selection", Math.abs(list.scrollTop - 30000) < 2 && FM.picked.has("file1005.bin") && !!rowByName("file1005.bin"), `${list.scrollTop} ${[...FM.picked]}`);
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
    check("rename enabled for one selection", !$("#btn-rename").disabled);
    click("a.txt", { ctrlKey: true });
    check("rename disabled for two", $("#btn-rename").disabled && !$("#btn-delete").disabled);

    // ---- rubber band on the empty area below the rows
    key("Escape");
    const lbox = list.getBoundingClientRect();
    const yBelow = lbox.top + 6 * 30 + 20; // in the bottom padding
    list.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: lbox.left + 200, clientY: yBelow }));
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: lbox.left + 220, clientY: lbox.top + 35 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, clientX: lbox.left + 220, clientY: lbox.top + 35 }));
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
    const enq = callsOf("enqueue").pop();
    const j = enq && enq.args.jobs[0];
    check("cross-storage cut queues a copy job with move_after", j && j.thor_copy === true && j.move_after === true && j.src === "/sdcard/B.iso" && j.dest === "/storage/1A2B-3C4D", JSON.stringify(j));
    click("sdfile"); key("c", { ctrlKey: true }); key("v", { ctrlKey: true });
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
    check("row context menu has the actions", ["Open", "Cut", "Copy", "Paste into folder", "Rename …", "Delete"].every((x) => items.includes(x)), items.join("|"));
    key("Escape");
    check("Escape closes the menu", !document.querySelector(".ctx"));
    list.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, clientX: 100, clientY: lbox.top + 7 * 30 + 10 }));
    const items2 = [...document.querySelectorAll(".ctx .ctx-item span")].map((s) => s.textContent);
    check("background menu", items2.includes("New folder") && items2.includes("Refresh"), items2.join("|"));
    [...document.querySelectorAll(".ctx .ctx-item")].find((b) => b.textContent.startsWith("Refresh")).click();
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

    // ---- drag out of the window hands over to drag_fetch (then drag_start)
    await sleep(100);
    click("Download");
    const d2 = rowByName("Download").getBoundingClientRect();
    rowByName("Download").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: d2.left + 60, clientY: d2.top + 10 }));
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: d2.left + 60, clientY: d2.top + 30 }));
    const nf3 = callsOf("drag_fetch").length;
    document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, button: 0, clientX: -5, clientY: 100 }));
    await sleep(100);
    check("leaving the window starts the drag-out fetch", callsOf("drag_fetch").length === nf3 + 1 && callsOf("drag_fetch").pop().args.path === "/sdcard/Download");
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));

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
    // ---- stream tab: detected displays, per-display start/stop, "Stream both", failures
    const cards = () => [...document.querySelectorAll(".stream-card")];
    const card = (id) => document.querySelector(`.stream-card[data-id="${id}"]`);
    showTab("stream");
    check("stream: displays detected on opening the tab", await until(() => cards().length === 2), cards().length);
    check("stream: labels and sizes shown", card(4).textContent.includes("Bottom screen") && card(4).textContent.includes("1240×1080"), card(4).textContent);
    check("stream: Stream both enabled, no hint", !$("#stream-both").disabled && $("#stream-note").classList.contains("hidden"));
    card(4).querySelector('[data-opt="bitrate_mbps"]').value = "8";
    card(4).querySelector('[data-opt="always_on_top"]').checked = true;
    $("#stream-both").click();
    await until(() => card(0).classList.contains("running") && card(4).classList.contains("running"));
    const started = callsOf("stream_start_displays").pop();
    check("stream: Stream both starts both displays with their card options",
      started && started.args.displays.map((d) => d.id).join() === "0,4" &&
      started.args.displays[1].options.bitrate_mbps === 8 && started.args.displays[1].options.always_on_top === true &&
      started.args.displays[0].options.bitrate_mbps === 16, JSON.stringify(started && started.args));
    check("stream: both cards running, options locked, Stop all visible",
      card(0).classList.contains("running") && card(4).querySelector("[data-opt]").disabled && !$("#stream-stop").classList.contains("hidden"));
    card(4).querySelector(".stream-toggle").click();
    await until(() => !card(4).classList.contains("running"));
    check("stream: Stop on one card stops only that display",
      callsOf("stream_stop_display").pop()?.args.display === 4 && card(0).classList.contains("running") && !card(4).classList.contains("running"));
    H.stream.failStart[4] = "Display 4 not found";
    const toastsBefore = document.querySelectorAll(".toast.err").length;
    card(4).querySelector(".stream-toggle").click();
    await until(() => card(4).classList.contains("failed"));
    check("stream: a failed start is shown on its card and as a toast, the other keeps running",
      card(4).querySelector(".stream-error").textContent === "Display 4 not found" && card(0).classList.contains("running") &&
      document.querySelectorAll(".toast.err").length === toastsBefore + 1);
    await streamStatus();
    check("stream: the same failure is not toasted twice", document.querySelectorAll(".toast.err").length === toastsBefore + 1);
    delete H.stream.failStart[4];
    $("#stream-stop").click();
    await until(() => !card(0).classList.contains("running"));
    check("stream: Stop all stops everything and clears the failure",
      callsOf("stream_stop").length === 1 && !card(4).classList.contains("failed") && $("#stream-stop").classList.contains("hidden"));
    H.stream.displays = H.stream.displays.slice(0, 1);
    $("#stream-detect").click();
    await until(() => cards().length === 1);
    check("stream: only one display -> clear hint, Stream both disabled",
      !$("#stream-note").classList.contains("hidden") && $("#stream-note").textContent.includes("only one display") && $("#stream-both").disabled,
      $("#stream-note").textContent);
    H.fail.stream_displays = "scrcpy not found – please install scrcpy (see README)";
    $("#stream-detect").click();
    await until(() => $("#stream-note").textContent.includes("Couldn't detect"));
    check("stream: detection error shown in the hint", $("#stream-note").textContent.includes("scrcpy not found") && cards().length === 0);
    delete H.fail.stream_displays;
    document.querySelector('.stream-btn[data-which="top"]').click();
    await until(() => document.querySelector('.stream-btn[data-which="top"]').classList.contains("running"));
    check("stream: quick start still works", callsOf("stream_start").pop()?.args.which === "top");
    $("#stream-stop").click();
    showTab("files");

    check("no JS errors logged", !(H.logs || []).length, (H.logs || []).join(" | "));
  } catch (e) {
    check("test crashed", false, (e && e.stack) || e);
  }
  return JSON.stringify(results);
})()
