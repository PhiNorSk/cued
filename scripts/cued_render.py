"""Shared stdlib-only rasterizer for Cued brand assets (app icon, DMG background).

Same philosophy as gen-tray-icons.py: analytic shape coverage + supersampling,
no image libraries, so every binary asset in the repo stays reproducible from
checked-in source.

Conventions: coordinates are y-down; angles are in degrees with 0 = +x,
increasing clockwise (a consequence of y pointing down). Colors are
(r, g, b, alpha) with r/g/b in 0..255 and alpha in 0..1 (straight, not
premultiplied).
"""

import math
import struct
import zlib


def hex_rgb(value):
    value = value.lstrip("#")
    return tuple(int(value[i : i + 2], 16) for i in (0, 2, 4))


def lerp_rgb(c0, c1, t):
    return tuple(c0[i] + (c1[i] - c0[i]) * t for i in range(3))


class Shape:
    """A filled region: boolean coverage test + a color sampler + a bbox.

    `color` may be a constant rgba tuple or a callable (x, y) -> rgba, which
    lets fills carry gradients. The bbox lets the renderer skip shapes cheaply.
    """

    def __init__(self, covers, color, bbox):
        self.covers = covers
        self.color = color if callable(color) else (lambda x, y, c=color: c)
        self.bbox = bbox


def rounded_rect(cx, cy, w, h, radius, color):
    hw, hh = w / 2 - radius, h / 2 - radius

    def covers(x, y):
        dx = abs(x - cx) - hw
        dy = abs(y - cy) - hh
        if dx <= 0 and dy <= 0:
            return True
        return math.hypot(max(dx, 0), max(dy, 0)) <= radius

    return Shape(covers, color, (cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2))


def circle(cx, cy, r, color):
    def covers(x, y):
        return math.hypot(x - cx, y - cy) <= r

    return Shape(covers, color, (cx - r, cy - r, cx + r, cy + r))


def triangle(a, b, c, color):
    (ax, ay), (bx, by), (cx2, cy2) = a, b, c

    def covers(x, y):
        d1 = (bx - ax) * (y - ay) - (by - ay) * (x - ax)
        d2 = (cx2 - bx) * (y - by) - (cy2 - by) * (x - bx)
        d3 = (ax - cx2) * (y - cy2) - (ay - cy2) * (x - cx2)
        return (d1 >= 0 and d2 >= 0 and d3 >= 0) or (d1 <= 0 and d2 <= 0 and d3 <= 0)

    xs, ys = (ax, bx, cx2), (ay, by, cy2)
    return Shape(covers, color, (min(xs), min(ys), max(xs), max(ys)))


def stroke(p0, p1, width, color):
    """A line segment with round caps."""
    (x0, y0), (x1, y1) = p0, p1
    dx, dy = x1 - x0, y1 - y0
    len2 = dx * dx + dy * dy
    half = width / 2

    def covers(x, y):
        t = 0.0 if len2 == 0 else max(0.0, min(1.0, ((x - x0) * dx + (y - y0) * dy) / len2))
        return math.hypot(x - (x0 + t * dx), y - (y0 + t * dy)) <= half

    bbox = (min(x0, x1) - half, min(y0, y1) - half, max(x0, x1) + half, max(y0, y1) + half)
    return Shape(covers, color, bbox)


def arc_stroke(cx, cy, r, a0_deg, a1_deg, width, color):
    """A circular arc stroked with round caps, covered clockwise from a0 to a1.

    a0 == 0 and a1 == 360 yields a full circle (no caps).
    """
    half = width / 2
    span = (a1_deg - a0_deg) % 360
    full = span == 0 and a0_deg != a1_deg
    caps = []
    if not full:
        for a in (a0_deg, a1_deg):
            phi = math.radians(a)
            caps.append((cx + r * math.cos(phi), cy + r * math.sin(phi)))

    def covers(x, y):
        dx, dy = x - cx, y - cy
        if abs(math.hypot(dx, dy) - r) <= half:
            if full:
                return True
            rel = (math.degrees(math.atan2(dy, dx)) - a0_deg) % 360
            if rel <= span:
                return True
        return any(math.hypot(x - ex, y - ey) <= half for ex, ey in caps)

    b = r + half
    return Shape(covers, color, (cx - b, cy - b, cx + b, cy + b))


def dashed_ring(cx, cy, r, width, dash_on, dash_period, rot_deg, color):
    """The Logo.tsx ring: a dashed circle stroke with round caps at dash ends."""
    rot = math.radians(rot_deg)
    half = width / 2
    caps = []
    for s_len in (0.0, dash_on):
        phi = s_len / r + rot
        caps.append((cx + r * math.cos(phi), cy + r * math.sin(phi)))

    def covers(x, y):
        dx, dy = x - cx, y - cy
        if abs(math.hypot(dx, dy) - r) <= half:
            s_len = ((math.atan2(dy, dx) - rot) % (2 * math.pi)) * r
            if s_len % dash_period <= dash_on:
                return True
        return any(math.hypot(x - ex, y - ey) <= half for ex, ey in caps)

    b = r + half
    return Shape(covers, color, (cx - b, cy - b, cx + b, cy + b))


def logo_mark(origin_x, origin_y, scale, ring_color, dot_color, tri_color):
    """The Logo.tsx mark (48x48 viewBox geometry) placed at origin, scaled.

    Geometry mirrors src/components/Logo.tsx: dashed ring r=19 stroke 3.5
    (dasharray 95 25, rotate -58), cue dot r=4 at (24,5), play triangle
    (20,17.5)(20,30.5)(31,24). The dot sits inside the ring's dash gap.
    """
    s = scale

    def pt(x, y):
        return (origin_x + x * s, origin_y + y * s)

    return [
        dashed_ring(*pt(24, 24), 19 * s, 3.5 * s, 95 * s, 120 * s, -58, ring_color),
        triangle(pt(20, 17.5), pt(20, 30.5), pt(31, 24), tri_color),
        circle(*pt(24, 5), 4 * s, dot_color),
    ]


def _over(dst, src):
    dr, dg, db, da = dst
    sr, sg, sb, sa = src
    oa = sa + da * (1 - sa)
    if oa == 0:
        return (0.0, 0.0, 0.0, 0.0)
    f = da * (1 - sa)
    return ((sr * sa + dr * f) / oa, (sg * sa + dg * f) / oa, (sb * sa + db * f) / oa, oa)


def _composite(x, y, shapes, background):
    rgba = background(x, y)
    for s in shapes:
        if s.covers(x, y):
            rgba = _over(rgba, s.color(x, y))
    return rgba


def _quant(rgba):
    r, g, b, a = rgba
    return (
        max(0, min(255, round(r))),
        max(0, min(255, round(g))),
        max(0, min(255, round(b))),
        max(0, min(255, round(a * 255))),
    )


def render(width, height, shapes, background, scale=1.0, ss=4):
    """Render RGBA scanlines (with PNG filter-0 prefix bytes).

    Sample coordinates are pixel coordinates divided by `scale`, so shapes can
    be authored in points and rendered @2x. Supersampling is adaptive: a pixel
    whose corner+center probes agree on shape coverage gets one sample; edge
    pixels get ss*ss samples (accumulated premultiplied so transparent samples
    don't tint the average).
    """
    inv = 1.0 / scale
    rows = []
    for py in range(height):
        y0, y1 = py * inv, (py + 1) * inv
        row_shapes = [s for s in shapes if s.bbox[1] <= y1 and s.bbox[3] >= y0]
        row = bytearray([0])
        for px in range(width):
            x0, x1 = px * inv, (px + 1) * inv
            xc, yc = (x0 + x1) / 2, (y0 + y1) / 2
            cands = [s for s in row_shapes if s.bbox[0] <= x1 and s.bbox[2] >= x0]
            if not cands:
                rgba = background(xc, yc)
            else:
                probes = ((x0, y0), (x1, y0), (x0, y1), (x1, y1), (xc, yc))
                sigs = {tuple(s.covers(x, y) for s in cands) for x, y in probes}
                if len(sigs) == 1:
                    rgba = _composite(xc, yc, cands, background)
                else:
                    pr = pg = pb = pa = 0.0
                    for sy in range(ss):
                        for sx in range(ss):
                            x = x0 + (sx + 0.5) * inv / ss
                            y = y0 + (sy + 0.5) * inv / ss
                            r, g, b, a = _composite(x, y, cands, background)
                            pr += r * a
                            pg += g * a
                            pb += b * a
                            pa += a
                    if pa == 0:
                        rgba = (0.0, 0.0, 0.0, 0.0)
                    else:
                        rgba = (pr / pa, pg / pa, pb / pa, pa / (ss * ss))
            row += bytes(_quant(rgba))
        rows.append(bytes(row))
    return rows


def _chunk(tag, data):
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data))
    )


def write_png(path, width, height, rows, dpi=None):
    """Write RGBA8 PNG. `dpi` adds a pHYs chunk (Finder uses it to derive the
    point size of DMG backgrounds, which is how one 2x image serves Retina)."""
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    png = b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", ihdr)
    if dpi is not None:
        ppm = round(dpi / 0.0254)
        png += _chunk(b"pHYs", struct.pack(">IIB", ppm, ppm, 1))
    png += _chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + _chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    print(f"wrote {path} ({width}x{height})")
