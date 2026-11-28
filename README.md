<p align="center">
  <img src="docs/thorbro-icon-transparent.svg" width="160" alt="ThorBro mascot">
</p>
<!--
  Dark-mode variant, prepared but not active: docs/thorbro-icon-transparent-dark.svg doesn't exist yet.
  Once it does, replace the <img> block above with this:

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/thorbro-icon-transparent-dark.svg">
    <img src="docs/thorbro-icon-transparent.svg" width="160" alt="ThorBro mascot">
  </picture>
</p>
-->

<h1 align="center">ThorBro</h1>

<p align="center">
  A desktop app for the <b>AYN Thor</b>. Manage files and games on the Thor,<br>
  sync save files with your PC, and stream between PC and Thor. Over USB, with adb.
</p>

<p align="center">
  <a href="../../releases/latest"><b>⬇ Download</b></a> ·
  <a href="#setup">Setup</a> ·
  <a href="#troubleshooting">Troubleshooting</a>
</p>

---

## What it is

ThorBro runs on Windows, Linux and macOS. It talks to the Thor over a USB cable through adb.
You need **USB debugging** turned on on the Thor. Nothing else has to be installed on the Thor.

ThorBro is a fan project. It is not affiliated with AYN.

## Features

| Area | What it does |
|---|---|
| **Start** | Shows the Thor's battery, storage (internal and SD card), CPU load, RAM and CPU temperature. The ThorBro mascot shows a mood based on these values. Lists your games per console, with covers. Shows your pinned folders. |
| **Files** | File manager for the Thor's internal storage and SD card. Multi-select, cut/copy/paste, drag into folders, right-click menu, sorting, pinned folders. |
| **Game sorting** | Drop a game anywhere in the window. ThorBro detects the system and suggests the matching folder in your games library. Switch updates and DLC can go to `switch/update`. |
| **Unpacking** | Extracts zip, 7z and tar itself, before copying or in place on the Thor. File names are made SD-card safe. |
| **File pool** | Collect files from the Thor, then copy them all to the PC in one go. |
| **Import ES-DE info** | Writes ES-DE `systeminfo.txt` files into console folders that ES-DE didn't create. |
| **Saves** | Push saves PC → Thor or Thor → PC with one click. Shows which side is newer. **Makes a backup before every overwrite.** Imports saves from archives. |
| **Mirror screens** | Mirrors the top screen, the bottom screen or both to your PC (via scrcpy). View only. |
| **Stream PC games** | Play PC games on the Thor with [Sunshine](https://github.com/LizardByte/Sunshine) and [Moonlight](https://github.com/moonlight-stream/moonlight-android). ThorBro checks the setup, helps with pairing and starts games on the Thor. |
| **Command line** | Mirror screens and sync saves from a terminal. |

## Screenshots

<!--
  Placeholders. Add real screenshots to docs/screenshots/ and uncomment the lines below.

  ![Start tab](docs/screenshots/start.png)
  ![Files tab](docs/screenshots/files.png)
  ![Saves tab](docs/screenshots/saves.png)
  ![Stream tab](docs/screenshots/stream.png)
-->

_Screenshots will follow._

---

## Setup

### 1. On the Thor: turn on USB debugging

**ThorBro can't see the Thor without USB debugging.** You only do this once.

1. Open **Settings → About device**. Tap **Build number** 7 times until it says you are a developer.
2. Open **Settings → System → Developer options**. Turn on **USB debugging**.
3. Connect the Thor to your PC with a USB cable.
4. The Thor asks *"Allow USB debugging?"*. Tick **"Always allow from this computer"** and tap **OK**.

### 2. On your PC: install ThorBro

Get the file for your system from the [Releases page](../../releases/latest).

| System | File | Notes |
|---|---|---|
| **Windows 10/11** | `ThorBro_x.y.z_x64-setup.exe` | Installer. adb and scrcpy are included. |
| | `ThorBro-x.y.z-windows-portable.zip` | No install. Unzip and run `ThorBro.exe`. Keep the `tools` folder next to it. |
| **Linux** | `ThorBro_x.y.z_amd64.AppImage` | Any distro. Install adb and scrcpy yourself. |
| | `.deb` / `.rpm` | Ubuntu/Debian/Mint or Fedora/openSUSE. Install adb and scrcpy yourself. |
| **macOS** *(experimental)* | `.dmg` | Untested. Only attached if the macOS build succeeded. Otherwise [build from source](#build-from-source). Install adb and scrcpy with Homebrew. |

<details>
<summary><b>Windows</b></summary>

ThorBro isn't signed with a paid certificate. SmartScreen may say *"Windows protected your PC"*.
Click **More info**, then **Run anyway**.

If Windows doesn't see the Thor at all, install the [Google USB Driver](https://developer.android.com/studio/run/win-usb) and plug the Thor back in.
</details>

<details>
<summary><b>Linux</b></summary>

Install adb and scrcpy:

```bash
# Arch / Manjaro / CachyOS
sudo pacman -S android-tools android-udev scrcpy

# Ubuntu / Debian / Mint / Pop!_OS
sudo apt install adb scrcpy

# Fedora
sudo dnf install android-tools scrcpy

# openSUSE
sudo zypper install android-tools scrcpy
```

Make the AppImage executable, then start it:

```bash
chmod +x ThorBro_*.AppImage
./ThorBro_*.AppImage
```

- The AppImage won't start and mentions *FUSE*: install `fuse2` (Arch) or `libfuse2` (Ubuntu).
- The `.deb` and `.rpm` need WebKitGTK 4.1 and GTK 3. The package manager installs them. The AppImage bundles them.
</details>

<details>
<summary><b>macOS</b></summary>

Install adb and scrcpy:

```bash
brew install android-platform-tools scrcpy
```

**The current macOS build is unsigned.** macOS may block it or say *"ThorBro is damaged and can't be opened"*.
The app is not damaged. macOS blocks unsigned apps downloaded from the internet.

Move ThorBro to **Applications**, then run this once in Terminal:

```bash
xattr -dr com.apple.quarantine /Applications/ThorBro.app
```

This removes the download flag, so macOS opens the app. It is only needed for unsigned builds.
</details>

### 3. Start ThorBro

Start ThorBro and plug in the Thor. The status in the top right shows the device name once the Thor is found.
Click it to reconnect.

---

## Usage

### Start

- **Thor status:** battery, internal storage, SD card, CPU load, RAM and CPU temperature.
  Values that can't be read are left out, e.g. the SD card when none is inserted.
  Readings refresh every 4 seconds while the Start tab is open.
- **ThorBro's mood:** the mascot on Start and in the header reacts to the status.
  Working hard at 80 % CPU or 70 °C and up. Tired at 20 % battery or less when not charging.
  Stuffed when internal storage is at least 90 % full. Asleep when no Thor is connected.
- **Games:** one block per console folder, with the installed emulators and square covers.
  Covers come from the iiSU launcher art on your Thor. ThorBro ships no art.
  Click a game to open its folder in Files.
- **Pinned folders:** one click opens them in Files.
- **Drop files anywhere:** works on every tab. Games go to their console folder, other files to the folder open in Files.

### Files and games

- **Add files:** drag files or folders into the window. Or press Ctrl+O, or right-click the list → *Add files …* / *Add folder …*.
  If ThorBro recognizes a game, it suggests a target folder (★). You can change it before copying.
  Other files go into the open folder.
- **Games library:** ThorBro looks for ES-DE `ROMs/` folders on internal storage and the SD card.
  ES-DE creates them on its first launch. For any other folder, open it and press **Set games folder**.
- **Archives:** tick *Extract first* when dropping. It is pre-ticked for systems whose emulator can't read packed games.
  rar and tar.xz need [7-Zip](https://www.7-zip.org) (or `unrar` / `bsdtar`) on the PC.
- **Organize:** select with click, Ctrl+click, Shift+click, Ctrl+A or by dragging a frame.
  Drag entries onto a folder or a part of the path bar to move them. Hold Ctrl to copy.
  Moves inside one storage are instant. Copies, and moves between internal storage and SD card, run in the copy queue with progress and *Cancel*.
- **Pinned folders:** press the pin button in the path bar (or Ctrl+D). Pins show in the Files sidebar and on Start.
  Up to 12 pins. A folder that is gone shows as "not found". Remove it with × or right-click.
- **Keys:** Ctrl+O add files · Backspace or Alt+Up up · Alt+Left/Right back/forward · F2 rename · Del delete ·
  Ctrl+Shift+N new folder · Ctrl+D pin · F5 refresh · Enter open · Shift+F10 menu · type letters to jump to a name.

### File pool (Thor → PC)

The file pool sits at the bottom of the Files sidebar. It is the way to copy files from the Thor to the PC.

1. Drag entries into the pool, or right-click → *Add to file pool*. Entries can come from different folders.
2. Press **Copy to PC…** and pick a folder on the PC.

The copy runs in the copy queue with progress and *Cancel*. It never overwrites files on the PC. The Thor stays untouched.
The pool is cleared when you close ThorBro.

### Import ES-DE info

For console folders that ES-DE didn't create, e.g. a `Wii` folder from another launcher.

1. Open the games folder in Files. Right-click empty space in the list → **Import ES-DE info …**.
2. ThorBro shows a preview. It writes a `systeminfo.txt` into each console folder it recognizes.
   Folders whose name only differs by case from the ES-DE name (`Wii` → `wii`) can be renamed.
3. Nothing is written until you confirm.

ROM files are never touched. Unknown folders are left alone. An existing `systeminfo.txt` is only replaced if you tick it.

### Saves

1. In the **Saves** tab, click **+ Add a game save**.
2. Search the game. The results come from the games on your Thor.
3. Pick the emulator. Emulators with a save for this game come first. The others are greyed out.
4. ThorBro shows the save folder it found (path, number of files, last change). Click **Add this save**.
   Wrong folder? Click **Wrong folder? Browse**.

Each game then shows which side has the newer save. **Continue on PC** and **Continue on Thor** copy it over.
The first **Continue on PC** asks where the save lives on the PC.

- The old save is backed up before every overwrite. Open the backups from the game's menu → *Open backups*.
- Import a save package (e.g. a 100 % save) from the game's menu → *Import save …*. Sources: an archive or folder on the PC, a dropped file, or the Thor.
- Some emulators can't be written to yet. Then *Continue on Thor* is disabled and says why.
- **Game not on the Thor?** Click *The game isn't on the Thor* in the dialog. ThorBro then searches all folders on the PC and the Thor.

> **Optional:** if [Claude Code](https://claude.com/claude-code) is installed and logged in (`claude` in a terminal),
> the *The game isn't on the Thor* search lets the AI pick the matching folders. This helps with Switch and 3DS emulators,
> which store saves in title-ID folders. Without Claude Code you pick them yourself.

### Mirror Thor screens

Open **Stream → Mirror Thor screens**. Click *Both screens*, *Top only* or *Bottom only*.
Each screen opens in its own window. Audio comes with the top screen only. *Stop mirroring* closes them.
You keep playing on the Thor.

The top screen is display id 0, the bottom screen id 4. If a screen fails, the error shows under the status and the other screen keeps running.

### Stream PC games to your Thor

You need [Sunshine](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2getting__started.html) on the PC
and [Moonlight](https://github.com/moonlight-stream/moonlight-android) on the Thor.
For playing away from home also [Tailscale](https://tailscale.com/download) on both. ThorBro doesn't install or ship any of them.

Open **Stream → Stream PC games**. The chips at the top show if Sunshine runs, if Tailscale is connected and if Moonlight is on the Thor.
Click a chip for details.

Enter your Sunshine web UI login once under **Help → Sunshine on this PC**. It is stored in your system keychain, not in ThorBro's settings.

1. **Pair:** tap the PC in Moonlight on the Thor. Click *Pair Moonlight* in ThorBro, type the PIN that Moonlight shows, click *Pair*. Only needed once.
2. **Play:** open the PC's game list in Moonlight once. Then click *Play on Thor* next to a game in ThorBro.

**Send to Thor app** (in the *Thor app* card) sends this PC and your game list to the
[ThorBro Android app](https://github.com/catzeyea/ThorBroAndroid) on the Thor.
ThorBro can't see if the app accepted it. Check the app's Status tab on the Thor.

### Command line

```text
thorbro                       start the app
thorbro stream [top|bottom]   mirror screens (default: both)
thorbro displays              list the screens scrcpy can mirror
thorbro list                  list configured games
thorbro status <game>         compare saves
thorbro pc <game>             pull the Thor's save to the PC
thorbro thor <game>           push the PC's save to the Thor
```

Game names can be shortened: `thorbro pc totk`, `thorbro status zelda`.

---

## Troubleshooting

| Problem | Fix |
|---|---|
| **"No Thor found"** | **Check that USB debugging is on** ([Setup step 1](#1-on-the-thor-turn-on-usb-debugging)). Try another cable (some only charge) or another port. Then click the device status at the top to reconnect. |
| **"Confirm the USB debugging prompt on the Thor"** | A dialog is open on the Thor. Confirm it. If none shows up, tap *Revoke USB debugging authorizations* in Developer options and plug the cable back in. |
| **Windows doesn't see the Thor** | Install the [Google USB Driver](https://developer.android.com/studio/run/win-usb) and plug the Thor back in. |
| **Linux: "no permissions" / "unauthorized"** | udev rules are missing. Install `android-udev` (Arch) or `android-sdk-platform-tools-common` (Debian/Ubuntu), then replug. |
| **"adb not found"** | Windows: keep the `tools` folder next to `ThorBro.exe`. Linux/macOS: install adb (see [Setup](#2-on-your-pc-install-thorbro)). |
| **macOS: "ThorBro is damaged"** | The build is unsigned. Run `xattr -dr com.apple.quarantine /Applications/ThorBro.app` (see the macOS section in [Setup](#2-on-your-pc-install-thorbro)). |
| **No games on Start / no game folders** | Launch ES-DE on the Thor once so it creates `ROMs/`. Or open your games folder in Files and press **Set games folder**. |
| **Bottom screen doesn't start or stays black** | Check that the bottom screen is on. Run `thorbro displays` to list the ids scrcpy sees. If the bottom screen isn't id 4, please [open an issue](../../issues). |

Found a bug? [Open an issue](../../issues). Include your OS, the ThorBro version and what happened.

---

## Where is my data?

| What | Linux | Windows | macOS |
|---|---|---|---|
| Configured saves | `~/.config/thor-save/games.json` | `%APPDATA%\thor-save\games.json` | `~/Library/Application Support/thor-save/games.json` |
| Save backups | `~/.local/share/thor-save/backups/` | `%APPDATA%\thor-save\backups\` | `~/Library/Application Support/thor-save/backups/` |
| Settings (pins, games folders) | `~/.config/thorBro/settings.json` | `%APPDATA%\thorBro\settings.json` | `~/Library/Application Support/thorBro/settings.json` |
| Cover cache | `~/.cache/thorBro/art/` | `%LOCALAPPDATA%\thorBro\art\` | `~/Library/Caches/thorBro/art/` |

ThorBro doesn't send anything to the internet. The one exception is the optional AI save search. It passes the folder paths it found to Claude Code.

---

## Build from source

You need [Rust](https://rustup.rs) (stable).
On Linux you also need WebKitGTK 4.1 (Arch: `webkit2gtk-4.1`, Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev`).

```bash
git clone https://github.com/catzeyea/ThorBro.git
cd ThorBro
cargo build --release          # -> target/release/thorbro(.exe)
```

Installers and packages are built with the Tauri CLI (needs Node.js):

```bash
npx @tauri-apps/cli@2 build    # -> target/release/bundle/
```

The Rust backend is in `src/` (Tauri 2). The UI is plain HTML/CSS/JS in `ui/`.
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for a walkthrough (German: [docs/ARCHITEKTUR.md](docs/ARCHITEKTUR.md)).

Releases are built by GitHub Actions from a version tag. See [CONTRIBUTING.md](CONTRIBUTING.md#releasing-a-new-version).

---

## Contributing

Issues and pull requests are welcome, especially macOS testing, other Thor firmware versions,
and more systems and emulators. See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## License

ThorBro uses two licenses.

| What | License |
|---|---|
| Source code (`src/`, `ui/` except the paths below, `tests/`, build files) | [MIT](LICENSE) |
| Mascot, logo, icons and design files: `ui/assets/mascot/`, `ui/assets/logo-256.png`, `icons/`, `docs/logo*`, `docs/thorbro-icon-transparent*.svg`, `docs/design/` | **All rights reserved** by Alek ([catzeyea](https://github.com/catzeyea)). Use only with written permission. See [ASSETS-LICENSE.md](ui/assets/ASSETS-LICENSE.md). |
| Font in `ui/assets/fonts/` (JetBrains Mono) | [SIL Open Font License 1.1](ui/assets/fonts/OFL.txt) |

**Forks** may use the code freely under MIT, but must replace the mascot, logo and icons with their own.

The Windows release bundles the [scrcpy](https://github.com/Genymobile/scrcpy) Windows release unchanged:
scrcpy (Apache 2.0), FFmpeg (LGPL 2.1 or later), SDL3 (zlib), libusb (LGPL 2.1 or later) and adb from the
[Android SDK Platform-Tools](https://developer.android.com/tools/releases/platform-tools)
([Android SDK terms](https://developer.android.com/studio/terms)).
Versions and license texts are in [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt), which is also in the Windows downloads.

ThorBro doesn't include any games, BIOS files or firmware. Only use games you own.
