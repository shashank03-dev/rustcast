CS# RustCast — Linux port (X11 / XWayland)

A faithful Linux port of [RustCast](https://github.com/RustCastLabs/rustcast), the
Rust-powered productivity launcher, plus two new features:

1. **Clipboard history** — everything you copy (text + images) is stored and shown
   in a popup on a hotkey (`Super+Shift+C`). History is **persisted to disk** under
   `~/.local/share/rustcast/clipboard` and survives restarts.
2. **Screenshot thumbnail with real drag-and-drop** — take a region screenshot with
   `Super+Shift+S` (or use PrintScreen). A thumbnail pops up in the **bottom-left
   corner**; click it to copy the image, or **drag it straight into any application**
   (browser, chat, file manager) as a PNG file via the XDND protocol.

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
sudo apt install libgtk-3-dev libxcb1-dev libxtst-dev gnome-screenshot
```

- `libgtk-3-dev` / `ayatana-appindicator` — tray icon
- `libxtst` — paste injection (XTEST)
- `gnome-screenshot` — region capture for the screenshot feature
- X11/XCB — windowing, EWMH tiling, XDND drag source (via the pure-Rust `x11rb`)

## Default hotkeys

| Action | Hotkey |
|---|---|
| Toggle launcher | `Alt+Space` |
| Clipboard history | `Super+Shift+C` |
| Screenshot capture | `Super+Shift+S` |

All are configurable in `~/.config/rustcast/config.toml`.

## Config

The config file is created on first run at `~/.config/rustcast/config.toml`. It is
the same schema as upstream RustCast, with one added field:

```toml
screenshot_hotkey = "SUPER+SHIFT+S"
```

## Known differences from the macOS build

- **Calendar `Events` page** is empty — there is no portable Linux equivalent of
  macOS EventKit.
- **Haptics** are a no-op.
- **Window tiling** moves other apps' windows via EWMH; this works for X11/XWayland
  windows. Native-Wayland-only windows cannot be tiled (a Wayland security limit).
- **Drag-and-drop** drops onto X11/XWayland targets; a drop target that is a
  native-Wayland-only window may not accept the XDND drop.
- App launching uses `.desktop` entries (`gio launch`) and `xdg-open`.
