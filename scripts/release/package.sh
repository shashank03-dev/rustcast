#!/usr/bin/env bash
# Package a built release binary into dist/rustcast-<version>-<arch>-linux.tar.gz
# (+ .sha256). Run after `cargo build --release`.
set -euo pipefail
cd "$(dirname "$0")/../.."

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
ARCH="$(uname -m)"
NAME="rustcast-v${VERSION}-${ARCH}-linux"
OUT="dist/$NAME"

[ -f target/release/rustcast ] || { echo "error: build with cargo build --release first" >&2; exit 1; }

rm -rf "$OUT" "dist/$NAME.tar.gz" "dist/$NAME.tar.gz.sha256"
mkdir -p "$OUT/scripts" "$OUT/assets"
install -m 755 target/release/rustcast "$OUT/rustcast"
strip "$OUT/rustcast" 2>/dev/null || true
cp -r assets/icons "$OUT/assets/icons"
install -m 755 scripts/install.sh scripts/uninstall.sh "$OUT/scripts/"
cp README.md LICENSE.md "$OUT/"
cat > "$OUT/install.sh" <<'SH'
#!/usr/bin/env bash
# Install the prebuilt RustCast binary to ~/.local and enable autostart.
# Pass --start to launch it right away, --no-autostart to skip autostart.
exec "$(dirname "$0")/scripts/install.sh" --no-build "$@"
SH
cat > "$OUT/uninstall.sh" <<'SH'
#!/usr/bin/env bash
exec "$(dirname "$0")/scripts/uninstall.sh" "$@"
SH
chmod 755 "$OUT/install.sh" "$OUT/uninstall.sh"

tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
(cd dist && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
echo "dist/$NAME.tar.gz"
