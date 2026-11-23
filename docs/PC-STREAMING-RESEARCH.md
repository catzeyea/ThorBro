# Research: streaming PC games to the AYN Thor (Sunshine + Moonlight)

Research only, no code. Written 2026-11-22 for two goals:

- **(a)** a ThorBro tab "Stream PC games to your Thor" that sets up and controls Sunshine on the
  PC and connects it with the Thor;
- **(b)** later, an own Android app on the Thor that starts Moonlight so streaming "just works".

Every claim links to its source. Statements marked **Not documented** or **Unconfirmed** could not be
backed by an official doc or the source code. Statements marked **Estimate** or **Recommendation**
are my own judgement.

## Sources and versions used

Source code was read from these exact revisions. Line links point there, so they stay valid when
upstream changes.

| Project | Revision | Notes |
|---|---|---|
| [moonlight-android](https://github.com/moonlight-stream/moonlight-android) | [`b48494c`](https://github.com/moonlight-stream/moonlight-android/tree/b48494cb96bff23d8886c4775cc4f39a1075495d) (2026-09-12) | latest release is [v12.2](https://github.com/moonlight-stream/moonlight-android/releases/tag/v12.2) from the same day |
| [moonlight-common-c](https://github.com/moonlight-stream/moonlight-common-c) | [`b4ee1ce`](https://github.com/moonlight-stream/moonlight-common-c/tree/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb) (2026-11-22) | |
| [Sunshine](https://github.com/LizardByte/Sunshine), latest stable | tag [`v2026.914.233613`](https://github.com/LizardByte/Sunshine/tree/v2026.914.233613) (2026-09-15) | main source for all Sunshine claims |
| Sunshine `master` | [`6b54f99`](https://github.com/LizardByte/Sunshine/tree/6b54f99fd8ef778474cf5da2f9e99a18ece9ec84) (2026-11-22) | two prereleases came out on 2026-11-22; only used to note differences |
| [Moonlight docs wiki](https://github.com/moonlight-stream/moonlight-docs/wiki) | `ff7a627` (2026-10-07) | |

Sunshine's user docs at docs.lizardbyte.dev are generated from the `docs/` folder of the repo. Both
are linked: the website for readability, the tagged file for the exact wording.

Thor facts below come from read-only `adb shell` queries on a real Thor on 2026-11-22. That Thor runs
Android 13 (API 33) and **Moonlight is not installed on it**, so nothing Moonlight-specific was tested on
the device.

---

## 1. Starting Moonlight on Android from outside (adb / Intent)

### Answer

**Yes, through one exported activity, `com.limelight.ShortcutTrampoline`.** This is the same entry point
Moonlight's own launcher shortcuts use. The actual stream activity, `com.limelight.Game`, is **not**
exported, so another app or adb cannot start it directly.

- Package name: `com.limelight` for the normal (`nonRoot`) build flavor; `com.limelight.root` for the
  root flavor; debug builds add `.debug`
  ([app/build.gradle#L40](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/build.gradle#L40),
  [#L52](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/build.gradle#L52),
  [#L86](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/build.gradle#L86)).
  The Java namespace is `com.limelight`
  ([#L8](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/build.gradle#L8)),
  so the component is `com.limelight/.ShortcutTrampoline`.
- `ShortcutTrampoline` is declared with `android:exported="true"` and `android:noHistory="true"`,
  and has no intent filter, so it has to be started with an explicit component
  ([AndroidManifest.xml#L108-L118](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/AndroidManifest.xml#L108-L118)).
- `Game` has no `android:exported` attribute and no intent filter
  ([AndroidManifest.xml#L146-L169](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/AndroidManifest.xml#L146-L169)).
  Without an intent filter, Android's default for `exported` is `false`
  ([`<activity>` docs](https://developer.android.com/guide/topics/manifest/activity-element#exported)).
- `PcView` (the host list) is exported and is the launcher activity
  ([#L89-L107](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/AndroidManifest.xml#L89-L107)).
- **There is no URI scheme.** No activity in the manifest declares a `<data>` element, so there is no
  `moonlight://...` style link
  ([AndroidManifest.xml](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/AndroidManifest.xml)).

### Extras accepted by `ShortcutTrampoline`

Read in `onCreate`
([ShortcutTrampoline.java#L280-L286](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L280-L286)).
The key strings are defined in
[AppView.java#L76-L77](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/AppView.java#L76-L77)
and [Game.java#L173-L182](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/Game.java#L173-L182).

| Extra key | Type | Meaning |
|---|---|---|
| `UUID` | String | Host UUID. Validated with `UUID.fromString` ([#L235-L245](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L235-L245)). |
| `Name` | String | Host name. Used only if `UUID` is missing; looked up in Moonlight's host database ([#L293-L309](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L293-L309)). At least one of `UUID`/`Name` is required ([#L227](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L227)). |
| `AppId` | **String** (parsed with `Integer.parseInt`) | App to start. Read with `getStringExtra`, so `adb shell am start --ei` (integer) would be ignored; use `--es` ([#L285](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L285), [#L311-L314](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L311-L314)). Moonlight's own shortcuts also pass it as a string (`""+app.getAppId()`, [ServerHelper.java#L49](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/utils/ServerHelper.java#L49)). |
| `AppName` | String | App to start, used if `AppId` is missing. Resolved against Moonlight's **cached** app list ([#L315-L355](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L315-L355)), see the caveat below. |
| `HDR` | boolean | Request HDR ([#L314](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L314)). |

If no app is given, the trampoline opens that host's app grid instead
([#L162-L186](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L162-L186)).

The official intent shape is what Moonlight builds for pinned shortcuts:
`ShortcutTrampoline` + `Name`, `UUID`, `AppName`, `AppId` (string), `HDR`, action
`Intent.ACTION_DEFAULT` (= `android.intent.action.VIEW`)
([ServerHelper.java#L36-L53](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/utils/ServerHelper.java#L36-L53)).

**Example**, derived from the code above. It was **not** run, because Moonlight isn't installed on the test Thor:

```sh
adb shell am start -n com.limelight/.ShortcutTrampoline \
  -a android.intent.action.VIEW \
  --es UUID "<uniqueid from Sunshine /serverinfo>" \
  --es AppName "Desktop"
```

From another Android app it is the same, with an explicit `ComponentName("com.limelight",
"com.limelight.ShortcutTrampoline")` and `putExtra(...)`.

### What the trampoline does, and where it can stop with a dialog

All of these steps show UI. The trampoline is not a silent background API.

1. It shows a "connecting" spinner
   ([#L362](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L362)).
2. If the host is offline and its MAC address is known, it sends Wake-on-LAN up to 10 times
   ([#L38](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L38),
   [#L94](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L94)).
3. It starts the stream only if the host is **online and paired**
   ([#L127](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L127)).
   Otherwise it shows an error dialog: "offline"
   ([#L190](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L190))
   or "not paired"
   ([#L196](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L196)).
4. If a **different** app is already running on the host, it asks the user to confirm quitting it
   ([#L130-L160](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L130-L160)).
5. It builds the internal `Game` intent itself with `ServerHelper.createStartIntent`
   ([ServerHelper.java#L55-L76](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/utils/ServerHelper.java#L55-L76)).
   Resolution, FPS, bitrate, codec and so on come from Moonlight's own preferences, **not** from the
   caller. Apart from `HDR`, an external caller cannot pass stream settings.

**Caveat: `AppName` depends on a cache.** The trampoline resolves names from the file `applist` in
Moonlight's cache directory
([#L320](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/ShortcutTrampoline.java#L320)).
That file is only written by the app-list poller
([ComputerManagerService.java#L898-L899](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/computers/ComputerManagerService.java#L898-L899)),
and the only caller that starts that poller is the host's app grid, `AppView`
([AppView.java#L263-L265](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/AppView.java#L263-L265)).
So a launch by name only works for apps that existed the last time the user opened that host's app grid
in Moonlight.

**Caveat: `AppId` is not exposed by Sunshine's admin API.** See section 4. Sunshine computes the ID as a CRC32
over the app name plus the SHA-256 of its cover image, with the list index added on a collision
([process.cpp#L633-L660](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/process.cpp#L633-L660),
[#L770-L776](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/process.cpp#L770-L776)).
ThorBro *could* replicate that calculation, but it is internal behaviour that may change.

### Does the stream activity need to be in the foreground?

**Yes.** `Game.onStop()` calls `stopConnection()`
([Game.java#L1078-L1093](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/Game.java#L1078-L1093)),
so the stream ends as soon as the activity is no longer visible. The one exception is picture-in-picture,
where the activity stays visible: `Game` declares `supportsPictureInPicture` and can enter PiP when the user
leaves
([AndroidManifest.xml#L150](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/AndroidManifest.xml#L150),
[Game.java#L711-L737](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/Game.java#L711-L737)).

So "starts Moonlight in the background" (goal b) is not possible with stock Moonlight. The stream
must be on screen.

**Thor-specific option (Unconfirmed):** the Thor has two physical displays. Android allows activities on
secondary displays when the device declares `FEATURE_ACTIVITIES_ON_SECONDARY_DISPLAYS`
([ActivityOptions.setLaunchDisplayId](https://developer.android.com/reference/android/app/ActivityOptions)).
Per the reference summary, this is ignored on devices without the feature; private or invalid display IDs throw
an exception. The API level was not visible verbatim and is believed to be 26. The test Thor declares
`android.software.activities_on_secondary_displays`, and its `am start` supports `--display <DISPLAY_ID>`
(read-only `adb shell pm list features` and `am help`).

Android's connected-displays guide says that when two activities are on different displays, both can be resumed
at the same time
([Support connected displays](https://developer.android.com/develop/adaptive-apps/guides/support-connected-displays)).
Where an activity launches is governed by the
[AOSP activity launch policy](https://source.android.com/docs/core/display/multi_display/activity-launch).
In theory, then, Moonlight's stream could run on the top screen (display 0) while ThorBro's own UI runs on
the bottom screen (display 4). **Whether Moonlight's `Game` activity behaves correctly when launched on a
specific display on the Thor is untested.**

**Background-start rules for an own app (goal b):** since Android 10, apps may only start activities
under listed conditions. The first is "The app has a visible window, such as an activity in the
foreground"
([Restrictions on starting activities from the background](https://developer.android.com/guide/components/activities/background-starts)).
An own ThorBro app that the user is looking at can therefore start the trampoline. A background service
of that app cannot, unless one of the other exceptions on that page applies.

---

## 2. Pairing

### Where Moonlight stores paired hosts and its identity

All of it is in Moonlight's **private app storage**:

- Host list: SQLite database `computers4.db`, opened with `Context.openOrCreateDatabase`, which lives in
  the app's private data directory
  ([ComputerDatabaseManager.java#L26](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/computers/ComputerDatabaseManager.java#L26),
  [#L49](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/computers/ComputerDatabaseManager.java#L49)).
- Client certificate and private key: `client.crt` / `client.key` in `getFilesDir()`
  ([AndroidCryptoProvider.java#L59-L62](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/binding/crypto/AndroidCryptoProvider.java#L59-L62)).
  Pairing means the host trusts this certificate.
- Client unique ID: file `uniqueid`
  ([IdentityManager.java#L15](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/computers/IdentityManager.java#L15)).
- App-list cache: `getCacheDir()` (see section 1).

### Can another app or adb pair on Moonlight's behalf?

- **Another app: no documented way.** Moonlight exposes no exported activity, service or broadcast receiver for
  pairing or adding hosts. `AddComputerManually` is not exported, and the only exported components are `PcView`,
  `ShortcutTrampoline` and a poster `ContentProvider`
  ([AndroidManifest.xml#L60-L118](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/AndroidManifest.xml#L60-L118)).
  Private storage is not readable by other apps.
- **adb: not practical.** `adb backup` would be the only non-root way into private data. For apps that target
  Android 12 (API 31) or higher, "app data is excluded" from `adb backup` unless the app is debuggable
  ([Android 12 behavior changes](https://developer.android.com/about/versions/12/behavior-changes-12)).
  Release Moonlight is not debuggable (no `debuggable` flag in
  [app/build.gradle](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/build.gradle)).
  **Not checked:** Moonlight's exact `targetSdk` value.
- **So pairing must be started inside Moonlight**, but the PC side can be automated:
  1. The user taps the host in Moonlight. Moonlight generates a random 4-digit PIN *on the Thor*
     ([PairingManager.java#L174-L179](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/nvstream/http/PairingManager.java#L174-L179))
     and shows it with "Please enter the following PIN on the target PC" plus a hint to use the Sunshine web UI
     ([PcView.java#L425-L430](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/PcView.java#L425-L430),
     [strings.xml#L41-L42](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/res/values/strings.xml#L41-L42)).
  2. ThorBro on the PC submits that PIN to Sunshine's API (below). The user still has to type the PIN once,
     into ThorBro instead of the Sunshine web UI.
  - **Unconfirmed, fragile idea:** read the PIN off the Thor screen over adb, for example with `uiautomator dump`.
    This is not a documented interface of Moonlight.
- If Thor and PC are on the same LAN, Moonlight finds Sunshine by itself through its mDNS discovery
  service ([AndroidManifest.xml#L171-L173](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/AndroidManifest.xml#L171-L173)).
  Otherwise the user has to add the PC manually; the Sunshine docs also note "In Moonlight, you may need to add
  the PC manually"
  ([Getting Started](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2getting__started.html),
  [file#L650-L680](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L650-L680)).

### Sunshine's PIN API

Yes, it is documented ([API docs](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2api.html),
[docs/api.md](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/api.md)). In the current
stable release, pairing is a **two-step** process:

1. `GET /api/pin` lists pending pairing requests as `{"pairings":[{"id","name","address"}]}`
   ([confighttp.cpp#L1746](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L1746)).
2. `POST /api/pin` with JSON `{"pairing_id": "<32 hex chars>", "pin": "1234", "name": "Thor"}`
   ([confighttp.cpp#L1810-L1870](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L1810-L1870)).
   The request needs `Content-Type: application/json`, stays open up to `ping_timeout` while Moonlight finishes
   the handshake, and returns `{"status": true|false}`.
   - The `pairing_id` field is not in older Sunshine versions, which took only `pin` and `name`. ThorBro has to
     handle both or require a minimum version. **Not checked:** which release introduced `pairing_id`.
   - `DELETE /api/pin` cancels a pairing
     ([route at #L2373](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L2373)).
     Paired clients are listed and removed with `GET /api/clients/list` and `POST /api/clients/unpair`
     ([#L2362-L2363](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L2362-L2363)).

**Authentication** ([`authenticate()`, confighttp.cpp#L595-L640](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L595-L640)):

- HTTPS on the web UI port (default 47990, see section 4) with a **self-signed certificate**, which the
  docs tell users to accept
  ([Getting Started, Configuration](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L650-L660)).
  ThorBro's HTTP client has to accept or pin that certificate.
- **HTTP Basic auth** with the web UI admin username and password. The password is compared as a salted hash.
- The caller's IP must be within `origin_web_ui_allowed` (`pc` = localhost only, `lan` = default, `wan`)
  ([configuration.md#L1623](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L1623)).
  ThorBro on the same PC is always allowed.
- If no admin user exists yet, every API call redirects to `/welcome`
  ([confighttp.cpp, `authenticate`](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L595-L640)).
  ThorBro can create the user headless with `sunshine --creds <user> <password>` (section 3).
- **CSRF:** POST and DELETE need a CSRF token **only if** the request carries an `Origin` or `Referer` header from a
  non-allowed origin. Requests without either header are allowed
  ([docs/api.md#L8-L21](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/api.md#L8-L21),
  [confighttp.cpp#L796-L830](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L796-L830)).
  **Consequence for ThorBro:** make these calls from Rust. A `fetch()` from the Tauri webview sends an `Origin`
  and would need the token from `GET /api/csrf-token`.

---

## 3. Sunshine: install, headless control, drivers, license

### Installation

**Windows** ([Getting Started → Windows](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L443-L514)):

- Recommended: the MSI `Sunshine-Windows-AMD64-installer.msi`, or `-ARM64-` (marked experimental) from the
  GitHub releases. The docs say: "The msi installer is preferred moving forward".
- The MSI runs `scripts\sunshine-setup.ps1 -Action install`; a `-Silent` variant exists
  ([sunshine-installer.wxs#L56-L57](https://github.com/LizardByte/Sunshine/blob/6b54f99fd8ef778474cf5da2f9e99a18ece9ec84/cmake/packaging/wix_resources/sunshine-installer.wxs#L56-L57),
  read at `master`). That script updates PATH, migrates the config, adds firewall rules, installs the service
  and configures autostart
  ([sunshine-setup.ps1](https://github.com/LizardByte/Sunshine/blob/6b54f99fd8ef778474cf5da2f9e99a18ece9ec84/src_assets/windows/misc/sunshine-setup.ps1)).
- The service is **`SunshineService`**, binary `tools\sunshinesvc.exe`, created with start type `demand`. The
  "Launch on Startup" component switches it to `auto`
  ([install-service.bat#L7-L13](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src_assets/windows/misc/service/install-service.bat#L7-L13),
  [autostart-service.bat#L4](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src_assets/windows/misc/autostart/autostart-service.bat#L4)).
- There is also a "lite" ZIP. The docs warn: "performance will be reduced… No support will be provided!"
  ([#L473-L514](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L473-L514)).
- **Not documented:** the MSI's silent-install properties for unattended setup, and whether `winget` packages
  exist. Both are unchecked.

**Arch Linux** ([Getting Started → ArchLinux](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L155-L189)):

- Prebuilt: add LizardByte's [pacman-repo](https://github.com/LizardByte/pacman-repo), then `pacman -S sunshine`.
- Or the PKGBUILD archive from the release: `sunshine.pkg.tar.gz` + `makepkg -si`.
- AUR: "Use AUR packages at your own risk." For NVIDIA, a locally built package needs CUDA installed *before*
  the build.
- Service: a **systemd user** unit, `systemctl --user start app-dev.lizardbyte.app.Sunshine` (alias
  `sunshine.service`); `--now enable` to start on boot
  ([#L537-L551](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L537-L551)).
  Input devices need the user in the `input` group
  ([Considerations](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L713-L740)).

### Headless control by another program

| Need | How | Source |
|---|---|---|
| Start/stop (Linux) | `systemctl --user start/stop app-dev.lizardbyte.app.Sunshine` | [getting_started.md#L537-L551](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L537-L551) |
| Start/stop (Windows) | the `SunshineService` service. The install scripts manage it with `sc`/`net`; **Not documented by Sunshine:** whether a non-elevated program can start or stop it (Windows normally requires admin rights for service control, unverified for this service's ACL) | [install-service.bat](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src_assets/windows/misc/service/install-service.bat) |
| Run without a service | `sunshine [options] [/path/to/sunshine.conf] [key=value ...]`. The docs say: "Running multiple instances of Sunshine is not advised." | [logging.cpp#L266-L273](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/logging.cpp#L266-L273), [getting_started.md#L605-L627](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L605-L627) |
| Config overrides on the command line | arguments containing `=` are parsed as config options | [config.cpp, argument loop](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/config.cpp#L1955-L2010) |
| Set admin credentials | `sunshine --creds <user> <password>` (writes `credentials_file`, default `sunshine_state.json`) | [entry_handler.cpp#L42-L49](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/entry_handler.cpp#L42-L49), [troubleshooting.md#L10](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/troubleshooting.md#L10), [configuration.md#L1893](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L1893) |
| Config file | `sunshine.conf` in `~/.config/sunshine` (Linux) or `%ProgramFiles%\Sunshine\config` (Windows); editable by hand | [configuration.md#L14-L37](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L14-L37) |
| Config via API | `GET`/`POST /api/config`, `POST /api/restart` | [docs/api.md](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/api.md), [confighttp.cpp#L1416](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L1416), [#L1470](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L1470), [#L1939](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L1939) |
| Apps | `apps.json` (path set by `file_apps`), or `GET`/`POST /api/apps`, `DELETE /api/apps/{index}`, `POST /api/apps/close` | [configuration.md#L1869](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L1869), [confighttp.cpp#L1025](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L1025), [#L1185](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L1185) |

Note on the Windows config path: under `%ProgramFiles%` it is not writable without admin rights. The
docs don't say how a non-elevated ThorBro should edit it, which suggests using the API (**Recommendation**).

### Drivers on Windows

- **Sunshine does not ship a gamepad driver.** The docs offer two backends: the **Virtual HID Driver**
  ("an optional paid upgrade", which "requires an active paid machine license"), and **ViGEmBus**, a separate
  download from [nefarius/ViGEmBus](https://github.com/nefarius/ViGEmBus/releases/latest) that must be version
  1.17 or newer and "has reached end of life"
  ([getting_started.md#L570-L603](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L570-L603),
  [troubleshooting.md#L337-L351](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/troubleshooting.md#L337-L351)).
  The option `gamepad_driver = all | virtualhid | vigembus` selects between them
  ([configuration.md#L316](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L316)),
  and `GET /api/virtual-input/status` reports both drivers
  ([confighttp.cpp#L2032](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L2032)).
  On `master` the paid driver has been renamed "Virtual HID Broker".
- **Audio:** `install_steam_audio_drivers` (default enabled) installs the "Steam Streaming Speakers" driver
  *if Steam is installed*
  ([configuration.md#L900](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L900)).
- **No virtual display driver is mentioned** in the stable Sunshine docs. Sunshine captures with the Desktop
  Duplication API "which only supports capturing from the GPU used for display"
  ([Considerations](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/getting_started.md#L713-L720)).
- The MSI itself adds firewall rules and a service (above). **Not checked:** whether the MSI installs anything else
  as a driver.

### License, and bundling with ThorBro

- Sunshine is **GPL-3.0**
  ([LICENSE](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/LICENSE),
  [Legal](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2legal.html)).
  Sunshine's legal page also warns that the GPL "does not grant any rights to distribute or sell the encoders
  contained within Sunshine", and that a commercial distributor may need MPEG-LA or similar licenses
  ([docs/legal.md](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/legal.md)).
- moonlight-android and moonlight-common-c are also **GPL-3.0**
  ([moonlight-android LICENSE.txt](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/LICENSE.txt),
  [moonlight-common-c LICENSE.txt](https://github.com/moonlight-stream/moonlight-common-c/blob/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb/LICENSE.txt)).

> **This is not legal advice.** It summarizes what the license texts and the FSF FAQ say. Ask a lawyer before
> shipping.

**Can Sunshine be shipped inside the Tauri bundle next to MIT-licensed ThorBro?** Under GPLv3 that is an
**"aggregate"**: "A compilation of a covered work with other separate and independent works, which are not by
their nature extensions of the covered work, and which are not combined with it such as to form a larger program
… Inclusion of a covered work in an aggregate does not cause this License to apply to the other parts of the
aggregate" ([GPLv3 §5, last paragraph](https://www.gnu.org/licenses/gpl-3.0.html)). The FSF FAQ says the GPL
permits distributing such an aggregate. Whether two parts are separate programs is "a legal question, which
ultimately judges will decide". Pipes, sockets and command-line arguments "are communication mechanisms normally
used between two separate programs"
([GPL FAQ](https://www.gnu.org/licenses/gpl-faq.html), entries on aggregates and on separate programs).
ThorBro would talk to Sunshine only over its HTTP API and by starting the process, which matches that description.

Conditions when ThorBro **redistributes** the Sunshine binaries:

1. Ship Sunshine's license text and keep its notices.
2. Provide the **Corresponding Source** of exactly the shipped version, for example under GPLv3 §6(d). Download
   access works if equivalent access to the source is offered; the source may be on a third-party server
   "provided you maintain clear directions next to the object code saying where to find the Corresponding Source",
   and you "remain obligated to ensure that it is available"
   ([GPLv3 §6](https://www.gnu.org/licenses/gpl-3.0.html)).
3. Don't put the aggregate under terms that restrict the GPL rights in Sunshine (aggregate definition above).
4. Sunshine's own bundled third-party components (FFmpeg and others) bring their own licenses. **Not checked:**
   the full list in the Windows MSI.

**Recommendation:** don't bundle. Let ThorBro download the official MSI or point to the pacman-repo, then talk to
the installed Sunshine. That avoids the source-offer duty and also the encoder-licensing question from
Sunshine's legal page.

---

## 4. Detecting Sunshine and listing its apps

### Ports

All ports are offsets from `port` (default **47989**, range 1029–65514)
([configuration.md#L1595](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L1595)):

| Port (default) | Proto | Purpose | Offset source |
|---|---|---|---|
| 47989 | TCP | GameStream HTTP (`/serverinfo`, `/pair`) | `PORT_HTTP = 0` [nvhttp.h#L47](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/nvhttp.h#L47) |
| 47984 | TCP | GameStream HTTPS (paired clients) | `PORT_HTTPS = -5` [nvhttp.h#L52](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/nvhttp.h#L52) |
| 47990 | TCP | Web UI + admin API (HTTPS) | `PORT_HTTPS = 1` [confighttp.h#L29](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.h#L29) |
| 48010 | TCP | RTSP setup | `RTSP_SETUP_PORT = 21` [rtsp.h#L15](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/rtsp.h#L15) |
| 47998 / 47999 / 48000 | UDP | video / control / audio | offsets 9 / 10 / 11 [stream.h#L19-L21](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/stream.h#L19-L21) |

Moonlight's port-forwarding list agrees: TCP 47984, 47989, 48010; UDP 47998, 47999, 48000, 48002, 48010
([Setup Guide → Manual port forwarding](https://github.com/moonlight-stream/moonlight-docs/wiki/Setup-Guide#manual-port-forwarding-advanced)).
The UDP ports are "only active during streaming".

### Is it running, and is it reachable?

Three levels, cheapest first (**Recommendation** built on the sources):

1. **Process/service check on the PC:** `systemctl --user is-active app-dev.lizardbyte.app.Sunshine` on Linux,
   the `SunshineService` state on Windows (sources in section 3).
2. **Unauthenticated: `GET http://<host>:47989/serverinfo`.** This is the same request Moonlight uses. It returns
   XML including `hostname`, `appversion`, **`uniqueid`**, `HttpsPort`, `currentgame` and
   `state` (`SUNSHINE_SERVER_BUSY` / `SUNSHINE_SERVER_FREE`)
   ([nvhttp.cpp#L1207-L1275](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/nvhttp.cpp#L1207-L1275);
   HTTP route [#L1711](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/nvhttp.cpp#L1711)).
   `uniqueid` is exactly what Moonlight stores as the host UUID
   ([NvHTTP.java#L370](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/nvstream/http/NvHTTP.java#L370)),
   so ThorBro can read it here and pass it as the trampoline's `UUID` extra (section 1).
   - Called from the **Thor** (for example `adb shell` with curl, or the own app), this also proves that the Thor
     reaches the PC over the network, which is the real reachability question.
   - **Not checked:** whether the Thor's Android shell has `curl` or `wget`.
3. **Authenticated admin API on 47990:** with credentials, `GET /api/config`, `GET /api/apps` and so on prove the
   admin API works (section 2 for auth).

### Listing apps

- **`GET /api/apps`** (admin API, Basic auth) returns the contents of `apps.json`
  ([confighttp.cpp#L1025-L1070](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/confighttp.cpp#L1025-L1070)).
  Each entry has `name`, `cmd`, `image-path` and so on, **but no app ID**. Sunshine computes IDs at load time
  ([process.cpp#L633-L660](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/process.cpp#L633-L660)).
- **`GET https://<host>:47984/applist`** (GameStream protocol) returns IDs, but it is served on the HTTPS GameStream
  server for **paired clients**
  ([nvhttp.cpp#L1696](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/src/nvhttp.cpp#L1696)).
  ThorBro would have to pair as its own client to use it. **Not checked in detail:** the client-certificate check in
  `applist`.
- **Practical consequence (Recommendation):** ThorBro lists apps by name through `/api/apps` and starts them on the
  Thor with `AppName`. To make sure Moonlight's app-list cache is fresh, the first-run flow should have the user open
  the host once in Moonlight (section 1 caveat).

---

## 5. Low latency

### Officially recommended settings

Sunshine's [Performance Tuning](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2performance__tuning.html)
page is short ([file](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/performance_tuning.md)):

- AMD on Windows: enabling *Enhanced Sync* "may help reduce the latency by an additional frame".
- NVIDIA: enabling *Fast Sync* "may help reduce latency".

Latency-related Sunshine options, with what their docs say:

- `nvenc_latency_over_power` (default enabled, Windows + NVENC): "Disabling this is not recommended since this can
  lead to significantly increased encoding latency"
  ([configuration.md#L2512](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L2512)).
- `nvenc_preset` (default 1): higher numbers "improve compression … at the cost of increased encoding latency"
  ([#L2288](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L2288)).
- `minimum_fps_target`: controls duplicate frames for static content
  ([#L1471](https://github.com/LizardByte/Sunshine/blob/v2026.914.233613/docs/configuration.md#L1471)).

Moonlight's official guidance ([wiki](https://github.com/moonlight-stream/moonlight-docs/wiki)):

- Network: "5 GHz WiFi 5 (802.11ac) or WiFi 6 (802.11ax) strongly recommended", and "Ethernet/wired connections
  highly recommended" for the PC ([Setup Guide](https://github.com/moonlight-stream/moonlight-docs/wiki/Setup-Guide)).
  Also "Make sure your client device is connected on 5 GHz WiFi or Ethernet, and your PC is wired to your router if
  possible". If the PC has both Ethernet and Wi-Fi, disconnect all but the fastest
  ([Troubleshooting](https://github.com/moonlight-stream/moonlight-docs/wiki/Troubleshooting#video-is-choppy-or-laggy)).
- If video gets stuck: "disable hardware-accelerated GPU scheduling on your host PC"; lower the bitrate to test
  whether bandwidth is the problem (same page).
- Android frame pacing: "Prefer lowest latency" renders immediately after decoding; "Balanced (recommended)" buffers
  up to one frame; "Prefer smoothest video" can lead to "very high latency"
  ([FAQ](https://github.com/moonlight-stream/moonlight-docs/wiki/Frequently-Asked-Questions#what-do-the-frame-pacing-options-on-the-android-client-mean)).
- "Increasing the frame rate to 90 or 120 FPS may decrease decode latency on some devices"
  ([FAQ, stats](https://github.com/moonlight-stream/moonlight-docs/wiki/Frequently-Asked-Questions#what-do-the-numbers-mean-in-the-on-screen-connection-statistics)).
  The Thor's top screen supports 60 and 120 Hz at 1920×1080 (read-only `dumpsys display` on the test Thor), so
  1080p120 is worth testing.

### Measuring latency on the Moonlight (Android) side

- **On-screen stats:** Settings → "Show performance stats while streaming"
  (pref key `checkbox_enable_perf_overlay`, default off)
  ([PreferenceConfiguration.java#L51](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/preferences/PreferenceConfiguration.java#L51),
  [#L90](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/preferences/PreferenceConfiguration.java#L90),
  [strings.xml#L253-L254](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/res/values/strings.xml#L253-L254)).
  The overlay shows host processing latency (min/max/avg), network latency and variance, average decoding time,
  frames dropped by the network, and incoming and rendering FPS
  ([strings.xml#L108-L115](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/res/values/strings.xml#L108-L115)).
  What each number means: [FAQ](https://github.com/moonlight-stream/moonlight-docs/wiki/Frequently-Asked-Questions#what-do-the-numbers-mean-in-the-on-screen-connection-statistics).
- **After the stream:** "Show latency message after streaming" (`checkbox_enable_post_stream_toast`) shows the average
  frame decoding latency when the stream ends
  ([PreferenceConfiguration.java#L62](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/preferences/PreferenceConfiguration.java#L62),
  [Game.java#L1095-L1100](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/Game.java#L1095-L1100),
  [strings.xml#L86-L88](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/res/values/strings.xml#L86-L88)).
- **Limits** (FAQ): compositor, display and input-device latency "can't be measured by apps … you should always measure
  using external testing hardware", and the numbers aren't comparable to other clients
  ([FAQ](https://github.com/moonlight-stream/moonlight-docs/wiki/Frequently-Asked-Questions#what-do-the-numbers-mean-in-the-on-screen-connection-statistics)).
  **Recommendation:** for end-to-end numbers, film the PC monitor and the Thor side by side with a high-speed phone
  camera.
- Both toggles are in Moonlight's private SharedPreferences, so ThorBro cannot switch them on remotely (section 2).

---

## 6. Effort: (A) orchestrator around stock Moonlight vs (B) own streaming module

**ThorBroAndroid folder:** visible at `/home/unterhose/System/ThorBroAndroid`. It has one commit ("Grundgerüst"):
a Kotlin Multiplatform project with `core` (models, game-system detection, library finder, file-system interface
with adb and local implementations), `desktop` (Compose Desktop) and `android` (an empty Compose `MainActivity`),
about 220 lines of Kotlin. Its Android manifest only requests `MANAGE_EXTERNAL_STORAGE` and has no `INTERNET`
permission. Its version catalog says it has "not yet been tested with a build". There is no networking, no
Moonlight-related code and no native (NDK) setup.

All effort numbers below are **Estimates** for one experienced developer and assume a working build.

### (A) Orchestrator app around stock Moonlight

The own app starts `com.limelight/.ShortcutTrampoline` with `UUID` + `AppName` (section 1). ThorBro on the PC does
the Sunshine side.

Work items:

1. **PC side (ThorBro, Rust):**
   - Sunshine detection (service/process plus `/serverinfo`).
   - Install helper: download the MSI, or point to the pacman-repo.
   - First-run credentials via `--creds`.
   - Admin-API client: self-signed TLS, Basic auth, no Origin header.
   - PIN flow: `GET /api/pin` → user types the PIN Moonlight shows → `POST /api/pin`.
   - App list through `/api/apps`.
   - UI tab.
   - **Estimate: 5–8 days.**
2. **Thor side via adb (no own app yet):**
   - Check that Moonlight is installed.
   - Read the `uniqueid` from `/serverinfo`.
   - Start the trampoline with `adb shell am start`, optionally with `--display 0`.
   - **Estimate: 1–2 days.** This already gives a usable "Play on Thor" button.
3. **Own Android app (goal b):**
   - Add the `INTERNET` permission.
   - Add `<queries><package android:name="com.limelight"/></queries>` to check for Moonlight. Package visibility
     applies to apps that target Android 11+ and query installed packages
     ([Package visibility](https://developer.android.com/training/package-visibility)). The page doesn't say that a
     plain explicit `startActivity` needs it.
   - Host discovery: mDNS or the address from ThorBro.
   - Game list from Sunshine: through ThorBro, or with Sunshine credentials on the Thor.
   - Launch intent; UI on the bottom screen while the stream runs on the top screen (Unconfirmed, section 1).
   - First-run guidance for pairing and opening the host once in Moonlight.
   - **Estimate: 1.5–3 weeks** on top of the skeleton, including Compose UI.

Limits of (A), all from section 1–2:

- Stream settings can't be set from outside (apart from HDR).
- The stream ends when Moonlight leaves the screen.
- Moonlight's dialogs (quit the running game? not paired? offline?) will appear.
- Pairing still needs one manual PIN step.
- Launching by `AppName` depends on Moonlight's app-list cache.

There is no GPL coupling: Moonlight stays a separate app, controlled through an Intent (see the FAQ statement on
separate programs in section 3; not legal advice).

### (B) Own streaming module on moonlight-common-c + MediaCodec

moonlight-common-c "contains the core GameStream client code" and requires its bundled ENet fork ("Attempting to
runtime link to another libenet library will cause your client to crash")
([README](https://github.com/moonlight-stream/moonlight-common-c/blob/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb/README.md),
[.gitmodules](https://github.com/moonlight-stream/moonlight-common-c/blob/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb/.gitmodules)).
It handles the stream after launch: `LiStartConnection` takes a `SERVER_INFORMATION` with fields that come from
`/serverinfo` and the RTSP session URL from `/launch`/`/resume`, plus video, audio and connection callbacks, and
it sends input
([Limelight.h#L536-L568](https://github.com/moonlight-stream/moonlight-common-c/blob/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb/src/Limelight.h#L536-L568),
[#L313](https://github.com/moonlight-stream/moonlight-common-c/blob/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb/src/Limelight.h#L313),
[#L367](https://github.com/moonlight-stream/moonlight-common-c/blob/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb/src/Limelight.h#L367),
[#L494](https://github.com/moonlight-stream/moonlight-common-c/blob/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb/src/Limelight.h#L494),
[#L720-L790](https://github.com/moonlight-stream/moonlight-common-c/blob/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb/src/Limelight.h#L720-L790)).
**Pairing, `/serverinfo`, `/applist` and `/launch` are not part of it.** moonlight-android implements those in Java.

Size of what moonlight-android builds around the library, as a yardstick (line counts at `b48494c`):

| Component | moonlight-android file | Lines |
|---|---|---|
| GameStream HTTP (serverinfo, applist, launch) | [NvHTTP.java](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/nvstream/http/NvHTTP.java) | 820 |
| Pairing handshake | [PairingManager.java](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/nvstream/http/PairingManager.java) | 334 |
| Client certificate | [AndroidCryptoProvider.java](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/binding/crypto/AndroidCryptoProvider.java) | 259 |
| Connection glue | [NvConnection.java](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/nvstream/NvConnection.java) | 591 |
| Video decode (MediaCodec) | [MediaCodecDecoderRenderer.java](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/binding/video/MediaCodecDecoderRenderer.java) + [MediaCodecHelper.java](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/binding/video/MediaCodecHelper.java) | 1924 + 1017 |
| Audio | [AndroidAudioRenderer.java](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/binding/audio/AndroidAudioRenderer.java) + Opus via JNI | 233 + native |
| Gamepad input | [ControllerHandler.java](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/binding/input/ControllerHandler.java) | 3301 |
| Stream activity | [Game.java](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/Game.java) | 2700 |
| JNI bridge | [jni/moonlight-core/*.c](https://github.com/moonlight-stream/moonlight-android/tree/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/jni/moonlight-core) (+ OpenSSL, Opus builds) | ~1000 |

Work items for (B):

1. NDK/CMake setup in ThorBroAndroid: moonlight-common-c with its ENet and nanors submodules, OpenSSL or another
   crypto library, Opus. JNI bridge for the callbacks.
2. GameStream HTTP client and pairing (certificate generation, PIN challenge), host storage.
3. Video: MediaCodec decoder for H.264/HEVC/AV1 on a `Surface`, low-latency flags, frame pacing.
4. Audio: Opus decode, then AudioTrack or AAudio.
5. Input: the Thor's built-in controls mapped to `LiSendMultiControllerEvent`, plus touch and mouse if wanted.
6. Lifecycle, reconnect, error handling, stats overlay.

**Estimate: 2–4 months** to reach a stable stream on one device (the Thor), even when reusing moonlight-android code.

**License consequence:** the own app links GPL-3.0 code (moonlight-common-c), so that app has to be GPL-3.0 too
([LICENSE.txt](https://github.com/moonlight-stream/moonlight-common-c/blob/b4ee1cea3d19e13c0b37c26585e6b1ee174effcb/LICENSE.txt);
not legal advice). The rest of ThorBroAndroid would be affected if it ships in the same APK.

What (B) gains over (A): full control of stream settings, its own UI around the stream, no Moonlight dialogs, and
the stream and the bottom-screen UI in one app. Whether that's worth months of work is the main decision.

---

## Recommended order

1. **On a real Thor (half a day, no code):**
   - Install Moonlight manually and pair it with Sunshine by hand.
   - Run the `am start … ShortcutTrampoline` example (section 1).
   - Try `--display 0` and start something on display 4 at the same time.
   - Measure with the stats overlay at 1080p60 and 1080p120.
   - This answers most open questions below before any code exists.
2. **ThorBro PC tab, read-only first:**
   - Detect Sunshine (service plus `/serverinfo`).
   - Show version, state and current game.
   - Link to the official installer / pacman-repo instead of bundling (section 3).
3. **ThorBro: credentials and pairing:**
   - `--creds` on first run; store the credentials in the OS keychain (**Recommendation**).
   - `GET /api/pin` → PIN field → `POST /api/pin`.
4. **ThorBro: app list plus "Play on Thor":** `/api/apps` → `adb shell am start` with `UUID` + `AppName`.
   - This delivers goal (a) without any Android app.
5. **Own Android app as an orchestrator (A):** same launch from the Thor, UI on the bottom screen if step 1 shows it
   works.
6. **Decide on (B)** only if (A)'s limits (no stream settings, Moonlight dialogs, foreground-only) turn out to
   matter in practice.

## Open questions that need a real Thor (or a real setup)

1. Does `am start -n com.limelight/.ShortcutTrampoline` with `--es UUID … --es AppName …` start the stream as
   the code suggests? (Moonlight wasn't installed on the test Thor.)
2. Can Moonlight's `Game` activity be placed on display 0 (`--display 0` / `setLaunchDisplayId`) while another app
   stays resumed on display 4? Does Moonlight's input capture still get the Thor's controls?
3. Is picture-in-picture useful on the Thor, or does it break the stream?
4. Which codec and frame rate give the lowest decode latency on the Thor's chip (HEVC vs AV1, 60 vs 120 FPS)?
   Use the stats overlay.
5. Does the Thor reach the PC's port 47989 on the user's network (Wi-Fi isolation, firewall)? Does the Android
   shell have `curl` for a check over adb?
6. Does Moonlight's mDNS discovery find Sunshine on the user's network, or is manual entry needed?
7. Does the Thor's built-in controller show up as a gamepad in Moonlight, and which `gamepad_driver` does it need on
   Windows (ViGEmBus vs the paid Virtual HID Driver)?
8. Not Thor-specific, but unverified: whether a non-admin ThorBro can start or stop `SunshineService` on Windows;
   the MSI's silent-install options; and which Sunshine release introduced `pairing_id` in `POST /api/pin`.
