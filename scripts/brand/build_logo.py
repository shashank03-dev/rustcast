"""Generate the RustCast mark as SVG.

Writes, into assets/brand/:
  rustcast-mark.svg           static mark (the final frame)
  rustcast-mark-animated.svg  the motion mark (CSS keyframes, loops idly)
  rustcast-mark-small.svg     heavier strokes for 16-32px renders
  rustcast-glyph-{dark,light}.svg  the mark without its tile, for in-app UI

The mark: a graphite squircle tile, an open "cast" arc that sweeps clockwise
from ten o'clock to five, a glass lens (orb) that refracts the arc where it
crosses it, and a rust ember at the arc's leading end.

Motion (animated file): the tile settles in, the arc draws itself with the
ember riding its head, the lens drops into place over the arc, then the
ember glow breathes slowly. `prefers-reduced-motion` shows the final frame.

Run `node scripts/brand/render.mjs` afterwards to rasterise the icon set.
"""

import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "assets" / "brand"

S = 1024  # canvas
INSET = 60  # room for the tile's shadow
N = 5.0  # superellipse exponent (macOS-like squircle)

# Arc: centre, radius, start angle and clockwise sweep (degrees, SVG space).
CX, CY, R = 500.0, 540.0, 258.0
A0, SWEEP = 203.0, 225.0
A1 = A0 + SWEEP

# Lens.
OX, OY, OR = 742.0, 292.0, 160.0

RUST_HI = "#FF9A5C"
RUST = "#F2542D"
RUST_DEEP = "#C23A1B"


def squircle_path(inset: float = INSET) -> str:
    half = (S - 2 * inset) / 2
    c = S / 2
    pts = []
    steps = 256
    for i in range(steps):
        t = 2 * math.pi * i / steps
        ct, st = math.cos(t), math.sin(t)
        x = c + half * math.copysign(abs(ct) ** (2 / N), ct)
        y = c + half * math.copysign(abs(st) ** (2 / N), st)
        pts.append(f"{x:.2f},{y:.2f}")
    return "M" + " L".join(pts) + " Z"


def polar(angle_deg: float, r: float = R) -> tuple[float, float]:
    a = math.radians(angle_deg)
    return CX + r * math.cos(a), CY + r * math.sin(a)


def arc_path() -> str:
    x0, y0 = polar(A0)
    x1, y1 = polar(A1)
    large = 1 if SWEEP > 180 else 0
    return f"M{x0:.2f},{y0:.2f} A{R},{R} 0 {large} 1 {x1:.2f},{y1:.2f}"


ARC_LEN = math.radians(SWEEP) * R


def svg(animated: bool, stroke: float, small: bool = False) -> str:
    x0, y0 = polar(A0)
    x1, y1 = polar(A1)
    tile = squircle_path(28 if small else INSET)
    arc = arc_path()
    ember_r = stroke * 0.95
    dash = f"{ARC_LEN + 2:.1f}"

    style = ""
    if animated:
        style = f"""
  <style>
    .tile   {{ transform-box: view-box; transform-origin: 512px 512px;
               animation: tile-in 700ms cubic-bezier(.2,.9,.25,1.15) both; }}
    .arc    {{ stroke-dasharray: {dash}; stroke-dashoffset: {dash};
               animation: draw 1150ms cubic-bezier(.65,0,.2,1) 250ms forwards; }}
    .rider  {{ transform-box: view-box; transform-origin: {CX}px {CY}px;
               transform: rotate(-{SWEEP}deg);
               animation: ride 1150ms cubic-bezier(.65,0,.2,1) 250ms forwards; }}
    .ember  {{ opacity: 0; animation: ember-in 260ms ease-out 250ms forwards; }}
    .glow   {{ transform-box: fill-box; transform-origin: center; opacity: 0;
               animation: glow-in 700ms ease-out 1300ms forwards,
                          breathe 4200ms ease-in-out 2000ms infinite; }}
    .lens   {{ transform-box: view-box; transform-origin: {OX}px {OY}px; opacity: 0;
               animation: lens-in 900ms cubic-bezier(.2,.9,.25,1.12) 850ms forwards; }}
    .shine  {{ opacity: 0; animation: shine 1400ms ease-in-out 1500ms forwards; }}
    .warm   {{ opacity: 0; animation: fade 1200ms ease-out 1200ms forwards; }}

    @keyframes tile-in  {{ from {{ transform: scale(.9); opacity: 0; }} to {{ transform: none; opacity: 1; }} }}
    @keyframes draw     {{ to {{ stroke-dashoffset: 0; }} }}
    @keyframes ride     {{ to {{ transform: rotate(0deg); }} }}
    @keyframes ember-in {{ to {{ opacity: 1; }} }}
    @keyframes glow-in  {{ from {{ opacity: 0; transform: scale(.4); }} 60% {{ opacity: 1; transform: scale(1.25); }} to {{ opacity: .85; transform: none; }} }}
    @keyframes breathe  {{ 0%, 100% {{ opacity: .85; transform: none; }} 50% {{ opacity: .45; transform: scale(.82); }} }}
    @keyframes lens-in  {{ from {{ opacity: 0; transform: translate(26px,-34px) scale(.7); }} to {{ opacity: 1; transform: none; }} }}
    @keyframes shine    {{ 0% {{ opacity: 0; }} 45% {{ opacity: 1; }} 100% {{ opacity: .7; }} }}
    @keyframes fade     {{ to {{ opacity: 1; }} }}

    @media (prefers-reduced-motion: reduce) {{
      .tile, .arc, .rider, .ember, .glow, .lens, .shine, .warm {{ animation: none; opacity: 1; transform: none; stroke-dashoffset: 0; }}
    }}
  </style>"""

    lens_fill_top = 0.16 if small else 0.11
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}" width="{S}" height="{S}">
  <title>RustCast</title>{style}
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#2E2F35"/>
      <stop offset=".55" stop-color="#1B1C20"/>
      <stop offset="1" stop-color="#101013"/>
    </linearGradient>
    <radialGradient id="warm" gradientUnits="userSpaceOnUse" cx="{x1:.1f}" cy="{y1:.1f}" r="460">
      <stop offset="0" stop-color="{RUST}" stop-opacity=".30"/>
      <stop offset=".55" stop-color="{RUST}" stop-opacity=".06"/>
      <stop offset="1" stop-color="{RUST}" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="rim" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity=".22"/>
      <stop offset=".35" stop-color="#FFFFFF" stop-opacity=".05"/>
      <stop offset="1" stop-color="#FFFFFF" stop-opacity=".02"/>
    </linearGradient>
    <linearGradient id="arcGrad" gradientUnits="userSpaceOnUse" x1="{x0:.1f}" y1="{y0 - 120:.1f}" x2="{x1:.1f}" y2="{y1:.1f}">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity=".62"/>
      <stop offset=".55" stop-color="#FFFFFF"/>
      <stop offset=".86" stop-color="#FFE3D4"/>
      <stop offset="1" stop-color="{RUST_HI}"/>
    </linearGradient>
    <radialGradient id="lensFill" cx=".32" cy=".26" r=".85">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity="{lens_fill_top}"/>
      <stop offset=".7" stop-color="#A9B4C8" stop-opacity=".06"/>
      <stop offset="1" stop-color="#A9B4C8" stop-opacity=".10"/>
    </radialGradient>
    <linearGradient id="lensRim" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity=".55"/>
      <stop offset=".45" stop-color="#FFFFFF" stop-opacity=".08"/>
      <stop offset="1" stop-color="#FFFFFF" stop-opacity=".22"/>
    </linearGradient>
    <radialGradient id="emberCore">
      <stop offset="0" stop-color="#FFE9DC"/>
      <stop offset=".45" stop-color="{RUST_HI}"/>
      <stop offset="1" stop-color="{RUST}"/>
    </radialGradient>
    <radialGradient id="emberGlow">
      <stop offset="0" stop-color="{RUST}" stop-opacity=".75"/>
      <stop offset=".4" stop-color="{RUST_DEEP}" stop-opacity=".28"/>
      <stop offset="1" stop-color="{RUST_DEEP}" stop-opacity="0"/>
    </radialGradient>
    <clipPath id="tileClip"><path d="{tile}"/></clipPath>
    <clipPath id="lensClip"><circle cx="{OX}" cy="{OY}" r="{OR - 2}"/></clipPath>
    <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">
      <feDropShadow dx="0" dy="14" stdDeviation="18" flood-color="#000" flood-opacity=".38"/>
    </filter>
    <filter id="soft" x="-50%" y="-50%" width="200%" height="200%">
      <feGaussianBlur stdDeviation="6"/>
    </filter>
  </defs>

  <g class="tile">
    <path d="{tile}" fill="url(#bg)"{"" if small else ' filter="url(#shadow)"'}/>
    <g clip-path="url(#tileClip)">
      <rect class="warm" width="{S}" height="{S}" fill="url(#warm)"/>

      <!-- the cast arc -->
      <path class="arc" d="{arc}" fill="none" stroke="url(#arcGrad)"
            stroke-width="{stroke}" stroke-linecap="round"/>

      <!-- glass lens: tinted body, the arc refracted inside it, rim, specular -->
      <g class="lens">
        <circle cx="{OX}" cy="{OY}" r="{OR}" fill="#25262B" fill-opacity=".94"/>
        <g clip-path="url(#lensClip)">
          <path d="{arc}" fill="none" stroke="#FFFFFF" stroke-opacity=".95"
                stroke-width="{stroke * 1.25:.1f}" stroke-linecap="round"
                transform="translate({OX} {OY}) scale(1.14) translate({-OX + 10} {-OY + 12})"/>
        </g>
        <circle cx="{OX}" cy="{OY}" r="{OR}" fill="url(#lensFill)"/>
        <circle cx="{OX}" cy="{OY}" r="{OR - 1.5}" fill="none" stroke="url(#lensRim)" stroke-width="{10 if small else 3}"/>
        <path class="shine" opacity=".7" d="M{OX - OR * 0.72:.1f},{OY - OR * 0.12:.1f} A{OR * 0.74:.1f},{OR * 0.74:.1f} 0 0 1 {OX - OR * 0.1:.1f},{OY - OR * 0.72:.1f}"
              fill="none" stroke="#FFFFFF" stroke-opacity=".55" stroke-width="{max(6.0, stroke * 0.3):.1f}"
              stroke-linecap="round" filter="url(#soft)"/>
      </g>

      <!-- the ember rides the arc head, then glows at rest -->
      <g class="rider">
        <circle class="glow" opacity=".85" cx="{x1:.1f}" cy="{y1:.1f}" r="{ember_r * 4.2:.1f}" fill="url(#emberGlow)"/>
        <circle class="ember" cx="{x1:.1f}" cy="{y1:.1f}" r="{ember_r:.1f}" fill="url(#emberCore)"/>
      </g>
    </g>
    <path d="{tile}" fill="none" stroke="url(#rim)" stroke-width="3"/>
  </g>
</svg>
"""


def glyph(ink: str) -> str:
    """The mark without its tile, for use inside the UI (search field,
    footer). `ink` is the arc colour: white on dark UI, near-black on light."""
    x1, y1 = polar(A1)
    arc = arc_path()
    stroke = 74
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="190 80 780 780" width="780" height="780">
  <title>RustCast</title>
  <path d="{arc}" fill="none" stroke="{ink}" stroke-width="{stroke}" stroke-linecap="round"/>
  <circle cx="{OX}" cy="{OY}" r="{OR - 12}" fill="{ink}" fill-opacity=".14" stroke="{ink}" stroke-opacity=".45" stroke-width="24"/>
  <circle cx="{x1:.1f}" cy="{y1:.1f}" r="{stroke * 0.95:.1f}" fill="{RUST}"/>
</svg>
"""


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "rustcast-mark.svg").write_text(svg(False, 30))
    (OUT / "rustcast-mark-animated.svg").write_text(svg(True, 30))
    (OUT / "rustcast-mark-small.svg").write_text(svg(False, 58, small=True))
    (OUT / "rustcast-glyph-dark.svg").write_text(glyph("#F5F5F7"))
    (OUT / "rustcast-glyph-light.svg").write_text(glyph("#1D1D1F"))
    print(f"wrote marks to {OUT}")


if __name__ == "__main__":
    main()
