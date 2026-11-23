# ThorBro PC: design update 2 (Stream cleanup, Files, mascot)

Builds on docs/design/DESIGN.md (same tokens, same one-font rule). Change styling and structure only.
Do NOT change Tauri commands, scrcpy/Sunshine/Moonlight logic or data flow. Everything that exists today must stay reachable.

Reference files (not runnable, custom tags): reference/stream.dc.html, mirror.dc.html, help.dc.html, files.dc.html.
Mascot images: mascot/thorbro-{base,ask,wait,done,busy}.{svg,png} (transparent background, made for the dark UI).

## 1. Stream tab: two sub-views, minimal
Sub-tabs: "Stream PC games" and "Mirror Thor screens", plus a round "?" + "Help" button on the right.

Stream PC games shows ONLY:
- status chips: Sunshine running, Tailscale connected, Moonlight installed on the Thor (real state, as today)
- card "Play on Thor": one row per Sunshine app with an orange "Play on Thor" button (existing behavior)
- card "Thor app": "Send to Thor app" (sage button) and a small "Pair Moonlight" button (opens the existing pairing UI: PIN, device name, Pair)
- mascot "wait" with "Nothing is streaming right now." when nothing streams; when something streams show a short
  "PC is streaming" note instead (as today) and mascot "busy".
Mirror Thor screens shows ONLY: heading, one description line, three big tiles: Both screens, Top only, Bottom only
(these are today's quick-start buttons with the fixed display ids). REMOVE the detection cards, "Detect screens",
"Stream both", per-screen size/bitrate/always-on-top controls from the UI. Keep a "Stop" action visible while mirroring runs.
If code for the removed controls becomes unused, remove it, but tell me what you deleted.

## 2. Help page (everything that is not needed for streaming)
Opened from the Help button, with "Back to Stream". Collapsible sections (native details/summary is fine):
Pair Moonlight (short steps), Addresses of this PC, Sunshine on this PC (service, name, ports, program, login, Open Sunshine web UI,
Use a different login), Low-latency checklist + the four external links, Stream profile (Show stream profile).
Mascot "ask" at the top. Keep all existing content/data, only move it. Shorten wording where the reference does.

## 3. Progress bar for transfers
A status strip at the bottom of the Files view (and any other view that moves data, if easy): mascot "busy" small, current
file name, "file X of Y" and percent, an orange bar, a Cancel button (danger style). Hidden when nothing runs.
Use the real progress/events the backend already emits. If a transfer has no progress events today, add the smallest
possible event for it and tell me.

## 4. Files toolbar
Remove the "Rename" button from the toolbar (keep Rename in the "More" menu). Add a small round "?" button (44px, cream outline) next to "More".
Hover, focus or click opens a small panel listing keyboard shortcuts (key chips + label). Escape closes it.
The panel must list ONLY shortcuts that actually work.
Shortcuts behave like a normal file manager:
- F2 rename, Delete delete (with the existing confirmation), Ctrl+A select all, F5 refresh, Backspace up one folder, Enter open.
- Ctrl+C copy, Ctrl+X cut, Ctrl+V paste into the current folder. Cut + paste = move. Works between folders on the Thor.
- Stretch goal: Ctrl+V also pastes files that were copied in the PC's own file manager onto the Thor (like dropping them).
  Only do this if the app can reliably read file lists from the system clipboard on this platform. If not, leave it out and tell me.
First check which shortcuts already exist and tell me. Implement the missing ones. Shortcuts must not fire while typing in an input field.

## 5. Mascot usage
Use the mascot only where the reference shows it: Stream (wait/busy), Help (ask), transfer bar (busy). Optional: "done" for a
short success state after a finished transfer, "ask" for empty/unknown states. Copy the images into the frontend assets;
prefer the SVGs if the project handles SVG, else the PNGs. Do not add it anywhere else.
