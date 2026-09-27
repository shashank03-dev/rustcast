"""Regenerate the app icon set in assets/icons/ from the RustCast logo.

The master artwork is docs/icon.png (1024px, the official RustCast mark).
Every size is a straight high-quality downscale of it, so the tray, the
About dialog, the window icon and the installed desktop icons all show the
same logo.

Usage: python3 scripts/gen_icon.py   (requires Pillow)
"""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
MASTER = ROOT / "docs" / "icon.png"
OUT = ROOT / "assets" / "icons"


def render(master: Image.Image, size: int) -> Image.Image:
    # Downscale in premultiplied space so the transparent margin doesn't
    # bleed a light fringe into the rounded corners.
    return master.convert("RGBa").resize((size, size), Image.LANCZOS).convert("RGBA")


def main() -> None:
    master = Image.open(MASTER).convert("RGBA")
    OUT.mkdir(parents=True, exist_ok=True)
    render(master, 512).save(OUT / "rustcast.png", optimize=True)
    for size in (16, 24, 32, 48, 64, 128, 256):
        render(master, size).save(OUT / f"rustcast-{size}.png", optimize=True)
    print(f"wrote icon set to {OUT}")


if __name__ == "__main__":
    main()
