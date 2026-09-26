#!/usr/bin/env python3
"""Generate macOS-style file and folder icons for file-search results.

Produces assets/icons/folder.png and assets/icons/file.png. Rendered at 40x40
in the launcher, so we draw at high resolution and downsample for clean edges.
"""
import os
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(__file__), "..", "assets", "icons")
SIZE = 256
SS = 4  # supersample factor


def _canvas():
    big = SIZE * SS
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    return img, ImageDraw.Draw(img), big


def _vgrad(box, top, bottom):
    """Vertical gradient image clipped to the given rounded box later."""
    x0, y0, x1, y1 = box
    w, h = int(x1 - x0), int(y1 - y0)
    grad = Image.new("RGBA", (w, h))
    for yy in range(h):
        t = yy / max(h - 1, 1)
        r = int(top[0] + (bottom[0] - top[0]) * t)
        g = int(top[1] + (bottom[1] - top[1]) * t)
        b = int(top[2] + (bottom[2] - top[2]) * t)
        for xx in range(w):
            grad.putpixel((xx, yy), (r, g, b, 255))
    return grad


def make_folder():
    img, d, big = _canvas()
    # macOS Big Sur-style folder: rounded body with a raised tab on the back.
    pad = int(big * 0.10)
    body_top = int(big * 0.34)
    radius = int(big * 0.085)

    # Back panel + tab (slightly darker blue).
    back_top = int(big * 0.26)
    tab_w = int(big * 0.40)
    d.rounded_rectangle(
        [pad, back_top, big - pad, big - pad], radius=radius, fill=(56, 146, 232, 255)
    )
    d.rounded_rectangle(
        [pad, back_top, pad + tab_w, body_top + radius],
        radius=radius,
        fill=(56, 146, 232, 255),
    )

    # Front panel (lighter blue gradient), offset down to reveal the tab lip.
    fbox = (pad, body_top, big - pad, big - pad)
    grad = _vgrad(fbox, (118, 196, 255), (72, 159, 240))
    mask = Image.new("L", (int(fbox[2] - fbox[0]), int(fbox[3] - fbox[1])), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, mask.width - 1, mask.height - 1], radius=radius, fill=255
    )
    img.paste(grad, (fbox[0], fbox[1]), mask)

    img = img.resize((SIZE, SIZE), Image.LANCZOS)
    img.save(os.path.join(OUT, "folder.png"))


def make_file():
    img, d, big = _canvas()
    # macOS-style document: white page, portrait, folded top-right corner.
    mx = int(big * 0.20)
    top = int(big * 0.10)
    bot = int(big * 0.90)
    right = big - mx
    fold = int(big * 0.22)
    radius = int(big * 0.05)

    # Page body with the corner cut for the fold.
    body = [
        (mx, top + radius),
        (mx, bot - radius),
        (mx + radius, bot),
        (right - radius, bot),
        (right, bot - radius),
        (right, top + fold),
        (right - fold, top),
        (mx + radius, top),
    ]
    d.polygon(body, fill=(252, 252, 254, 255))
    d.line([(mx, top + radius), (mx, bot - radius)], fill=(214, 217, 224, 255), width=SS * 2)

    # Folded corner (light gray triangle).
    d.polygon(
        [(right - fold, top), (right, top + fold), (right - fold, top + fold)],
        fill=(214, 219, 228, 255),
    )

    # A few faint text lines.
    lx0, lx1 = mx + int(big * 0.07), right - int(big * 0.07)
    ly = top + int(big * 0.34)
    gap = int(big * 0.11)
    for i in range(4):
        x1 = lx1 if i < 3 else lx0 + int((lx1 - lx0) * 0.55)
        d.rounded_rectangle(
            [lx0, ly, x1, ly + int(big * 0.028)],
            radius=int(big * 0.014),
            fill=(196, 201, 210, 255),
        )
        ly += gap

    img = img.resize((SIZE, SIZE), Image.LANCZOS)
    img.save(os.path.join(OUT, "file.png"))


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    make_folder()
    make_file()
    print("wrote", os.path.join(OUT, "folder.png"), "and", os.path.join(OUT, "file.png"))
