# ThorBro PC – Update 6: Header with ThorBro, connection state, always start on Start

Reference: the header in all boards (Main, Files, PCStream, PCMirror, PCHelp, FilesClean1-5), and FilesClean5 for the not-connected state.
Rules: Do NOT commit or push. UI text and code comments in English. Do not guess, report deviations.

## 1. Header on every page (Start, Files, Saves, Stream incl. Mirror and Help)
- Left: the "ThorBro" wordmark, big (about 62 px), semi-transparent (about 72 % opacity), bold, same font as the UI. Replaces the square app logo + title.
- In front of the wordmark sit three ThorBros in different colors, overlapping a little, slight tilt (about -5, 3, -2 degrees), slightly see-through (90 %), sizes about 82 / 90 / 82 px, shifted toward the middle of the word, feet near the top edge of the letters.
  Colors: sage #A5B98C (assets/thorbro-bro-sage), orange #F0A94B (thorbro-bro-orange), coral #FF9C84 (thorbro-bro-coral). Files are in assets/ (SVG + PNG). Use the SVG.
- Middle: the main nav (Start, Files, Saves, Stream), unchanged.
- Right: connection status pill plus ThorBro (about 124 px) next to it. No line under it.
  - Connected: pill "Connected: AYN Thor" (green dot), ThorBro awake (existing mascot "done").
  - Not connected: pill "Not connected" (dashed outline, hollow grey dot, muted text), ThorBro asleep, tilted about 14 degrees, using assets/thorbro-wait-off.svg (zzz, and the lit thing in his hand is out: no smoke, grey tip). Do not use the old "wait" mascot here.
- The header switches live when the Thor connects or disconnects. Check how the app detects the connection today and hook into that. Report what you find, do not guess.
- With no Thor connected: the Files view shows the empty card "No Thor connected / Plug the Thor in with USB and make sure USB debugging is on. ThorBro wakes up as soon as it sees it." instead of the list (see FilesClean5). Check what the other pages do without a Thor today and report.
- Header height grows to about 150 px. Check that content below still fits on a small window.

## 2. Always start on Start
- Every launch lands on the Start page, regardless of the last used tab. If the app currently remembers the last page, remove that behavior (report if it is a setting).
- Check whether Stream currently opens on Mirror. Leave it as is, but report what it does.

## Tests and report
Add tests for: the header rendering in both connection states, the switch on connect/disconnect, the empty Files view without a Thor, and the launch page. Run all suites and clippy.
Report what changed, what only works on a real Thor, and every deviation from this spec.
