// ThorBro UI – Thor status on the Start page and ThorBro's mood (DESIGN-UPDATE-7).
//
// One reading (→ commands::thor_status → status::read, a single adb call) feeds both the gauges
// on the Start page and the mood, which the Start page AND the header ThorBro show (device.js
// calls renderHeaderMood). Readings are only taken while the Start page is open, the window is
// visible and a Thor is connected; on other pages the header keeps the last mood.
// A value that couldn't be read is null: its column is not shown and it doesn't count for the mood.
"use strict";

// ---------------------------------------------------------------- Tuning (all in one place)

/** How often the Start page asks for a new reading (ms). */
const STATUS_POLL_MS = 4000;
/** The CPU load needs two readings; after the first one, the second comes this much later (ms). */
const CPU_SECOND_SAMPLE_MS = 1000;
/** An older CPU reading is not used for the load (the load would be an average over minutes). */
const CPU_SAMPLE_MAX_AGE_MS = STATUS_POLL_MS * 2.5;

/**
 * Mood thresholds (proposals from DESIGN-UPDATE-7, tune here).
 * Priority, first match wins: asleep (no Thor) → busy → tired → stuffed → fit.
 */
const MOOD_LIMITS = {
  cpuBusyPct: 80,      // "working hard": CPU load at or above this …
  tempHotC: 70,        // … or the hottest CPU zone at or above this (55 °C under load is normal)
  batteryTiredPct: 20, // "tired": battery at or below this and not charging
  storageFullPct: 90,  // "stuffed": internal storage at least this full
};

/** Wording thresholds for the small lines under the gauges (not used for the mood). */
const GAUGE_HINTS = { tempWarmC: 55, cpuSomePct: 40 };

/** Gauge drawing: storage block grid (10 per row) and the chips on the RAM stick. */
const STORAGE_BLOCKS = 50;
const RAM_CHIPS = 6;
const CPU_HISTORY = 10;

/**
 * Charging = `dumpsys battery` status 2 (BATTERY_STATUS_CHARGING). Only then the bolt is drawn,
 * and only then a low battery doesn't make ThorBro tired. "Plugged in" is not enough: the Thor
 * reports status 3 (discharging) while plugged in at its charge limit.
 * UNVERIFIED on a real Thor: status 2 was never seen while testing. Change only this function.
 */
const BATTERY_STATUS_CHARGING = 2;
function isCharging(battery) {
  return !!battery && battery.status === BATTERY_STATUS_CHARGING;
}

/** Mascot and headline per mood. */
const MOODS = {
  asleep:  { img: "assets/mascot/thorbro-wait-off.svg", head: "Zzz…" },
  busy:    { img: "assets/mascot/thorbro-busy.svg", head: "Working hard!" },
  tired:   { img: "assets/mascot/thorbro-tired.svg", head: "I'm tired…" },
  stuffed: { img: "assets/mascot/thorbro-stuffed.svg", head: "I'm stuffed!" },
  fit:     { img: "assets/mascot/thorbro-base.svg", head: "I feel great!" },
};

// ---------------------------------------------------------------- State

const STATUS = {
  data: null,       // last status::ThorStatus, null = none (not connected, not read yet, or failed)
  prevCpu: null,    // { busy, total, at } of the previous reading, for the load
  cpuPct: null,     // CPU load in %, null until two readings exist
  cpuHistory: [],   // last CPU_HISTORY loads, for the sparkline
  busy: false,      // a reading is in flight: the next tick is skipped
  lastError: "",    // logged once per different error
  cpuRetry: null,   // timer for the quick second reading
};

// ---------------------------------------------------------------- Values and mood (pure)

/**
 * The readable values in the form the gauges and the mood use. Missing → null.
 * `data` is a status::ThorStatus, `cpuPct` the load from two readings (or null).
 */
function statusValues(data, cpuPct) {
  if (!data) return { battery: null, internal: null, sd: null, cpuPct: null, ram: null, tempC: null };
  const space = (s) => (s && s.total > 0 ? { total: s.total, free: s.free, usedPct: Math.round(100 * (s.total - s.free) / s.total) } : null);
  const ram = data.ram && data.ram.total > 0
    ? { total: data.ram.total, used: data.ram.total - data.ram.available, pct: Math.round(100 * (data.ram.total - data.ram.available) / data.ram.total) }
    : null;
  return {
    battery: data.battery ? { percent: data.battery.percent, charging: isCharging(data.battery) } : null,
    internal: space(data.internal),
    sd: space(data.sd),
    cpuPct: cpuPct ?? null,
    ram,
    tempC: typeof data.cpu_temp_c === "number" ? data.cpu_temp_c : null,
  };
}

/** The mood for these values: { key, head, line, img }. First match of the priority table wins. */
function moodFor(connected, v) {
  const L = MOOD_LIMITS;
  const mood = (key, line) => ({ key, line, ...MOODS[key] });
  if (!connected) return mood("asleep", "Waiting for my Thor. Plug it in with USB.");
  const cpuBusy = v.cpuPct !== null && v.cpuPct >= L.cpuBusyPct;
  const hot = v.tempC !== null && v.tempC >= L.tempHotC;
  if (cpuBusy) return mood("busy", `My CPU is at ${v.cpuPct} %. Give me a minute.`);
  if (hot) return mood("busy", `I'm at ${Math.round(v.tempC)} °C. Give me a minute to cool down.`);
  if (v.battery && v.battery.percent <= L.batteryTiredPct && !v.battery.charging) {
    return mood("tired", v.battery.percent <= 10
      ? `Only ${v.battery.percent} % battery left. Plug me in now.`
      : `Battery is at ${v.battery.percent} %. Plug me in soon.`);
  }
  if (v.internal && v.internal.usedPct >= L.storageFullPct) {
    return mood("stuffed", v.sd && v.sd.free > v.internal.free
      ? `Internal storage is ${v.internal.usedPct} % full. Move some games to the SD card?`
      : `Internal storage is ${v.internal.usedPct} % full. Clean out some games?`);
  }
  // Fit: only say what is known.
  const parts = [];
  if (v.battery) parts.push(v.battery.charging ? "charging up" : "plenty of battery");
  if (v.internal) parts.push("plenty of room");
  const sentences = [];
  if (parts.length) sentences.push(parts.join(", ") + ".");
  if (v.cpuPct !== null) sentences.push("nothing heavy running.");
  const line = sentences.map((s) => s[0].toUpperCase() + s.slice(1)).join(" ") || "All good here.";
  return mood("fit", line);
}

/**
 * Which gauge columns are shown, in order. With an SD card: battery, internal, SD, CPU, temperature.
 * Without one (or when it can't be read): battery, internal, CPU, RAM, temperature.
 */
function statusColumnKeys(v) {
  const keys = [];
  if (v.battery) keys.push("battery");
  if (v.internal) keys.push("internal");
  if (v.sd) keys.push("sd");
  if (v.cpuPct !== null) keys.push("cpu");
  if (!v.sd && v.ram) keys.push("ram");
  if (v.tempC !== null) keys.push("temp");
  return keys;
}

/** The mood shown right now (Start page and header). */
function currentMood() {
  return moodFor(S.connected, statusValues(STATUS.data, STATUS.cpuPct));
}

// ---------------------------------------------------------------- Reading

/** Takes the CPU load from this reading and the previous one. */
function takeCpuSample(cpu) {
  const now = performance.now();
  const prev = STATUS.prevCpu;
  STATUS.prevCpu = cpu ? { busy: cpu.busy, total: cpu.total, at: now } : null;
  if (!cpu || !prev || now - prev.at > CPU_SAMPLE_MAX_AGE_MS || cpu.total <= prev.total) {
    STATUS.cpuPct = null;
    if (!cpu || !prev) STATUS.cpuHistory = [];
    return;
  }
  const pct = Math.round((100 * (cpu.busy - prev.busy)) / (cpu.total - prev.total));
  STATUS.cpuPct = Math.max(0, Math.min(100, pct));
  STATUS.cpuHistory = STATUS.cpuHistory.concat(STATUS.cpuPct).slice(-CPU_HISTORY);
}

/**
 * One reading, if the Start page is open, the window visible and a Thor connected. Skipped
 * while the previous reading is still running. Called by the timer in main.js, by showTab and
 * when the Thor gets connected.
 */
async function statusTick() {
  if (S.tab !== "start" || !S.connected || document.hidden || STATUS.busy) return;
  STATUS.busy = true;
  clearTimeout(STATUS.cpuRetry);
  try {
    // → commands::thor_status → status::read
    const data = await invoke("thor_status");
    if (!S.connected) return; // unplugged while reading
    STATUS.data = data;
    STATUS.lastError = "";
    takeCpuSample(data.cpu);
    if (data.cpu && STATUS.cpuPct === null) STATUS.cpuRetry = setTimeout(statusTick, CPU_SECOND_SAMPLE_MS);
  } catch (e) {
    // Not readable: nothing is shown and nothing counts for the mood.
    STATUS.data = null;
    STATUS.prevCpu = null;
    STATUS.cpuPct = null;
    STATUS.cpuHistory = [];
    if (String(e) !== STATUS.lastError) { STATUS.lastError = String(e); invoke("log", { msg: "thor_status: " + e }); }
  } finally {
    STATUS.busy = false;
  }
  renderStatus();
}

/** Called by pollDevice when the Thor is plugged in or out. */
function statusConnectionChanged() {
  clearTimeout(STATUS.cpuRetry);
  STATUS.data = null;
  STATUS.prevCpu = null;
  STATUS.cpuPct = null;
  STATUS.cpuHistory = [];
  renderStatus();
  statusTick();
}

// ---------------------------------------------------------------- Drawing

/** Gigabytes as Android's storage screen counts them (1 GB = 10^9 bytes). */
function formatGB(bytes) {
  const tenths = Math.round(bytes / 1e8) / 10; // one decimal …
  return tenths >= 10 ? `${Math.round(tenths)}` : tenths.toFixed(1); // … below 10 GB only
}

function batterySvg(percent, charging) {
  const filled = percent > 0 ? Math.max(1, Math.round(percent / 10)) : 0;
  let cells = "";
  for (let i = 0; i < 10; i++) {
    const x = (14 + i * 15.4).toFixed(1);
    cells += i < filled
      ? `<rect class="g-fill" x="${x}" y="16" width="11.4" height="58" rx="4"/><rect class="g-shine" x="${(+x + 2).toFixed(1)}" y="19" width="7.4" height="4" rx="2"/>`
      : `<rect class="g-empty-dash" x="${x}" y="16" width="11.4" height="58" rx="4"/>`;
  }
  const bolt = charging
    ? `<path class="g-bolt" d="M100 10 L76 50 H93 L84 84 L116 40 H98 L106 10Z"/><path class="g-spark" d="M128 8 l6 -5 M136 18 l8 -2 M131 30 l7 3"/>`
    : "";
  return `<svg width="176" height="88" viewBox="0 0 196 96" aria-hidden="true"><rect class="g-outline" x="3" y="5" width="170" height="82" rx="18"/><path class="g-ink" d="M178 32 h6 a6 6 0 0 1 6 6 v14 a6 6 0 0 1 -6 6 h-6z"/>${cells}${bolt}</svg>`;
}

function blocksHtml(space) {
  const free = Math.round((space.free / space.total) * STORAGE_BLOCKS);
  let html = "";
  for (let i = 0; i < STORAGE_BLOCKS; i++) html += `<span class="blk${i >= STORAGE_BLOCKS - free ? " free" : ""}"></span>`;
  return `<div class="blocks" aria-hidden="true">${html}</div>`;
}

function cpuSvg(history, pct) {
  const points = history.length ? history : [pct];
  const step = points.length > 1 ? 186 / (points.length - 1) : 0;
  const d = points.map((p, i) => `${i ? "L" : "M"}${(2 + i * step).toFixed(1)} ${(54 - p * 0.5).toFixed(1)}`).join(" ")
    + (points.length === 1 ? " h186" : "");
  const hot = pct >= MOOD_LIMITS.cpuBusyPct ? " hot" : "";
  return `<svg width="160" height="60" viewBox="0 0 190 64" aria-hidden="true"><path class="g-base" d="M2 54 H188"/><path class="g-line${hot}" d="${d}"/></svg>`;
}

function ramSvg(pct) {
  const filled = Math.round((pct / 100) * RAM_CHIPS);
  let chips = "";
  for (let i = 0; i < RAM_CHIPS; i++) chips += `<rect class="${i < filled ? "chip on" : "chip"}" x="${18 + i * 30}" y="12" width="22" height="26" rx="4"/>`;
  let pins = "";
  for (let i = 0; i < 22; i++) pins += `<path class="g-pin" d="M${(16 + i * 7.6).toFixed(1)} 52 v8"/>`;
  return `<svg width="160" height="66" viewBox="0 0 200 66" aria-hidden="true"><path class="g-stick" d="M4 8 a6 6 0 0 1 6 -6 H190 a6 6 0 0 1 6 6 V50 H112 a8 6 0 0 1 -16 0 H4Z"/>${chips}${pins}</svg>`;
}

function tempSvg(c) {
  // 20 °C → nearly empty tube, 80 °C → full (so 70 °C, the "hot" limit, is clearly high but not maxed).
  const top = Math.round(84 - (Math.max(20, Math.min(80, c)) - 20) * (74 / 60));
  const hot = c >= MOOD_LIMITS.tempHotC ? " hot" : "";
  let ticks = "";
  for (let i = 0; i < 8; i++) ticks += `<path class="${i % 2 ? "g-tick-s" : "g-tick"}" d="M40 ${16 + i * 10} h${i % 2 ? 7 : 12}"/>`;
  return `<svg width="58" height="104" viewBox="0 0 58 112" aria-hidden="true"><rect class="g-tube" x="12" y="4" width="22" height="84" rx="11"/><circle class="g-bulb${hot}" cx="23" cy="92" r="16"/><rect class="g-merc${hot}" x="18" y="${top}" width="10" height="${94 - top}" rx="5"/>${ticks}</svg>`;
}

/** label, drawing, value (number + unit) and the small line for one column. */
function columnContent(key, v) {
  switch (key) {
    case "battery":
      return { label: "battery", draw: batterySvg(v.battery.percent, v.battery.charging), num: `${v.battery.percent}`, unit: v.battery.charging ? " % · charging" : " %", sub: "" };
    case "internal":
    case "sd": {
      const space = v[key];
      return { label: key === "sd" ? "SD card" : "internal storage", draw: blocksHtml(space), num: `${formatGB(space.free)} GB`, unit: " free", sub: `each block is about ${formatGB(space.total / STORAGE_BLOCKS)} GB` };
    }
    case "cpu": {
      const sub = v.sd && v.ram ? `RAM ${v.ram.pct} %`
        : v.cpuPct >= MOOD_LIMITS.cpuBusyPct ? "working hard" : v.cpuPct >= GAUGE_HINTS.cpuSomePct ? "doing some work" : "nothing heavy running";
      return { label: "cpu", draw: cpuSvg(STATUS.cpuHistory, v.cpuPct), num: `${v.cpuPct}`, unit: " %", sub };
    }
    case "ram":
      return { label: "ram", draw: ramSvg(v.ram.pct), num: `${v.ram.pct}`, unit: " %", sub: `${formatGB(v.ram.used)} of ${formatGB(v.ram.total)} GB used` };
    case "temp": {
      const sub = v.tempC >= MOOD_LIMITS.tempHotC ? "running hot" : v.tempC >= GAUGE_HINTS.tempWarmC ? "warm" : "cool and calm";
      return { label: "temperature", draw: tempSvg(v.tempC), num: `${Math.round(v.tempC)}`, unit: " °C", sub };
    }
  }
  return null;
}

/** Sets an element's text / HTML only when it changed, so nothing flickers. */
function setText(node, text) { if (node.textContent !== text) node.textContent = text; }
function setHtml(node, html) { if (node._html !== html) { node._html = html; node.innerHTML = html; } }

/** Updates the gauge row in place: columns are kept by key, only changed parts are touched. */
function renderGauges(v) {
  const row = $("#gauges");
  const keys = S.connected ? statusColumnKeys(v) : [];
  const existing = new Map([...row.children].map((col) => [col.dataset.key, col]));
  for (const [key, col] of existing) if (!keys.includes(key)) col.remove();
  keys.forEach((key, i) => {
    let col = existing.get(key);
    if (!col) {
      col = el(`<div class="gauge" data-key="${key}"><span class="gauge-label"></span><div class="gauge-draw"></div><span class="gauge-value"><b></b><span class="gauge-unit"></span></span><span class="gauge-sub"></span></div>`);
    }
    if (row.children[i] !== col) row.insertBefore(col, row.children[i] || null);
    const c = columnContent(key, v);
    setText(col.querySelector(".gauge-label"), c.label);
    setHtml(col.querySelector(".gauge-draw"), c.draw);
    setText(col.querySelector(".gauge-value b"), c.num);
    setText(col.querySelector(".gauge-unit"), c.unit);
    setText(col.querySelector(".gauge-sub"), c.sub);
  });
  row.classList.toggle("hidden", !keys.length);
}

/** The header ThorBro shows the same mood as the Start page. Tilted only while asleep. */
function renderHeaderMood() {
  const mood = currentMood();
  const img = $("#device-mascot");
  if (img.getAttribute("src") !== mood.img) img.setAttribute("src", mood.img);
  $("#status").dataset.mood = mood.key;
  $("#status").classList.toggle("asleep", mood.key === "asleep");
}

/** Redraws the status section of the Start page and the header ThorBro. */
function renderStatus() {
  const v = statusValues(STATUS.data, STATUS.cpuPct);
  const mood = moodFor(S.connected, v);
  const img = $("#mood-img");
  if (img.getAttribute("src") !== mood.img) img.setAttribute("src", mood.img);
  $("#thor-status").dataset.mood = mood.key;
  setText($("#mood-head"), mood.head);
  setText($("#mood-line"), mood.line);
  renderGauges(v);
  renderHeaderMood();
}
