#!/usr/bin/env python3
"""Generate the Project Bedrock application icons.

The executable icon is the white creeper mark. It is the right shape for the
job for a specific, checkable reason: an icon is mostly seen at 16-32px in a
title bar and taskbar, and only bold flat regions with hard contrast survive
that. The creeper is a white rounded square with three near-black cutouts and
nothing else, so it stays perfectly legible at 16px. The project's detailed
amethyst-cube artwork does not — it is a fine-grained PBR-style texture, and
rendering it at 16/24/32/48px and comparing showed it turning to an
indistinct dark smudge below roughly 48px, whatever cropping or contrast
boosting was applied first.

The cube stays as the sidebar mark, where it is displayed around 56px and
reads properly.

Output:
    assets/icon.png             window/title-bar icon (creeper)
    assets/icon.ico             embedded into the EXE by
                                crates/bedrock-app/build.rs, all sizes creeper
    assets/ui/cube_logo.png     sidebar mark (cube artwork)

Sources, both already trimmed of transparent padding — untrimmed art wastes
most of its frame and renders visibly smaller than the size it is given,
which is what made earlier icons look undersized no matter what number they
were drawn at:
    assets/ui/creeper.png
    assets/ui/detailed_cube.png

Re-run after changing the artwork:
    python tools/generate_icon.py
"""

import struct
from io import BytesIO
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
CREEPER_SOURCE = ROOT / "assets" / "ui" / "creeper.png"
CUBE_SOURCE = ROOT / "assets" / "ui" / "detailed_cube.png"

# Sizes Windows asks for: 16 in the title bar and Explorer lists, 32 in the
# taskbar and Alt-Tab, up to 256 for large-icon views.
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]


def square(img: Image.Image, size: int) -> Image.Image:
    """Fit `img` into a transparent `size`x`size` frame, preserving aspect.

    Art is not always square — the cube is taller than it is wide — and
    stretching it to a square frame would distort it. This scales the long
    edge to fit and centres the result.
    """
    img = img.crop(img.getbbox())
    scale = size / max(img.width, img.height)
    w = max(1, round(img.width * scale))
    h = max(1, round(img.height * scale))
    resized = img.resize((w, h), Image.LANCZOS)
    frame = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    frame.paste(resized, ((size - w) // 2, (size - h) // 2), resized)
    return frame


def write_ico(path: Path, images: list[Image.Image]) -> None:
    """Write a multi-size .ico with PNG-compressed frames.

    Written directly rather than via `Image.save(format="ICO")` so each size
    is an independently prepared image: Pillow's writer resizes one source
    for every entry, which gives no control over per-size treatment. The
    container is simple — a 6-byte header, a 16-byte directory entry per
    frame, then the PNG data. Modern Windows accepts PNG-compressed frames at
    any size, so no legacy BMP encoding is needed.
    """
    entries, frames = [], []
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
        for entry in entries:
            f.write(entry)
        for frame in frames:
            f.write(frame)


def main() -> int:
    for source in (CREEPER_SOURCE, CUBE_SOURCE):
        if not source.is_file():
            print(f"missing {source}")
            return 1

    creeper = Image.open(CREEPER_SOURCE).convert("RGBA")
    cube = Image.open(CUBE_SOURCE).convert("RGBA")

    square(creeper, 256).save(ROOT / "assets" / "icon.png")
    write_ico(ROOT / "assets" / "icon.ico", [square(creeper, s) for s in ICO_SIZES])
    square(cube, 256).save(ROOT / "assets" / "ui" / "cube_logo.png")

    print(f"wrote {ROOT / 'assets' / 'icon.png'} (creeper)")
    print(f"wrote {ROOT / 'assets' / 'icon.ico'} (creeper, {len(ICO_SIZES)} sizes)")
    print(f"wrote {ROOT / 'assets' / 'ui' / 'cube_logo.png'} (cube)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
