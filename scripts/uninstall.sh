#!/usr/bin/env bash
# Remove a RustCast install created by scripts/install.sh and clear the GNOME
# global hotkeys it registered.
set -euo pipefail

cd "$(dirname "$0")/.."

PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin/rustcast"

echo "==> Stopping running instance"
pkill -x rustcast 2>/dev/null || true

# Clear the GNOME custom keybindings before removing the binary.
if [ -x "$BIN" ]; then
  echo "==> Removing GNOME hotkeys"
  "$BIN" rustcast://unregister-hotkeys 2>/dev/null || true
fi

echo "==> Removing files"
rm -f "$BIN"
rm -f "$PREFIX/share/applications/rustcast.desktop"
rm -f "$HOME/.config/autostart/rustcast.desktop"
for size in 16 24 32 48 64 128 256 512; do
  rm -f "$PREFIX/share/icons/hicolor/${size}x${size}/apps/rustcast.png"
done

command -v update-desktop-database >/dev/null 2>&1 && \
  update-desktop-database "$PREFIX/share/applications" || true

echo "Done. (Config in ~/.config/rustcast was left untouched.)"
