# ThorBro PC – Update 9, step 1: INVESTIGATE ONLY (no code changes)

Do NOT commit or push. Do NOT change any code in this step. Report only.
Mark every claim as "verified on the device" or "unverified". No guessing.

## Goal of the feature (design in the attached boards)
1. Start page: games grouped per console -> emulators -> games with cover art, plus "+ Add" (console / folder / emulator).
2. Saves for emulators, not only inside ROM libraries. Flow: search a game -> ThorBro suggests the emulators that have a save for it (incl. native Android ones like Peanuts) -> pick one -> ThorBro shows exactly ONE save folder -> "Add this save". Replaces the flat "Folder on the Thor" search dialog.
3. ThorBro is public: emulator support must come from profiles that the community can extend, not hardcoded for one person's setup.

## What to find out (on the real Thor, via adb)
A. Cover art
 - Where do Cocoon and iiSU store game art on the device? (package names, paths under Android/data, /sdcard, app-private?)
 - Is it readable over adb without root? Which formats and sizes?
 - How is an image tied to a game (file name, title ID, hash, database)? Read-only: do not modify anything.
 - If it is NOT readable: say so, and list fallbacks (e.g. art next to ROMs, emulator caches, RetroArch thumbnails). Do not download anything from the internet yet.
B. Installed emulators
 - How to detect them (pm list packages). List the emulators actually installed on this Thor with package names (Eden, Eden Duo, Peanuts, PPSSPP, RetroArch, others).
C. Save locations per emulator (verified paths only)
 - Switch: Eden, Eden Duo, Peanuts (package dev.nxnative.android.peanuts): save root, folder layout, how the game is identified (title ID?). What do the numbered subfolders (0-3) and anything like a tracker mean? Test with Zelda BotW (title ID 01007EF00011E000) if present.
 - PPSSPP, RetroArch, other installed emulators: same questions.
 - Which of these roots are readable over adb on this firmware?
D. Matching game <-> save
 - For emulators without title IDs in folder names, how could a save be tied to a game? Options and risks, no implementation.
E. Profile format
 - Propose a small declarative profile (JSON or TOML) per emulator: package, console, save root pattern, identification method, art source. Show 2 real examples from what you verified. Where would user-added profiles live, and how would they be validated?
F. Existing code
 - Which parts of the current PC app (Saves page, games.json sync, library scan, Start page) would change? List files and functions, keep it short.

## Report format
Short sections A-F, a table of verified vs unverified, and open decisions for me. No code.
