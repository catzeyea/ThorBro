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

  const H = (window.HARNESS = { calls: [], latency: {}, fail: {}, defaultLatency: 30, fs, pins: [], maxPins: 12, bigSize: {} });
  // Answer of the "device" command (adb::Device). Tests flip it to fake plugging the Thor in or out.
  H.device = { connected: true, message: "Thor (fake)" };
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
  // Fake Sunshine / Moonlight side for the "Stream PC games" sub-tab.
  H.pc = {
    status: {
      os: "linux",
      sunshine: {
        executable: "/usr/bin/sunshine", service: { state: "running", detail: "active" }, port: 47989, admin_port: 47990,
        server: { hostname: "my-pc", gamestream_version: "7.1.431.-1", uniqueid: "0123ABCD-4567-89EF-0123-456789ABCDEF", https_port: 47984, current_game: 0, busy: false },
        error: null,
      },
      addresses: {
        lan: [{ address: "192.168.1.20", interface: "eth0", kind: "lan", virtual_adapter: false }, { address: "172.17.0.1", interface: "docker0", kind: "lan", virtual_adapter: true }],
        recommended_lan: "192.168.1.20",
        tailscale: { cli_found: false, backend_state: "", running: false, ipv4: null, dns_name: null, note: "Tailscale CLI not found, showing LAN addresses only." },
        error: null,
      },
    },
    thor: { connected: true, device: "AYN Thor", moonlight: null, tailscale: null, error: null },
    login: { state: "not_stored", username: null, sunshine_version: null, message: "Sunshine has a login." },
    goodLogin: { username: "admin", password: "secret" },
    pairings: { supported: true, pairings: [] },
    pinResult: true,
    apps: ["Desktop", "Tom's \"Game\""],
    // Answer of pc_stream_send_profile (thor::SendProfileResult); failures via H.fail.pc_stream_send_profile.
    sendProfile: { message: "Starting: Intent { cmp=dev.catzeyea.thorbro/.android.MainActivity (has extras) }", games: 2, encoded_chars: 424, note: null },
  };
  const delay = (cmd, args) => {
    const l = H.latency[cmd];
    return typeof l === "function" ? l(args) : (l ?? H.defaultLatency);
  };
  const handlers = {
    device: () => ({ ...H.device }),
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
    // Sizes for the Pool and the large-transfer check (files::remote_sizes). H.bigSize[path] fakes a huge folder.
    remote_sizes: ({ paths }) => paths.map((p) => {
      if (p in H.bigSize) return H.bigSize[p];
      const e = fs.get(p); if (!e) return null;
      if (!e.dir) return e.size;
      let total = 0;
      for (const [k, v] of fs) if (k.startsWith(p + "/") && !v.dir) total += v.size;
      return total;
    }),
    paths_exist: ({ paths }) => paths.map((p) => fs.has(p)),
    // Drop card: total size of the dragged-in PC files (commands::local_sizes). Fake: 2 KiB per path.
    local_sizes: ({ paths }) => paths.length * 2048,
    log: ({ msg }) => { H.logs = (H.logs || []).concat(msg); },
    known_systems: () => [],
    // Pinned folders, like commands::pins / add_pin / remove_pin / pins_found (settings.rs: order kept, cap 12).
    pins: () => H.pins.map((p) => ({ ...p })),
    add_pin: ({ storage, path }) => {
      if (!(path === storage || path.startsWith(storage + "/"))) throw `${path} is not on ${storage}`;
      if (!H.pins.some((p) => p.storage === storage && p.path === path)) {
        if (H.pins.length >= H.maxPins) throw `You can pin up to ${H.maxPins} folders. Unpin one first.`;
        H.pins.push({ storage, path });
      }
      return H.pins.map((p) => ({ ...p }));
    },
    remove_pin: ({ storage, path }) => { H.pins = H.pins.filter((p) => !(p.storage === storage && p.path === path)); return H.pins.map((p) => ({ ...p })); },
    pins_found: ({ paths }) => paths.map((p) => !!fs.get(p)?.dir),
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
    pc_stream_detect: () => JSON.parse(JSON.stringify(H.pc.status)),
    pc_stream_thor_apps: () => JSON.parse(JSON.stringify(H.pc.thor)),
    pc_stream_login_status: () => ({ ...H.pc.login }),
    pc_stream_save_login: ({ username, password }) => {
      if (username !== H.pc.goodLogin.username || password !== H.pc.goodLogin.password) throw "Sunshine rejected the login (wrong username or password).";
      H.pc.login = { state: "ok", username, sunshine_version: "2026.914.233613", message: null };
      return { ...H.pc.login };
    },
    pc_stream_forget_login: () => { H.pc.login = { state: "not_stored", username: null, sunshine_version: null, message: null }; },
    pc_stream_create_login: () => "Login created and saved in the keychain.",
    pc_stream_restart_sunshine: () => null,
    pc_stream_pairings: () => JSON.parse(JSON.stringify(H.pc.pairings)),
    pc_stream_submit_pin: () => H.pc.pinResult,
    pc_stream_apps: () => H.pc.apps.slice(),
    pc_stream_play: () => ({ message: "Starting: Intent { cmp=com.limelight/.ShortcutTrampoline }", pc_busy: false }),
    pc_stream_profile: () => ({ json: '{\n  "format": "thorbro.stream-profile"\n}', note: null }),
    pc_stream_send_profile: () => ({ ...H.pc.sendProfile }),
    pc_stream_open_link: () => null,
    pc_stream_open_web_ui: () => null,
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
    // The drag & drop handler is kept, so tests can send "enter" / "over" / "leave" / "drop" (H.dragDrop).
    webview: { getCurrentWebview: () => ({ onDragDropEvent: (cb) => { H.dragDrop = cb; return Promise.resolve(() => {}); } }) },
    dialog: { open: async () => null },
  };
  H.emit = (name, payload) => (listeners[name] || []).forEach((cb) => cb({ payload }));
})();
