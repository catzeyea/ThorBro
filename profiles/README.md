# Emulator profiles

ThorBro finds the saves of an emulator through a small JSON profile. A profile only *describes*
where things are; it can't run commands. ThorBro reads the Thor over adb and copies whole save
folders, nothing else.

- Built-in profiles live in this folder and are compiled into ThorBro (`src/emulators/mod.rs`).
  Only emulators whose save layout was checked on a real AYN Thor are shipped here.
- Your own profiles go into `~/.config/thorBro/profiles/*.json` (Windows:
  `%APPDATA%\thorBro\profiles`, macOS: `~/Library/Application Support/thorBro/profiles`).
  A profile there with the same `id` as a built-in one replaces it.
- A broken profile is skipped. The Start page ("+ Add emulator") lists the reason.

## Example

```json
{
  "format": 1,
  "id": "peanuts",
  "name": "Peanuts",
  "package": "dev.nxnative.android.peanuts",
  "console": "switch",
  "games": { "dirs": "{appdata}/files/packs", "name_file": "game.tsv" },
  "saves": {
    "folder": "{appdata}/files/saves/{title_id}",
    "title_id": "switch",
    "write": false,
    "write_note": "Why ThorBro may not write here."
  },
  "art": ["iisu"]
}
```

## Fields

| Field | Required | Meaning |
|---|---|---|
| `format` | yes | Always `1`. |
| `id` | yes | Lowercase letters, digits and `-`, max 40 characters. |
| `name` | yes | Shown in the app, max 40 characters. |
| `package` | yes | Android package name. ThorBro only uses the profile when this app is installed, and closes it before it writes a save. |
| `console` | yes | ES-DE system id, e.g. `switch`, `n3ds`. |
| `games` | no | `{ "dirs": "<folder>", "name_file": "<file>" }`: one sub folder per game, named by its title ID. The first line of `name_file` may hold the name (`# Game profile of NAME (…` or `# NAME (…`). |
| `saves.folder` | yes | The save folder of one game. Must contain the title ID placeholder as a whole path part. |
| `saves.title_id` | yes | `switch` (16 hex digits, `{title_id}`) or `3ds` (`{title_id_low}`: the last 8 hex digits in lowercase, or `{title_id}`). |
| `saves.base` | no | Defines `{base}`: either `{ "path": "…", "ini": { "file", "section", "key" } }` (the ini value wins if it is set) or `{ "search": { "marker": "sdmc/Nintendo 3DS", "depth": 5 } }` (a folder found on any storage that ends in `marker`; `{base}` is the part before it). |
| `saves.user` | no | Defines `{user}`. Only `"resolver": "switch-profiles-dat"`: the user folder from yuzu-style `profiles.dat` (`file`), optionally the slot from an ini key (`index`). |
| `saves.write` | no | `false` = ThorBro reads and backs up saves but never writes them to the Thor. Default `true`. |
| `saves.write_note` | no | Shown when writing is turned off. |
| `art` | no | Cover sources. Only `iisu` for now. |

## Placeholders

| Placeholder | Value |
|---|---|
| `{appdata}` | `/sdcard/Android/data/<package>` |
| `{base}` | see `saves.base` |
| `{user}` | see `saves.user` |
| `{title_id}` | the game's title ID, uppercase |
| `{title_id_low}` | 3DS only: last 8 hex digits, lowercase |

Rules ThorBro checks: unknown fields and unknown placeholders are rejected, every path must start
with a placeholder, `/sdcard/` or `/storage/<volume>/`, and `..` is not allowed anywhere. Paths read
from the Thor (ini values, search results) must pass the same check.
