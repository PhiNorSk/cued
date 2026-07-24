"""Generate the DMG installer-window background for Cued.

660x400 pt, rendered at 2x (1320x800 px) with 144-dpi metadata: Finder reads
the pHYs chunk and shows the image at point size, so one file is crisp on
Retina and correct on non-Retina. Design: --ground with a faint radial glow,
the Cued mark + a stroke-built "Cued" wordmark small at the top, and a
soundwave that swells from the app-icon position toward the /Applications
alias (grey fading into accent green, small chevron tip). No text — the wave
carries the "drag to Applications" instruction.

Finder draws the icon labels itself — black in light mode, white in dark mode,
with no override — so a mid-luminance pad sits behind each label position to
keep both readable on the dark backdrop (~4.5:1 contrast either way). The pad
is a soft-edged "spotlight" capsule (flat core, feathered falloff): the exact
label baseline can only be derived from Finder's icon-view geometry, and the
feather keeps a few points of vertical drift invisible.

ICON_Y / APP_X / FOLDER_X must stay in sync with bundle > macOS > dmg in
src-tauri/tauri.conf.json. Token hex values mirror src/index.css.

Usage:
    python3 scripts/gen-dmg-background.py src-tauri/dmg/dmg-background.png
    ... --grid   # overlay a calibration grid in the label zone (dev only):
                 # horizontal lines every 5 pt (green = LABEL_CENTER_Y,
                 # amber = +-10 pt) plus vertical center lines at the icon
                 # positions, to locate Finder's real label placement.
"""

import math
import sys

from cued_render import (
    Shape,
    arc_stroke,
    hex_rgb,
    lerp_rgb,
    logo_mark,
    render,
    stroke,
    write_png,
)

GROUND = hex_rgb("#0c0f0d")  # --ground
SURFACE_2 = hex_rgb("#1b211d")  # --surface-2 (glow peak)
TEXT = hex_rgb("#eaf0ec")  # --text
TEXT_MUT = hex_rgb("#8b978f")  # --text-mut
ACCENT = (*hex_rgb("#3e8e6e"), 1.0)  # --accent
ACCENT_HI = (*hex_rgb("#4fa684"), 1.0)  # --accent-hi

W_PT, H_PT = 660, 400  # must match dmg.windowSize
SCALE = 2
DPI = 144

APP_X, FOLDER_X, ICON_Y = 170, 490, 195  # must match dmg icon positions
ICON_SIZE = 128  # Finder icon size the Tauri dmg script configures
# 16 pt label line under the icon; +20 measured against a calibration-grid
# build of THIS dmg in Finder (label glyphs sat 13 pt below the grid center)
LABEL_CENTER_Y = ICON_Y + ICON_SIZE / 2 + 20
PAD_H = 22  # full-contrast core height
PAD_FEATHER = 8
PAD_WIDTHS = {APP_X: 56, FOLDER_X: 110}  # sized to "Cued" / "Applications"

GLOW_CENTER = (330, 150)
GLOW_SIGMA = 170
GLOW_PEAK = 0.55

LOCKUP_CENTER_Y = 52
MARK_SIDE = 26
WORD_CAP = 18
WORD_GAP = 12

# Stroke-built geometric wordmark, authored on a 20-unit cap height (y down,
# baseline at 20, x-height top at 6). Circular bowls + round caps echo the mark.
LETTER_STROKE = 2.6


def wordmark(x0, y0, cap_h, color):
    u = cap_h / 20.0
    w = LETTER_STROKE * u
    shapes = []

    def seg(x, p0, p1):
        shapes.append(
            stroke((x0 + (x + p0[0]) * u, y0 + p0[1] * u), (x0 + (x + p1[0]) * u, y0 + p1[1] * u), w, color)
        )

    def arc(x, cx, cy, r, a0, a1):
        shapes.append(arc_stroke(x0 + (x + cx) * u, y0 + cy * u, r * u, a0, a1, w, color))

    x = 0.0
    # C — near-full circle, gap on the right (echoes the ring mark)
    arc(x, 10, 10, 8.7, 35, 325)
    x += 25
    # u — two stems + bottom bowl, tail down to the baseline
    seg(x, (1.3, 7.3), (1.3, 12.2))
    arc(x, 7.8, 12.2, 6.5, 0, 180)
    seg(x, (14.3, 7.3), (14.3, 18.7))
    x += 20.6
    # e — crossbar + circle open to the lower right
    seg(x, (1.3, 13), (12.7, 13))
    arc(x, 7, 13, 5.7, 60, 360)
    x += 19
    # d — bowl + ascender stem
    arc(x, 7, 13, 5.7, 0, 360)
    seg(x, (12.7, 1.3), (12.7, 18.7))
    x += 14
    return shapes, x * u


def wordmark_width(cap_h):
    return wordmark(0, 0, cap_h, (0, 0, 0, 1))[1]


def soft_pill(cx, cy, w, h, feather, rgb, max_alpha):
    """A capsule with a flat-alpha core and a smoothstep falloff over `feather`."""
    half_span = (w - h) / 2
    r = h / 2

    def sd(x, y):
        dx = max(abs(x - cx) - half_span, 0.0)
        return math.hypot(dx, y - cy) - r

    def covers(x, y):
        return sd(x, y) <= feather

    def color(x, y):
        t = max(0.0, min(1.0, sd(x, y) / feather))
        s = 1.0 - t
        return (*rgb, max_alpha * s * s * (3 - 2 * s))

    b = feather + r
    return Shape(covers, color, (cx - w / 2 - feather, cy - b, cx + w / 2 + feather, cy + b))


def backdrop(x, y):
    d2 = (x - GLOW_CENTER[0]) ** 2 + (y - GLOW_CENTER[1]) ** 2
    g = GLOW_PEAK * math.exp(-d2 / (2 * GLOW_SIGMA * GLOW_SIGMA))
    return (*lerp_rgb(GROUND, SURFACE_2, g), 1.0)


def main(out_path):
    shapes = []

    lockup_w = MARK_SIDE + WORD_GAP + wordmark_width(WORD_CAP)
    lockup_x = (W_PT - lockup_w) / 2
    shapes += logo_mark(
        lockup_x,
        LOCKUP_CENTER_Y - MARK_SIDE / 2,
        MARK_SIDE / 48,
        ACCENT,
        ACCENT_HI,
        (*TEXT, 1.0),
    )
    word_shapes, _ = wordmark(
        lockup_x + MARK_SIDE + WORD_GAP,
        LOCKUP_CENTER_Y - 13 * (WORD_CAP / 20.0),  # optical middle of the x-height bowls
        WORD_CAP,
        (*TEXT, 0.92),
    )
    shapes += word_shapes

    # soundwave "arrow": pill bars swelling toward Applications, grey -> accent
    bars = 18
    for i in range(bars):
        t = i / (bars - 1)
        h = (8 + 20 * t) * (0.6, 1.0, 0.8, 0.95, 0.7)[i % 5]
        color = (*lerp_rgb(TEXT_MUT, ACCENT_HI[:3], 0.85 * t), 0.4 + 0.35 * t)
        x = 248 + i * 8
        shapes.append(stroke((x, ICON_Y - h / 2), (x, ICON_Y + h / 2), 3.5, color))
    tip = (404, ICON_Y)
    chevron = (*ACCENT_HI[:3], 0.75)
    shapes.append(stroke(tip, (395, ICON_Y - 7), 3.5, chevron))
    shapes.append(stroke(tip, (395, ICON_Y + 7), 3.5, chevron))

    # label pads (see module docstring): one soft mid-grey spotlight per label
    for cx, pad_w in PAD_WIDTHS.items():
        shapes.append(soft_pill(cx, LABEL_CENTER_Y, pad_w, PAD_H, PAD_FEATHER, TEXT_MUT, 0.78))

    if "--grid" in sys.argv:
        for dy in range(-25, 30, 5):
            y = LABEL_CENTER_Y + dy
            color = (
                (*ACCENT_HI[:3], 1.0)
                if dy == 0
                else (*hex_rgb("#c9915b"), 0.9)
                if abs(dy) == 10
                else (*TEXT, 0.45)
            )
            shapes.append(stroke((40, y), (W_PT - 40, y), 1, color))
        for cx in (APP_X, FOLDER_X):
            shapes.append(
                stroke((cx, LABEL_CENTER_Y - 28), (cx, LABEL_CENTER_Y + 28), 1, (*ACCENT_HI[:3], 1.0))
            )

    rows = render(W_PT * SCALE, H_PT * SCALE, shapes, backdrop, scale=SCALE)
    write_png(out_path, W_PT * SCALE, H_PT * SCALE, rows, dpi=DPI)


if __name__ == "__main__":
    main(sys.argv[1])
