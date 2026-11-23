<p align="center">
  <img src="docs/logo-256.png" width="128" alt="ThorBro logo">
</p>

<h1 align="center">ThorBro</h1>

<p align="center">
  A desktop companion for the <b>AYN Thor</b>: drag & drop games into the right folder,<br>
  sync save files between PC and Thor, and mirror both screens to your PC.
</p>

<p align="center">
  <a href="../../releases/latest"><b>⬇ Download</b></a> ·
  <a href="#setup-in-5-minutes">Setup</a> ·
  <a href="#troubleshooting">Troubleshooting</a>
</p>

---

## Features

| Area | What it does |
|---|---|
| **Files** | File manager for the Thor (internal storage and SD card) that works like Explorer or Nautilus: multi-select, cut/copy/paste, drag entries into folders, right-click menu, sorting. Drag files in from your PC, drag entries out of the window straight onto your desktop. |
| **Game sorting** | Drop a game into the window and ThorBro detects the system from the ES-DE folders and file extensions, then suggests the matching `ROMs/<system>` folder. Switch updates and DLC can go to `switch/update`. |
| **Unpacking** | ThorBro extracts zip, 7z and tar itself, and can do it before copying. Archives already on the Thor can be extracted in place. File names are made SD-card safe automatically. |
| **Saves** | Set up save folders per game, then push them PC → Thor or Thor → PC with one click. ThorBro shows which side is newer and **makes a backup before every overwrite**. You can also import saves from archives, e.g. 100% saves from the internet. |
| **Stream** | Mirror the top screen, the bottom screen or both to your PC over USB (via scrcpy), each in its own window. ThorBro detects the screens, and size, bitrate and always-on-top are remembered per screen. View only, with audio from the top screen. |
| **Stream PC games** | The other direction: play PC games on the Thor with [Sunshine](https://github.com/LizardByte/Sunshine) (PC) and [Moonlight](https://github.com/moonlight-stream/moonlight-android) (Thor), at home or over Tailscale. ThorBro checks the setup, helps with pairing and starts a game on the Thor with one click. Sunshine and Moonlight are installed separately. |

---

## Setup in 5 minutes

### 1. On the Thor: turn on USB debugging

You only need to do this once.

1. Open **Settings → About device** and tap **Build number 7 times** until it says you are now a developer.
2. Go to **Settings → System → Developer options** and turn on **USB debugging**.
3. Connect the Thor to your PC with a USB cable. The Thor asks *"Allow USB debugging?"*. Tick **"Always allow from this computer"** and tap **OK**.

> For game sorting, [ES-DE](https://es-de.org) must have created its ROM folders on the Thor
> (it does this on first launch). ThorBro looks for `ROMs/` on internal storage and on the SD card.

### 2. On your PC: install ThorBro

Grab the right file from the [Releases page](../../releases/latest):

| System | File | Notes |
|---|---|---|
| **Windows 10/11** | `ThorBro_x.y.z_x64-setup.exe` | Installer. adb and scrcpy are included, nothing else needed. |
| | `ThorBro-x.y.z-windows-portable.zip` | No install: unzip and run `ThorBro.exe`. Keep the `tools` folder next to it. |
| **Linux** | `ThorBro_x.y.z_amd64.AppImage` | Runs on any distro. You also need adb and scrcpy, see below. |
| | `.deb` / `.rpm` | For Ubuntu/Debian/Mint or Fedora/openSUSE. |
| **macOS** *(experimental)* | `.dmg` | Untested, and only attached to a release if the macOS build succeeded. If it's missing, [build from source](#building-from-source). Install adb and scrcpy via Homebrew, see below. |

On Linux, ThorBro needs WebKitGTK 4.1 and GTK 3 at runtime: the `.deb` and `.rpm` depend on them (on Debian/Ubuntu `libwebkit2gtk-4.1-0` and `libgtk-3-0`), the AppImage bundles them.

<details>
<summary><b>Windows: "Windows protected your PC"</b></summary>

ThorBro isn't signed with a paid certificate, so SmartScreen warns you on first launch.
Click **"More info"**, then **"Run anyway"**.
</details>

<details>
<summary><b>Linux: install adb and scrcpy</b></summary>

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

Make the AppImage executable before the first launch:

```bash
chmod +x ThorBro_*.AppImage
./ThorBro_*.AppImage
```

If the AppImage won't start and mentions *FUSE*, install `fuse2` (Arch) or `libfuse2` (Ubuntu).
</details>

<details>
<summary><b>macOS: install adb and scrcpy</b></summary>

```bash
brew install android-platform-tools scrcpy
```

Because the app isn't signed, the first time you open it use **right-click → Open** instead of double-clicking.
</details>

### 3. Go

Start ThorBro. The device name shows up in the top right once the Thor is detected. That's it.

---

## Usage

### Files & games

- **Add games:** drag a file or folder into the window. If ThorBro recognizes a game, a dialog shows the suggested target folder (★). You can change it before copying. Regular files go straight into the current folder.
- **Copy to PC:** drag entries out of the window onto your desktop or file manager, or press *Copy to PC*. Files end up in `Downloads/thor`.
- **Organize on the Thor:** select with click, Ctrl+click, Shift+click, Ctrl+A or by dragging a frame on the empty area. Drag entries onto a folder or a part of the path bar to move them (hold Ctrl to copy). Cut/copy/paste work with Ctrl+X/C/V and the right-click menu. Moving inside one storage is instant; copying, or moving between internal storage and SD card, runs in the copy queue with progress and *Cancel*.
- **Keys:** Enter open · Backspace or Alt+Up up · Alt+Left/Right back/forward · F2 rename · Del delete · Ctrl+Shift+N new folder · F5 refresh · type letters to jump to a name.
- **Archives:** when dropping, you can tick *Extract first*. It's pre-ticked for systems whose emulator can't read packed games. rar and tar.xz need [7-Zip](https://www.7-zip.org) on the PC.

### Saves

1. In the **Saves** tab, click **+ Add game** and type the game's name.
2. ThorBro searches the PC and the Thor for matching save folders. You confirm the right folder on each side.
3. After that, each game shows whether the PC or the Thor has the newer save, and one click copies it over.

The old save is backed up before every overwrite. You'll find the backups in the game's menu under *Open backups*.

> **Optional:** if [Claude Code](https://claude.com/claude-code) is installed and logged in (`claude` in a terminal),
> ThorBro lets the AI pick the right folders from the candidates. This helps with Switch and 3DS emulators,
> which store saves in title-ID folders. Without Claude Code you just pick them yourself.

### Stream

Open the **Stream** tab. ThorBro asks scrcpy which screens it can see and shows one card per screen.
Start a single screen from its card or click *Stream both*. Each screen opens in its own window, and every
card has its own *Stop* button (*Stop all* closes everything). You keep playing on the Thor.

Size, bitrate and *Always on top* are set per screen and remembered for the next time.
If a stream fails to start, the error appears on its card and the other screen keeps running.
*Quick start* below the cards starts the screens with the fixed ids (0 = top, 4 = bottom) without detecting them first.

### Stream PC games to your Thor

Needs [Sunshine](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2getting__started.html) on the PC
and [Moonlight](https://github.com/moonlight-stream/moonlight-android) on the Thor; for playing away from home also
[Tailscale](https://tailscale.com/download) on both. ThorBro doesn't install or ship any of them.

Open **Stream → Stream PC games to your Thor**. ThorBro shows whether Sunshine runs, the PC's LAN and Tailscale
addresses and whether Moonlight is on the Thor. Enter your Sunshine web UI login once (it is kept in your system's
keychain, not in ThorBro's settings). Then:

1. **Pair:** tap the PC in Moonlight on the Thor, type the PIN Moonlight shows into ThorBro, click *Pair*.
2. **Play:** open the PC's game list in Moonlight once, then use *Play on Thor* next to any game in ThorBro.

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

Game names can be shortened (`thorbro pc totk`, `thorbro status zelda`).

---

## Troubleshooting

| Problem | Fix |
|---|---|
| **"No Thor found"** | Try another USB cable (some only charge) or another port, and check USB debugging on the Thor. Then click the device indicator at the top to reconnect. |
| **"Confirm the USB debugging prompt on the Thor"** | There's a dialog open on the Thor, so confirm it. If none shows up, use *"Revoke USB debugging authorizations"* in Developer options and plug the cable back in. |
| **Windows doesn't see the Thor at all** | Install the [Google USB Driver](https://developer.android.com/studio/run/win-usb) and plug the Thor back in. |
| **Linux: "no permissions" / "unauthorized"** | udev rules are missing. Install `android-udev` (Arch) or `android-sdk-platform-tools-common` (Debian/Ubuntu), then replug. |
| **"adb not found"** | Windows: keep the `tools` folder next to `ThorBro.exe`. Linux/macOS: install adb as shown above. |
| **No game folders in the sidebar** | Launch ES-DE on the Thor once so it creates `ROMs/` with the system folders, then reload ThorBro. |
| **Stream tab finds only one screen** | scrcpy can't see the bottom screen right now. Check that it's switched on and click *Detect screens* again. `thorbro displays` shows the same list in a terminal. If it still finds one, please [open an issue](../../issues). |
| **Bottom screen stays black in the stream** | The display number can change between firmware versions. Use the cards (they use the detected ids) instead of *Quick start*, and please [open an issue](../../issues). |

Found a bug or have an idea? [Open an issue](../../issues). Please include your OS, the ThorBro version and what exactly happened.

---

## Where is my data?

| What | Linux | Windows | macOS |
|---|---|---|---|
| Configured games | `~/.config/thor-save/games.json` | `%APPDATA%\thor-save\games.json` | `~/Library/Application Support/thor-save/games.json` |
| Save backups | `~/.local/share/thor-save/backups/` | `%APPDATA%\thor-save\backups\` | `~/Library/Application Support/thor-save/backups/` |
| Files pulled from the Thor | `~/Downloads/thor/` | `Downloads\thor\` | `~/Downloads/thor/` |

ThorBro doesn't send anything to the internet. The only exception is the optional AI save search, which passes the full folder paths it found to Claude Code.

---

## Building from source

You need [Rust](https://rustup.rs) (stable). On Linux you also need WebKitGTK 4.1 (Arch: `webkit2gtk-4.1`, Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev`).

```bash
git clone https://github.com/catzeyea/ThorBro.git
cd ThorBro
cargo build --release          # -> target/release/thorbro(.exe)
```

Installers and packages are built with the Tauri CLI (needs Node.js):

```bash
npx @tauri-apps/cli@2 build    # -> target/release/bundle/
```

The Rust backend lives in `src/` (Tauri 2). The UI is plain HTML/CSS/JS in `ui/`.

| File | Contents |
|---|---|
| `src/main.rs` | entry point, Tauri setup |
| `src/commands.rs` | all commands the UI can call |
| `src/cli.rs` | command line interface |
| `src/adb.rs` | finding and calling adb |
| `src/files.rs` | file manager, copy queue |
| `src/library.rs` | reading ES-DE folders, detecting games |
| `src/saves/` | finding, comparing, syncing and importing saves |
| `src/unpack.rs` | extracting archives |
| `src/stream.rs` | launching scrcpy |
| `src/pcstream/` | Sunshine, Tailscale and Moonlight for streaming PC games to the Thor |
| `ui/js/` | the UI scripts (loaded in order by `ui/index.html`) |

A longer walkthrough is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (German version: [docs/ARCHITEKTUR.md](docs/ARCHITEKTUR.md)).

### Releasing a new version

Bump the version in `Cargo.toml` and `tauri.conf.json`, commit, then:

```bash
git tag v0.3.0
git push origin v0.3.0
```

GitHub Actions builds Windows, Linux and macOS and creates a **draft release**. Check it on the Releases page and publish it.

---

## Contributing

Issues and pull requests are welcome, especially:

- Testing on macOS and on different Thor firmware versions.
- Detection for more systems and emulators.

---

## Legal

ThorBro is a fan project and isn't affiliated with AYN. It doesn't include any games, BIOS files or firmware. Please only use games you own.

The Windows release bundles the files of the [scrcpy](https://github.com/Genymobile/scrcpy) Windows release unchanged: scrcpy itself (Apache 2.0), the FFmpeg libraries (LGPL 2.1 or later), SDL3 (zlib license), libusb (LGPL 2.1 or later) and adb from the [Android SDK Platform-Tools](https://developer.android.com/tools/releases/platform-tools) (see the [Android SDK terms](https://developer.android.com/studio/terms)). Versions and license texts are in [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt), which is also included in the Windows downloads.

ThorBro itself is licensed under the [MIT License](LICENSE).
