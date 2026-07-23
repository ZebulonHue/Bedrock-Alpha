#!/usr/bin/env python3
"""Generate the Project Bedrock application icons.

Draws a 256x256 RGBA PNG (dark rounded tile with a blocky grass-green "B")
using only the standard library, then wraps the same PNG in a PNG-compressed
ICO for the Windows executable.

Output:
    assets/icon.png   used as the window/taskbar icon at runtime
    assets/icon.ico   embedded into the EXE by crates/bedrock-app/build.rs

Re-run after changing the design:
    python tools/generate_icon.py
"""

import math
import struct
import zlib
from pathlib import Path

SIZE = 256
CORNER_RADIUS = 48

BG = (26, 29, 36, 255)  # dark slate tile
TOP = (139, 217, 79, 255)  # light grass green (top of glyph)
BOTTOM = (78, 154, 46, 255)  # deep grass green (bottom of glyph)
SHADOW = (13, 16, 20, 255)  # glyph drop shadow

# 5x7 blocky "B".
GLYPH = [
    "11110",
    "10001",
    "10001",
    "11110",
    "10001",
    "10001",
    "11110",
]

GLYPH_SCALE = 26  # pixels per glyph cell
SHADOW_OFFSET = 6

ASSETS = Path(__file__).resolve().parent.parent / "assets"


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(4))


def inside_rounded_tile(x, y):
    """Signed-distance test for the rounded-square tile."""
    c = SIZE / 2 - 0.5
    half = SIZE / 2
    qx = abs(x - c) - (half - CORNER_RADIUS)
    qy = abs(y - c) - (half - CORNER_RADIUS)
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    inside = min(max(qx, qy), 0.0)
    return outside + inside - CORNER_RADIUS <= 0.0


def glyph_rects(offset_x, offset_y):
    """Yield (x, y, size, color) rectangles for every lit glyph cell."""
    ox = (SIZE - 5 * GLYPH_SCALE) // 2 + offset_x
    oy = (SIZE - 7 * GLYPH_SCALE) // 2 + offset_y
    for row, line in enumerate(GLYPH):
        color = lerp(TOP, BOTTOM, row / (len(GLYPH) - 1))
        for col, bit in enumerate(line):
            if bit == "1":
                yield ox + col * GLYPH_SCALE, oy + row * GLYPH_SCALE, GLYPH_SCALE, color


def render_png_bytes():
    px = bytearray(SIZE * SIZE * 4)

    def put(x, y, rgba):
        i = (y * SIZE + x) * 4
        px[i : i + 4] = bytes(rgba)

    def fill_rect(x0, y0, size, color):
        for yy in range(y0, y0 + size):
            for xx in range(x0, x0 + size):
                if 0 <= xx < SIZE and 0 <= yy < SIZE:
                    put(xx, yy, color)

    for y in range(SIZE):
        for x in range(SIZE):
            if inside_rounded_tile(x, y):
                put(x, y, BG)

    # Shadow pass first, glyph on top. The glyph is inset far enough that
    # the offset shadow never spills outside the tile.
    for x0, y0, size, _ in glyph_rects(SHADOW_OFFSET, SHADOW_OFFSET):
        fill_rect(x0, y0, size, SHADOW)
    for x0, y0, size, color in glyph_rects(0, 0):
        fill_rect(x0, y0, size, color)

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes(px[y * SIZE * 4 : (y + 1) * SIZE * 4]) for y in range(SIZE))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def wrap_ico(png_bytes):
    """Single-image ICO whose payload is a PNG (supported since Vista)."""
    header = struct.pack("<HHH", 0, 1, 1)
    entry = struct.pack(
        "<BBBBHHII",
        0,  # width byte: 0 means 256
        0,  # height byte: 0 means 256
        0,  # palette colors
        0,  # reserved
        1,  # color planes
        32,  # bits per pixel
        len(png_bytes),
        6 + 16,  # offset of image data
    )
    return header + entry + png_bytes


def main():
    ASSETS.mkdir(exist_ok=True)
    png = render_png_bytes()
    (ASSETS / "icon.png").write_bytes(png)
    (ASSETS / "icon.ico").write_bytes(wrap_ico(png))
    print(f"wrote {ASSETS / 'icon.png'} ({len(png)} bytes)")
    print(f"wrote {ASSETS / 'icon.ico'}")


if __name__ == "__main__":
    main()
