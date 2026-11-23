# UI tests for the file manager and the stream tab

Runs the real `ui/index.html` and `ui/js/*` in headless Firefox with a fake Tauri bridge
(`stub.js`: in-memory Thor file system, adjustable adb latency, injectable failures) and checks
the file manager's behavior (`tests.js`): cache, request ordering, virtualization, keyed updates,
selection, rename/new folder/move/delete with rollback, cut/copy/paste, context menu, drag & drop.
For the stream tab (fake scrcpy in `stub.js`): display cards, per-display start/stop with options,
"Stream both", failures on the card, the one-display hint, detection errors and the quick start.
For the "Stream PC games" sub-tab (fake Sunshine/Moonlight in `stub.js`): status cards, login save/reject,
pairing with and without `pairing_id`, game list, "Play on Thor" with the experimental display id, stream profile,
"Send to Thor app" (sent / note / app not installed / too large).

```sh
node tests/ui/run.mjs           # needs node >= 22 and firefox
node tests/ui/run.mjs --shots   # also writes shot-list.png and shot-menu.png here
```

Firefox is Gecko, not WebView2 (Windows) or WebKitGTK (Linux), so this checks the logic, not
engine-specific behavior. The shell side is checked on a real device by the ignored Rust tests:
`cargo test device_ -- --ignored --test-threads=1` (works only inside its own
`thorbro-selftest-*` folders and deletes them afterwards).
