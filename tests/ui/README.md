# UI tests for the file manager and the stream tab

Runs the real `ui/index.html` and `ui/js/*` in headless Firefox with a fake Tauri bridge
(`stub.js`: in-memory Thor file system, adjustable adb latency, injectable failures) and checks
the file manager's behavior (`tests.js`): cache, request ordering, virtualization, keyed updates,
selection, rename/new folder/move/delete with rollback, cut/copy/paste, context menu, drag & drop.
The file pool in the sidebar (empty/filled states, dropping only collects, "in pool" tags, "Copy to PC…" and Clear, count and size,
duplicates and nested paths), the toolbar, back/forward history and the path bar, both context menus, Ctrl+O / "Add files …" /
"Add folder …", the drop card for files dragged in from the PC, the "are you sure?" check for large amounts (direct drops, paste,
"Copy to PC…"; size, item count, unknown size), the Cancel button of the strip, that rows can't be dragged out of the window,
and `user-select` / `selectstart`.
For the stream tab (fake scrcpy in `stub.js`): the three mirror tiles (both / top / bottom),
failures under the status (toasted once) and Stop. Also: the transfer strip (events, "file X of Y", Copy to PC),
the keyboard shortcuts panel and that shortcuts don't fire while typing.
Pinned folders (fake settings in `stub.js`): pin/unpin by button, Ctrl+D, "×" and right-click, pin order, per-storage pins,
opening from the sidebar and the Start tab, "not found" (folder gone or Thor not connected) and the cap of 12.
For the "Stream PC games" sub-tab and Help (fake Sunshine/Moonlight in `stub.js`): status chips, Help sections, login save/reject,
pairing with and without `pairing_id`, game list, "Play on Thor" with the experimental display id, stream profile,
"Send to Thor app" (sent / note / app not installed / too large).
The header and the launch page (Update 6): every launch opens Start and no tab is stored, the shared header
(wordmark, three ThorBros, pill and status ThorBro) on every tab, connected and not connected looks, the live switch
on the regular 3 s poll when the fake Thor (`HARNESS.device` in `stub.js`) is unplugged and plugged in again, and the
"No Thor connected" card that replaces the file list. After `tests.js`, `run.mjs` resizes the window from 1440 x 900
down to the minimum 520 x 480 and checks that the header fits (no sideways overflow, about 150 px or less, the Files
list keeps its room where it had room before).
The Start page status and ThorBro's mood (Update 7, fake readings in `HARNESS.thorStatus`): the mood priority table and
its thresholds, "charging" only for status 2, the columns with and without an SD card (RAM takes the SD card's place),
no CPU column before the second reading, the same mood in the header on every page, readings only while Start is open
and a Thor is connected, the skipped tick while a reading (or a device poll) still runs, and that the drop line is not a
button. The Rust side (`src/status.rs`) parses output captured from a real Thor; `cargo test device_status_reads --
--ignored` reads the connected Thor.

```sh
node tests/ui/run.mjs           # needs node >= 22 and firefox
node tests/ui/run.mjs --shots   # also writes shot-list.png and shot-menu.png here
```

Firefox is Gecko, not WebView2 (Windows) or WebKitGTK (Linux), so this checks the logic, not
engine-specific behavior. The shell side is checked on a real device by the ignored Rust tests:
`cargo test device_ -- --ignored --test-threads=1` (works only inside its own
`thorbro-selftest-*` folders and deletes them afterwards).
