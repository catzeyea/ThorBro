# ThorBro PC: design update 3 (pinned folders)

Builds on DESIGN.md and DESIGN-UPDATE.md (same tokens, same font). Reference files (not runnable, custom tags):
reference/files.dc.html and reference/start.dc.html. These two replace the earlier reference copies.

## Feature
Pin folders of the Thor so they are always at hand, e.g. the folder where the Thor's screenshots land.
Pins are Thor folders (storage + path), not PC folders.

## Files view
- Pin button: round-ish 46px button next to the refresh button in the path bar. Toggles pinned state of the folder that is open.
  Pinned = orange filled pin icon, unpinned = outline. aria-label "Pin this folder" / "Unpin this folder".
- Sidebar card "pinned" between the Storage card and the Libraries card. One row per pin: small orange pin icon + folder name.
  Click opens the folder. Unpin via a small "x" that shows on hover/focus and via right-click "Unpin". Full path as tooltip.
  Empty state: just the hint line "Pin the open folder with the pin button."
- Keyboard: Ctrl+D pins/unpins the open folder. Add it to the "?" shortcuts panel.

## Start view
Row "pinned" above Libraries: one chip per pin (pin icon + name), click opens Files at that folder. Hidden when there are no pins.

## Behavior and data
- Save pins in the app's existing settings storage, per storage (internal / SD card). Keep order of pinning. Sensible cap (e.g. 12) with a short message.
- A pinned folder that no longer exists (or the Thor is not connected): show the pin dimmed with "not found", do not crash, let the user unpin.
- Do not hardcode where the Thor stores screenshots. If ThorBro already knows or can detect that folder, tell me and offer it as a one-click
  suggestion; otherwise leave it to manual pinning.
- No new permissions, no change to existing commands except what is needed to store pins.
