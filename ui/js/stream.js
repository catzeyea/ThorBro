// ThorBro UI – stream tab: mirror the Thor's screens with scrcpy (src/stream.rs).
// The displays come from `scrcpy --list-displays` (→ commands::stream_displays). Each display
// gets a card with its own options and Start/Stop; every stream is a separate scrcpy window.
"use strict";

/** State of the stream tab. */
const STREAM = {
  displays: null,     // [{ id, width, height, label, options }] from stream_displays; null = not detected yet
  detecting: false,
  detectError: "",    // why the last detection failed ("" = it didn't)
  status: { running: [], failures: [] }, // last answer of stream_status (stream::StreamStatus)
  shownFailureSeq: 0, // failures up to this seq were already shown as a toast
};

/** Choices for "Size" (scrcpy --max-size, longest side in pixels; 0 = original size). */
const STREAM_SIZES = [0, 1920, 1600, 1280, 1024, 720];

/** After starting, check again at these delays (ms): scrcpy may fail a moment after it started. */
const STREAM_RECHECK_MS = [1500, 4000];

// ---------------------------------------------------------------- Detection

$("#stream-detect").onclick = () => detectDisplays();

/** Asks scrcpy which displays it can mirror and rebuilds the cards. */
async function detectDisplays() {
  if (!S.connected) return toast("Thor is not connected", true);
  if (STREAM.detecting) return;
  STREAM.detecting = true;
  $("#stream-detect").disabled = true;
  $("#stream-detect").textContent = "Detecting…";
  try {
    STREAM.displays = await invoke("stream_displays"); // → commands::stream_displays → stream::list_displays
    STREAM.detectError = "";
  } catch (e) {
    STREAM.displays = [];
    STREAM.detectError = String(e);
  } finally {
    STREAM.detecting = false;
    $("#stream-detect").disabled = false;
    $("#stream-detect").textContent = "Detect screens";
  }
  renderDisplays();
}

/** User-facing name of a display id (from the detection, or a fallback). */
function displayName(id) {
  const display = (STREAM.displays || []).find((d) => d.id === id);
  return display ? `${display.label} (${id})` : `display ${id}`;
}

/** Builds one card per detected display, plus the hint above the cards. */
function renderDisplays() {
  const displays = STREAM.displays || [];
  const list = $("#stream-displays");
  list.replaceChildren(...displays.map(displayCard));

  let note = "";
  if (!S.connected) note = "Connect the Thor to detect its screens.";
  else if (STREAM.detectError) note = `Couldn't detect the screens: ${STREAM.detectError}\nYou can still try the quick start below.`;
  else if (displays.length === 1)
    note = `scrcpy reports only one display (id ${displays[0].id}). The Thor's second screen isn't visible to scrcpy right now, ` +
      "so only this screen can be mirrored. Check that the bottom screen is switched on, then click Detect screens again. " +
      "The quick start below can still try display 4.";
  $("#stream-note").classList.toggle("hidden", !note);
  $("#stream-note span").textContent = note;

  const both = $("#stream-both");
  both.disabled = displays.length < 2;
  both.textContent = displays.length > 2 ? `Stream all (${displays.length})` : "Stream both";
  showStreams(STREAM.status);
}

/** The card for one display: label, size, options and a Start/Stop button. */
function displayCard(display) {
  const { id, width, height, label, options } = display;
  const sizes = STREAM_SIZES.includes(options.max_size) ? STREAM_SIZES : [...STREAM_SIZES, options.max_size];
  const sizeOptions = sizes
    .map((size) => `<option value="${size}" ${size === options.max_size ? "selected" : ""}>${size ? size + " px" : "Original"}</option>`)
    .join("");
  const resolution = width && height ? ` · ${width}×${height}` : "";
  const card = el(`
    <div class="card stream-card" data-id="${id}">
      <div class="card-top"><span class="big">${id === 0 ? "▣" : "▢"}</span><h2>${esc(label)}</h2></div>
      <div class="dim">Display ${id}${resolution}</div>
      <div class="stream-opts">
        <label>Size <select data-opt="max_size">${sizeOptions}</select></label>
        <label>Bitrate <input type="number" data-opt="bitrate_mbps" min="1" max="200" value="${options.bitrate_mbps}"> Mbit/s</label>
        <label><input type="checkbox" data-opt="always_on_top" ${options.always_on_top ? "checked" : ""}> Always on top</label>
      </div>
      <div class="stream-error hidden"></div>
      <div class="btns one"><button class="stream-toggle">Start</button></div>
    </div>`);
  $(".stream-toggle", card).onclick = () => {
    if (STREAM.status.running.includes(id)) stopDisplay(id);
    else startDisplays([id]);
  };
  return card;
}

/** The options currently entered on the card of display `id` (stream::StreamOptions). */
function cardOptions(id) {
  const card = $(`.stream-card[data-id="${id}"]`);
  return {
    max_size: Number($('[data-opt="max_size"]', card).value),
    bitrate_mbps: Number($('[data-opt="bitrate_mbps"]', card).value),
    always_on_top: $('[data-opt="always_on_top"]', card).checked,
  };
}

// ---------------------------------------------------------------- Start / stop

/** Starts a stream for each id with the options from its card. Rust remembers the options. */
async function startDisplays(ids) {
  if (!S.connected) return toast("Thor is not connected", true);
  const displays = ids.map((id) => ({ id, options: cardOptions(id) }));
  // → commands::stream_start_displays → stream::Streams::start_many
  const status = await call("stream_start_displays", { displays }).catch(() => null);
  if (status) showStreams(status);
  STREAM_RECHECK_MS.forEach((ms) => setTimeout(streamStatus, ms));
}

async function stopDisplay(id) {
  showStreams(await invoke("stream_stop_display", { display: id })); // → stream::Streams::stop_display
}

$("#stream-both").onclick = () => startDisplays((STREAM.displays || []).map((d) => d.id));

$("#stream-stop").onclick = async () => {
  await invoke("stream_stop"); // → commands::stream_stop → stream::Streams::stop
  streamStatus();
};

// Quick start: the fixed ids 0 / 4 with default options, works without the detection.
document.querySelectorAll(".stream-btn").forEach((button) => (button.onclick = async () => {
  if (!S.connected) return toast("Thor is not connected", true);
  // → commands::stream_start → stream::Streams::start
  const status = await call("stream_start", { which: button.dataset.which }).catch(() => null);
  if (status) showStreams(status);
  STREAM_RECHECK_MS.forEach((ms) => setTimeout(streamStatus, ms));
}));

// ---------------------------------------------------------------- Status

/**
 * Asks Rust what is running (→ commands::stream_status) and shows it. Polled every few seconds
 * while the tab is open (main.js). Also detects the displays once per connection.
 */
async function streamStatus() {
  if (!S.connected && STREAM.displays !== null) {
    STREAM.displays = null;
    STREAM.detectError = "";
    renderDisplays();
  } else if (S.connected && STREAM.displays === null && !STREAM.detecting) {
    detectDisplays();
  } else if (!S.connected && STREAM.displays === null) {
    renderDisplays();
  }
  showStreams(await invoke("stream_status").catch(() => STREAM.status));
}

/** status: stream::StreamStatus { running: [id], failures: [{ display, seq, message }] }. */
function showStreams(status) {
  STREAM.status = status;
  const { running, failures } = status;

  $("#stream-status").textContent = running.length ? `Running: ${running.map(displayName).join(", ")}` : "Nothing running right now.";
  $("#stream-stop").classList.toggle("hidden", !running.length);

  document.querySelectorAll(".stream-card").forEach((card) => {
    const id = Number(card.dataset.id);
    const isRunning = running.includes(id);
    const failure = failures.find((f) => f.display === id);
    card.classList.toggle("running", isRunning);
    card.classList.toggle("failed", !!failure && !isRunning);
    $(".stream-error", card).classList.toggle("hidden", !failure);
    $(".stream-error", card).textContent = failure ? failure.message : "";
    const toggle = $(".stream-toggle", card);
    toggle.textContent = isRunning ? "Stop" : "Start";
    toggle.classList.toggle("danger", isRunning);
    // Options apply when a stream starts, so they are locked while it runs.
    card.querySelectorAll("[data-opt]").forEach((input) => (input.disabled = isRunning));
  });

  document.querySelectorAll(".stream-btn").forEach((button) => {
    const which = button.dataset.which;
    const on = which === "both" ? running.includes(0) && running.includes(4) : running.includes(which === "top" ? 0 : 4);
    button.classList.toggle("running", on);
  });

  // Each failure is shown once as a toast; the card keeps showing it until the next start.
  for (const failure of failures) {
    if (failure.seq > STREAM.shownFailureSeq) toast(failure.message, true);
  }
  STREAM.shownFailureSeq = Math.max(STREAM.shownFailureSeq, ...failures.map((f) => f.seq));
}
