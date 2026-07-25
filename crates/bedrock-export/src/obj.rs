//! Wavefront OBJ export: one quad per exposed block face, one material per
//! block type, and a matching `.mtl` file mapped onto a texture atlas.
//!
//! Uses Mineways-style separate position/UV storage:
//! - Positions are deduplicated by world coordinate `(x, y, z)` only — adjacent
//!   blocks share vertices at their boundaries, producing a watertight mesh.
//! - UVs are stored per face-vertex (one `vt` per corner, never deduplicated),
//!   so each block face has independent texture coordinates. This fixes the
//!   per-face-UV-correctness problem that fusing vertex+UV cannot solve.
//! - OBJ output uses `f v/vt v/vt v/vt v/vt` with separate `v` and `vt`
//!   indices (like Mineways' OBJ export).
//!
//! Coordinates are written Y-up with 1 unit = 1 block, centered on the
//! exported region so the scene lands on Blender's origin with no manual
//! cleanup.
//!
//! The atlas is provided externally (typically Mineways' `terrainExt.png`).
//! A sibling `.mtl` file references the atlas.

use bedrock_parser::block_shape;
use bedrock_parser::blocks::is_air;
use bedrock_parser::block_shapes::is_full_cube;
use bedrock_parser::chunk::{strip_namespace, Chunk};
use bedrock_parser::texture::FaceAwareTileSet;
use std::collections::{BTreeMap, HashMap};

/// Block-state properties, ordered so the map can act as a lookup key.
type BlockProps = BTreeMap<String, String>;

/// Fluids that fill whole volumes and so must stay in the terrain mesh.
///
/// A fluid is not a full cube — its height varies with `level` — so the
/// not-a-cube test that selects blocks for per-block instancing sweeps it up.
/// That is badly wrong for the one case where volume matters: an ocean is
/// hundreds of thousands of blocks, nearly all of them buried inside the body
/// of water where no face is ever visible. Instancing gives every one of them
/// a full six-sided cube, so a modest world exported over a million
/// translucent faces that a renderer then has to depth-sort. Left in the mesh,
/// `block_shape::is_self_occluding` drops every shared face and only the
/// surface survives — which is also what the game draws.
fn is_bulk_fluid(short_name: &str) -> bool {
    matches!(
        short_name,
        "water" | "flowing_water" | "lava" | "flowing_lava" | "bubble_column"
    )
}
use std::fmt;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Inclusive block-coordinate bounds of the region to export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportRegion {
    /// Minimum corner (inclusive), as `[x, y, z]`.
    pub min: [i32; 3],
    /// Maximum corner (exclusive), as `[x, y, z]`.
    pub max: [i32; 3],
}

impl ExportRegion {
    /// True when world block coordinate `(x, y, z)` lies inside the region.
    pub fn contains(&self, x: i32, y: i32, z: i32) -> bool {
        x >= self.min[0]
            && x < self.max[0]
            && y >= self.min[1]
            && y < self.max[1]
            && z >= self.min[2]
            && z < self.max[2]
    }

    /// True when the 16×16 column of chunk `(chunk_x, chunk_z)` overlaps this
    /// region in X/Z at all.
    ///
    /// Lets an export skip a whole chunk up front instead of rejecting each of
    /// its ~98k blocks individually inside the innermost loop — without this,
    /// scan cost is driven by how many chunks were *loaded* rather than how
    /// many were actually *selected*.
    pub fn overlaps_chunk(&self, chunk_x: i32, chunk_z: i32) -> bool {
        let (x0, z0) = (chunk_x * 16, chunk_z * 16);
        x0 < self.max[0] && x0 + 16 > self.min[0] && z0 < self.max[2] && z0 + 16 > self.min[2]
    }
}

/// What was written by an export.
pub struct ExportStats {
    /// Path of the `.obj` file.
    pub obj_path: PathBuf,
    /// Path of the `.mtl` file.
    pub mtl_path: PathBuf,
    /// Exported (non-air) blocks.
    pub blocks: usize,
    /// Exported quads.
    pub faces: usize,
    /// Distinct block materials.
    pub materials: usize,
}

/// Why an export failed.
#[derive(Debug)]
pub enum ExportError {
    /// A file could not be written.
    Io(std::io::Error),
    /// The region contains no exportable blocks.
    EmptyRegion,
    /// The selection is too large to export in one piece.
    RegionTooLarge {
        /// Chunks the selection covers.
        chunks: usize,
        /// Block positions that would have to be scanned.
        slots: u64,
    },
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::Io(err) => write!(f, "I/O error: {err}"),
            ExportError::EmptyRegion => {
                write!(f, "the selected region contains no blocks to export")
            }
            ExportError::RegionTooLarge { chunks, slots } => write!(
                f,
                "selection is too large: {chunks} chunks, {slots} block positions.                  The exporter builds the whole model in memory, so this would use                  many gigabytes and take minutes with no output. Select a smaller                  area, or narrow the Y range — most of a tall selection is solid                  stone that contributes nothing visible."
            ),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<std::io::Error> for ExportError {
    fn from(err: std::io::Error) -> Self {
        ExportError::Io(err)
    }
}

/// Quantized position key for vertex dedup (1/16th-block grid).
fn quantize(v: f64) -> i64 {
    (v * 16.0).round() as i64
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal vertex/face records
// ─────────────────────────────────────────────────────────────────────────────

/// A single face vertex: index into the global positions array + UV value.
#[derive(Debug, Clone, Copy)]
struct FaceVert {
    pos_idx: usize,
    uv: [f32; 2],
}

/// One quad face: 4 vertices, all sharing the same material.
#[derive(Debug, Clone)]
struct FaceRecord {
    verts: [FaceVert; 4],
}

/// Accumulated geometry grouped by material name.
struct MaterialGroup {
    name: String,
    faces: Vec<FaceRecord>,
    /// Distinct atlas rects this material samples, quantised to whole pixels.
    ///
    /// Kept so the MTL can declare transparency per material by inspecting the
    /// tiles the block actually uses. Bounded in practice — a block draws from
    /// a handful of tiles — and capped below regardless.
    uv_rects: Vec<[i32; 4]>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public export entry-point
// ─────────────────────────────────────────────────────────────────────────────

/// Optional, non-geometry side-products of an export.
///
/// These are off by default because their cost scales with the *block* count
/// rather than the *visible face* count, which makes them far more expensive
/// than the OBJ itself on a real world-sized selection.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Write a `blocks.json` manifest of where each block sits and in what
    /// block state, so an importer can place a real 3D asset per instance
    /// instead of relying on the textured cubes in the OBJ.
    ///
    /// Only non-full-cube blocks are recorded — plants, torches, rails,
    /// glass, leaves, fluids. Those are the ones whose true shape a cube
    /// misrepresents, and confining the manifest to them is what keeps it
    /// small: recording *every* block instead made this file 5.65 GB on a
    /// ~7,900-chunk world, because buried stone dominates the block count
    /// while carrying no information a cube mesh doesn't already convey.
    pub write_block_manifest: bool,

    /// Also write one prototype mesh per manifest block type, textured from
    /// the player's client JAR, into a sibling `prototypes/` folder.
    ///
    /// Lets an importer place a real mesh per block instead of drawing a
    /// textured cube from the shared atlas — which is what removes atlas
    /// seams and wrong-swatch errors rather than patching around them.
    pub write_prototypes: bool,
}

/// Export every non-air block inside `region` (drawn from `chunks`) to
/// `obj_path`, writing a sibling `.mtl` file. Blocking — call from a worker
/// thread.
///
/// `tiles` provides the texture atlas and per-face UV lookups (typically
/// built from Mineways' `terrainExt.png` via `build_mineways_tileset`).
///
/// Uses [`ExportOptions::default()`] — see [`export_obj_with_options`] to
/// additionally emit the `blocks.json` manifest.
pub fn export_obj(
    chunks: &[Chunk],
    region: &ExportRegion,
    obj_path: &Path,
    tiles: &FaceAwareTileSet,
) -> Result<ExportStats, ExportError> {
    export_obj_with_options(chunks, region, obj_path, tiles, &ExportOptions::default())
}

/// [`export_obj`], plus control over the optional side-products in
/// [`ExportOptions`].
pub fn export_obj_with_options(
    chunks: &[Chunk],
    region: &ExportRegion,
    obj_path: &Path,
    tiles: &FaceAwareTileSet,
    options: &ExportOptions,
) -> Result<ExportStats, ExportError> {
    let by_coord: HashMap<(i32, i32), &Chunk> = chunks.iter().map(|c| ((c.x, c.z), c)).collect();
    let block_at = |wx: i32, y: i32, wz: i32| -> Option<&str> {
        let chunk = by_coord.get(&(wx.div_euclid(16), wz.div_euclid(16)))?;
        chunk.block_at(wx.rem_euclid(16) as usize, y, wz.rem_euclid(16) as usize)
    };

    // The OBJ is centered on the region so Blender imports it at the origin.
    let center = [
        (region.min[0] + region.max[0]) as f64 / 2.0,
        region.min[1] as f64,
        (region.min[2] + region.max[2]) as f64 / 2.0,
    ];

    // ── Collect geometry ──────────────────────────────────────────────────
    // Positions are deduplicated on a 1/16th-block grid (full + fractional
    // positions for slabs, stairs, etc.).
    let mut positions: Vec<[f64; 3]> = Vec::new();
    let mut pos_map: HashMap<[i64; 3], usize> = HashMap::new();
    // UVs are NOT deduplicated: every face-vertex writes its own `vt`. They
    // live only in the face records — writing them out walks the same
    // groups/faces/verts order the `f` lines use, so a second flattened copy
    // of every UV (32 bytes per face) would be pure duplication.
    // Geometry grouped by material name.
    let mut groups: Vec<MaterialGroup> = Vec::new();
    let mut group_ids: HashMap<String, usize> = HashMap::new();
    // Borrowed-key fast path into `group_ids`, avoiding a per-block allocation
    // (keys borrow from `chunks`, which outlives this loop).
    let mut group_of_name: HashMap<&str, usize> = HashMap::new();
    // Placement data for the manifest, keyed by (block id, block state) so a
    // north-facing torch and a south-facing one stay distinguishable — an
    // importer cannot orient a swapped asset without the state. `BTreeMap`
    // rather than `HashMap` because the state is part of the key here, and
    // it needs a stable ordering (and to be hashable) to serve as one.
    let mut block_positions: HashMap<(String, BlockProps), Vec<[f64; 3]>> = HashMap::new();
    let mut blocks = 0usize;

    // Everything below accumulates in memory before a single byte is written,
    // so an over-large selection climbs to many gigabytes and looks like a
    // hang — there is no output file and no progress until the very end.
    // Check the size up front and refuse rather than grind.
    let selected: Vec<&Chunk> = chunks
        .iter()
        .filter(|c| region.overlaps_chunk(c.x, c.z))
        .collect();
    let span_y = (region.max[1] - region.min[1]).max(0) as u64;
    let slots = selected.len() as u64 * 256 * span_y;
    tracing::info!(
        "Export: {} chunk(s) in region, up to {} block position(s) to scan",
        selected.len(),
        slots
    );
    // Roughly where a 32 GB machine starts swapping; the practical limit is
    // far lower, but this catches the runaway case rather than the marginal one.
    const MAX_BLOCK_SLOTS: u64 = 400_000_000;
    if slots > MAX_BLOCK_SLOTS {
        return Err(ExportError::RegionTooLarge {
            chunks: selected.len(),
            slots,
        });
    }

    let total_chunks = selected.len();
    let mut done_chunks = 0usize;
    let mut next_report = 10usize;
    for chunk in selected {
        done_chunks += 1;
        let percent = done_chunks * 100 / total_chunks.max(1);
        if percent >= next_report {
            tracing::info!("Export: {percent}% ({done_chunks}/{total_chunks} chunks)");
            next_report = percent + 10;
        }
        let Some((chunk_min_y, chunk_max_y)) = chunk.y_range() else {
            continue;
        };
        let min_y = chunk_min_y.max(region.min[1]);
        let max_y = chunk_max_y.min(region.max[1]);
        for y in min_y..max_y {
            for z in 0..16usize {
                for x in 0..16usize {
                    let wx = chunk.x * 16 + x as i32;
                    let wz = chunk.z * 16 + z as i32;
                    if !region.contains(wx, y, wz) {
                        continue;
                    }
                    let Some(state) = chunk.block_state_at(x, y, z) else {
                        continue;
                    };
                    let name = state.name.as_str();
                    if is_air(name) {
                        continue;
                    }
                    blocks += 1;
                    let clean_name = strip_namespace(name);
                    // Record placement data for the blocks an importer will
                    // want to swap for a real 3D asset: every block whose true
                    // shape is not a plain cube. That set is derived from
                    // vanilla's own block models (see
                    // `tools/gen_block_shapes.py`) rather than hand-listed —
                    // hand lists kept silently omitting things like
                    // `red_mushroom`, `bush`, `leaf_litter` and every stair
                    // and slab, which then never reached the importer at all.
                    // Skipping the solid terrain is what keeps this from
                    // becoming a second copy of the world.
                    if options.write_block_manifest
                        && !is_full_cube(clean_name)
                        && !is_bulk_fluid(clean_name)
                    {
                        let block_center = [
                            (wx as f64 + 0.5) - center[0],
                            (y as f64 + 0.5) - center[1],
                            (wz as f64 + 0.5) - center[2],
                        ];
                        block_positions
                            .entry((
                                clean_name.to_owned(),
                                state.properties.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                            ))
                            .or_default()
                            .push(block_center);
                    }

                    // Resolve block id -> material group. Keyed by the
                    // borrowed block name so the common case (a type already
                    // seen) costs one hash and no allocation; sanitising the
                    // name — which allocates — happens once per distinct block
                    // type rather than once per block. Grouping still keys off
                    // the *sanitised* name, so two ids that sanitise alike
                    // share one group exactly as before.
                    let group_id = match group_of_name.get(clean_name) {
                        Some(&id) => id,
                        None => {
                            let material = sanitize_material(clean_name);
                            let id = if let Some(&existing) = group_ids.get(&material) {
                                existing
                            } else {
                                let new_id = groups.len();
                                groups.push(MaterialGroup {
                                    name: material.clone(),
                                    faces: Vec::new(),
                                    uv_rects: Vec::new(),
                                });
                                group_ids.insert(material, new_id);
                                new_id
                            };
                            group_of_name.insert(clean_name, id);
                            id
                        }
                    };

                    // Colour-aware key: distinguishes e.g. red wool from
                    // blue wool, which share `name` but need different
                    // atlas swatches. Must match the key `tiles` was built
                    // with (see build_mineways_tileset / Chunk::texture_keys).
                    let tex_key = state.texture_key();
                    let axis = state.axis();
                    let quads = block_shape::block_quads_stated(
                        wx,
                        y,
                        wz,
                        state.name.as_str(),
                        &state.properties,
                        &block_at,
                    );
                    for quad in &quads {
                        let [u0, v0, u1, v1] = if let Some(tex) = &quad.texture {
                            tiles.tile_uv(tex)
                        } else {
                            let logical_face =
                                bedrock_parser::mineways::remap_pillar_face(quad.face_idx, axis);
                            tiles.face_uv(&tex_key, logical_face)
                        };
                        // Remember which atlas tiles this material draws from,
                        // so the MTL can declare transparency per material.
                        {
                            let aw = tiles.atlas.width as f32;
                            let ah = tiles.atlas.height as f32;
                            let rect = [
                                (u0 * aw).round() as i32,
                                (v0 * ah).round() as i32,
                                (u1 * aw).round() as i32,
                                (v1 * ah).round() as i32,
                            ];
                            let rects = &mut groups[group_id].uv_rects;
                            if rects.len() < 12 && !rects.contains(&rect) {
                                rects.push(rect);
                            }
                        }
                        let face_uvs = normalized_face_uvs(&quad.corners, quad.normal);

                        let mut verts = [FaceVert {
                            pos_idx: 0,
                            uv: [0.0, 0.0],
                        }; 4];
                        for (slot, (corner, face_uv)) in verts
                            .iter_mut()
                            .zip(quad.corners.iter().zip(face_uvs.iter()))
                        {
                            let qkey = [
                                quantize(corner[0]),
                                quantize(corner[1]),
                                quantize(corner[2]),
                            ];
                            let pos_len = positions.len();
                            slot.pos_idx = *pos_map.entry(qkey).or_insert_with(|| {
                                positions.push(*corner);
                                pos_len
                            });
                            slot.uv = [u0 + face_uv[0] * (u1 - u0), v0 + face_uv[1] * (v1 - v0)];
                        }
                        groups[group_id].faces.push(FaceRecord { verts });
                    }
                }
            }
        }
    }

    if blocks == 0 {
        return Err(ExportError::EmptyRegion);
    }

    // A material group is created as soon as a block of that type is seen,
    // but a block that is fully enclosed by its neighbours contributes no
    // visible quads — so a type that only ever occurs buried ends up with an
    // empty group. Emitting `o`/`g`/`usemtl` for it writes an object with no
    // faces, and Blender's OBJ importer then attaches the *next* group's
    // geometry to this name: the object labelled `sulfur` came in holding
    // `lily_of_the_valley`'s faces. Drop them before writing.
    groups.retain(|group| !group.faces.is_empty());
    if groups.is_empty() {
        return Err(ExportError::EmptyRegion);
    }

    // ── Write OBJ ─────────────────────────────────────────────────────────
    let mtl_path = obj_path.with_extension("mtl");
    let mtl_name = mtl_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export.mtl".to_owned());
    let atlas_path = obj_path.with_extension("atlas.png");
    let atlas_name = atlas_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export.atlas.png".to_owned());

    let mut obj = BufWriter::new(std::fs::File::create(obj_path)?);
    writeln!(obj, "# Project Bedrock OBJ export")?;
    writeln!(obj, "mtllib {mtl_name}")?;

    // Write all positions.
    for p in &positions {
        writeln!(
            obj,
            "v {:.3} {:.3} {:.3}",
            p[0] - center[0],
            p[1] - center[1],
            p[2] - center[2]
        )?;
    }

    // Write all texcoords (OBJ V runs bottom-up; atlas is top-down, so flip).
    // Walked in the same groups → faces → verts order as the `f` lines below,
    // so the Nth face-vertex written here is `vt` index N+1 there.
    for group in &groups {
        for face in &group.faces {
            for v in &face.verts {
                writeln!(obj, "vt {:.6} {:.6}", v.uv[0], 1.0 - v.uv[1])?;
            }
        }
    }

    // Write faces grouped by material.
    // Each face consumes 4 vt entries; we track the global vt cursor.
    let mut vt_cursor = 0usize;
    let mut total_faces = 0usize;
    for group in &groups {
        writeln!(obj, "o {}", group.name)?;
        writeln!(obj, "g {}", group.name)?;
        writeln!(obj, "usemtl {}", group.name)?;
        for face in &group.faces {
            let v0 = face.verts[0].pos_idx + 1;
            let v1 = face.verts[1].pos_idx + 1;
            let v2 = face.verts[2].pos_idx + 1;
            let v3 = face.verts[3].pos_idx + 1;
            let t0 = vt_cursor + 1;
            let t1 = vt_cursor + 2;
            let t2 = vt_cursor + 3;
            let t3 = vt_cursor + 4;
            writeln!(obj, "f {v0}/{t0} {v1}/{t1} {v2}/{t2} {v3}/{t3}")?;
            vt_cursor += 4;
            total_faces += 1;
        }
    }
    obj.flush()?;

    // ── Write atlas PNG ───────────────────────────────────────────────────
    image::save_buffer(
        &atlas_path,
        &tiles.atlas.pixels,
        tiles.atlas.width,
        tiles.atlas.height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|err| ExportError::Io(std::io::Error::other(err)))?;

    // ── Write MTL ─────────────────────────────────────────────────────────
    // MTL that states its own transparency, the way Mineways' does. An
    // importer reading `map_d` sets up cutout alpha natively, so nothing has to
    // guess afterwards — and, just as importantly, a material *without* it is
    // left fully opaque. Declaring alpha only where the tiles actually have it
    // is what stops solid blocks (grass, dirt) being punched through.
    let mut mtl = BufWriter::new(std::fs::File::create(&mtl_path)?);
    writeln!(mtl, "# Project Bedrock materials (texture atlas)")?;
    let mut cutout_materials = 0usize;
    for group in &groups {
        let cutout = group
            .uv_rects
            .iter()
            .any(|rect| atlas_rect_has_alpha(&tiles.atlas, *rect));
        writeln!(mtl, "
newmtl {}", group.name)?;
        writeln!(mtl, "Ka 0.0000 0.0000 0.0000")?;
        writeln!(mtl, "Kd 1.0000 1.0000 1.0000")?;
        writeln!(mtl, "Ks 0.0000 0.0000 0.0000")?;
        writeln!(mtl, "Ns 0")?;
        // 2 = plain lighting, 4 = ray-traced transparency.
        writeln!(mtl, "illum {}", if cutout { 4 } else { 2 })?;
        writeln!(mtl, "map_Kd {atlas_name}")?;
        if cutout {
            // The atlas carries its own alpha channel, so it doubles as the
            // dissolve map; no separate greyscale file is needed.
            writeln!(mtl, "map_d {atlas_name}")?;
            cutout_materials += 1;
        }
    }
    mtl.flush()?;
    tracing::info!(
        "MTL: {} of {} material(s) declared as alpha cutout",
        cutout_materials,
        groups.len()
    );

    // ── Write blocks.json Manifest & Extract Individual Textures ──────────
    if options.write_block_manifest {
        write_block_manifest(obj_path, center, &block_positions);

        if options.write_prototypes {
            // One representative state per block: the prototype is a single
            // mesh reused at every position, so it takes the commonest form.
            let mut representative: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
            for ((name, props), positions) in &block_positions {
                let entry = representative.entry(name.clone());
                match entry {
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(props.clone());
                    }
                    std::collections::btree_map::Entry::Occupied(mut slot) => {
                        // Prefer the state with the most occurrences.
                        let current = block_positions
                            .get(&(name.clone(), slot.get().clone()))
                            .map_or(0, Vec::len);
                        if positions.len() > current {
                            slot.insert(props.clone());
                        }
                    }
                }
            }
            let stats = crate::prototypes::write_block_prototypes(obj_path, &representative);
            tracing::info!(
                "Prototypes: {} block mesh(es), {} texture(s) from the client JAR{}",
                stats.written,
                stats.textures,
                if stats.skipped.is_empty() {
                    String::new()
                } else {
                    format!(" ({} had no geometry)", stats.skipped.len())
                }
            );
        }
    }

    Ok(ExportStats {
        obj_path: obj_path.to_path_buf(),
        mtl_path,
        blocks,
        faces: total_faces,
        materials: groups.len(),
    })
}

/// Placement data for blocks an importer may swap for a real 3D asset.
///
/// Positions are in the same space as the OBJ's vertices — Minecraft axes
/// (Y-up), already offset by `center` — so an importer applies whatever
/// axis conversion it used for the OBJ and the two line up exactly. For
/// Blender's `up_axis='Y', forward_axis='NEGATIVE_Z'` that is
/// `(x, y, z) -> (x, -z, y)`.
#[derive(serde::Serialize)]
struct BlockManifest<'a> {
    format_version: u32,
    /// Region centre that was subtracted from every position.
    center: [f64; 3],
    /// Axis mapping an importer must apply, spelled out so a consumer does
    /// not have to rediscover it.
    up_axis: &'static str,
    blocks: HashMap<String, Vec<BlockVariant<'a>>>,
}

/// One block id in one block state, plus every position it occurs at.
#[derive(serde::Serialize)]
struct BlockVariant<'a> {
    /// Block state properties (`facing`, `half`, `axis`, ...). Empty for
    /// blocks with no state. Needed to orient a swapped asset.
    properties: &'a BlockProps,
    /// Borrowed rather than copied — these lists get long.
    positions: &'a [[f64; 3]],
}

fn write_block_manifest(
    obj_path: &Path,
    center: [f64; 3],
    block_positions: &HashMap<(String, BlockProps), Vec<[f64; 3]>>,
) {
    // Named after the OBJ, not a bare "blocks.json": a shared name lets an
    // importer pick up the manifest from a *previous* export of a different
    // region, placing every asset at coordinates belonging to another part of
    // the world.
    let json_path = obj_path.with_extension("blocks.json");

    let mut manifest_blocks: HashMap<String, Vec<BlockVariant>> = HashMap::new();
    for ((name, properties), positions) in block_positions {
        manifest_blocks
            .entry(name.clone())
            .or_default()
            .push(BlockVariant {
                properties,
                positions: positions.as_slice(),
            });
    }

    let total: usize = block_positions.values().map(Vec::len).sum();
    tracing::info!(
        "Block manifest: {} block type(s), {} placement(s) -> {}",
        manifest_blocks.len(),
        total,
        json_path.display()
    );

    let manifest = BlockManifest {
        format_version: 2,
        center,
        up_axis: "Y",
        blocks: manifest_blocks,
    };

    if let Ok(file) = std::fs::File::create(&json_path) {
        let mut writer = BufWriter::new(file);
        // Compact, not pretty: the position list dominates this file, and
        // pretty-printing puts every coordinate on its own indented line.
        let _ = serde_json::to_writer(&mut writer, &manifest);
        let _ = writer.flush();
    }
}

/// True when an atlas rect contains meaningfully transparent pixels.
///
/// Used to decide whether a material needs `map_d` in the MTL. A tile is
/// judged on its own: a block that samples several tiles (grass block: top,
/// side, dirt, side-overlay) must not be called cutout just because one of
/// them has alpha, or its solid faces render see-through.
fn atlas_rect_has_alpha(atlas: &bedrock_parser::texture::TileSet, rect: [i32; 4]) -> bool {
    let (width, height) = (atlas.width as i32, atlas.height as i32);
    let (x0, y0) = (rect[0].clamp(0, width), rect[1].clamp(0, height));
    let (x1, y1) = (rect[2].clamp(0, width), rect[3].clamp(0, height));
    if x1 <= x0 || y1 <= y0 {
        return false;
    }
    let (mut transparent, mut total) = (0usize, 0usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let idx = ((y * width + x) * 4 + 3) as usize;
            match atlas.pixels.get(idx) {
                Some(&alpha) => {
                    total += 1;
                    if alpha < 128 {
                        transparent += 1;
                    }
                }
                None => return false,
            }
        }
    }
    // A few stray edge pixels are not a cutout; a real cutout texture is
    // substantially empty.
    total > 0 && transparent * 10 > total
}

/// Compute per-corner UV coordinates (0..1) within a face, normalised to the
/// face's bounding box so the full texture tile maps onto any rectangular face
/// regardless of its world-space extent.
fn normalized_face_uvs(corners: &[[f64; 3]; 4], normal: [i32; 3]) -> [[f32; 2]; 4] {
    // Determine which two axes form the face plane.
    let (ua, va) = if normal[1] != 0 {
        (0usize, 2usize) // top/bottom: x & z
    } else if normal[0] != 0 {
        (2, 1) // ±x: z & y
    } else {
        (0, 1) // ±z: x & y
    };

    let u_min = corners.iter().map(|c| c[ua]).reduce(f64::min).unwrap();
    let u_max = corners.iter().map(|c| c[ua]).reduce(f64::max).unwrap();
    let v_min = corners.iter().map(|c| c[va]).reduce(f64::min).unwrap();
    let v_max = corners.iter().map(|c| c[va]).reduce(f64::max).unwrap();
    let ur = if u_max > u_min { u_max - u_min } else { 1.0 };
    let vr = if v_max > v_min { v_max - v_min } else { 1.0 };

    // Atlas rects run top-down (`v0` is the tile's *upper* edge), so a face
    // coordinate of 0 lands on the top of the texture. For a side face the
    // V axis is world Y, where 0 is the *bottom* of the block — leaving that
    // unflipped renders every side texture upside down, which is why a grass
    // block showed its green strip along the bottom edge instead of the top.
    // Top/bottom faces map V to world Z, where 0 is north and correctly
    // corresponds to the top of the texture, so they must not be flipped.
    let flip_v = normal[1] == 0;

    let mut out = [[0.0f32; 2]; 4];
    for (i, c) in corners.iter().enumerate() {
        let v = ((c[va] - v_min) / vr) as f32;
        out[i] = [
            ((c[ua] - u_min) / ur) as f32,
            if flip_v { 1.0 - v } else { v },
        ];
    }
    out
}

/// Make a block id safe for an OBJ material name.
fn sanitize_material(short: &str) -> String {
    short
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bedrock_parser::jar_textures::JarTextureLoader;
    use bedrock_parser::texture::FaceAwareTileSet;
    use fastnbt::LongArray;
    use serde::Serialize;

    #[derive(Serialize)]
    struct TestChunk {
        sections: Vec<TestSection>,
        #[serde(rename = "xPos")]
        x: i32,
        #[serde(rename = "zPos")]
        z: i32,
    }

    #[derive(Serialize)]
    struct TestSection {
        #[serde(rename = "Y")]
        y: i8,
        block_states: TestBlockStates,
    }

    #[derive(Serialize)]
    struct TestBlockStates {
        palette: Vec<TestPaletteEntry>,
        data: LongArray,
    }

    #[derive(Serialize)]
    struct TestPaletteEntry {
        #[serde(rename = "Name")]
        name: String,
    }

    /// A 2×1×1 stone bar at y=0, x=0..2, z=0 in chunk (0, 0).
    fn test_chunk() -> Chunk {
        let mut indices = vec![0u16; 4096];
        indices[0] = 1; // (0, 0, 0)
        indices[1] = 1; // (1, 0, 0)
        let nbt = fastnbt::to_bytes(&TestChunk {
            sections: vec![TestSection {
                y: 0,
                block_states: TestBlockStates {
                    palette: vec![
                        TestPaletteEntry {
                            name: "minecraft:air".into(),
                        },
                        TestPaletteEntry {
                            name: "minecraft:stone".into(),
                        },
                    ],
                    data: LongArray::new(indices_to_longs(&indices, 4)),
                },
            }],
            x: 0,
            z: 0,
        })
        .unwrap();
        Chunk::from_nbt(&nbt).unwrap()
    }

    fn indices_to_longs(indices: &[u16], bits: usize) -> Vec<i64> {
        let per_long = 64 / bits;
        let mut out = vec![0i64; indices.len().div_ceil(per_long)];
        for (i, &index) in indices.iter().enumerate() {
            out[i / per_long] |= (index as i64) << ((i % per_long) * bits);
        }
        out
    }

    /// Build a procedural FaceAwareTileSet for testing (no JAR needed).
    fn test_tileset(names: &[String]) -> FaceAwareTileSet {
        FaceAwareTileSet::build(names.iter().cloned(), &JarTextureLoader::empty())
    }

    /// A 3×3×3 stone cube with a single dirt block buried dead centre. The
    /// dirt is enclosed on all six sides, so it contributes no visible quads.
    fn test_chunk_with_buried_block() -> Chunk {
        let mut indices = vec![0u16; 4096];
        for y in 0..3usize {
            for z in 0..3usize {
                for x in 0..3usize {
                    indices[(y << 8) | (z << 4) | x] = 1; // stone
                }
            }
        }
        indices[(1 << 8) | (1 << 4) | 1] = 2; // dirt, fully buried
        let nbt = fastnbt::to_bytes(&TestChunk {
            sections: vec![TestSection {
                y: 0,
                block_states: TestBlockStates {
                    palette: vec![
                        TestPaletteEntry {
                            name: "minecraft:air".into(),
                        },
                        TestPaletteEntry {
                            name: "minecraft:stone".into(),
                        },
                        TestPaletteEntry {
                            name: "minecraft:dirt".into(),
                        },
                    ],
                    data: LongArray::new(indices_to_longs(&indices, 4)),
                },
            }],
            x: 0,
            z: 0,
        })
        .unwrap();
        Chunk::from_nbt(&nbt).unwrap()
    }

    /// A block type that only ever occurs fully buried still gets a material
    /// group, but that group has no faces. Writing it emits an `o`/`g`/`usemtl`
    /// header with nothing under it, and Blender's OBJ importer then hands the
    /// *next* group's geometry to this name — so an object called `sulfur`
    /// arrives holding a completely different block's mesh.
    /// The MTL must state its own transparency, per material.
    ///
    /// This is what makes a Mineways OBJ "just work" on import: an importer
    /// reading `map_d` wires cutout alpha itself, per pixel, and a material
    /// without it stays opaque. Guessing afterwards in Blender could only
    /// judge a whole material at once, which turned solid ground transparent
    /// because one of the tiles it samples has alpha.
    #[test]
    fn mtl_declares_cutout_only_where_the_texture_has_alpha() {
        let chunk = test_chunk();
        let region = ExportRegion {
            min: [0, 0, 0],
            max: [16, 16, 16],
        };
        let dir = std::env::temp_dir().join("bedrock-export-test");
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("mtl.obj");

        let names: Vec<String> = chunk.block_names().into_iter().map(str::to_owned).collect();
        let tiles = bedrock_parser::mineways::build_mineways_tileset(&names);
        export_obj(&[chunk], &region, &obj_path, &tiles).unwrap();

        let mtl = std::fs::read_to_string(obj_path.with_extension("mtl")).unwrap();
        // Stone is solid: no dissolve map, plain lighting model.
        let stone = mtl
            .split("newmtl ")
            .find(|block| block.starts_with("stone
"))
            .expect("stone material");
        assert!(
            !stone.contains("map_d"),
            "solid stone must not be declared as cutout:
{stone}"
        );
        assert!(stone.contains("illum 2"), "solid stone should use illum 2");
        // And every material that *is* cutout must say so both ways.
        for block in mtl.split("newmtl ").skip(1) {
            if block.contains("map_d") {
                assert!(
                    block.contains("illum 4"),
                    "a cutout material needs illum 4 as well:
{block}"
                );
            }
        }
    }

    #[test]
    fn empty_material_groups_are_not_written() {
        let chunk = test_chunk_with_buried_block();
        let region = ExportRegion {
            min: [0, 0, 0],
            max: [16, 16, 16],
        };
        let dir = std::env::temp_dir().join("bedrock-export-test");
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("buried.obj");

        let block_names: Vec<String> = chunk.block_names().into_iter().map(str::to_owned).collect();
        let tiles = test_tileset(&block_names);
        let stats = export_obj(&[chunk], &region, &obj_path, &tiles).unwrap();

        // The dirt block is counted (it exists) but contributes no geometry.
        assert_eq!(stats.blocks, 27);
        assert_eq!(stats.materials, 1, "only stone should reach the file");

        let obj = std::fs::read_to_string(&obj_path).unwrap();
        assert!(obj.contains("usemtl stone"));
        assert!(
            !obj.contains("usemtl dirt"),
            "buried-only `dirt` was written as an empty group:\n{obj}"
        );
        // Every `usemtl` must be followed by at least one face.
        for chunk_after in obj.split("usemtl ").skip(1) {
            let body = chunk_after.split_once('\n').map(|(_, r)| r).unwrap_or("");
            assert!(
                body.trim_start().starts_with("f "),
                "a material group was written with no faces"
            );
        }
    }

    #[test]
    fn exports_a_bar_with_correct_face_count() {
        let chunk = test_chunk();
        let region = ExportRegion {
            min: [0, 0, 0],
            max: [16, 16, 16],
        };
        let dir = std::env::temp_dir().join("bedrock-export-test");
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("bar.obj");

        let block_names: Vec<String> = chunk.block_names().into_iter().map(str::to_owned).collect();
        let tiles = test_tileset(&block_names);
        let stats = export_obj(&[chunk], &region, &obj_path, &tiles).unwrap();
        // Two adjacent stone cubes: 12 faces − 2 shared = 10.
        assert_eq!(stats.blocks, 2);
        assert_eq!(stats.faces, 10);
        assert_eq!(stats.materials, 1);

        let obj = std::fs::read_to_string(&obj_path).unwrap();
        assert!(obj.contains("mtllib bar.mtl"));
        assert!(obj.contains("usemtl stone"));
        assert_eq!(obj.matches("\nf ").count(), 10);
        // Positions should be deduped: two cubes share the shared face's
        // 4 vertices, so we have 12 unique positions (16 − 4 shared).
        let v_count = obj.matches("\nv ").count();
        assert_eq!(v_count, 12);
        // UV entries = 4 per face = 40.
        assert_eq!(obj.matches("\nvt ").count(), 40);

        let mtl = std::fs::read_to_string(&stats.mtl_path).unwrap();
        assert!(mtl.contains("newmtl stone"));
        assert!(mtl.contains("map_Kd bar.atlas.png"));
        assert!(stats.obj_path.with_extension("atlas.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_region_is_an_error() {
        let chunk = test_chunk();
        let region = ExportRegion {
            min: [100, 0, 100],
            max: [116, 16, 116],
        };
        let dir = std::env::temp_dir().join("bedrock-export-test-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let block_names: Vec<String> = chunk.block_names().into_iter().map(str::to_owned).collect();
        let tiles = test_tileset(&block_names);
        let result = export_obj(&[chunk], &region, &dir.join("empty.obj"), &tiles);
        assert!(matches!(result, Err(ExportError::EmptyRegion)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A side face's texture must not be vertically mirrored.
    ///
    /// Atlas rects are top-down while world Y runs bottom-up, so mapping one
    /// straight onto the other flips every side texture — a grass block ends
    /// up with its green strip along the bottom edge. Top/bottom faces map V
    /// to world Z and must stay unflipped.
    #[test]
    fn side_faces_are_not_vertically_flipped() {
        // A +X face spanning a unit block: corners ordered as CUBE_FACES has
        // them, i.e. starting at the bottom edge.
        let side = [
            [1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
        ];
        let uvs = normalized_face_uvs(&side, [1, 0, 0]);
        // Corner 0 sits at the block's bottom (y = 0). Atlas V grows downward,
        // so the bottom of the block must land at the BOTTOM of the tile (V=1).
        assert!(
            uvs[0][1] > 0.5,
            "bottom-of-block corner mapped to top of texture: {uvs:?}"
        );
        assert!(
            uvs[2][1] < 0.5,
            "top-of-block corner mapped to bottom of texture: {uvs:?}"
        );

        // The top face maps V to world Z (north -> top of texture); flipping
        // it would rotate every top texture 180 degrees.
        let top = [
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let top_uvs = normalized_face_uvs(&top, [0, 1, 0]);
        assert!(
            top_uvs[3][1] < 0.5,
            "north corner of a top face must map to the top of the texture: {top_uvs:?}"
        );
    }

    #[test]
    fn material_names_are_sanitized() {
        assert_eq!(sanitize_material("grass_block"), "grass_block");
        assert_eq!(sanitize_material("weird-block!"), "weird_block_");
    }

    #[test]
    fn full_export_with_mineways_atlas_leaves_files() {
        use bedrock_parser::mineways::build_mineways_tileset;
        let chunk = test_chunk();
        let names: Vec<String> = chunk.texture_keys();
        eprintln!("Palette: {names:?}");
        let tiles = build_mineways_tileset(&names);
        eprintln!("Atlas: {}x{}", tiles.atlas.width, tiles.atlas.height);

        let region = ExportRegion {
            min: [0, 0, 0],
            max: [2, 1, 1],
        };
        let dir = std::env::temp_dir().join("mineways-export-full-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("test.obj");
        let stats = export_obj(&[chunk], &region, &obj_path, &tiles).unwrap();
        eprintln!(
            "Export: {} blocks, {} faces, {} materials",
            stats.blocks, stats.faces, stats.materials
        );

        // Read and print OBJ
        let obj = std::fs::read_to_string(&obj_path).unwrap();
        eprintln!("\n=== OBJ ({}) ===", obj_path.display());
        for line in obj.lines() {
            eprintln!("{line}");
        }

        // Read MTL
        let mtl_path = obj_path.with_extension("mtl");
        let mtl = std::fs::read_to_string(&mtl_path).unwrap();
        eprintln!("\n=== MTL ===");
        for line in mtl.lines() {
            eprintln!("{line}");
        }

        // Verify atlas PNG exists
        let atlas_path = obj_path.with_extension("atlas.png");
        assert!(atlas_path.exists(), "Atlas file missing!");
        let atlas_meta = std::fs::metadata(&atlas_path).unwrap();
        eprintln!("\nAtlas PNG: {} bytes", atlas_meta.len());

        // Clean up
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Build a world with varied block shapes to verify per-block geometry.
    /// Layout (y=0, z=0): x=0 stone, x=1 oak_slab, x=2 oak_stairs, x=3 glass_pane
    fn mixed_chunk() -> Chunk {
        let mut indices = vec![0u16; 4096];
        indices[0] = 1; // (0,0,0) stone
        indices[1] = 2; // (1,0,0) oak_slab
        indices[2] = 3; // (2,0,0) oak_stairs
        indices[3] = 4; // (3,0,0) glass_pane
        let nbt = fastnbt::to_bytes(&TestChunk {
            sections: vec![TestSection {
                y: 0,
                block_states: TestBlockStates {
                    palette: vec![
                        TestPaletteEntry {
                            name: "minecraft:air".into(),
                        },
                        TestPaletteEntry {
                            name: "minecraft:stone".into(),
                        },
                        TestPaletteEntry {
                            name: "minecraft:oak_slab".into(),
                        },
                        TestPaletteEntry {
                            name: "minecraft:oak_stairs".into(),
                        },
                        TestPaletteEntry {
                            name: "minecraft:glass_pane".into(),
                        },
                    ],
                    data: LongArray::new(indices_to_longs(&indices, 4)),
                },
            }],
            x: 0,
            z: 0,
        })
        .unwrap();
        Chunk::from_nbt(&nbt).unwrap()
    }

    #[test]
    fn diagnostic_mixed_blocks_geometry() {
        use bedrock_parser::mineways::build_mineways_tileset;
        let chunk = mixed_chunk();
        let names: Vec<String> = chunk.texture_keys();
        let tiles = build_mineways_tileset(&names);

        let region = ExportRegion {
            min: [0, 0, 0],
            max: [4, 1, 1],
        };
        let dir = std::env::temp_dir().join("mineways-mixed-geom");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("mixed.obj");

        let stats = export_obj(&[chunk], &region, &obj_path, &tiles).unwrap();
        eprintln!(
            "STATS: {} blocks, {} faces, {} materials",
            stats.blocks, stats.faces, stats.materials
        );

        let obj = std::fs::read_to_string(&obj_path).unwrap();

        // Collect positions
        let mut v_count = 0usize;
        let mut min_pos = [f64::MAX; 3];
        let mut max_pos = [f64::MIN; 3];
        let mut bad_pos = false;
        for line in obj.lines() {
            if line.starts_with("v ") {
                let coords: Vec<f64> = line
                    .split_whitespace()
                    .skip(1)
                    .map(|s| s.parse::<f64>().unwrap())
                    .collect();
                v_count += 1;
                for i in 0..3 {
                    if coords[i].is_nan() || coords[i].is_infinite() {
                        bad_pos = true;
                    }
                    min_pos[i] = min_pos[i].min(coords[i]);
                    max_pos[i] = max_pos[i].max(coords[i]);
                }
            }
        }
        eprintln!("Vertices: {v_count}");
        eprintln!("Pos range X: [{}, {}]", min_pos[0], max_pos[0]);
        eprintln!("Pos range Y: [{}, {}]", min_pos[1], max_pos[1]);
        eprintln!("Pos range Z: [{}, {}]", min_pos[2], max_pos[2]);
        eprintln!("Bad positions: {bad_pos}");

        // Collect UVs
        let mut vt_count = 0usize;
        let mut bad_uv = false;
        let mut min_uv = [f64::MAX; 2];
        let mut max_uv = [f64::MIN; 2];
        for line in obj.lines() {
            if line.starts_with("vt ") {
                let uv: Vec<f64> = line
                    .split_whitespace()
                    .skip(1)
                    .map(|s| s.parse::<f64>().unwrap())
                    .collect();
                vt_count += 1;
                if uv[0] < -0.01 || uv[0] > 1.01 || uv[1] < -0.01 || uv[1] > 1.01 {
                    bad_uv = true;
                }
                min_uv[0] = min_uv[0].min(uv[0]);
                min_uv[1] = min_uv[1].min(uv[1]);
                max_uv[0] = max_uv[0].max(uv[0]);
                max_uv[1] = max_uv[1].max(uv[1]);
            }
        }
        eprintln!("UVs: {vt_count}");
        eprintln!("UV range U: [{}, {}]", min_uv[0], max_uv[0]);
        eprintln!("UV range V: [{}, {}]", min_uv[1], max_uv[1]);
        eprintln!("Bad UVs: {bad_uv}");

        // Material list
        eprintln!("Materials:");
        for line in obj.lines() {
            if line.starts_with("usemtl ") {
                eprintln!("  {}", line);
            }
        }

        // Assertions
        assert!(!bad_pos, "Found NaN/infinite positions!");
        assert!(!bad_uv, "Found UVs outside [0,1] range!");
        assert!(v_count > 0, "No vertices emitted!");
        assert_eq!(stats.blocks, 4, "Should have 4 blocks");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
