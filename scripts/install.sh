#!/usr/bin/env bash
# Install RustCast as a background launcher on Linux:
#   - build a release binary and copy it to ~/.local/bin
#   - install the app icon and a .desktop launcher (shows in the app grid)
#   - enable autostart so the tray is running after login
#   - register GNOME global hotkeys (done by the app itself on first launch)
#
# Usage: scripts/install.sh [--no-build] [--no-autostart] [--start]
set -euo pipefail

cd "$(dirname "$0")/.."

PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
APP_DIR="$PREFIX/share/applications"
ICON_DIR="$PREFIX/share/icons/hicolor"
AUTOSTART_DIR="$HOME/.config/autostart"

DO_BUILD=1
DO_AUTOSTART=1
DO_START=0
for arg in "$@"; do
  case "$arg" in
    --no-build)     DO_BUILD=0 ;;
    --no-autostart) DO_AUTOSTART=0 ;;
    --start)        DO_START=1 ;;
    *) echo "Unknown option: $arg" >&2; exit 2 ;;
  esac
done

if [ "$DO_BUILD" -eq 1 ]; then
  echo "==> Building release binary"
  cargo build --release
fi

BIN_SRC="target/release/rustcast"
[ -f "$BIN_SRC" ] || { echo "error: $BIN_SRC not found (build first)" >&2; exit 1; }

echo "==> Installing binary to $BIN_DIR/rustcast"
mkdir -p "$BIN_DIR"
install -m 755 "$BIN_SRC" "$BIN_DIR/rustcast"
BIN="$BIN_DIR/rustcast"

echo "==> Installing icons"
for size in 16 24 32 48 64 128 256; do
  dst="$ICON_DIR/${size}x${size}/apps"
  mkdir -p "$dst"
  install -m 644 "assets/icons/rustcast-${size}.png" "$dst/rustcast.png"
done
mkdir -p "$ICON_DIR/512x512/apps"
install -m 644 "assets/icons/rustcast.png" "$ICON_DIR/512x512/apps/rustcast.png"

echo "==> Installing desktop launcher"
mkdir -p "$APP_DIR"
cat > "$APP_DIR/rustcast.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=RustCast
Comment=Productivity launcher
Exec=$BIN
Icon=rustcast
Terminal=false
Categories=Utility;
StartupNotify=false
EOF

if [ "$DO_AUTOSTART" -eq 1 ]; then
  echo "==> Enabling autostart"
  mkdir -p "$AUTOSTART_DIR"
  cat > "$AUTOSTART_DIR/rustcast.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=RustCast
Exec=$BIN
Icon=rustcast
X-GNOME-Autostart-enabled=true
NoDisplay=true
EOF
fi

# Best-effort cache refresh so the launcher/icon appear immediately.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APP_DIR" || true
command -v gtk-update-icon-cache    >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$ICON_DIR" >/dev/null 2>&1 || true

echo
echo "Installed RustCast to $BIN"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "NOTE: $BIN_DIR is not on your PATH — add it to your shell profile." ;;
esac
echo "Global hotkeys register on first launch (GNOME) or when the app runs."

if [ "$DO_START" -eq 1 ]; then
  echo "==> Starting RustCast"
  pkill -x rustcast 2>/dev/null || true
  nohup "$BIN" >/dev/null 2>&1 &
  disown || true
fi

echo "Done. Log out/in for autostart, or run: $BIN"
