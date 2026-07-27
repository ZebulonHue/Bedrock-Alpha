# The End → Blender: Method & Session Log

A consolidated record of importing a Minecraft world's End dimension into Blender, including terrain, structures, items, entities, and effects. Sources: the player's own world save (`DIM1`), their `1.20.1.jar`, and Mojang's official `bedrock-samples` GitHub repository.

## Architecture

Two environments cooperate throughout. A **Linux sandbox** handles heavy data processing: parsing region files, computing visibility, running headless Blender to author `.blend` deliverables. The **live MCP connection** to the user's Blender 5.2 session handles everything interactive: patches, model construction, and placement. A key discovery mid-project: the user's local machine has `%APPDATA%\.minecraft\versions\1.20.1\1.20.1.jar`, and Blender's Python can read it directly with `zipfile` — so textures and even block model JSONs are extracted on the user's machine with no data transfer.

Coordinate convention everywhere: Minecraft is Y-up, Blender is Z-up. The conversion `(x, y, z)_mc → (x, −z, y)_blender` is a proper rotation (no mirroring), at 1 block = 1 metre. Block centers sit at integer + 0.5.

## Part 1 — World extraction (sandbox)

The Anvil region format (`.mca`) was parsed from scratch with `nbtlib` + `numpy`: a 4 KB header of chunk offsets, then zlib-compressed NBT per chunk. Since 1.18, block data lives in `sections[].block_states` as a palette plus bit-packed indices — bit width is `max(4, ceil(log2(palette_size)))`, values packed into 64-bit longs *without* crossing long boundaries (1.16+ rule). Decoding is vectorized with numpy shifts.

The island was loaded as a 352×352×256 int16 grid covering x,z ∈ [−176, 176). Out of ~1.1 M solid blocks, only **exterior** blocks are kept (any block with at least one non-occluding neighbour), reducing to ~83 k. Important refinement: the occluder set must contain only full opaque cubes (end stone, obsidian, bedrock). Treating torches/fire as occluders wrongly culled the bedrock block at (0, 67, 0) between the exit portal's four torches.

The end ship at (359, 159, 1207) was isolated by **6-connected flood fill** seeded from the mast — it floats free of the city, so the fill returns exactly the ship: 903 blocks, 13×24×23, including ladder, end rods, chest, and brewing stand. Block properties (e.g. `facing` on wall torches) required a second parse pass since the first pass discarded Properties.

## Part 2 — The Blender scene

Delivered as `the_end.blend`, authored headless in the sandbox (Blender 4.0, forward-compatible with 5.2), 230 KB with packed textures. The structure honours the "instance everything" request:

Each block type is a **vertex-only point-cloud mesh** (positions = block centers). A shared geometry node group, `BlockInstancer` (Instance on Points ← Object Info with *As Instance*), instances one **prototype cube** per block type (`BLK_end_stone`, `BLK_obsidian`, …) living in a hidden `Block_Prototypes` collection. Editing a prototype updates all instances. Materials use the real 16×16 textures from the player's jar with **Closest interpolation** (critical for the pixelated look).

The ship lives in `End_Ship_Source` (unlinked from the scene, kept via fake user) and is placed by three collection-instance empties (`EndShip_1..3`) in the background — move, rotate, or duplicate the empties for more ships.

Bugs found and fixed here: (1) Blender's default cube unwraps as a cross layout, so each face showed a stretched fraction of the texture — fixed by rewriting prototype UVs with a **box projection** (every face maps the full tile, upright). (2) Hiding `End_Ship_Source` with `hide_viewport/hide_render` also hid its collection instances — fixed by *unlinking* it from the scene instead. (3) `ray_cast` against geometry-node instances reports the **prototype** object (`BLK_end_stone`), not the point-cloud object — relevant for ground snapping.

## Part 3 — Blocks & items rebuilt as proper models

**Exit portal dressing.** The four wall torches were rebuilt as real torch models: a 2×2×10-pixel cuboid UV-mapped to the torch column of `torch.png` (sides u ∈ [7/16, 9/16]), tilted 25° and offset toward their wall using the `facing` values parsed from the save (west/north/east/south). Fire became the classic **crossed planes** with `fire_0.png` — an animated strip of 32 stacked frames; the first frame is UV band v ∈ [31/32, 1]. Emission + texture alpha.

**Dragon egg.** Built from the *vanilla block model* `assets/minecraft/models/block/dragon_egg.json` read from the local jar: 8 stacked cuboids with per-face UVs. First attempt failed (flat dark colour) because the UV function received already-converted Blender coordinates and divided by 16 again, sampling one corner texel; the rebuild uses the model's own face UVs verbatim (v flipped for Blender).

**Items (netherite sword, bow, arrow).** Pixel-extrusion: every opaque pixel of the 16×16 item texture becomes a 1/16 m cube, all six faces UV'd to that pixel's own texel (with a 5 % inset against bleeding). The sword's pixels were computed in the sandbox with PIL; bow (70 px) and arrow (38 px) were computed **entirely live** by reading `image.pixels` in Blender (RGBA floats, bottom-left origin — which conveniently matches Blender UV space, no flip needed). Placement uses `scene.ray_cast` from above to sit items on the terrain.

## Part 4 — Entities (the part with no files in the world)

Entity *models* don't exist as assets in Java Edition — they're compiled code. The workaround: **Mojang's official `bedrock-samples` repo** on GitHub publishes the geometry (`resource_pack/models/entity/*.geo.json`), while the Java jar supplies the matching textures (`textures/entity/…`).

**Ender dragon** (`ender_dragon_rigged.blend`, 36 bones, 520 verts). The geo file is in legacy v1.8-derived style: bones mostly parentless (the game animates them in code), with two coordinate conventions that had to be reverse-engineered:

1. Chained child bones (wingtips, leg tips, feet) store pivots as `stored = java_offset` with **Java's y flipped around 24**: true relative pivot = `stored − (0, 24, 0)`, absolute pivot = parent's absolute + relative.
2. Their cube coordinates are **relative to the stored pivot**: local = `origin − stored_pivot`, absolute = local + absolute pivot.

Root bones (head, neck, body, wings, upper legs) use absolute coordinates directly. Box UV follows the standard Java layout: from origin (u, v) with size (sx, sy, sz): sides at v+sz — order east(u), north(u+sz), west(u+sz+sx), south(u+sz+sx+sz); top/bottom above at v. The neck and tail are generated procedurally by the game, so the build replicates them: 5 neck segments forward and 12 tail segments backward, all reusing the single "neck" cube pair (segment + spike), chained at 10-unit spacing, head repositioned to the neck's end. Mirrored right-side parts negate x (`x' = −(x + size_x)`) and flip the UV mirror flag. The mesh is rigidly skinned (weight 1.0 per segment) to an armature mirroring the hierarchy — correct for hard-surface parts and ideal for posing. Life-size: ~16 m long.

**Enderman** (built live, 6 bones). Simple humanoid from `enderman.geo.json` (64×32 texture): head, body, two 30-unit arms, two 30-unit legs — all absolute coordinates except the head cube, which carries a legacy offset placing it at y 24–32 inside the body; corrected to 38–46 (the "hat"/jaw overlay at 37.5 confirms the true position). The glowing eyes are a **second cube** over the head, inflated by 0.15, using `enderman_eyes.png` with texture alpha and emission strength 6, as its own material slot. Rig: `body` root → head, both arms, both legs; rigid vertex groups; armature built via `edit_bones` in a scripted edit-mode session.

## Part 5 — Particles

Portal/ambient particles use the game's real sprites: `assets/minecraft/textures/particle/generic_0..7.png` (the game tints them purple in code — the tint is not an asset). An initial auto-scatter (two point clouds + Instance-on-Points groups with random rotation/scale) was **removed at the user's request**. What remains is a single object, **`End_Particle`**: a 0.25 m plane at (2.5, −2.5, 70) with material `portal_particle` — image node labelled "Sprite" (`generic_7`), Mix node labelled "Particle Tint" (change the purple there), emission strength on the Principled BSDF (1.8), alpha from the sprite. Scatter it however you like; other `generic_*` frames can be swapped into the image node.

## Current scene inventory (the user's live file, `the_end.blend`)

Collections: `End_Island` (terrain point clouds, wall torches, egg, sword, bow, arrow), `End_Ships_Background` (3 collection-instance empties), `End_Particles`-related content removed, `Block_Prototypes` (hidden prototypes incl. fire crossed-planes), `Setup` (camera, lights, target). Node groups: `BlockInstancer`. Notable user-added content observed (untouched): a "Thomas Rig Legacy" player character at the portal, and two large default cubes ("Cube", "Cube.001") spanning far beyond the visible area. Deliverable files produced: `the_end.blend`, `ender_dragon_rigged.blend`.

## Known quirks & notes

Distant blocks resolve to their average colour — a 16 px texture at 300 m is sub-pixel; zoom in for detail. Stairs, slabs, iron bars, chests, and brewing stands render as full cubes (iron bars use a wireframe cage). Purpur pillars use the side texture on all faces. The scripted changes share the undo stack with manual edits — a Ctrl+Z can silently remove script-created objects (suspected cause of the enderman's disappearance). Save (Ctrl+S) after script patches; live changes exist only in the session until saved. The scene renders in Cycles; particle sprites and torch/fire emission are tuned for its light transport.

## Outstanding

Rebuild the enderman (one call, ground-corrected at z 67.25). Append `ender_dragon_rigged.blend` into the scene once downloaded, scale-check, and pose for the planned still image.
