# UI tests for the file manager

Runs the real `ui/index.html` and `ui/js/*` in headless Firefox with a fake Tauri bridge
(`stub.js`: in-memory Thor file system, adjustable adb latency, injectable failures) and checks
the file manager's behavior (`tests.js`): cache, request ordering, virtualization, keyed updates,
selection, rename/new folder/move/delete with rollback, cut/copy/paste, context menu, drag & drop.

```sh
node tests/ui/run.mjs           # needs node >= 22 and firefox
node tests/ui/run.mjs --shots   # also writes shot-list.png and shot-menu.png here
```

Firefox is Gecko, not WebView2 (Windows) or WebKitGTK (Linux), so this checks the logic, not
engine-specific behavior. The shell side is checked on a real device by the ignored Rust tests:
`cargo test device_ -- --ignored --test-threads=1` (works only inside its own
`thorbro-selftest-*` folders and deletes them afterwards).
