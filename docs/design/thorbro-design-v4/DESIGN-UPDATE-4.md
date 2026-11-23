# ThorBro PC – Update 4: Files drag & drop (Pool, Cancel, drag-out)

Reference design: FilesPool.dc.html (board "Files with Pool"). Same tokens as before. UI text and code comments in English.

## Rules
- Do NOT commit or push. Alek does that himself.
- Do not guess how things work. Investigate first, report, then change.

## 1. Pool (staging tray)
- Dragging files or folders INTO the Pool never moves anything. It only collects them.
- Pool button in the Files toolbar ("Pool · N"), opens a 340px side panel (see design).
- Panel: item count + total size, item rows with a remove (x) button, current target folder, actions:
  Move N items here / Copy here / Save to PC… / Clear pool.
- Pool is per session (in memory). Items are references (storage + path), not copies.
- Confirmation before Move/Copy when total size is large (suggest > 1 GB) or item count is high; show count + size.
- Same item added twice is ignored.

## 2. Direct drag inside the Thor
- Dropping directly onto a folder in the list must no longer start a move immediately for large items.
  Decide with Alek's report: show a confirm step (with Cancel) or route through the Pool. Propose, don't silently pick.

## 3. Progress strip + Cancel
- Running move/copy shows the progress strip with a Cancel button.
- Cancel must be safe: no data loss. For a move: never delete a source before its copy is verified complete.
  Report how cancel behaves for the file currently in flight (partial target file removed?) and for already finished files.
- Investigate the current transfer code first and tell Alek what is and isn't cancelable.

## 4. Drag-out (Thor -> outside)
Investigate FIRST and report how drag-out works today (mechanism, temp files, where items land). Do not guess.
Known problems from Alek:
- not reliable
- unclear where items land
- dropping on non-file-manager targets (desktop, editors, random windows) does something that feels wrong
Goals:
- Clear feedback while dragging (what is dragged, count) and clear result afterwards (where it ended up).
- No side effects when dropped on a target that cannot take files.
- Reliable path for big items: "Save to PC…" (folder picker) as the dependable alternative, also from the Pool.

## 5. Text selection while dragging
- user-select: none on rows, tiles, toolbars, panels. Keep selection in inputs and text areas.
- No text highlighting when a drag starts.

## Tests and report
Add/adjust tests, run the existing suites, report what was verified and what only works on a real Thor.
