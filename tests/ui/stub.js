// Fake Tauri bridge for testing the ThorBro UI in a plain browser (see tests/ui/README.md).
// In-memory Thor file system, artificial latency, failure injection, call log.
(function () {
  const now = 1700000000;
  const fs = new Map(); // path -> { dir, size, mtime }
  const add = (path, dir, size = 0, mtime = now) => fs.set(path, { dir, size, mtime });
  add("/sdcard", true);
  add("/sdcard/Download", true);
  add("/sdcard/Music", true);
  add("/sdcard/a.txt", false, 10, now - 100);
  add("/sdcard/B.iso", false, 5000, now - 50);
  add("/sdcard/c.zip", false, 300, now - 10);
  add("/sdcard/Download/inner.txt", false, 1);
  add("/sdcard/big", true);
  for (let i = 0; i < 5000; i++) add(`/sdcard/big/file${i}.bin`, false, i, now + i);
  add("/storage/1A2B-3C4D", true);
  add("/storage/1A2B-3C4D/sdfile", false, 7);

  const children = (dir) => [...fs.entries()]
    .filter(([p]) => p !== dir && p.startsWith(dir + "/") && !p.slice(dir.length + 1).includes("/"))
    .map(([p, e]) => ({ name: p.slice(dir.length + 1), dir: e.dir, size: e.size, mtime: e.mtime }));

  const H = (window.HARNESS = { calls: [], latency: {}, fail: {}, defaultLatency: 30, fs });
  // Fake scrcpy: detected displays, running display ids, failures; failStart[id] = error text.
  const opts = () => ({ max_size: 0, bitrate_mbps: 16, always_on_top: false });
  H.stream = {
    displays: [
      { id: 0, width: 1920, height: 1080, label: "Top screen", options: opts() },
      { id: 4, width: 1240, height: 1080, label: "Bottom screen", options: opts() },
    ],
    running: [], failures: [], seq: 0, failStart: {},
  };
  const streamStatus = () => ({ running: [...H.stream.running].sort((a, b) => a - b), failures: H.stream.failures.slice() });
  const streamStartOne = (id) => {
    H.stream.failures = H.stream.failures.filter((f) => f.display !== id);
    if (H.stream.failStart[id]) H.stream.failures.push({ display: id, seq: ++H.stream.seq, message: H.stream.failStart[id] });
    else if (!H.stream.running.includes(id)) H.stream.running.push(id);
  };
  const delay = (cmd, args) => {
    const l = H.latency[cmd];
    return typeof l === "function" ? l(args) : (l ?? H.defaultLatency);
  };
  const handlers = {
    device: () => ({ connected: true, message: "Thor (fake)" }),
    places: () => ({
      places: [{ label: "Internal storage", path: "/sdcard", games: false }, { label: "SD card (1A2B-3C4D)", path: "/storage/1A2B-3C4D", games: false }],
      libs: [], start: "/sdcard",
    }),
    refresh_libs: () => [],
    list_dir: ({ path }) => { if (!fs.get(path)?.dir) throw "No such folder"; return children(path); },
    remember_dir: () => null,
    rename: ({ from, to }) => {
      const e = fs.get(from); if (!e) throw "gone";
      for (const [p, v] of [...fs]) if (p === from || p.startsWith(from + "/")) { fs.delete(p); fs.set(to + p.slice(from.length), v); }
    },
    make_dir: ({ path }) => add(path, true),
    move_paths: ({ sources, dest }) => sources.map((src) => {
      const name = src.split("/").pop(); const to = dest + "/" + name;
      if (fs.has(to)) return { path: src, error: `${name}: already exists there` };
      for (const [p, v] of [...fs]) if (p === src || p.startsWith(src + "/")) { fs.delete(p); fs.set(to + p.slice(src.length), v); }
      return { path: src, error: null };
    }),
    delete_paths: ({ paths }) => paths.map((p) => {
      if (H.fail.deleteOne === p) return { path: p, error: `${p.split("/").pop()}: Permission denied` };
      for (const k of [...fs.keys()]) if (k === p || k.startsWith(p + "/")) fs.delete(k);
      return { path: p, error: null };
    }),
    enqueue: () => null,
    log: ({ msg }) => { H.logs = (H.logs || []).concat(msg); },
    known_systems: () => [],
    classify: () => [],
    cancel_transfer: () => false,
    games: () => ({}),
    stream_displays: () => JSON.parse(JSON.stringify(H.stream.displays)),
    stream_start_displays: ({ displays }) => { displays.forEach((d) => streamStartOne(d.id)); return streamStatus(); },
    stream_start: ({ which }) => { (which === "top" ? [0] : which === "bottom" ? [4] : [0, 4]).forEach(streamStartOne); return streamStatus(); },
    stream_stop_display: ({ display }) => {
      H.stream.running = H.stream.running.filter((id) => id !== display);
      H.stream.failures = H.stream.failures.filter((f) => f.display !== display);
      return streamStatus();
    },
    stream_stop: () => { H.stream.running = []; H.stream.failures = []; },
    stream_status: () => streamStatus(),
  };
  async function invoke(cmd, args = {}) {
    H.calls.push({ cmd, args: JSON.parse(JSON.stringify(args)), t: performance.now() });
    await new Promise((r) => setTimeout(r, delay(cmd, args)));
    if (H.fail[cmd]) throw H.fail[cmd];
    if (!handlers[cmd]) throw `unknown command ${cmd}`;
    return handlers[cmd](args);
  }
  const listeners = {};
  window.__TAURI__ = {
    core: { invoke },
    event: { listen: (name, cb) => { (listeners[name] ||= []).push(cb); return Promise.resolve(() => {}); } },
    webview: { getCurrentWebview: () => ({ onDragDropEvent: () => Promise.resolve(() => {}) }) },
    dialog: { open: async () => null },
  };
  H.emit = (name, payload) => (listeners[name] || []).forEach((cb) => cb({ payload }));
})();
