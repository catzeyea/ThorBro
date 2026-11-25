# ThorBro PC – Update 9, step 2: IMPLEMENT

Do NOT commit or push. Alek commits himself.
Read DESIGN-UPDATE-9.md and boards/ (StartConsoles, PCSaves, PCSavesAdd1, PCSavesAdd2) first.
UI text and code comments in English. Keep everything verified-only; list anything you could not verify at the end.

## Decisions (from your step-1 report)
1. Profiles are JSON (serde_json), purely declarative, validated as you proposed (unknown fields rejected, format version, fixed placeholders/resolvers, paths only under /sdcard or /storage/<vol>, no "..").
   Built-in profiles in the app + profiles/ folder in the repo; user profiles in ~/.config/thorBro/profiles/*.json override by id.
   Ship profiles ONLY for verified emulators: Eden, Eden Duo, Peanuts, Azahar. No PPSSPP/RetroArch/melonDS/RPCSX/Dolphin/Cemu/AetherSX2 save profiles yet.
2. Covers: iiSU art only (SD card path from your report, folder = ROM file name minus extension, filter updates/DLC via existing title-ID logic).
   Use icon.* (square). Cache on the PC. Placeholder with initials when no art. Cover tiles are SQUARE. Cocoon: postponed, keep "art" a list in the profile.
3. One managed save per game AND emulator (BotW in Eden Duo and Peanuts = two entries). No copying between emulators on the Thor.
4. Peanuts: read/pull/backup only for now. Push to Peanuts stays disabled (clear message in the UI) until a throwaway-save test shows rename-swap works and what Peanuts does with .work. Do the test only on a throwaway save, and report the result. Ignore .work and .lock when reading.
5. Eden / Eden Duo: always use the user folder from profiles.dat (reversed UUID), ignore orphan user folders. Read nand_directory from config.ini.
6. PS2/GC memory cards: not supported yet. Do not offer them.
7. push_dir_shared: replace rm -rf + copy with a rename-swap for the emulators that allow writes. Take stop_pkg from the profile (also fixes Azahar on SD).
8. games.json migration: tag the 5 existing entries automatically (4 Eden Duo, 1 Azahar), keep a backup of the old file.

## Build
- Start: replace Libraries tiles with per-console blocks (heading + count, emulator chips with counts, "+ Add emulator", square cover grid, "+ Add a console or a folder"). Consoles/emulators from the real Thor, nothing hardcoded for one setup.
- Saves: emulator chip on cards; "+ Add a game save" 2-step dialog (search -> emulators that have a save, others greyed -> exactly one folder with path/file count/last change -> "Add this save"). "Wrong folder? Browse" = old picker, starting in the emulator's data folder. Remove the flat "Folder on the Thor" search as the main path.
- Update existing tests, add tests for profile validation, Eden user resolver, title-ID matching and the games.json migration.

## Report
What works, what you ran on the real Thor, what is still unverified, files changed. Do NOT commit or push.
