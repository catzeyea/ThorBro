// Runs tests.js inside the real ThorBro UI (ui/index.html + ui/js/*) in headless Firefox via
// WebDriver BiDi, with the fake Tauri bridge from stub.js. See README.md in this folder.
// Usage: node tests/ui/run.mjs [--shots]
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";

const here = new URL(".", import.meta.url).pathname;
const repo = resolve(here, "../..");
const shots = process.argv.includes("--shots");
const html = readFileSync(join(repo, "ui/index.html"), "utf8")
  .replace("<head>", `<head><base href="file://${repo}/ui/">`)
  .replace('<script src="js/core.js"></script>', `<script src="file://${here}stub.js"></script><script src="js/core.js"></script>`);
writeFileSync(join(here, "test.html"), html);

const profile = mkdtempSync(join(tmpdir(), "thorbro-ui-test-"));
const port = 9300 + Math.floor(Math.random() * 500);
const ff = spawn("firefox", ["--headless", "--no-remote", "--profile", profile, "--remote-debugging-port", String(port), "--window-size", "1100,800"], { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let ws;
for (let i = 0; i < 100; i++) {
  try {
    ws = new WebSocket(`ws://127.0.0.1:${port}/session`);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
    break;
  } catch { ws = null; await sleep(200); }
}
if (!ws) { ff.kill(); throw new Error("no WebDriver BiDi connection to Firefox"); }
let id = 0;
const pending = new Map();
ws.onmessage = (m) => { const msg = JSON.parse(m.data); if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); } };
const send = (method, params) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });

try {
  await send("session.new", { capabilities: {} });
  const tree = await send("browsingContext.getTree", {});
  const context = tree.result.contexts[0].context;
  await send("browsingContext.navigate", { context, url: `file://${here}test.html`, wait: "complete" });
  await sleep(800);
  const r = await send("script.evaluate", { expression: readFileSync(join(here, "tests.js"), "utf8"), target: { context }, awaitPromise: true });
  if (r.type === "error" || r.result?.type === "exception") { console.log(JSON.stringify(r, null, 1)); process.exitCode = 2; }
  else {
    const results = JSON.parse(r.result.result.value);
    // The header at other window sizes (default 980 x 760 down to the minimum 520 x 480, tauri.conf.json):
    // it must not overflow sideways, stays at about 150 px or less, and the Files list keeps its room.
    const sizes = [[1440, 900, 300], [1180, 700, 200], [980, 760, 250], [1000, 560, 150], [700, 560, 0], [520, 480, 0]];
    await send("script.evaluate", { expression: `(async()=>{HARNESS.device={connected:true,message:"AYN Thor"};await pollDevice(true);showTab("files")})()`, target: { context }, awaitPromise: true });
    for (const [width, height, minList] of sizes) {
      await send("browsingContext.setViewport", { context, viewport: { width, height } });
      await sleep(300);
      const m = await send("script.evaluate", {
        expression: `JSON.stringify((()=>{const h=$(".topbar");const t=$("#device-text");return{h:h.getBoundingClientRect().height,over:h.scrollWidth>h.clientWidth||document.documentElement.scrollWidth>innerWidth,cut:t.scrollWidth>t.clientWidth,list:$("#filelist").clientHeight,w:innerWidth}})())`,
        target: { context }, awaitPromise: false,
      });
      const v = JSON.parse(m.result.result.value);
      const info = `${v.w}x${height}: header ${Math.round(v.h)} px, list ${v.list} px${v.cut ? ", pill cut" : ""}${v.over ? ", overflow" : ""}`;
      results.push({ name: `small windows: header fits at ${width} x ${height}`, ok: v.w === width && !v.over && v.h <= 152 && v.list >= minList && (width < 980 || !v.cut), info });
    }
    await send("browsingContext.setViewport", { context, viewport: null });
    let failed = 0;
    for (const t of results) { if (!t.ok) failed++; console.log(`${t.ok ? "PASS" : "FAIL"}  ${t.name}${t.info && !t.ok ? "  -> " + t.info : t.info ? "  (" + t.info + ")" : ""}`); }
    console.log(`\n${results.length - failed}/${results.length} passed`);
    process.exitCode = failed ? 1 : 0;
    if (shots) {
      // A selection in a big folder with a cut row, then the context menu.
      await send("script.evaluate", { expression: `(async()=>{go("/sdcard/big");await new Promise(r=>setTimeout(r,300));selectOnly("file3.bin");toggleSelect("file5.bin");FM.clip={mode:"cut",items:[],paths:new Set(["/sdcard/big/file7.bin"])};renderRows();$("#filelist").focus();})()`, target: { context }, awaitPromise: true });
      let shot = await send("browsingContext.captureScreenshot", { context });
      writeFileSync(join(here, "shot-list.png"), Buffer.from(shot.result.data, "base64"));
      await send("script.evaluate", { expression: `(async()=>{closeMenu();go("/sdcard");await new Promise(r=>setTimeout(r,200));document.querySelectorAll(".toast").forEach(t=>t.remove());addToPool(S.shown.slice(0,4).map(e=>({path:joinPath(S.cwd,e.name),entry:e})));await new Promise(r=>setTimeout(r,200));})()`, target: { context }, awaitPromise: true });
      shot = await send("browsingContext.captureScreenshot", { context });
      writeFileSync(join(here, "shot-pool.png"), Buffer.from(shot.result.data, "base64"));
      // Files dragged in from the PC: the drop card over the list.
      await send("script.evaluate", { expression: `(async()=>{document.querySelectorAll(".toast").forEach(t=>t.remove());clearPool();HARNESS.dragDrop({payload:{type:"enter",paths:["/a","/b","/c"]}});await new Promise(r=>setTimeout(r,200));})()`, target: { context }, awaitPromise: true });
      shot = await send("browsingContext.captureScreenshot", { context });
      writeFileSync(join(here, "shot-drop.png"), Buffer.from(shot.result.data, "base64"));
      await send("script.evaluate", { expression: `HARNESS.dragDrop({payload:{type:"leave"}})`, target: { context }, awaitPromise: false });
      await send("script.evaluate", { expression: `(()=>{const r=document.querySelector("#filelist .row");selectOnly(r._entry.name);const b=r.getBoundingClientRect();r.dispatchEvent(new MouseEvent("contextmenu",{bubbles:true,cancelable:true,button:2,clientX:b.left+200,clientY:b.top+15}));})()`, target: { context }, awaitPromise: true });
      shot = await send("browsingContext.captureScreenshot", { context });
      writeFileSync(join(here, "shot-menu.png"), Buffer.from(shot.result.data, "base64"));
    }
  }
} finally {
  ws.close();
  ff.kill();
  await sleep(300);
  rmSync(profile, { recursive: true, force: true });
}
