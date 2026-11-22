// ThorBro UI – stream tab: mirror the Thor's screens with scrcpy (src/stream.rs).
"use strict";

// ---------------------------------------------------------------- Buttons

document.querySelectorAll(".stream-btn").forEach((button) => (button.onclick = async () => {
  if (!S.connected) return toast("Thor is not connected", true);
  // → commands::stream_start → stream::Streams::start; returns the running screens
  const running = await call("stream_start", { which: button.dataset.which }).catch(() => null);
  if (running) showStreams(running);
  // scrcpy may still fail shortly after starting, so check again a bit later.
  setTimeout(streamStatus, 2500);
}));

$("#stream-stop").onclick = async () => {
  await invoke("stream_stop"); // → commands::stream_stop → stream::Streams::stop
  streamStatus();
};

// ---------------------------------------------------------------- Status

/** Asks Rust which screens are running (→ commands::stream_status) and shows it. */
async function streamStatus() {
  showStreams(await invoke("stream_status").catch(() => []));
}

/** running: list of "top" / "bottom". Highlights the matching buttons. */
function showStreams(running) {
  $("#stream-status").textContent = running.length ? `Running: ${running.map((screen) => screen + " screen").join(", ")}` : "Nothing running right now.";
  $("#stream-stop").classList.toggle("hidden", !running.length);
  document.querySelectorAll(".stream-btn").forEach((button) => {
    const which = button.dataset.which;
    button.classList.toggle("running", which === "both" ? running.length === 2 : running.includes(which));
  });
}
