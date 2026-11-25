// ThorBro UI – the game catalog (Update 9): consoles, games with covers, installed emulators and
// their saves, all read from the real Thor (→ commands::game_catalog → catalog::build).
// The Start page (start.js) draws it, the "Add a game save" dialog (saves-add.js) searches it.
// Covers are iiSU's icons, copied into the PC cache on demand (→ commands::fetch_art) and shown
// through the thorart:// protocol (art.rs).
"use strict";

/** Catalog state. `data` is the last catalog (null before the first one or without a Thor). */
const CAT = {
  data: null,
  loading: null,   // the running load (Promise) or null
  again: false,    // another load was asked for while one was running
  error: "",
  profiles: null,  // answer of emulator_profiles (no adb, loaded once and on demand)
  cached: new Set(), // cover keys that are in the PC cache
};

/** Covers fetched per fetch_art call (each is one adb `cat`). */
const COVER_BATCH = 6;

/** Placeholder colors (from the boards), picked by the title. */
const COVER_COLORS = ["#A5B98C", "#F0A94B", "#FF9C84", "#d9cfa9", "#8fa7a0", "#c9a07a"];

/**
 * Loads the catalog (one at a time; a request during a load runs once more afterwards) and
 * redraws Start. Without a Thor the catalog is dropped.
 */
function loadCatalog() {
  if (!S.connected) {
    CAT.data = null;
    CAT.error = "";
    return Promise.resolve(null);
  }
  if (CAT.loading) {
    CAT.again = true;
    return CAT.loading;
  }
  CAT.loading = (async () => {
    try {
      // → commands::game_catalog → catalog::build (libraries, profiles, saves, iiSU art)
      const data = await call("game_catalog", {}, true);
      for (const block of data.consoles) for (const game of block.games) if (game.art && game.art.cached) CAT.cached.add(game.art.key);
      CAT.data = data;
      CAT.error = "";
    } catch (e) {
      CAT.error = String(e);
    }
    if (S.tab === "start") renderStart();
    fetchCovers();
    return CAT.data;
  })().finally(() => {
    CAT.loading = null;
    if (CAT.again) { CAT.again = false; loadCatalog(); }
  });
  return CAT.loading;
}

/** The emulator profiles (cached). `force` reloads them (after the user added a profile). */
async function loadProfiles(force = false) {
  if (!CAT.profiles || force) {
    // → commands::emulator_profiles (built-in + ~/.config/thorBro/profiles, no adb)
    CAT.profiles = await call("emulator_profiles", {}, true).catch(() => ({ profiles: [], errors: [], user_dir: "" }));
  }
  return CAT.profiles;
}

/** Profile summary by id (from loadProfiles), or null. */
function profileById(id) {
  return ((CAT.profiles || {}).profiles || []).find((profile) => profile.id === id) || null;
}

/** Display name of an emulator id (falls back to the id). */
function emulatorName(id) {
  const fromCatalog = CAT.data && CAT.data.emulators.find((emulator) => emulator.id === id);
  return (fromCatalog || profileById(id) || { name: id }).name;
}

/** URL of a cached cover (thorart://localhost/<key>, http://thorart.localhost/<key> on Windows). */
function coverUrl(key) {
  return T.core.convertFileSrc ? T.core.convertFileSrc(key, "thorart") : `thorart://localhost/${encodeURIComponent(key)}`;
}

/** Copies the missing covers of the catalog into the PC cache, a few at a time, and shows each one when it arrives. */
async function fetchCovers() {
  const data = CAT.data;
  if (!data) return;
  const wanted = new Map();
  for (const block of data.consoles) {
    for (const game of block.games) if (game.art && !CAT.cached.has(game.art.key)) wanted.set(game.art.key, game.art);
  }
  const items = [...wanted.values()];
  for (let i = 0; i < items.length; i += COVER_BATCH) {
    if (CAT.data !== data || !S.connected) return; // a newer catalog takes over
    const batch = items.slice(i, i + COVER_BATCH).map((art) => ({ path: art.path, key: art.key }));
    // → commands::fetch_art → art::fetch (adb cat into ~/.cache/thorBro/art)
    const done = await call("fetch_art", { items: batch }, true).catch(() => batch.map(() => false));
    batch.forEach((item, index) => {
      if (!done[index]) return;
      CAT.cached.add(item.key);
      document.querySelectorAll(`.cover[data-art="${item.key}"]`).forEach(showCoverImage);
    });
  }
}

/** Replaces the initials placeholder of a cover element with its cached image. */
function showCoverImage(cover) {
  const key = cover.dataset.art;
  if (!key || !CAT.cached.has(key) || cover.querySelector("img")) return;
  const img = document.createElement("img");
  img.alt = "";
  img.loading = "lazy";
  img.decoding = "async";
  // A broken image keeps the placeholder (e.g. the cache was cleared meanwhile).
  img.onerror = () => { img.remove(); cover.classList.remove("has-img"); };
  img.onload = () => cover.classList.add("has-img");
  img.src = coverUrl(key);
  cover.append(img);
}

/** Initials for the placeholder: first letters of the first two words ("The Legend …" → "TL"). */
function coverInitials(title) {
  const words = String(title).split(/[\s:._-]+/).filter((word) => /[A-Za-z0-9]/.test(word));
  return words.slice(0, 2).map((word) => word.match(/[A-Za-z0-9]/)[0].toUpperCase()).join("") || "?";
}

/** Placeholder color for a title (stable: same title, same color). */
function coverColor(title) {
  let hash = 0;
  for (const char of String(title)) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return COVER_COLORS[hash % COVER_COLORS.length];
}

/** A square cover: the cached image, or initials on a color until it arrives (or if there is none). */
function coverElement(title, art) {
  const cover = el(`<div class="cover" aria-hidden="true"><span class="cover-initials"></span></div>`);
  cover.querySelector(".cover-initials").textContent = coverInitials(title);
  cover.style.setProperty("--cover-color", coverColor(title));
  if (art) {
    cover.dataset.art = art.key;
    showCoverImage(cover);
  }
  return cover;
}

/** "today 08:30", "yesterday 18:11" or "2026-11-17 18:11" for Unix seconds (0 = "—"). */
function friendlyWhen(seconds) {
  if (!seconds) return "—";
  const date = new Date(seconds * 1000);
  const pad = (n) => String(n).padStart(2, "0");
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  const day = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  const today = new Date();
  const yesterday = new Date(today.getTime() - 86400000);
  if (day(date) === day(today)) return `today ${time}`;
  if (day(date) === day(yesterday)) return `yesterday ${time}`;
  return `${day(date)} ${time}`;
}
