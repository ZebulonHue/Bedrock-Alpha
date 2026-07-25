#!/usr/bin/env python3
"""Regenerate crates/bedrock-parser/src/mineways_data.rs from Mineways' tiles.h.

The swatch table is *positional*: entry N describes whichever 16x16 tile sits
at slot N of `assets/terrainExt.png`. So the table and the PNG must come from
the same Mineways revision. When they don't, lookups still "succeed" but point
at the wrong picture -- the symptom is blocks past roughly index 400 rendering
as unrelated textures (deepslate as white, tuff as diamond-blue, calcite as
slate) while early blocks like stone and dirt look perfectly fine.

Usage:
    python tools/gen_mineways_data.py
"""

import io
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TILES_H = os.path.join(REPO, "third_party", "Mineways", "Win", "tiles.h")
OUT = os.path.join(REPO, "crates", "bedrock-parser", "src", "mineways_data.rs")

# { col, row, <int>, <int-or-symbol>, L"name", L"altname", flags },
ROW_RE = re.compile(
    r'^\s*\{\s*(\d+)\s*,\s*(\d+)\s*,\s*[^,]+,\s*[^,]+,\s*L"([^"]*)"', re.M
)


def main() -> int:
    if not os.path.isfile(TILES_H):
        print(f"missing {TILES_H} (is the Mineways submodule checked out?)")
        return 1

    src = io.open(TILES_H, encoding="utf-8", errors="replace").read()
    try:
        start = src.index("gTilesTable[TOTAL_TILES] = {")
    except ValueError:
        print("could not find gTilesTable in tiles.h")
        return 1

    entries = ROW_RE.findall(src[start:])
    if not entries:
        print("parsed 0 entries -- has the tiles.h row format changed?")
        return 1

    # Every entry carries its own (col, row), and lookups always go
    # index -> stored (col, row) -- never index -> arithmetic position. That
    # matters because the table is *not* densely positional: Mineways repeats
    # a tile's coordinates across several entries when one image serves
    # multiple block states, so there are more entries than distinct tiles.
    aliases = len(entries) - len({(c, r) for c, r, _ in entries})

    for i, (col, row, name) in enumerate(entries):
        if int(col) >= 16:
            print(f"entry {i} ({name}) has col {col} outside the 16-tile width")
            return 1

    atlas_h = (max(int(r) for _, r, _ in entries) + 1) * 16
    print(f"{len(entries)} entries, {aliases} sharing a tile with another entry")

    out = io.StringIO()
    out.write(
        "// Auto-generated from third_party/Mineways/Win/tiles.h gTilesTable\n"
        "//\n"
        "// Regenerate with tools/gen_mineways_data.py after updating the Mineways\n"
        "// submodule. This table MUST come from the same Mineways revision as\n"
        "// assets/terrainExt.png: the entries are positional, so a table generated\n"
        "// from a different revision silently maps blocks onto whatever tile now\n"
        "// occupies that slot (deepslate rendering as white, tuff as diamond-blue).\n"
        "//\n"
        f"// {len(entries)} tile entries (one per swatch index); atlas is 256 x {atlas_h}, 16x16 tiles.\n"
        "// Maps swatch_index -> (col, row, texture_name)\n"
        "\n"
        "#![allow(dead_code)]\n"
        "\n"
        "#[rustfmt::skip]\n"
        "pub const TILE_TABLE: &[(u32, u32, &str)] = &[\n"
    )
    for i, (col, row, name) in enumerate(entries):
        out.write(f'    ({col}, {row}, "{name}"),  // {i}\n')
    out.write(
        "];\n"
        "\n"
        "/// Swatch index for a texture filename (first match wins; a few names repeat).\n"
        "pub fn swatch_by_filename(name: &str) -> Option<usize> {\n"
        "    TILE_TABLE.iter().position(|&(_, _, n)| n == name)\n"
        "}\n"
        "\n"
        "/// Height of `terrainExt.png` in pixels; every row of `TILE_TABLE` must fall\n"
        "/// inside it, otherwise the UVs below would sample past the image.\n"
        f"pub const ATLAS_HEIGHT: f32 = {atlas_h}.0;\n"
        "/// Width of `terrainExt.png` in tiles.\n"
        "pub const ATLAS_COLS: f32 = 16.0;\n"
        "\n"
        "/// Get UV coordinates for a swatch: [u0, v0, u1, v1] in atlas space.\n"
        "pub fn swatch_uv(swatch: usize) -> Option<[f32; 4]> {\n"
        "    let (col, row, _) = TILE_TABLE.get(swatch)?;\n"
        "    // Inset by half a texel. Sampling exactly on a tile boundary lets\n"
        "    // the neighbouring tile bleed in under any filtering, drawing a\n"
        "    // thin grid of wrong-coloured lines along every block edge.\n"
        "    let du = 0.5 / (ATLAS_COLS * 16.0);\n"
        "    let dv = 0.5 / ATLAS_HEIGHT;\n"
        "    let u0 = *col as f32 / ATLAS_COLS + du;\n"
        "    let v0 = *row as f32 * 16.0 / ATLAS_HEIGHT + dv;\n"
        "    let u1 = (*col + 1) as f32 / ATLAS_COLS - du;\n"
        "    let v1 = (*row + 1) as f32 * 16.0 / ATLAS_HEIGHT - dv;\n"
        "    Some([u0, v0, u1, v1])\n"
        "}\n"
    )

    io.open(OUT, "w", encoding="utf-8").write(out.getvalue())
    print(f"wrote {OUT}: {len(entries)} entries, atlas height {atlas_h}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
