# Shader targets

Captured from `D:\Blender Assets\Blend files\MC-shaders-to-be-used.blend`,
authored by the user as the reference look the importer should build. Values
are exact, read out of the node graphs rather than eyeballed.

Both build on the base the importer already makes — an atlas `Diffuse Texture`
node with `interpolation = 'Closest'` feeding a `Mix (Add Color)` node whose
result drives Base Color, with the texture's Alpha wired to Principled Alpha —
so this is added to that, not a replacement.

**Ordering constraint:** MCprep's `prep_materials` rebuilds material node
graphs from scratch, so any of this applied before it runs is silently
discarded. It has to run in the same late pass as the `Closest` sweep.

## Leaves

Applies to every leaf block. `surface_render_method = 'DITHERED'` — dithered
rather than blended so dense canopy sorts correctly against itself.

Principled BSDF:

| input | value |
| --- | --- |
| Roughness | 0.7 |
| Specular IOR Level | 0.0 |
| Metallic | 0.0 |
| Transmission Weight | 0.0 |
| IOR | 1.5 |

Wiring is the base graph unchanged: `Diffuse Texture.Color -> Add Color.A`,
`Add Color.Result -> Principled.Base Color`, `Diffuse Texture.Alpha ->
Principled.Alpha`, `Principled.BSDF -> Material Output.Surface`.

`dark_oak_leaves` additionally mixes in a Transparent BSDF with the mix factor
driven by `Add Color.Result`, at Roughness 1.0. Every other leaf material in
the file uses the plain form above, so treat the plain form as canonical and
the dark oak variant as optional.

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
   - Transparent Color white. This is what stops deep water going opaque.

Surface normal comes from a procedural wave chain, not the texture:

```
Texture Coordinate.Generated
  -> Mapping        Rotation [0.5236, 0, 0]   Scale [38.15, 38.15, 38.15]
  -> Gabor Texture  Scale 4.9  Frequency 2  Anisotropy 1
                    Orientation [1.4142, 1.4142, 0]
  -> Color Ramp     stops: 0.3318 -> black, 0.6045 -> white
  -> Bump           Distance 0.003  Strength 1
  -> Principled BSDF.Normal
```

## Related open items

- `leaf_litter` renders green but should be **brown**: it uses Minecraft's
  *dry foliage* colormap (1.21.5), not the foliage colormap. The
  `PLAINS_FOLIAGE` tint in `texture.rs` is the wrong constant for it, and the
  comment in `mineways.rs` calling foliage green "the closest single
  approximation" is the assumption to remove.
- Fence and stair geometry does not match the game. Both are in the
  not-full-cube table and both have models, so the fault is downstream of
  block classification — start by exporting a single fence and comparing its
  prototype geometry against the vanilla model JSON.
- The Bedrock app viewport cannot draw fences or stairs at all, and has
  backface-culling / alpha problems distinct from the exporter's.
- `D:\Downloads\sampler-mineways-v501\source\Mineways2Skfb.obj` is a known-good
  Mineways export covering a wide block variety — a good fixture for checking
  which blocks should resolve to MCprep assets.
