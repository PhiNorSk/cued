"""Generate the 1024x1024 Cued app-icon master PNG.

Design: a macOS-style rounded square (824/1024 with margin, per Apple's icon
grid) filled with a subtle vertical gradient from --surface down to --ground,
carrying the Logo.tsx mark at 72% of the square: ring in --accent, cue dot in
--accent-hi, play triangle in --text. Corners outside the square are
transparent (macOS composes the shape as-is; no full bleed).

Usage:
    python3 scripts/gen-app-icon.py <out.png>
    npx tauri icon <out.png>          # regenerates src-tauri/icons/*

Token hex values mirror src/index.css — keep them in sync if tokens change.
"""

import sys

from cued_render import hex_rgb, lerp_rgb, logo_mark, render, rounded_rect, write_png

GROUND = hex_rgb("#0c0f0d")  # --ground
SURFACE = hex_rgb("#141815")  # --surface
ACCENT = (*hex_rgb("#3e8e6e"), 1.0)  # --accent (ring)
ACCENT_HI = (*hex_rgb("#4fa684"), 1.0)  # --accent-hi (cue dot)
TEXT = (*hex_rgb("#eaf0ec"), 1.0)  # --text (play triangle)

SIZE = 1024
SQUARE = 824  # Apple icon grid: the rounded square fills 824/1024 of the canvas
CORNER = 0.225 * SQUARE  # ~185 px, the Big Sur squircle corner proportion
MARK_SCALE = 0.72  # mark side relative to the square side


def plate_color(x, y):
    top = (SIZE - SQUARE) / 2
    t = max(0.0, min(1.0, (y - top) / SQUARE))
    return (*lerp_rgb(SURFACE, GROUND, t), 1.0)


def main(out_path):
    mark_side = MARK_SCALE * SQUARE
    origin = (SIZE - mark_side) / 2
    shapes = [rounded_rect(SIZE / 2, SIZE / 2, SQUARE, SQUARE, CORNER, plate_color)]
    shapes += logo_mark(origin, origin, mark_side / 48, ACCENT, ACCENT_HI, TEXT)
    rows = render(SIZE, SIZE, shapes, lambda x, y: (0.0, 0.0, 0.0, 0.0))
    write_png(out_path, SIZE, SIZE, rows)


if __name__ == "__main__":
    main(sys.argv[1])
