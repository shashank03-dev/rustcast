🎉 **The first public release of RustCast for Linux.**

A Rust-powered launcher for X11 and GNOME/Wayland (via XWayland). Open apps,
files and commands with one keystroke, and get a full screenshot, OCR and
screen-recording toolkit built in.

Based on [RustCast](https://github.com/RustCastLabs/rustcast) by Umang Surana, the
original macOS launcher, ported to Linux and extended.

## Highlights

- **Launcher** (`Alt+Space`): apps, files, calculator, emoji search, web search.
- **Jev**: type what you want in plain words. `jev open downloads then tile left`,
  `jev screenshot chromium in 5 seconds`, `jev record firefox`.
- **Screenshot studio** (`Super+Shift+S`): arrows, numbered steps, spotlight,
  pixelate/blur, text, beautify, pin to screen, and a draggable floating thumbnail.
- **Copy text from anything** (`Super+Shift+T`): OCR as text, code (indentation
  kept) or a table, plus one-click links, emails, phone numbers, colours and QR codes.
- **Colour palette** and **before/after compare** for any capture.
- **Screen recorder** (`Super+Shift+R`): lock onto one window; anything dragged
  over it never shows up in the video.
- **Clipboard history** (`Super+Shift+C`): text and images, searchable, kept on disk.

## Install

```sh
tar xzf rustcast-v*-x86_64-linux.tar.gz
cd rustcast-v*-x86_64-linux
./install.sh --start      # installs to ~/.local, enables autostart, launches it
```

Remove it again with `./uninstall.sh`.

Runtime packages (Debian/Ubuntu names):

```sh
sudo apt install libgtk-3-0 libayatana-appindicator3-1 tesseract-ocr ffmpeg \
                 gstreamer1.0-pipewire libnotify-bin
```

`tesseract-ocr` is needed for OCR and `ffmpeg` for the recorder; everything else
works without them. Built on Ubuntu 22.04 (glibc 2.35), so it runs on that and
newer distributions.

Verify the download with `sha256sum -c rustcast-v*-x86_64-linux.tar.gz.sha256`.
