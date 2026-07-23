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
use bedrock_parser::chunk::{strip_namespace, Chunk};
use bedrock_parser::jar_textures::JarTextureLoader;
use bedrock_parser::texture::FaceAwareTileSet;
use std::collections::HashMap;
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
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::Io(err) => write!(f, "I/O error: {err}"),
            ExportError::EmptyRegion => {
                write!(f, "the selected region contains no blocks to export")
            }
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
}

// ─────────────────────────────────────────────────────────────────────────────
// Public export entry-point
// ─────────────────────────────────────────────────────────────────────────────

/// Export every non-air block inside `region` (drawn from `chunks`) to
/// `obj_path`, writing a sibling `.mtl` file. Blocking — call from a worker
/// thread.
///
/// `tiles` provides the texture atlas and per-face UV lookups (typically
/// built from Mineways' `terrainExt.png` via `build_mineways_tileset`).
pub fn export_obj(
    chunks: &[Chunk],
    region: &ExportRegion,
    obj_path: &Path,
    tiles: &FaceAwareTileSet,
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
    // UVs are NOT deduplicated: every face-vertex writes its own `vt`.
    let mut texcoords: Vec<[f32; 2]> = Vec::new();
    // Geometry grouped by material name.
    let mut groups: Vec<MaterialGroup> = Vec::new();
    let mut group_ids: HashMap<String, usize> = HashMap::new();
    let mut block_positions: HashMap<String, Vec<[f64; 3]>> = HashMap::new();
    let mut blocks = 0usize;

    for chunk in chunks {
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
                    let material = sanitize_material(clean_name);
                    let block_center = [
                        (wx as f64 + 0.5) - center[0],
                        (y as f64 + 0.5) - center[1],
                        (wz as f64 + 0.5) - center[2],
                    ];
                    block_positions
                        .entry(clean_name.to_owned())
                        .or_default()
                        .push(block_center);

                    let group_id = group_ids.len();
                    let group_id = *group_ids.entry(material.clone()).or_insert(group_id);
                    if group_id == groups.len() {
                        groups.push(MaterialGroup {
                            name: material.clone(),
                            faces: Vec::new(),
                        });
                    }

                    // Colour-aware key: distinguishes e.g. red wool from
                    // blue wool, which share `name` but need different
                    // atlas swatches. Must match the key `tiles` was built
                    // with (see build_mineways_tileset / Chunk::texture_keys).
                    let tex_key = state.texture_key();
                    let quads = block_shape::block_quads(wx, y, wz, state.name.as_str(), &block_at);
                    for quad in &quads {
                        let [u0, v0, u1, v1] = if let Some(tex) = &quad.texture {
                            tiles.tile_uv(tex)
                        } else {
                            tiles.face_uv(&tex_key, quad.face_idx)
                        };
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

    // ── Flatten texcoords from face records ───────────────────────────────
    // We need to assign a unique `vt` index to every face-vertex.
    // Build a parallel vec so writing the OBJ is straightforward.
    for group in &mut groups {
        for face in &mut group.faces {
            for v in &mut face.verts {
                texcoords.push(v.uv);
            }
        }
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
    for uv in &texcoords {
        writeln!(obj, "vt {:.6} {:.6}", uv[0], 1.0 - uv[1])?;
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
    let mut mtl = BufWriter::new(std::fs::File::create(&mtl_path)?);
    writeln!(mtl, "# Project Bedrock materials (texture atlas)")?;
    for group in &groups {
        writeln!(mtl, "newmtl {}", group.name)?;
        writeln!(mtl, "Kd 1.0000 1.0000 1.0000")?;
        writeln!(mtl, "Ka 0.0000 0.0000 0.0000")?;
        writeln!(mtl, "Ks 0.0000 0.0000 0.0000")?;
        writeln!(mtl, "map_Kd {atlas_name}")?;
    }
    mtl.flush()?;

    // ── Write blocks.json Manifest & Extract Individual Textures ──────────
    write_block_manifest_and_textures(obj_path, center, &block_positions);

    Ok(ExportStats {
        obj_path: obj_path.to_path_buf(),
        mtl_path,
        blocks,
        faces: total_faces,
        materials: groups.len(),
    })
}

#[derive(serde::Serialize)]
struct BlockManifest {
    format_version: u32,
    center: [f64; 3],
    blocks: HashMap<String, BlockManifestEntry>,
}

#[derive(serde::Serialize)]
struct BlockManifestEntry {
    positions: Vec<[f64; 3]>,
    textures: HashMap<String, String>,
}

fn write_block_manifest_and_textures(
    obj_path: &Path,
    center: [f64; 3],
    block_positions: &HashMap<String, Vec<[f64; 3]>>,
) {
    let json_path = obj_path.with_file_name("blocks.json");
    let textures_dir = obj_path
        .parent()
        .map(|p| p.join("textures"))
        .unwrap_or_else(|| PathBuf::from("textures"));

    let _ = std::fs::create_dir_all(&textures_dir);

    let jar_loader = JarTextureLoader::load().ok();
    let mut manifest_blocks = HashMap::new();

    for (name, positions) in block_positions {
        let ft = bedrock_parser::block_model::face_textures(name);
        let mut textures_map = HashMap::new();

        if ft.top == ft.bottom
            && ft.top == ft.south
            && ft.top == ft.north
            && ft.top == ft.east
            && ft.top == ft.west
        {
            textures_map.insert("all".to_string(), format!("{}.png", ft.top));
        } else if ft.top == ft.bottom
            && ft.south == ft.north
            && ft.south == ft.east
            && ft.south == ft.west
        {
            textures_map.insert("top".to_string(), format!("{}.png", ft.top));
            textures_map.insert("bottom".to_string(), format!("{}.png", ft.bottom));
            textures_map.insert("sides".to_string(), format!("{}.png", ft.south));
        } else {
            textures_map.insert("top".to_string(), format!("{}.png", ft.top));
            textures_map.insert("bottom".to_string(), format!("{}.png", ft.bottom));
            textures_map.insert("east".to_string(), format!("{}.png", ft.east));
            textures_map.insert("west".to_string(), format!("{}.png", ft.west));
            textures_map.insert("north".to_string(), format!("{}.png", ft.north));
            textures_map.insert("south".to_string(), format!("{}.png", ft.south));
        }

        // Export individual texture files if jar_loader is available
        if let Some(loader) = &jar_loader {
            for tex_filename in textures_map.values() {
                let tex_name = tex_filename.strip_suffix(".png").unwrap_or(tex_filename);
                if let Some(bytes) = loader.get(tex_name) {
                    let out_file = textures_dir.join(tex_filename);
                    if !out_file.exists() {
                        let _ = std::fs::write(out_file, bytes);
                    }
                }
            }
        }

        manifest_blocks.insert(
            name.clone(),
            BlockManifestEntry {
                positions: positions.clone(),
                textures: textures_map,
            },
        );
    }

    let manifest = BlockManifest {
        format_version: 1,
        center,
        blocks: manifest_blocks,
    };

    if let Ok(file) = std::fs::File::create(&json_path) {
        let mut writer = BufWriter::new(file);
        let _ = serde_json::to_writer_pretty(&mut writer, &manifest);
    }
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

    let mut out = [[0.0f32; 2]; 4];
    for (i, c) in corners.iter().enumerate() {
        out[i] = [((c[ua] - u_min) / ur) as f32, ((c[va] - v_min) / vr) as f32];
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
