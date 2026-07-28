#!/usr/bin/env python3
"""Generate the Project Bedrock application icons.

Draws a flat-shaded isometric block, in the style of Minecraft's own icon:
three plain colour facets and a bold outline, nothing else. That shape is
deliberate, not a simplification of taste — a first attempt used the
project's detailed amethyst-block artwork directly, and at 16-32px (title
bar, taskbar, Alt-Tab) it read as an indistinct dark smudge no matter how it
was cropped or contrast-boosted. Fine PBR-style texture cannot survive that
much downscaling; Mojang's own icons work at those sizes precisely because
they carry almost none. The palette here is sampled from that original
artwork, so the identity survives even though the rendering style doesn't.

Output:
    assets/icon.png       window/taskbar icon at runtime (256px)
    assets/icon.ico       embedded into the EXE by crates/bedrock-app/build.rs
    assets/ui/cube_logo.png   sidebar mark (same glyph, used at ~56px)

Re-run after changing the design:
    python tools/generate_icon.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent

# Sampled from the original 1056px amethyst-cube artwork (assets/ui/*.png as
# supplied): a bright lavender facet, a deep violet facet, and a near-black
# outline. `LEFT` is a blend between them so the three faces read as one
# consistent material rather than two unrelated colours plus a shadow.
TOP = (0x9E, 0x8A, 0xDA, 255)
LEFT = (0x6A, 0x55, 0xC4, 255)
RIGHT = (0x3A, 0x27, 0xB1, 255)
OUTLINE = (0x0A, 0x08, 0x10, 255)

# Sizes Windows actually asks for: title bar and Explorer list use 16, the
# taskbar and Alt-Tab use 32, large-icon views use up to 256.
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]


def draw_block_icon(size: int, supersample: int = 8) -> Image.Image:
    """One flat-shaded isometric block at `size`x`size`, transparent background.

    Drawn at `supersample`x and downscaled with Lanczos rather than drawn
    directly at the target size: `ImageDraw.polygon` has no anti-aliasing of
    its own, and without it every edge of a 16px cube is a visible staircase.
    This is the one case in this project where that resampling trick is
    correct — it only works because the source is flat vector-like shapes,
    the exact opposite of the detailed cube texture that motivated this file.
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


def main() -> int:
    icon_png = draw_block_icon(256)
    icon_png.save(ROOT / "assets" / "icon.png")

    draw_block_icon(256).save(
        ROOT / "assets" / "icon.ico",
        sizes=[(s, s) for s in ICO_SIZES],
    )

    (ROOT / "assets" / "ui").mkdir(parents=True, exist_ok=True)
    draw_block_icon(256).save(ROOT / "assets" / "ui" / "cube_logo.png")

    print(f"wrote {ROOT / 'assets' / 'icon.png'}")
    print(f"wrote {ROOT / 'assets' / 'icon.ico'} ({len(ICO_SIZES)} sizes)")
    print(f"wrote {ROOT / 'assets' / 'ui' / 'cube_logo.png'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
