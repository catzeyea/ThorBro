# ThorBro UI design (reference)

Goal: restyle the existing ThorBro PC frontend to this look. Change styling and markup structure only.
Do NOT change behavior, Tauri commands, file logic or data flow.

Reference files (not runnable as-is, custom tags `x-dc` / `sc-for`; read them for exact inline styles):
- `reference/start.dc.html`  -> Start screen
- `reference/files.dc.html`  -> Files screen
- `logo-256.png`             -> app logo, shown in header (56px) and in the Start drop zone (112px, rotated -5deg)

## Look in one line
Dark forest background, sage green as main color, thick cream outlines, hard offset shadows,
orange only for the one main action. Slightly irregular corners and tiny rotations so it looks
hand-made, not templated. No gradients, no glow, no icon-library icons in buttons or lists.

## Tokens
| Token        | Value                                  | Use |
|--------------|----------------------------------------|-----|
| bg           | #1a1e1f                                | page background (flat) |
| panel        | #252a2a                                | cards, tiles, tables, sidebar |
| ink          | #EFE7D2                                | text, outlines |
| ink-line     | rgba(239,231,210,0.78)                 | 2px outline of panels/buttons |
| mute         | #B4B6AA                                | secondary text (#C4C6BA for body copy) |
| sage         | #A5B98C                                | selected rows, "Choose files", device dot, dashed drop border |
| sage-shadow  | #4a5c38                                | shadow of sage buttons |
| orange       | #F0A94B                                | main action + active tab + game counts |
| orange-shadow| #7a4a0f                                | shadow of orange buttons |
| danger       | #FF9C84                                | Delete (outline + text) |
| shadow       | #0c0f0f                                | panel shadow, 4-6px offset, no blur |
| text on sage/orange | #1a1e1f                         | |

## Typography
ONE font family everywhere. Candidates (switch in the reference): JetBrains Mono (default),
Inconsolata, IBM Plex Sans. Use the chosen one for everything, incl. headings and buttons
(`font-family: inherit` on buttons). No handwriting fonts.
Sizes: wordmark 28/700, page heading 46/700, section heading 30/700, tile name 30/700,
body 17, buttons 15 (primary 600, others 500), small labels 13-14, secondary labels in mute.

## Shape rules
- Border: 2px solid, ink-line. Primary buttons use full ink.
- Radii are irregular on purpose, e.g. `22px 18px 24px 16px / 18px 24px 16px 22px`; vary them per element.
- Panels: `box-shadow: 5px 5px 0 #0c0f0f` (vary 4-6px per element). Hover on tiles: translate(-2px,-3px), shadow 8px.
- Libraries tiles: rotate between -0.7deg and +0.7deg, alternating. Hover: rotate(0).
- Drop zone: 2px DASHED sage border, transparent background, irregular radius.
- Active tab: orange fill, ink border, `3px 3px 0 #7a4a0f`, rotate(-1deg). Inactive tabs: text only, hover tint rgba(239,231,210,.12).
- Device chip (top right): pill, outline, green(sage) dot + "AYN Thor, USB".

## Screens
Start: header (logo + wordmark | tabs Start/Files/Saves/Stream | device chip) -> drop zone
(logo, "Drop games here", description, right side "games folder" label + path + sage "Choose files")
-> "Libraries" heading -> 4-column grid of tiles (name, folder in small text, orange "N games").
Files: same header -> left column 270px (Storage card, Libraries card with counts; selected = sage fill + ink border)
-> right: path bar with up/refresh buttons, toolbar (orange "Send files", then Send folders | New folder,
Rename, Copy to PC, Delete in danger color, More at the far right), file table (NAME / SIZE / CHANGED,
selected row sage fill with dark text, folders end with "/").
Saves and Stream: not designed yet. Apply the same tokens and rules, keep their existing behavior.

## Layout
Page padding 28px 48px, content max-width 1344px, fluid. Sidebar + content wrap on narrow windows.
Table scrolls horizontally in its own box. Buttons are real <button>, links real <a>; icon-only buttons need aria-label.
