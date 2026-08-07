"""Generate `block_entity_models.rs`.

Some blocks have no geometry in any model JSON, because the game draws them
from Java code in a BlockEntityRenderer instead: chests, bells, shulker boxes,
conduits, decorated pots, banners. Parsing the vanilla assets -- which is how
every other block's shape reaches us -- finds nothing, so those blocks fall back
to a plain cube. That is why a bell exports as a bare mounting post and a chest
as a featureless box.

The shapes here are therefore transcribed from the renderers' own model
definitions rather than extracted. Each is a list of boxes in Minecraft's model
space, with a texture origin, and this script lays out the box UVs the way the
game does so the numbers are computed once rather than by hand.

    python tools/gen_block_entity_models.py > crates/bedrock-parser/src/block_entity_models.rs

Model space is 0..16 pixels per block with Y up. Texture coordinates are given
in the entity texture's own pixels and converted here into the 0..16 space the
rest of the model code uses.
"""

# Our CUBE_FACES order: 0 up(+y), 1 down(-y), 2 +x, 3 -x, 4 +z, 5 -z.
UP, DOWN, POS_X, NEG_X, POS_Z, NEG_Z = 0, 1, 2, 3, 4, 5


def box_uvs(u, v, w, h, d, tex_w, tex_h):
    """Lay out a cuboid's six faces the way Minecraft unwraps them.

    A box of size (w, h, d) at texture origin (u, v) occupies a fixed cross
    shape: the depth-wide side faces flank the width-wide front and back, with
    the top and bottom sitting above them. Every entity model in the game uses
    this layout, so deriving it once is what keeps the transcribed shapes from
    needing per-face UV numbers.
    """
    faces = {
        NEG_X: (u, v + d, d, h),
        NEG_Z: (u + d, v + d, w, h),
        POS_X: (u + d + w, v + d, d, h),
        POS_Z: (u + d + w + d, v + d, w, h),
        UP: (u + d, v, w, d),
        DOWN: (u + d + w, v, w, d),
    }
    # A box's unwrap needs 2*(w+d) across and h+d down. If that overruns the
    # texture the faces sample past its edge and the block comes out as
    # garbage, so refuse to emit numbers that cannot be right.
    need_w, need_h = u + 2 * (w + d), v + h + d
    if need_w > tex_w or need_h > tex_h:
        raise ValueError(
            f"box {w}x{h}x{d} at uv ({u},{v}) needs {need_w}x{need_h} "
            f"but the texture is only {tex_w}x{tex_h}"
        )

    out = {}
    for face, (x, y, fw, fh) in faces.items():
        # Into the 0..16 space the model code divides by 16, regardless of how
        # large the entity texture actually is.
        out[face] = [
            x / tex_w * 16.0,
            y / tex_h * 16.0,
            (x + fw) / tex_w * 16.0,
            (y + fh) / tex_h * 16.0,
        ]
    return out


class Box:
    def __init__(self, pos, size, uv, texture, tex_size, faces=None):
        self.pos, self.size, self.uv = pos, size, uv
        self.texture, self.tex_size, self.only = texture, tex_size, faces


def emit(name, variants):
    """One block's entry: a list of (conditions, boxes)."""
    lines = [f'    ("{name}", BlockEntityModel {{ variants: &[']
    for when, boxes in variants:
        conds = ", ".join(
            f'BlockEntityCondition {{ key: "{k}", values: &[{", ".join(chr(34) + x + chr(34) for x in v)}] }}'
            for k, v in when
        )
        lines.append(f"        BlockEntityVariant {{ when: &[{conds}], elements: &[")
        for b in boxes:
            x, y, z = b.pos
            w, h, d = b.size
            uvs = box_uvs(b.uv[0], b.uv[1], w, h, d, b.tex_size[0], b.tex_size[1])
            wanted = b.only if b.only is not None else sorted(uvs)
            lines.append(
                f"            ModelElement {{ from: [{x:.2f}, {y:.2f}, {z:.2f}], "
                f"to: [{x + w:.2f}, {y + h:.2f}, {z + d:.2f}], rotation: None, faces: &["
            )
            for f in wanted:
                a, bb, c, dd = uvs[f]
                lines.append(
                    f"                ModelFace {{ face: {f}, "
                    f"uv: [{a:.2f}, {bb:.2f}, {c:.2f}, {dd:.2f}], "
                    f'texture: "{b.texture}" }},'
                )
            lines.append("            ] },")
        lines.append("        ] },")
    lines.append("    ] }),")
    return lines


def chest(texture):
    """Base, lid and latch of a closed single chest.

    Transcribed from ChestBlockEntityRenderer: a 14x10x14 base with a 14x5x14
    lid resting on it, and the latch standing proud of the front face -- which
    is why its box reaches past z=16.
    """
    T = (64, 64)
    return [
        Box((1, 0, 1), (14, 10, 14), (0, 19), texture, T),
        Box((1, 10, 1), (14, 5, 14), (0, 0), texture, T),
        Box((7, 8, 0), (2, 4, 1), (0, 0), texture, T),
    ]


BELL_T = (32, 32)
BANNER_T = (64, 64)

MODELS = [
    # The mounting bar comes from the block model; only the bell itself is
    # missing, and it hangs in the same place whatever the bar looks like.
    ("bell", [([], [
        Box((5, 6, 5), (6, 7, 6), (0, 0), "entity/bell/bell_body", BELL_T),
        Box((4, 4, 4), (8, 2, 8), (0, 13), "entity/bell/bell_body", BELL_T),
    ])]),
    ("chest", [([], chest("entity/chest/normal"))]),
    ("trapped_chest", [([], chest("entity/chest/trapped"))]),
    ("ender_chest", [([], chest("entity/chest/ender"))]),
    # Lid closed, sitting on the base, as a placed shulker box always is.
    ("shulker_box", [([], [
        Box((0, 0, 0), (16, 8, 16), (0, 28), "entity/shulker/shulker", (64, 64)),
        Box((0, 4, 0), (16, 12, 16), (0, 0), "entity/shulker/shulker", (64, 64)),
    ])]),
    ("conduit", [([], [
        Box((5, 5, 5), (6, 6, 6), (0, 0), "entity/conduit/base", (32, 16)),
    ])]),
    # decorated_pot is deliberately absent. Its body does not unwrap onto
    # decorated_pot_base the way every other box here does -- the renderer
    # paints each side from its own sherd texture instead -- so the shape
    # cannot be expressed as one textured box and a guess renders as garbage.
    # It keeps the cube fallback until the sherd faces are done properly.
    # A standing banner: post, crossbar, and the cloth hanging from it.
    #
    # The renderer draws this model at two thirds scale, so the sizes below are
    # the game's own numbers already multiplied out. That is also why a banner
    # stands 28 pixels tall and genuinely overflows its own block -- it does
    # in game too.
    ("banner", [([], [
        Box((7.33, 0, 7.33), (1.33, 28, 1.33), (44, 0), "entity/banner/base", BANNER_T),
        Box((1.33, 26.67, 7.33), (13.33, 1.33, 1.33), (0, 42), "entity/banner/base", BANNER_T),
        Box((1.33, 1.33, 7.67), (13.33, 26.67, 0.67), (0, 0), "entity/banner/base", BANNER_T),
    ])]),
]


def main():
    out = [
        "// Auto-generated by tools/gen_block_entity_models.py.",
        "//",
        "// Do not hand-edit; change the script and regenerate.",
        "//",
        "// Geometry for blocks the game draws from code rather than a model",
        "// JSON. Parsing the vanilla assets finds nothing for these, so without",
        "// this table a chest is a featureless cube and a bell is a bare post.",
        "",
        "use crate::block_models::{ModelElement, ModelFace};",
        "",
        "/// A block-state condition; empty `values` never matches.",
        "#[derive(Debug, Clone, Copy)]",
        "pub struct BlockEntityCondition {",
        "    pub key: &'static str,",
        "    pub values: &'static [&'static str],",
        "}",
        "",
        "/// One form of a code-drawn block.",
        "#[derive(Debug, Clone, Copy)]",
        "pub struct BlockEntityVariant {",
        "    /// All must match; empty means always.",
        "    pub when: &'static [BlockEntityCondition],",
        "    pub elements: &'static [ModelElement],",
        "}",
        "",
        "/// Every form of one code-drawn block.",
        "#[derive(Debug, Clone, Copy)]",
        "pub struct BlockEntityModel {",
        "    pub variants: &'static [BlockEntityVariant],",
        "}",
        "",
        "#[rustfmt::skip]",
        "static BLOCK_ENTITY_MODELS: &[(&str, BlockEntityModel)] = &[",
    ]
    for name, variants in sorted(MODELS):
        out += emit(name, variants)
    out += [
        "];",
        "",
        "/// The code-drawn geometry for a block, if it has any.",
        "///",
        "/// Colour-prefixed blocks share one shape: every shulker box is the same",
        "/// box and every banner the same cloth, differing only in texture, so the",
        "/// suffix is what identifies them.",
        "pub fn block_entity_model(short_name: &str) -> Option<&'static BlockEntityModel> {",
        "    let key = if short_name.ends_with(\"_shulker_box\") {",
        "        \"shulker_box\"",
        "    } else if short_name.ends_with(\"_banner\") {",
        "        \"banner\"",
        "    } else {",
        "        short_name",
        "    };",
        "    BLOCK_ENTITY_MODELS",
        "        .binary_search_by_key(&key, |(name, _)| name)",
        "        .ok()",
        "        .map(|i| &BLOCK_ENTITY_MODELS[i].1)",
        "}",
        "",
        "impl BlockEntityVariant {",
        "    /// True when every condition holds for `props`.",
        "    pub fn matches<'p>(&self, props: &dyn Fn(&str) -> Option<&'p str>) -> bool {",
        "        self.when",
        "            .iter()",
        "            .all(|cond| props(cond.key).is_some_and(|got| cond.values.contains(&got)))",
        "    }",
        "}",
        "",
    ]
    print("\n".join(out))


if __name__ == "__main__":
    main()
