// ThorBro UI – start-up. Loaded last, so every function from the other files exists.
"use strict";

/** How often the device connection and the stream status are checked (ms). */
const POLL_INTERVAL_MS = 3000;

loadPins(); // pins.js (settings.json, no Thor needed)
pollDevice(true);
setInterval(() => pollDevice(false), POLL_INTERVAL_MS);
setInterval(() => { if (S.tab === "stream") streamStatus(); }, POLL_INTERVAL_MS);
setInterval(() => { if (S.tab === "stream" && PCS.sub === "pc") pcPoll(); }, POLL_INTERVAL_MS);
