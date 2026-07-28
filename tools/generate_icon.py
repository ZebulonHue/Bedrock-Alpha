#!/usr/bin/env python3
"""Generate the Project Bedrock application icons.

Two icon designs, used at the sizes each one actually works at, decided by
rendering both and comparing them directly rather than by guessing:

  * assets/ui/detailed_cube.png — the project's real amethyst-cube artwork.
    Genuinely reads once there is enough resolution for its facets and
    texture to register: legible from 48px up, confirmed by rendering it at
    16/24/32/48/64px and comparing side by side. Below 48px it is an
    indistinct dark smudge no matter how it is cropped or contrast-boosted —
    all three were tried before concluding this, not assumed.

  * A flat-shaded isometric block, in the style of Minecraft's own icon:
    three plain colour facets and a bold outline, nothing else. Used only
    where the real art fails -- 16/24/32px, i.e. the title bar and taskbar,
    the two places an icon is seen constantly rather than glanced at. Its
    palette is sampled from the real artwork so the identity still carries
    over even though the rendering style doesn't.

Output:
    assets/icon.png            window/title-bar icon at runtime (flat glyph:
                                that context is always tiny on screen)
    assets/icon.ico             embedded into the EXE by
                                crates/bedrock-app/build.rs -- flat glyph at
                                16/24/32px, real artwork at 48px and above,
                                so Explorer/Alt-Tab/large-icon views show the
                                real art while the taskbar stays legible
    assets/ui/cube_logo.png     sidebar mark, real artwork (displayed at
                                ~56px, comfortably inside the range it reads)

Re-run after changing the design:
    python tools/generate_icon.py
"""

import struct
from io import BytesIO
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
DETAILED_SOURCE = ROOT / "assets" / "ui" / "detailed_cube.png"

# Sampled from the real artwork: a bright lavender facet, a deep violet
# facet, and a near-black outline. `LEFT` is a blend between them so the
# three faces read as one consistent material rather than two unrelated
# colours plus a shadow.
TOP = (0x9E, 0x8A, 0xDA, 255)
LEFT = (0x6A, 0x55, 0xC4, 255)
RIGHT = (0x3A, 0x27, 0xB1, 255)
OUTLINE = (0x0A, 0x08, 0x10, 255)

# Below this, only the flat glyph is legible; at and above it the real
# artwork is. See the module docstring for how this boundary was found.
REAL_ART_MIN_SIZE = 48
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]


def draw_block_icon(size: int, supersample: int = 8) -> Image.Image:
    """One flat-shaded isometric block at `size`x`size`, transparent background.

    Drawn at `supersample`x and downscaled with Lanczos rather than drawn
    directly at the target size: `ImageDraw.polygon` has no anti-aliasing of
    its own, and without it every edge of a 16px cube is a visible staircase.
    This resampling trick is only correct here because the source is flat
    vector-like shapes -- the exact opposite of the detailed cube texture
    this glyph exists to stand in for.
    """
    s = size * supersample
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    cx = s / 2
    half_w = s * 0.42
    top_h = s * 0.24
    side_h = s * 0.40
    top_y = s * 0.5 - side_h * 0.5

    top_pt = (cx, top_y - top_h * 0.5)
    left_pt = (cx - half_w, top_y + top_h * 0.25)
    right_pt = (cx + half_w, top_y + top_h * 0.25)
    front_pt = (cx, top_y + top_h)

    left_bot = (left_pt[0], left_pt[1] + side_h)
    right_bot = (right_pt[0], right_pt[1] + side_h)
    front_bot = (front_pt[0], front_pt[1] + side_h)

    d.polygon([top_pt, right_pt, front_pt, left_pt], fill=TOP)
    d.polygon([left_pt, front_pt, front_bot, left_bot], fill=LEFT)
    d.polygon([right_pt, front_pt, front_bot, right_bot], fill=RIGHT)

    width = max(1, s // 48)
    for poly in (
        [top_pt, right_pt, front_pt, left_pt, top_pt],
        [left_pt, front_pt, front_bot, left_bot, left_pt],
        [right_pt, front_pt, front_bot, right_bot, right_pt],
    ):
        d.line(poly, fill=OUTLINE, width=width, joint="curve")

    return img.resize((size, size), Image.LANCZOS)


def icon_at(size: int, detailed: Image.Image) -> Image.Image:
    """The right design for `size`, real artwork or flat glyph."""
    if size >= REAL_ART_MIN_SIZE:
        return detailed.resize((size, size), Image.LANCZOS)
    return draw_block_icon(size)


def write_ico(path: Path, images: list[Image.Image]) -> None:
    """Write a multi-size .ico with PNG-compressed frames, one PNG per size.

    Pillow's own `Image.save(..., format="ICO", sizes=[...])` only resizes a
    single source image for every requested size, which cannot mix two
    different designs into one file. The ICO container itself is simple
    enough to write directly: a 6-byte directory header, one 16-byte entry
    per frame, then the PNG-encoded frames back to back. Every modern
    Windows version accepts PNG-compressed ICO frames at any size, not only
    256px, so no legacy BMP encoding is needed.
    """
    entries = []
    frames = []
    offset = 6 + 16 * len(images)
    for img in images:
        buf = BytesIO()
        img.save(buf, format="PNG")
        data = buf.getvalue()
        w = img.width if img.width < 256 else 0
        h = img.height if img.height < 256 else 0
        entries.append(struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(data), offset))
        frames.append(data)
        offset += len(data)

    with open(path, "wb") as f:
        f.write(struct.pack("<HHH", 0, 1, len(images)))
        for e in entries:
            f.write(e)
        for frame in frames:
            f.write(frame)


def main() -> int:
    if not DETAILED_SOURCE.is_file():
        print(f"missing {DETAILED_SOURCE} -- the real cube artwork must exist first")
        return 1
    detailed = Image.open(DETAILED_SOURCE).convert("RGBA")

    # Window/title-bar icon: always shown tiny on screen, so always the flat
    # glyph regardless of what size PNG is provided -- there is no OS-level
    # per-size selection for this one, unlike the .ico.
    draw_block_icon(256).save(ROOT / "assets" / "icon.png")

    write_ico(
        ROOT / "assets" / "icon.ico",
        [icon_at(s, detailed) for s in ICO_SIZES],
    )

    (ROOT / "assets" / "ui").mkdir(parents=True, exist_ok=True)
    detailed.resize((256, 256), Image.LANCZOS).save(ROOT / "assets" / "ui" / "cube_logo.png")

    print(f"wrote {ROOT / 'assets' / 'icon.png'} (flat glyph)")
    print(f"wrote {ROOT / 'assets' / 'icon.ico'}: flat glyph <{REAL_ART_MIN_SIZE}px, real artwork >=")
    print(f"wrote {ROOT / 'assets' / 'ui' / 'cube_logo.png'} (real artwork)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
