# Shader targets

Captured from the user's own Blender setup on 2026-07-25. These are the
reference looks the importer should build, replacing the flat materials it
currently produces. Recorded here because the blend they were authored in was
unsaved at the time.

Both start from the same base the importer already makes — an atlas
`Diffuse Texture` node with `interpolation = 'Closest'` feeding a
`Mix (Add Color)` node whose result drives Base Color, and the texture's Alpha
wired to Principled Alpha — so the work is adding nodes to that, not replacing
it.

## Water

`surface_render_method = 'BLENDED'`

Principled BSDF:

| input | value |
| --- | --- |
| Roughness | 0.0909 |
| IOR | 1.333 |
| Transmission Weight | 1.0 |
| Specular IOR Level | 0.0 |
| Metallic | 0.0 |

Two shader mixes in series:

1. `Mix Shader` (Factor **0.6**): Principled BSDF + **Glass BSDF**
   - Glass Color `[0.2281, 0.3547, 0.5, 1.0]`, Roughness `0.3729`, IOR `1.5`
2. `Mix Shader.001` (Factor **0.15**): result of (1) + **Transparent BSDF**
   - Transparent Color white; this is what keeps deep water from going opaque

Surface normal comes from a procedural wave chain, not the texture:

```
Texture Coordinate.Generated
  -> Mapping        Rotation [0.5236, 0, 0]   Scale [38.15, 38.15, 38.15]
  -> Gabor Texture  Scale 4.9  Frequency 2  Anisotropy 1
                    Orientation [1.4142, 1.4142, 0]
  -> Color Ramp
  -> Bump           Distance 0.003  Strength 1
  -> Principled BSDF.Normal
```

Note the ordering constraint: MCprep's `prep_materials` rebuilds material node
graphs from scratch, so this has to be applied *after* it runs, like the
`Closest` interpolation sweep already is.

## Leaves (from `dark_oak_leaves`)

`surface_render_method = 'DITHERED'` — dithered rather than blended, so leaves
sort correctly against each other without the ordering artefacts blending
gives on dense canopy.

Principled BSDF: Roughness `1.0`, Specular IOR Level `0.0`, Transmission `0`.

`Mix Shader` between Principled and a **Transparent BSDF**, with the mix
**Factor driven by `Add Color.Result`** — the texture colour itself, not its
alpha. Alpha is wired separately to Principled Alpha.

Applies to every leaf block, not just dark oak.

## Related open items

- `leaf_litter` renders green but should be **brown**: it uses Minecraft's
  *dry foliage* colormap (1.21.5), not the foliage colormap. The current
  `PLAINS_FOLIAGE` tint in `texture.rs` is wrong for it, and the comment in
  `mineways.rs` calling foliage green "the closest single approximation" is
  the assumption to remove.
- Flat decorative blocks (`leaf_litter`, `moss_carpet`, `glow_lichen`, `vine`)
  should set `visible_shadow = False` on their prototype source objects: a
  zero-thickness plane casting a full shadow reads as a dark smear.
