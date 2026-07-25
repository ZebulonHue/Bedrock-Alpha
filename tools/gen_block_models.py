#!/usr/bin/env python3
"""Generate real per-block geometry from vanilla's block model JSONs.

The exporter has always emitted a cube for every block -- `block_quads` takes
the block name and ignores it. That is why `bush`, `leaf_litter`,
`amethyst_cluster`, `tall_seagrass` and friends come out as solid textured
boxes: the shape information exists in the game's own models and was simply
never read.

This flattens each non-cube block's model chain into a list of boxes with
per-face UVs and texture names, in the model's native 0..16 pixel space, so
the exporter can emit the true shape instead.

Usage:
    python tools/gen_block_models.py
"""

import io
import json
import math
import os
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(REPO, "extracted_mc_assets")
BLOCKSTATES = os.path.join(ASSETS, "blockstates")
MODELS = os.path.join(ASSETS, "models")
OUT = os.path.join(REPO, "crates", "bedrock-parser", "src", "block_models.rs")

# Face name -> index in block_shape::CUBE_FACES ([top, bottom, east, west, south, north]).
FACE_INDEX = {"up": 0, "down": 1, "east": 2, "west": 3, "south": 4, "north": 5}
AXIS_INDEX = {"x": 0, "y": 1, "z": 2}

_cache = {}


def load_model(ref):
    ref = ref.split(":", 1)[-1]
    if ref in _cache:
        return _cache[ref]
    path = os.path.join(MODELS, *ref.split("/")) + ".json"
    data = None
    if os.path.isfile(path):
        try:
            data = json.load(io.open(path, encoding="utf-8"))
        except Exception:
            data = None
    _cache[ref] = data
    return data


def flatten(ref, depth=0):
    """Resolve a model's parent chain into (elements, textures)."""
    if depth > 12:
        return None, {}
    model = load_model(ref)
    if model is None:
        return None, {}
    textures = dict(model.get("textures") or {})
    elements = model.get("elements")
    parent = model.get("parent")
    if parent:
        p_elements, p_textures = flatten(parent, depth + 1)
        merged = dict(p_textures)
        merged.update(textures)
        textures = merged
        if elements is None:
            elements = p_elements
    return elements, textures


def resolve_texture(ref, textures, depth=0):
    """Follow '#name' indirection to a concrete texture path.

    Some newer models give a texture as an object rather than a string, so
    only strings are followed.
    """
    if not isinstance(ref, str) or depth > 8:
        return None
    if ref.startswith("#"):
        return resolve_texture(textures.get(ref[1:]), textures, depth + 1)
    return ref.split(":", 1)[-1].split("/")[-1]


def parse_when(when):
    """Blockstate condition -> list of (key, [accepted values]) ANDed together.

    Returns None for conditions we cannot express (nested OR/AND), so the
    caller can skip that part rather than apply it unconditionally.
    """
    if not when:
        return []
    if "OR" in when or "AND" in when:
        return None
    out = []
    for key, value in when.items():
        out.append((key, str(value).split("|")))
    return out


def collect_variants(state):
    """All (conditions, model_ref, x_rot, y_rot) a blockstate can produce."""
    out = []
    variants = state.get("variants")
    if variants:
        for key, entry in variants.items():
            if isinstance(entry, list):
                entry = entry[0]          # weighted variants: take the first
            if not isinstance(entry, dict) or not entry.get("model"):
                continue
            conditions = []
            for pair in key.split(","):
                if "=" in pair:
                    k, v = pair.split("=", 1)
                    conditions.append((k, [v]))
            out.append(
                (conditions, entry["model"], float(entry.get("x", 0)), float(entry.get("y", 0)))
            )
        return out, False

    for part in state.get("multipart") or []:
        apply = part.get("apply")
        if isinstance(apply, list):
            apply = apply[0]
        if not isinstance(apply, dict) or not apply.get("model"):
            continue
        conditions = parse_when(part.get("when"))
        if conditions is None:
            continue
        out.append(
            (conditions, apply["model"], float(apply.get("x", 0)), float(apply.get("y", 0)))
        )
    return out, True


def is_plain_cube(elements):
    return (
        elements
        and len(elements) == 1
        and list(elements[0].get("from", [])) == [0, 0, 0]
        and list(elements[0].get("to", [])) == [16, 16, 16]
    )


def elements_of(ref):
    """Flatten a model ref into emit-ready element tuples."""
    elements, textures = flatten(ref)
    if not elements:
        return None, False
    plain = is_plain_cube(elements)
    out_elements = []
    for element in elements:
        frm, to = element.get("from"), element.get("to")
        if frm is None or to is None:
            continue
        rot = element.get("rotation")
        if rot:
            rot_tuple = (
                AXIS_INDEX.get(rot.get("axis", "y"), 1),
                float(rot.get("angle", 0.0)),
                [float(v) for v in rot.get("origin", [8, 8, 8])],
                bool(rot.get("rescale", False)),
            )
        else:
            rot_tuple = None
        faces = []
        for face_name, face in (element.get("faces") or {}).items():
            idx = FACE_INDEX.get(face_name)
            if idx is None:
                continue
            texture = resolve_texture(face.get("texture"), textures)
            if not texture:
                continue
            uv = face.get("uv") or [0, 0, 16, 16]
            faces.append((idx, [float(v) for v in uv], texture))
        if faces:
            out_elements.append(
                ([float(v) for v in frm], [float(v) for v in to], rot_tuple, faces)
            )
    return out_elements, plain


def main():
    if not os.path.isdir(BLOCKSTATES):
        print(f"missing {BLOCKSTATES}")
        return 1

    entries = []
    skipped = 0
    for filename in sorted(os.listdir(BLOCKSTATES)):
        if not filename.endswith(".json"):
            continue
        block = filename[:-5]
        try:
            state = json.load(io.open(os.path.join(BLOCKSTATES, filename), encoding="utf-8"))
        except Exception:
            continue

        raw_variants, multipart = collect_variants(state)
        if not raw_variants:
            continue

        built = []
        all_plain = True
        for conditions, ref, x_rot, y_rot in raw_variants:
            out_elements, plain = elements_of(ref)
            if not out_elements:
                continue
            # Rotation alone does not make a cube interesting: a rotated
            # full cube is still a full cube, and its texture orientation is
            # already handled by the per-face swatch lookup. Including them
            # here pulled `deepslate` (an axis-variant cube) into the model
            # path, where uncullable faces took the export from 1.5M to
            # 11.8M faces.
            if not plain:
                all_plain = False
            built.append((conditions, x_rot, y_rot, out_elements))

        # A block every one of whose variants is an unrotated full cube is
        # already drawn correctly by the exporter's default path.
        if not built or (all_plain and not multipart):
            continue
        entries.append((block, multipart, built))

    print(f"{len(entries)} blocks with per-state geometry")

    out = io.StringIO()
    out.write(
        "// Auto-generated by tools/gen_block_models.py from vanilla block models.\n"
        "//\n"
        "// Do not hand-edit. Regenerate after updating extracted_mc_assets/.\n"
        "//\n"
        "// Real per-block geometry in the models' native 0..16 pixel space, keyed\n"
        "// by block state. Blocks whose every variant is a plain unrotated cube are\n"
        "// omitted: the exporter's default cube path already draws those correctly.\n"
        "\n"
        "/// One textured face of a model box.\n"
        "#[derive(Debug, Clone, Copy)]\n"
        "pub struct ModelFace {\n"
        "    /// Index into [`crate::block_shape::CUBE_FACES`].\n"
        "    pub face: u8,\n"
        "    /// Face UV within the texture, 0..16 pixel space `[x1, y1, x2, y2]`.\n"
        "    pub uv: [f32; 4],\n"
        "    /// Texture name (no path, no extension).\n"
        "    pub texture: &'static str,\n"
        "}\n"
        "\n"
        "/// One box of a block model.\n"
        "#[derive(Debug, Clone, Copy)]\n"
        "pub struct ModelElement {\n"
        "    /// Corner in 0..16 pixel space.\n"
        "    pub from: [f32; 3],\n"
        "    /// Opposite corner in 0..16 pixel space.\n"
        "    pub to: [f32; 3],\n"
        "    /// Optional rotation: `(axis 0=x/1=y/2=z, degrees, origin, rescale)`.\n"
        "    pub rotation: Option<(u8, f32, [f32; 3], bool)>,\n"
        "    /// Faces this box actually draws.\n"
        "    pub faces: &'static [ModelFace],\n"
        "}\n"
        "\n"
        "/// A block-state condition: `key` must equal one of `values`.\n"
        "#[derive(Debug, Clone, Copy)]\n"
        "pub struct ModelCondition {\n"
        "    pub key: &'static str,\n"
        "    pub values: &'static [&'static str],\n"
        "}\n"
        "\n"
        "/// One selectable form of a block, plus the blockstate rotation that\n"
        "/// orients it (degrees, applied about the block centre).\n"
        "#[derive(Debug, Clone, Copy)]\n"
        "pub struct ModelVariant {\n"
        "    /// All must match for this variant to apply; empty means always.\n"
        "    pub when: &'static [ModelCondition],\n"
        "    pub x_rot: f32,\n"
        "    pub y_rot: f32,\n"
        "    pub elements: &'static [ModelElement],\n"
        "}\n"
        "\n"
        "/// All forms of one block.\n"
        "#[derive(Debug, Clone, Copy)]\n"
        "pub struct BlockModel {\n"
        "    /// Multipart blocks (fences, walls, panes) combine *every* matching\n"
        "    /// variant; ordinary blockstates select the first match only.\n"
        "    pub multipart: bool,\n"
        "    pub variants: &'static [ModelVariant],\n"
        "}\n"
        "\n"
        "#[rustfmt::skip]\n"
        "static BLOCK_MODELS: &[(&str, BlockModel)] = &[\n"
    )

    for block, multipart, built in entries:
        flag = "true" if multipart else "false"
        out.write(f'    ("{block}", BlockModel {{ multipart: {flag}, variants: &[\n')
        for conditions, x_rot, y_rot, elements in built:
            cond_src = ", ".join(
                'ModelCondition {{ key: "{}", values: &[{}] }}'.format(
                    key, ", ".join('"{}"'.format(v) for v in values)
                )
                for key, values in conditions
            )
            out.write(
                "        ModelVariant {{ when: &[{}], x_rot: {:.1f}, y_rot: {:.1f}, elements: &[\n".format(
                    cond_src, x_rot, y_rot
                )
            )
            for frm, to, rot, faces in elements:
                if rot is None:
                    rot_src = "None"
                else:
                    rot_src = "Some(({}, {:.1f}, [{:.1f}, {:.1f}, {:.1f}], {}))".format(
                        rot[0], rot[1], rot[2][0], rot[2][1], rot[2][2],
                        "true" if rot[3] else "false",
                    )
                out.write(
                    "            ModelElement {{ from: [{:.2f}, {:.2f}, {:.2f}], to: [{:.2f}, {:.2f}, {:.2f}], rotation: {}, faces: &[\n".format(
                        frm[0], frm[1], frm[2], to[0], to[1], to[2], rot_src
                    )
                )
                for idx, uv, texture in faces:
                    out.write(
                        '                ModelFace {{ face: {}, uv: [{:.2f}, {:.2f}, {:.2f}, {:.2f}], texture: "{}" }},\n'.format(
                            idx, uv[0], uv[1], uv[2], uv[3], texture
                        )
                    )
                out.write("            ] },\n")
            out.write("        ] },\n")
        out.write("    ] }),\n")

    out.write(
        "];\n"
        "\n"
        "/// Geometry for a block, or `None` when a plain cube is correct.\n"
        "pub fn model_for(short_name: &str) -> Option<&'static BlockModel> {\n"
        "    BLOCK_MODELS\n"
        "        .binary_search_by_key(&short_name, |(name, _)| name)\n"
        "        .ok()\n"
        "        .map(|i| &BLOCK_MODELS[i].1)\n"
        "}\n"
        "\n"
        "impl ModelVariant {\n"
        "    /// True when every condition holds for `props`.\n"
        "    pub fn matches<'p>(&self, props: &dyn Fn(&str) -> Option<&'p str>) -> bool {\n"
        "        self.when\n"
        "            .iter()\n"
        "            .all(|cond| props(cond.key).is_some_and(|got| cond.values.contains(&got)))\n"
        "    }\n"
        "}\n"
        "\n"
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    use super::*;\n"
        "\n"
        "    #[test]\n"
        "    fn table_is_sorted_for_binary_search() {\n"
        "        assert!(BLOCK_MODELS.windows(2).all(|w| w[0].0 < w[1].0));\n"
        "    }\n"
        "\n"
        "    #[test]\n"
        "    fn cross_plants_are_two_rotated_planes_not_a_cube() {\n"
        "        for block in [\"bush\", \"amethyst_cluster\"] {\n"
        "            let model = model_for(block).unwrap_or_else(|| panic!(\"{block} missing\"));\n"
        "            let elements = model.variants[0].elements;\n"
        "            assert_eq!(elements.len(), 2, \"{block} should be a cross\");\n"
        "            for element in elements {\n"
        "                assert!(element.rotation.is_some());\n"
        "                assert!((0..3).any(|i| (element.to[i] - element.from[i]).abs() < 0.01));\n"
        "            }\n"
        "        }\n"
        "    }\n"
        "\n"
        "    #[test]\n"
        "    fn plain_cubes_are_absent() {\n"
        "        for block in [\"stone\", \"dirt\", \"oak_planks\"] {\n"
        "            assert!(model_for(block).is_none(), \"{block} needs no entry\");\n"
        "        }\n"
        "    }\n"
        "\n"
        "    /// Stairs must resolve to a different orientation per `facing`,\n"
        "    /// otherwise every stair in a build points the same way.\n"
        "    #[test]\n"
        "    fn stairs_orient_by_facing() {\n"
        "        let model = model_for(\"oak_stairs\").expect(\"oak_stairs\");\n"
        "        assert!(!model.multipart);\n"
        "        let pick = |facing: &str| {\n"
        "            let props = |k: &str| match k {\n"
        "                \"facing\" => Some(facing),\n"
        "                \"half\" => Some(\"bottom\"),\n"
        "                \"shape\" => Some(\"straight\"),\n"
        "                _ => None,\n"
        "            };\n"
        "            model\n"
        "                .variants\n"
        "                .iter()\n"
        "                .find(|v| v.matches(&props))\n"
        "                .map(|v| v.y_rot)\n"
        "        };\n"
        "        let (east, west) = (pick(\"east\"), pick(\"west\"));\n"
        "        assert!(east.is_some() && west.is_some(), \"both facings must match\");\n"
        "        assert_ne!(east, west, \"east and west stairs must not share geometry\");\n"
        "    }\n"
        "\n"
        "    /// A fence is multipart: the post always, plus one side per connection.\n"
        "    #[test]\n"
        "    fn fence_combines_post_and_connected_sides() {\n"
        "        let model = model_for(\"oak_fence\").expect(\"oak_fence\");\n"
        "        assert!(model.multipart, \"fences are multipart\");\n"
        "        let props = |k: &str| match k {\n"
        "            \"north\" => Some(\"true\"),\n"
        "            _ => Some(\"false\"),\n"
        "        };\n"
        "        let matched = model.variants.iter().filter(|v| v.matches(&props)).count();\n"
        "        assert_eq!(matched, 2, \"post + north side only\");\n"
        "    }\n"
        "}\n"
    )
    io.open(OUT, "w", encoding="utf-8").write(out.getvalue())
    print(f"wrote {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
