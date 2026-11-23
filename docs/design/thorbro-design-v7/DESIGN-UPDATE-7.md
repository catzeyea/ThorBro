# ThorBro – Design Update 7: Start page (Thor status + ThorBro mood)

PC app only (Tauri). The Android companion is NOT touched.
UI text and code comments in English. Do NOT commit or push.

## Reference files (open in a browser)
- `ref/Main.dc.html` – Start page, Thor connected, SD card present (final look)
- `ref/StartNoSD.dc.html` – Start page without SD card (RAM column replaces SD)
- `ref/StartMoods.dc.html`, `ref/StartMood1-5.dc.html` – mood ideas, OLD look (use only for the faces/sentences, not for layout; the layout is Main.dc.html)
- `assets/` – mascot SVGs: tired, stuffed (new). Existing: base/busy/wait/wait-off, bro-sage/orange/coral (from Update 6)

## 1. Start page layout (see Main.dc.html)
Top to bottom:
1. Shared header (already done in Update 6)
2. **Status section**: big ThorBro (~210px) on the left, mood headline (e.g. "I feel great!") + one first-person sentence next to it. Below: ONE single row of status columns, no cards or tiles, drawn gauges:
   - Battery: 10-cell battery, percent, bolt icon when charging
   - Internal storage: block grid (free vs used), "X GB free"
   - SD card: same block grid. **Only if an SD card is present.**
   - CPU: small sparkline + percent
   - RAM: memory-stick drawing + percent + "X of Y GB used". **Shown only when there is no SD card (takes the SD column's place).**
   - Temperature: thermometer + °C, sits on the same row
   Order with SD: Battery, Internal, SD, CPU, Temp. Without SD: Battery, Internal, CPU, RAM, Temp.
3. **Drop info line**: dashed sage outline, arrow icon, text "Drop files anywhere in ThorBro". Info only, NOT a button, no click action.
4. Pinned folders, Libraries (unchanged)

## 2. ThorBro mood
One mood at a time, priority top to bottom (first match wins). Thresholds are PROPOSALS, put them in one constants block so they are easy to tune.

| Priority | Mood | Trigger (proposal) | Mascot | Headline |
|---|---|---|---|---|
| 1 | asleep | no Thor connected | wait-off (sleeping, joint out) | "Zzz…" |
| 2 | working hard | CPU >= 80 % or temp >= 42 °C | busy | "Working hard!" |
| 3 | tired | battery <= 20 % and not charging | tired | "I'm tired…" |
| 4 | stuffed | internal storage >= 90 % full | stuffed | "I'm stuffed!" |
| 5 | fit | everything else | base | "I feel great!" |

- Sentence under the headline: short, first person, says WHY (e.g. "Battery is at 14 %. Plug me in soon."). Write 1-2 variants per mood.
- Header ThorBro (top right, from Update 6) shows the same mood on every page, not just asleep/awake.
- Mood sentence and gauges use the same data source (no separate polling).

## 3. Data – VERIFY FIRST (nothing here is confirmed)
I do not know which values the app can really read from the Thor over adb. Check in the code and, if a Thor is connected, on the device:
- Battery level + charging state (probably `dumpsys battery`)
- Internal storage and SD card (free/total, whether an SD card exists)
- CPU load, RAM used/total, temperature
- (GPU was dropped from the design, ignore it)

Rule: a value that cannot be read reliably is NOT shown (its column disappears) and is NOT used for the mood. Do not show fake or placeholder numbers. "5 of 12 GB" in the design is a placeholder.

## 4. Live updating
- While the Start page is open: refresh every few seconds (suggest 3-5 s, constant).
- Stop polling when Start is not visible or when no Thor is connected. No background polling on other pages.
- Polling must never block the UI or pile up (skip a tick if the previous one is still running).
- Mood and gauges update in place (no page flash).

## 5. Drop anywhere
The new info line claims files can be dropped anywhere in the window. Check whether drag-in from the PC really works on EVERY page (Start, Files, Stream, Saves, Help …) and where it goes (file pool). Report the result. If it does not work everywhere, implement it so it does (drop on any page adds to the file pool, with a small toast "Added N items to the file pool"). Do not change the file pool behavior itself.

## 6. Report only (do not change)
Tell me what happens today when the Stream page opens: does it open on "Mirror"? I am considering opening on "Stream PC games" instead. Just report the current behavior.
