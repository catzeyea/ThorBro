# Stream profile (format `thorbro.stream-profile`, version 1)

A small JSON document that ThorBro on the PC builds from what it knows about Sunshine. It is meant
for the ThorBro app on the Thor (separate repo, ThorBroAndroid), so that app can start PC games in
Moonlight without asking the user for anything ThorBro already knows.

Built by `pcstream::profile::build_profile` (`src/pcstream/profile.rs`). The Stream tab shows it
under "Stream PC games to your Thor" → "Stream profile" and sends it with "Send to Thor app"
(see "How the profile gets to the Thor" below). Its unit tests compare the output against the
example below, so change both together.

## Example

```json
{
  "format": "thorbro.stream-profile",
  "version": 1,
  "pc": {
    "name": "my-pc",
    "uniqueid": "0123ABCD-4567-89EF-0123-456789ABCDEF",
    "lan_address": "192.168.1.20",
    "tailscale_address": "100.101.102.103",
    "tailscale_name": "my-pc.tail1234.ts.net",
    "sunshine_port": 47989
  },
  "games": ["Desktop", "Steam Big Picture"]
}
```

## Fields

| Field | Type | Meaning | Source on the PC |
|---|---|---|---|
| `format` | string | Always `"thorbro.stream-profile"`. Lets a reader reject unrelated JSON. | constant |
| `version` | integer | Format version, currently `1`. | constant |
| `pc.name` | string | The PC name Moonlight shows. May be empty. | `hostname` from Sunshine's `GET /serverinfo` |
| `pc.uniqueid` | string | Sunshine's host ID. Moonlight stores it as the host UUID; it is the `UUID` extra of `com.limelight/.ShortcutTrampoline`. | `uniqueid` from `/serverinfo` |
| `pc.lan_address` | string or null | The PC's recommended LAN IPv4 address (private range, not a container/VM adapter). | network interfaces |
| `pc.tailscale_address` | string or null | The PC's Tailscale IPv4 address. Null if Tailscale isn't connected on the PC or its CLI is missing. | `tailscale ip -4` |
| `pc.tailscale_name` | string or null | MagicDNS name without the trailing dot. Null if MagicDNS is off or Tailscale isn't connected. | `Self.DNSName` from `tailscale status --json` |
| `pc.sunshine_port` | integer | Sunshine's GameStream HTTP port (default 47989). In Moonlight's "Add PC" it goes after the address as `address:port`, but only if it isn't 47989. | `port` in sunshine.conf |
| `games` | array of strings | Sunshine's app names, in Sunshine's order, exactly as Sunshine has them. Each one is a valid `AppName` extra for the trampoline. Empty if ThorBro has no working Sunshine login. | Sunshine's `GET /api/apps` |

## Rules

- **No credentials, ever.** No Sunshine username or password, no pairing PIN, no certificates or
  keys. Pairing stays inside Moonlight (see `PC-STREAMING-RESEARCH.md`, section 2). A test
  checks that the JSON contains none of these words.
- **Readers ignore unknown fields.** New optional fields can be added without changing `version`.
- **`version` changes only for breaking changes**: a field removed, renamed, or its meaning or type
  changed. A reader must refuse a `version` it doesn't know.
- **Addresses are hints, not truth.** The PC's addresses can change (DHCP, another network). The
  stable key is `pc.uniqueid`. A Thor app should prefer letting Moonlight find the PC by itself and
  use the addresses only to help the user add it.
- **Game names are matched exactly** (case and spaces), because Moonlight compares `AppName` with
  `String.equals` against its cached app list. That cache is only filled after the user opened the
  PC's game list in Moonlight once (research doc, section 1).

## How a Thor app would use it

Starting a game is the same intent ThorBro sends over adb today (`src/pcstream/thor.rs`):

```
component: com.limelight/com.limelight.ShortcutTrampoline
action:    android.intent.action.VIEW
extras:    UUID    = pc.uniqueid   (String)
           AppName = games[i]      (String)
```

Both extras must be strings (`putExtra(String, String)`), because Moonlight reads them with
`getStringExtra`.

## How the profile gets to the Thor (method A: `am start` with `profile_b64`)

Decided: method A. ThorBro starts the Thor app's main activity over adb and passes the profile as a
string extra. No file is written on the Thor, so ThorBro needs no storage permission there and the
app needs none either.

```sh
adb shell am start \
  -n dev.catzeyea.thorbro/dev.catzeyea.thorbro.android.MainActivity \
  --es profile_b64 <BASE64>
```

- **Component**: `dev.catzeyea.thorbro` is the app's `applicationId` (ThorBroAndroid,
  `android/build.gradle.kts`). The manifest declares the activity as `.MainActivity`, which Android's
  build resolves against the Gradle `namespace` `dev.catzeyea.thorbro.android`, not against the
  applicationId. On the PC side the value lives only in `thor::THOR_APP_COMPONENT`
  (`src/pcstream/thor.rs`); the app has the same string as `MainActivity.COMPONENT_FOR_ADB`.
- **Payload**: the profile as compact JSON (UTF-8), base64 with the standard alphabet
  (`A-Z a-z 0-9 + /`), `=` padding and no line breaks. Game names with quotes, `$`, backticks or
  spaces therefore never reach the Thor's shell as such. Every argument still goes through
  `adb::shell_quote`, like "Play on Thor".
- **Checks before sending**: exactly one Thor connected; the app is installed (`pm list packages`,
  exact package match), otherwise ThorBro says so and points to the APK; no field name in the
  profile looks like a credential (`profile::credential_field`, a superset of the app's own list);
  the base64 text is at most `thor::MAX_PROFILE_B64_CHARS` (24,576) characters.
- **No answer from the app.** `am start` only reports that Android delivered the intent. ThorBro
  therefore says "sent", never "accepted". The app validates the profile, stores it and shows the
  result on its Status tab; a refused profile shows up there as "Profile rejected: …", and the
  previous profile stays in place.
- If the app is already open, the intent goes to the running activity (`launchMode="singleTop"`);
  `am start` then prints "intent has been delivered to currently running top-most instance", which
  ThorBro treats as sent.

### Size

The base64 text is one argument of one `adb shell` command line. Limits on its way, smallest first:

| Where | Limit |
|---|---|
| Windows, `CreateProcess` for `adb.exe` | 32,767 characters for the whole command line, adb path included |
| adb client → adb server | request framed with a 4-digit hex length: 65,535 bytes for `shell:<command>` |
| Thor, `sh -c <command>` | 131,072 bytes per argument (Linux `MAX_ARG_STRLEN`) |
| Thor app (`ProfileParser`) | 256 KiB of JSON |

ThorBro's limit is 24,576 base64 characters (about 18 KiB of JSON), below all of them. Measured on
2026-11-23 (`cargo test measure_profile_size -- --ignored --nocapture`, typical game titles of
18.7 characters on average):

| Games | Pretty JSON | Compact JSON | Base64 | Whole `am start` command |
|---:|---:|---:|---:|---:|
| 2 | 358 B | 289 B | 388 | 480 |
| 20 | 852 B | 693 B | 924 | 1,016 |
| 100 | 2,988 B | 2,429 B | 3,240 | 3,332 |
| 250 | 6,947 B | 5,638 B | 7,520 | 7,612 |
| 500 | 13,668 B | 11,109 B | 14,812 | 14,904 |

The limit is reached at about 830 games of that length. The real profile of a test PC (Sunshine with
3 apps) was 317 B compact JSON, 424 base64 characters. Above the limit ThorBro sends nothing and asks
the user to remove apps they don't stream from Sunshine's app list.

### Format on the app side

The app's parser (`ProfileParser` in ThorBroAndroid, described in its `docs/STREAM-PROFILE-ANDROID.md`)
reads this format and rejects the whole profile, keeping the previous one, if:

- `format` is not `"thorbro.stream-profile"` or `version` is not `1`;
- any field name is one of `password`, `passwd`, `pin`, `token`, `secret`, `credentials`, `apiKey`,
  `api_key`, `auth` (case-insensitive, any depth);
- `pc.uniqueid` is missing or not in 8-4-4-4-12 hex form;
- a game name is not a string or is blank;
- `pc.sunshine_port` is not a whole number from 1 to 65535;
- the JSON is larger than 256 KiB.
