"""Rasterize the Cued logo (src/components/Logo.tsx SVG geometry) as a
monochrome macOS template PNG: black shapes, alpha = coverage.

Shapes in the 48x48 viewBox:
  - ring: circle r=19 at (24,24), stroke 3.5, dasharray 95 25, rotate(-58),
    round linecaps
  - cue dot: circle r=4 at (24,5)
  - play head: triangle (20,17.5) (20,30.5) (31,24)
"""

import math
import struct
import sys
import zlib

CX, CY, R, HALF_STROKE = 24.0, 24.0, 19.0, 1.75
DASH_ON, DASH_PERIOD = 95.0, 120.0
ROT = math.radians(-58.0)
DOT = (24.0, 5.0, 4.0)
TRI = ((20.0, 17.5), (20.0, 30.5), (31.0, 24.0))


def arc_endpoints():
    pts = []
    for s in (0.0, DASH_ON):
        phi = s / R + ROT
        pts.append((CX + R * math.cos(phi), CY + R * math.sin(phi)))
    return pts


CAPS = arc_endpoints()


def covered(x, y):
    # ring (dashed arc with round caps)
    dx, dy = x - CX, y - CY
    dist = math.hypot(dx, dy)
    if abs(dist - R) <= HALF_STROKE:
        s = ((math.atan2(dy, dx) - ROT) % (2 * math.pi)) * R
        if s % DASH_PERIOD <= DASH_ON:
            return True
    for ex, ey in CAPS:
        if math.hypot(x - ex, y - ey) <= HALF_STROKE:
            return True
    # cue dot
    if math.hypot(x - DOT[0], y - DOT[1]) <= DOT[2]:
        return True
    # play triangle (convex, consistent winding via sign of cross products)
    (ax, ay), (bx, by), (cx2, cy2) = TRI
    d1 = (bx - ax) * (y - ay) - (by - ay) * (x - ax)
    d2 = (cx2 - bx) * (y - by) - (cy2 - by) * (x - bx)
    d3 = (ax - cx2) * (y - cy2) - (ay - cy2) * (x - cx2)
    return (d1 >= 0 and d2 >= 0 and d3 >= 0) or (d1 <= 0 and d2 <= 0 and d3 <= 0)


def render(size):
    scale = 48.0 / size
    ss = 4  # 4x4 supersampling
    rows = []
    for py in range(size):
        row = bytearray([0])  # PNG filter type 0
        for px in range(size):
            hits = 0
            for sy in range(ss):
                for sx in range(ss):
                    x = (px + (sx + 0.5) / ss) * scale
                    y = (py + (sy + 0.5) / ss) * scale
                    hits += covered(x, y)
            alpha = round(255 * hits / (ss * ss))
            row += bytes((0, 0, 0, alpha))
        rows.append(bytes(row))
    return b"".join(rows)


def chunk(tag, data):
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data))
    )


def write_png(path, size):
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # RGBA8
    idat = zlib.compress(render(size), 9)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", idat)
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as f:
        f.write(png)
    print(f"wrote {path} ({size}x{size})")


if __name__ == "__main__":
    out = sys.argv[1]
    write_png(f"{out}/tray-template.png", 22)
    write_png(f"{out}/tray-template@2x.png", 44)
