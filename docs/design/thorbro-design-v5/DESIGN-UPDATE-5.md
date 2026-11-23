# ThorBro PC – Update 5: Files view cleanup (file pool in the sidebar, no drag-out)

Reference boards (open in a browser, they are static mockups):
FilesClean1 (pool empty), FilesClean2 (pool filled), FilesClean3 (right-click menu), FilesClean4 (drop from PC).
This replaces the Pool panel from FilesPool.dc.html and the toolbar of the current Files view. Same tokens as before.

Rules
- Do NOT commit or push. Alek does that himself.
- UI text and code comments in English.
- Do not guess. If something in the current code differs from what is described here, report it and propose, do not silently pick.

## 1. Remove drag-out completely
- Remove dragging items out of ThorBro to other programs: dragOut in files-actions.js, the drag_start command, the GTK drag, the ~/.cache/thorbro/ziehen cache (fetch, .part, .stamp, 24 h cleanup), the drag-failed event and the drag-out toasts/confirmation.
- Drag-in from the PC into the window stays (see 5).
- Getting files to the PC is only done via the file pool ("Copy to PC…", reuses the existing Save to PC flow: folder picker, queue, progress, Cancel, never overwrites, partial copy removed).

## 2. File pool in the left sidebar
- Always visible under storage, pinned and libraries (libraries stay in Files). Remove the "Pool · N" toolbar button and the 340 px side panel.
- Empty: dashed orange border, mascot "wait", short text: "Collect files and folders from the Thor here, then copy them all to your PC in one go. Drag them in from the list. The Thor stays untouched." and a "Drop here" field.
- Filled: solid orange border, mascot "ask", item rows with name, size and a remove x, header with item count and total size, hint "You pick the folder on your PC next. Nothing on the Thor changes.", exactly two buttons: "Copy to PC…" (primary) and "Clear pool".
- The pool only holds items from the Thor. Items from the list are dragged into it. Keep the existing pool logic (duplicates ignored, items inside a pooled folder ignored, a folder replaces pooled children, large-size confirmation).
- Remove "Move here" and "Copy here" from the pool. Moving/copying on the Thor goes through cut/copy/paste and direct drops.
- Rows that are in the pool are shown dimmed with a small orange "in pool" tag in the list (FilesClean2).

## 3. Toolbar
- Only: Back, Forward, Refresh, the path bar (pin button inside the bar), "New folder", and the "?" button. Use drawn SVG icons for the arrows (see boards), Forward is dimmed when there is nothing to go forward to.
- Remove: Up, Send files, Send folders, Copy to PC, Delete, Pool, More.
- Check and report: is there a back/forward history today? If not, add one. Are the path segments clickable (jump to that folder)? If not, make them clickable, because Up is gone.
- Report where everything that was in "More" ends up (context menu, shortcut, or removed). Propose, do not drop silently.
- Hint line under the toolbar: "Drag files or folders from your PC into this window to add them to this folder. No drag and drop? Press Ctrl+O or right-click the list." In other states see boards (FilesClean2/3). Check whether Ctrl+O exists, add it if not (file picker, then a folder picker via right-click entry).

## 4. Context menu
- Right-click on selected rows: Add to file pool, separator, Cut (Ctrl+X), Copy (Ctrl+C), Paste (Ctrl+V), separator, Rename (F2, only for exactly one item), Delete (Del, danger color).
- Right-click on empty list space: Add files…, Add folder…, Paste (if something is cut/copied), New folder.
- Delete keeps its existing confirmation. Cut/Copy/Paste keep the existing large-amount confirmation.
- Add the shortcuts to the "?" panel.

## 5. Drag from PC into the window
- While dragging over the list: dim the list and show the drop card with the mascot (FilesClean4): "Drop to add to the Thor", target folder, item count and size if known.
- Dropping goes into the open folder.

## 6. Mascot
Use the existing mascot PNGs from update 2 (base, ask, wait, done, busy). Do not redraw them. Placement: wait in the empty pool, ask in the filled pool, base in the drop card, busy stays in the progress strip.

## 7. Keep
- Progress strip with Cancel and the verified-move logic, direct drop confirmation thresholds, user-select fixes, pinned folders, libraries.

## Tests and report
Update or remove tests for removed code, add tests for the new pool states, the toolbar, back/forward history, the context menus and the dropped drop path. Run all suites and clippy.
Report: what changed, what was removed, what was not verifiable without a real Thor, and every place where you had to deviate from this spec.
