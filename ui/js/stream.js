// ThorBro UI – stream tab, sub-tab "Mirror Thor screens": mirror the Thor's screens with scrcpy
// (src/stream.rs). Three quick-start tiles with the fixed display ids (0 = top, 4 = bottom) and
// default options; every stream is a separate scrcpy window.
// The scrcpy display detection (`scrcpy --list-displays`) is still here because the experimental
// screen picker of "Stream PC games" uses it (pcstream.js, Help page).
"use strict";

/** State of the mirror sub-tab. */
const STREAM = {
  displays: null,     // [{ id, width, height, label, options }] from stream_displays; null = not detected yet
  detecting: false,
  detectError: "",    // why the last detection failed ("" = it didn't)
  status: { running: [], failures: [] }, // last answer of stream_status (stream::StreamStatus)
  shownFailureSeq: 0, // failures up to this seq were already shown as a toast
};

/** Names of the fixed display ids the tiles use (same ids as stream::Streams::start). */
const SCREEN_NAMES = { 0: "Top screen", 4: "Bottom screen" };

/** After starting, check again at these delays (ms): scrcpy may fail a moment after it started. */
const STREAM_RECHECK_MS = [1500, 4000];

// ---------------------------------------------------------------- Detection (for the screen picker)

/** Asks scrcpy which displays it can mirror (→ commands::stream_displays → stream::list_displays). */
async function detectDisplays() {
  if (!S.connected) return toast("Thor is not connected", true);
  if (STREAM.detecting) return;
  STREAM.detecting = true;
  try {
    STREAM.displays = await invoke("stream_displays");
    STREAM.detectError = "";
  } catch (e) {
    STREAM.displays = [];
    STREAM.detectError = String(e);
  } finally {
    STREAM.detecting = false;
  }
}

/** User-facing name of a display id (from the detection, the fixed names, or a fallback). */
function displayName(id) {
  const display = (STREAM.displays || []).find((d) => d.id === id);
  if (display) return display.label;
  return SCREEN_NAMES[id] || `display ${id}`;
}

// ---------------------------------------------------------------- Start / stop

// The tiles: fixed ids 0 / 4 with default options, no detection needed.
document.querySelectorAll(".stream-btn").forEach((button) => (button.onclick = async () => {
  if (!S.connected) return toast("Thor is not connected", true);
  // → commands::stream_start → stream::Streams::start
  const status = await call("stream_start", { which: button.dataset.which }).catch(() => null);
  if (status) showStreams(status);
  STREAM_RECHECK_MS.forEach((ms) => setTimeout(streamStatus, ms));
}));

$("#stream-stop").onclick = async () => {
  await invoke("stream_stop"); // → commands::stream_stop → stream::Streams::stop
  streamStatus();
};

// ---------------------------------------------------------------- Status

/** Asks Rust what is running (→ commands::stream_status) and shows it. Polled while the tab is open (main.js). */
async function streamStatus() {
  showStreams(await invoke("stream_status").catch(() => STREAM.status));
}

/** status: stream::StreamStatus { running: [id], failures: [{ display, seq, message }] }. */
function showStreams(status) {
  STREAM.status = status;
  const { running, failures } = status;

  $("#stream-status").textContent = running.length ? `Mirroring: ${running.map(displayName).join(", ")}` : "Nothing running right now.";
  $("#stream-stop").classList.toggle("hidden", !running.length);
  $("#mirror-state-img").src = `assets/mascot/thorbro-${running.length ? "busy" : "wait"}.svg`;

  document.querySelectorAll(".stream-btn").forEach((button) => {
    const which = button.dataset.which;
    const on = which === "both" ? running.includes(0) && running.includes(4) : running.includes(which === "top" ? 0 : 4);
    button.classList.toggle("running", on);
  });

  // Failures of screens that aren't running stay visible under the status until the next start or Stop.
  const shown = failures.filter((failure) => !running.includes(failure.display));
  $("#stream-error").textContent = shown.map((failure) => `${displayName(failure.display)}: ${failure.message}`).join("\n");
  $("#stream-error").classList.toggle("hidden", !shown.length);

  // Each failure is shown once as a toast.
  for (const failure of failures) {
    if (failure.seq > STREAM.shownFailureSeq) toast(failure.message, true);
  }
  STREAM.shownFailureSeq = Math.max(STREAM.shownFailureSeq, ...failures.map((f) => f.seq));
}
