CS# RustCast — Linux port (X11 / XWayland)

A faithful Linux port of [RustCast](https://github.com/RustCastLabs/rustcast), the
Rust-powered productivity launcher, plus these new features:

1. **Clipboard history** — everything you copy (text + images) is stored and shown
   on a hotkey (`Super+Shift+C`) in a full-size view: cards with type badges,
   a large preview with details, All / Text / Images filters (`←` `→`), typing
   to search, `↑` `↓` to browse, `Enter` to copy, `Ctrl+1…9` for quick picks. History is **persisted to disk** under
   `~/.local/share/rustcast/clipboard` and survives restarts.
2. **Screenshot thumbnail with real drag-and-drop** — take a region screenshot with
   `Super+Shift+S` (or use PrintScreen). A thumbnail pops up in the **bottom-left
   corner**; click it to copy the image, or **drag it straight into any application**
   (browser, chat, file manager) as a PNG file via the XDND protocol.

3. **Jev, the command operator** — type `jev` and say what you want in plain words.
   Jev turns it into actions you run with Enter:

   | You type | Jev does |
   |---|---|
   | `jev open downloads` · `jev go to desktop/projects` | opens folders (desktop, documents, `~/code`, nested paths…) |
   | `jev open report.pdf in documents` · `jev find invoice` | finds and opens files (things on your Desktop come first) |
   | `jev launch firefox` · `jev switch to terminal` · `jev close spotify` | apps and windows |
   | `jev create folder Ideas on desktop` · `jev make todo.txt` | makes folders / files and opens them |
   | `jev show desktop` · `jev tile left` · `jev screenshot` | window management |
   | `jev record firefox` · `jev record screen` · `jev stop recording` | the screen recorder |
   | `jev add terminal to recording` | bring a window into a locked recording |
   | `jev open downloads and firefox then show desktop` | several steps → a "Run all" row |

   Type just `jev` for examples.
4. **Screen recorder** (`Super+Shift+R`) — opens a recorder view laid out for the
   job: screen cards, a grid of window cards, switches for the options, and — while
   recording — a live banner with the timer and a big Stop button:
   - **Lock onto a window**: the recording follows that one app. Windows dragged
     over it never show up, moving it around doesn't matter, and nothing turns
     black. **Minimizing it keeps recording**: the window is hidden (invisible,
     click-through, behind everything) but keeps rendering; activate it from the
     dock to bring it back. It returns to minimized when you stop.
   - **Bring other windows in**: while a locked recording runs, press
     `Super+Shift+R` (or **＋ Add window** on the floating ● REC pill) and pick
     *Add … to Recording*. Added windows are drawn over the locked window exactly
     where you place them, or — with **Picture-in-Picture** — as tidy rounded
     corner tiles. Switch layouts or remove windows mid-recording.
   - **Full screen** recording of any monitor.
   - **Aspect lock** (default 1920×1080): every frame is fitted into a fixed
     size, so resizing the window never changes the video.
   - Cursor, audio, FPS, output folder (`~/Videos/RustCast`) — in the page's
     quick toggles and Settings → Recorder.
   - Pressing the hotkey while recording stops it; the file is announced in a
     notification.

   Needs `ffmpeg`. On **GNOME/Wayland**, full-screen and native-Wayland windows
   are recorded through the system screen-sharing dialog (xdg-desktop-portal +
   PipeWire, needs `gstreamer1.0-pipewire`); X11/XWayland windows are locked onto
   directly and support minimizing and adding windows.

## Display server

This build targets **X11**. It runs natively on X11 sessions and on **GNOME/Wayland
through XWayland** (no configuration needed). Global hotkeys, window tiling, paste
injection, and drag-and-drop all rely on the X11 path.

## Install (recommended)

Installs RustCast as a background launcher: builds a release binary into
`~/.local/bin`, adds an app-grid launcher and icon, and enables autostart so the
tray is always running after login.

```sh
make install          # build + install + enable autostart
make install-start    # same, and launch it right now
make uninstall        # remove binary, launcher, autostart, GNOME hotkeys
```

On **GNOME** the global hotkeys are registered as GNOME custom keybindings
(`gsettings`) instead of in-process X11 grabs, so they fire reliably on Wayland
regardless of which app is focused. If your toggle key clashes with GNOME's
window menu (the default `Alt+Space`), the install clears that conflict.

## Build (manual)

```sh
cargo build --release
./target/release/rustcast
```

### System packages

Runtime/build dependencies (Debian/Ubuntu names):

```sh
sudo apt install libgtk-3-dev libxcb1-dev libxtst-dev gnome-screenshot \
                 ffmpeg gstreamer1.0-tools gstreamer1.0-pipewire libnotify-bin
```

- `libgtk-3-dev` / `ayatana-appindicator` — tray icon
- `libxtst` — paste injection (XTEST)
- `gnome-screenshot` — region capture for the screenshot feature
- `ffmpeg` — video encoding for the screen recorder
- `gstreamer1.0-pipewire` — Wayland (portal) recordings
- X11/XCB — windowing, EWMH tiling, XDND drag source (via the pure-Rust `x11rb`)

## Default hotkeys

| Action | Hotkey |
|---|---|
| Toggle launcher | `Alt+Space` |
| Clipboard history | `Super+Shift+C` |
| Screenshot capture | `Super+Shift+S` |
| Screen recorder (again to stop) | `Super+Shift+R` |

All are configurable in `~/.config/rustcast/config.toml`.

## Config

The config file is created on first run at `~/.config/rustcast/config.toml`. It is
the same schema as upstream RustCast, with these added fields:

```toml
screenshot_hotkey = "SUPER+SHIFT+S"
recorder_hotkey = "SUPER+SHIFT+R"

[recorder]
fps = 30
aspect_lock = true            # fit every frame into output_width × output_height
output_width = 1920
output_height = 1080
keep_recording_when_minimized = true
show_cursor = true
record_audio = false
show_indicator = true         # floating ● REC pill
picture_in_picture = false    # layout for windows added to a locked recording
output_dir = "~/Videos/RustCast"
```

## Known limitations

- **Calendar `Events` page** is empty; there is no portable Linux calendar source
  wired up yet.
- **Window tiling** moves other apps' windows via EWMH; this works for X11/XWayland
  windows. Native-Wayland-only windows cannot be tiled (a Wayland security limit).
- **Drag-and-drop** drops onto X11/XWayland targets; a drop target that is a
  native-Wayland-only window may not accept the XDND drop.
- **Recorder on Wayland**: native-Wayland windows go through the system picker,
  so the minimize trick and adding windows only apply to X11/XWayland windows.
- App launching uses `.desktop` entries (`gio launch`) and `xdg-open`.
