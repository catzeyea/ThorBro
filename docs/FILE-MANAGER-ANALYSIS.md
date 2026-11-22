# File manager: why it feels sluggish (analysis before the UX rework)

Written before changing any code on `feat/file-manager-ux`, against `main` at `5e29b3f`.
Line numbers refer to that commit. Everything below was found by reading the code; nothing was
measured on a real Thor (no device was attached while writing this).

## What is *not* the problem

- **The UI thread is not blocked by adb.** Every file manager command (`list_dir`, `make_dir`,
  `rename`, `delete`, `pull`, `drag_fetch`) is an `async` Tauri command that runs its adb call
  through `run_blocking` → `spawn_blocking` (`src/commands.rs:55-58`, `:180-208`). `invoke` returns a
  promise; the webview keeps painting while adb runs.
- **The listing itself is one adb round trip.** `files::listdir` runs a single
  `adb shell 'cd … && stat -c … * .[!.]* ..?*'` (`src/files.rs:58-74`), not one call per entry.
  The cost is the fixed adb process spawn + USB round trip (typically tens to a few hundred ms),
  paid once per folder open.

So the sluggishness is in how the UI *uses* those calls, not in the calls being synchronous.

## Real causes, in order of how much they hurt

1. **No cache: every folder open waits for adb, and the old list is wiped first.**
   `go()` (`ui/js/files.js:242-258`) calls `renderFiles("Loading …")` (line 248), which clears the
   list, then awaits `list_dir`. Going back into a folder you just left costs a full adb round trip
   again and shows an empty "Loading …" list in between. This is the main "flash and wait" effect.

2. **Stale responses can overwrite the current folder (race).**
   `go()` sets `S.cwd` immediately (line 244) but writes `S.entries` whenever *its* `list_dir`
   resolves (line 251). Navigating quickly (double-click into a folder, then Backspace, or two quick
   double-clicks) starts two listings; if the older one resolves last, the list shows folder A's
   entries while `S.cwd` is folder B. File actions then build paths as `${S.cwd}/${entry.name}`
   (lines 374, 383, 398, 407) and target the wrong folder. There is no request token and no
   debounce.

3. **Every change re-lists the whole folder over adb and rebuilds the whole list.**
   After New folder / Rename / Delete the handlers call `go(S.cwd)` (lines 375, 385, 400), and after
   every finished transfer `onTransferFinished` does too (`ui/js/transfer.js:178`). That means:
   another adb round trip after the action's own round trip, the "Loading …" flash from cause 1,
   lost selection (`S.sel = -1`, line 245) and lost scroll position (the list is rebuilt from
   scratch). A rename therefore feels like "wait, flash, jump to top".

4. **Full DOM rebuild on every render, no virtualization.**
   `renderFiles` (lines 287-304) does `list.innerHTML = ""` and then builds one row per entry via
   `buildFileRow` (lines 315-342). Each row is parsed from an HTML string through a `<template>`
   (`el()`, `ui/js/core.js:56-60`), queried three times with `querySelector`, and gets five
   closures. A ROM folder with a few thousand files creates thousands of nodes and closures at once
   on every render, including renders that change nothing visible: toggling "Hide empty"
   (line 311) or setting a system (line 236).

5. **Selection touches every row.**
   `select()` (lines 345-350) runs `querySelectorAll("#filelist .row")` and toggles a class on
   every row for each arrow key press, then `scrollIntoView`. O(n) per key press; noticeable when
   holding an arrow key in a large folder.

6. **The action is only applied after adb confirms.**
   Rename / delete / new folder show nothing until adb has finished *and* the re-listing from cause 3
   has finished. There is no optimistic update, so each action has at least two round trips of dead
   time.

7. **Minor: forced layout in the breadcrumb.**
   `renderCrumbs()` rebuilds the crumbs with `innerHTML` and then reads `crumbs.scrollWidth`
   (line 279), which forces a synchronous layout. It happens once per navigation, so it is a small
   cost, not a main cause.

8. **Minor: the listing remembers the "last folder" on every call.**
   `commands::list_dir` writes `last_path` on each listing (`src/commands.rs:183`). With background
   refreshes and stale responses this would save the wrong folder; it needs to move to the point
   where the UI knows which folder is current.

## Missing interactions (not performance, but part of "feels sloppy")

- Single selection only (`S.sel` is one index, `ui/js/core.js:74`); no Ctrl/Shift click, no Ctrl+A,
  no rubber band.
- No cut / copy / paste, no context menu, no back / forward history, no sorting, no column for
  modification date (the Rust `Entry` has no time field, `src/files.rs:47-52`).
- Dragging a row always starts a drag *out to the PC* (lines 459-493), which first downloads the
  file into the cache. There is no way to drag an entry into another folder on the Thor.
- Rename and New folder use modal dialogs instead of editing in place.

## Conclusion for the fix

None of the causes need a frontend framework. They are fixed by:

- a per-path listing cache with stale-while-revalidate, a request token so the latest navigation
  wins, and a short debounce for the adb call (causes 1, 2);
- applying actions to the cached listing first and reconciling in the background, with rollback on
  error (causes 3, 6);
- a virtualized list with fixed row height and rows keyed by file name, so a refresh only touches
  rows whose data changed and selection only touches visible rows (causes 4, 5);
- batch move / delete commands in Rust (one adb call for many entries), `mtime` in the listing,
  and a Thor → Thor copy job in the existing queue (progress + cancel).

A framework like React would give the keyed diff, but not the virtualization, cache or request
ordering, which are the parts that matter here. A few hundred lines of plain JS in the existing
style (global scripts, no bundler) are cheaper than adding a build step.
