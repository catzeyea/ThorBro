# ThorBro PC – Design Update 9 (reference for Claude Code)

Boards in boards/: StartConsoles, PCSaves, PCSavesAdd1, PCSavesAdd2 (.png = screenshot, .dc.html = source). Mock data only.

## Start
- Replace the "Libraries" tile grid with per-console blocks: heading + game count, emulator chips (each with games/saves count), "+ Add emulator", then a cover grid (6 columns).
- Last block: "+ Add a console or a folder".
- Covers must come from art that already lies on the Thor (Cocoon / iiSU). ThorBro ships no art. Source and readability are unverified: step 1 investigates. Placeholder (initials on colour) when no art is found.

## Saves
- List of managed saves: game, emulator chip, state (in sync / Thor newer / PC newer / only on Thor), last sync, "Continue on PC", "Continue on Thor". Every copy makes a backup first.
- "+ Add a game save": step 1 search game -> emulators that have a save (greyed out when none). Step 2 shows exactly one folder (path, file count, last change) + "Add this save"; "Wrong folder? Browse" opens a tree picker starting in the emulator's data folder.
- Emulator support via community-extendable profiles (format proposed after step 1).
