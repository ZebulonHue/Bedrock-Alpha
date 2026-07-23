//! glTF 2.0 export: one mesh with multiple primitives (one per block
//! material), a matching `.gltf` JSON file, a `.bin` file with vertex/index
//! data, and the texture atlas as a sibling PNG.
//!
//! Coordinates are written Y-up with 1 unit = 1 block, centred on the
//! exported region so the scene lands on Blender's origin.
//!
//! The atlas is provided externally (typically Mineways' `terrainExt.png`).
//! The glTF uses the KHR_materials_pbrMetallicRoughness model with the
//! atlas as the base colour texture.

use bedrock_blender::material::pbr_preset;
use bedrock_parser::block_shape;
use bedrock_parser::blocks::is_air;
use bedrock_parser::chunk::{strip_namespace, Chunk};
use bedrock_parser::texture::FaceAwareTileSet;
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::obj::{ExportError, ExportRegion, ExportStats};

/// Quantization grid for vertex deduplication (1/16th-block).
fn quantize(v: f64) -> i64 {
    (v * 16.0).round() as i64
}

/// A single vertex used for deduplication: position + normal + UV.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VertKey {
    pos: [i64; 3],
    normal: [i32; 3],
    uv: [u32; 2], // quantised UV 0..65536
}

/// Collected geometry for one material group.
struct MeshPrimitive {
    material_name: String,
    vertices: Vec<VertKey>,
    indices: Vec<u32>,
}

/// Compute per-corner UV coordinates (0..1) within a face.
fn normalized_face_uvs(corners: &[[f64; 3]; 4], normal: [i32; 3]) -> [[f32; 2]; 4] {
    let (ua, va) = if normal[1] != 0 {
        (0usize, 2usize)
    } else if normal[0] != 0 {
        (2, 1)
    } else {
        (0, 1)
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

/// Sanitise a block name for use as a glTF material name.
fn sanitise(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Export the given region to glTF format.
///
/// Produces three sibling files:
/// - `*.gltf` — JSON manifest
/// - `*.bin` — vertex/index binary data
/// - `*.atlas.png` — texture atlas (via `tiles`)
pub fn export_gltf(
    chunks: &[Chunk],
    region: &ExportRegion,
    gltf_path: &Path,
    tiles: &FaceAwareTileSet,
) -> Result<ExportStats, ExportError> {
    let by_coord: HashMap<(i32, i32), &Chunk> = chunks.iter().map(|c| ((c.x, c.z), c)).collect();
    let block_at = |wx: i32, y: i32, wz: i32| -> Option<&str> {
        let chunk = by_coord.get(&(wx.div_euclid(16), wz.div_euclid(16)))?;
        chunk.block_at(wx.rem_euclid(16) as usize, y, wz.rem_euclid(16) as usize)
    };

    // The glTF is centred on the region so Blender imports at the origin.
    let center = [
        (region.min[0] + region.max[0]) as f64 / 2.0,
        region.min[1] as f64,
        (region.min[2] + region.max[2]) as f64 / 2.0,
    ];

    // ── Collect geometry per material ────────────────────────────────────
    let mut primitives: Vec<MeshPrimitive> = Vec::new();
    let mut mat_to_prim: HashMap<String, usize> = HashMap::new();
    let mut blocks = 0usize;
    let mut total_faces = 0usize;

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
                    let material = sanitise(strip_namespace(name));
                    let prim_idx = mat_to_prim.len();
                    let prim_idx = *mat_to_prim.entry(material.clone()).or_insert(prim_idx);
                    if prim_idx == primitives.len() {
                        primitives.push(MeshPrimitive {
                            material_name: material.clone(),
                            vertices: Vec::new(),
                            indices: Vec::new(),
                        });
                    }
                    let prim = &mut primitives[prim_idx];

                    let tex_key = state.texture_key();
                    let quads = block_shape::block_quads(wx, y, wz, state.name.as_str(), &block_at);

                    for quad in &quads {
                        let [u0, v0, u1, v1] = if let Some(tex) = &quad.texture {
                            tiles.tile_uv(tex)
                        } else {
                            tiles.face_uv(&tex_key, quad.face_idx)
                        };
                        let corner_uvs = normalized_face_uvs(&quad.corners, quad.normal);

                        let mut vert_indices = [0u32; 4];
                        for (slot, (corner, &[fu, fv])) in
                            quad.corners.iter().zip(corner_uvs.iter()).enumerate()
                        {
                            let key = VertKey {
                                pos: [
                                    quantize(corner[0]),
                                    quantize(corner[1]),
                                    quantize(corner[2]),
                                ],
                                normal: quad.normal,
                                uv: [
                                    ((u0 + fu * (u1 - u0)) * 65536.0) as u32,
                                    ((v0 + fv * (v1 - v0)) * 65536.0) as u32,
                                ],
                            };
                            // Deduplicate within this primitive.
                            let idx = match prim.vertices.iter().position(|vk| *vk == key) {
                                Some(i) => i as u32,
                                None => {
                                    let i = prim.vertices.len() as u32;
                                    prim.vertices.push(key);
                                    i
                                }
                            };
                            vert_indices[slot] = idx;
                        }
                        // Two triangles per quad.
                        prim.indices.extend_from_slice(&[
                            vert_indices[0],
                            vert_indices[1],
                            vert_indices[2],
                        ]);
                        prim.indices.extend_from_slice(&[
                            vert_indices[0],
                            vert_indices[2],
                            vert_indices[3],
                        ]);
                        total_faces += 1;
                    }
                }
            }
        }
    }

    if blocks == 0 {
        return Err(ExportError::EmptyRegion);
    }

    // ── Build binary buffer ──────────────────────────────────────────────
    // Layout: For each primitive in order:
    //   positions (F32 × 3 × vertex_count)
    //   normals   (F32 × 3 × vertex_count)
    //   UVs       (F32 × 2 × vertex_count)
    //   indices   (U32 × 1 × index_count)
    // All primitives' data is concatenated in the same .bin file.

    let bin_path = gltf_path.with_extension("bin");
    let atlas_path = gltf_path.with_extension("atlas.png");
    let bin_name = bin_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export.bin".to_owned());
    let atlas_name = atlas_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export.atlas.png".to_owned());

    let mut bin = BufWriter::new(std::fs::File::create(&bin_path)?);
    let mut byte_offset = 0u64;
    // Track JSON arrays we'll emit.
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes_json = Vec::new();
    let mut materials_json = Vec::new();

    for (pi, prim) in primitives.iter().enumerate() {
        let vcount = prim.vertices.len() as u32;
        let icount = prim.indices.len() as u32;
        if vcount == 0 || icount == 0 {
            continue;
        }

        // Dequantise vertex data and write to bin.
        let pos_off = byte_offset;
        for vk in &prim.vertices {
            let p = [
                vk.pos[0] as f32 / 16.0 - center[0] as f32,
                vk.pos[1] as f32 / 16.0 - center[1] as f32,
                vk.pos[2] as f32 / 16.0 - center[2] as f32,
            ];
            bin.write_all(bytemuck::cast_slice(&p))?;
        }
        let pos_size = vcount as u64 * 12;
        byte_offset += pos_size;

        let norm_off = byte_offset;
        for vk in &prim.vertices {
            let n: [f32; 3] = [
                vk.normal[0] as f32,
                vk.normal[1] as f32,
                vk.normal[2] as f32,
            ];
            bin.write_all(bytemuck::cast_slice(&n))?;
        }
        let norm_size = vcount as u64 * 12;
        byte_offset += norm_size;

        let uv_off = byte_offset;
        for vk in &prim.vertices {
            let uv: [f32; 2] = [vk.uv[0] as f32 / 65536.0, vk.uv[1] as f32 / 65536.0];
            bin.write_all(bytemuck::cast_slice(&uv))?;
        }
        let uv_size = vcount as u64 * 8;
        byte_offset += uv_size;

        let idx_off = byte_offset;
        for &i in &prim.indices {
            bin.write_all(&i.to_le_bytes())?;
        }
        let idx_size = icount as u64 * 4;
        byte_offset += idx_size;

        // Compute bounding box for positions.
        let mut min_pos = [f32::MAX; 3];
        let mut max_pos = [f32::MIN; 3];
        for vk in &prim.vertices {
            let p = [
                vk.pos[0] as f32 / 16.0 - center[0] as f32,
                vk.pos[1] as f32 / 16.0 - center[1] as f32,
                vk.pos[2] as f32 / 16.0 - center[2] as f32,
            ];
            for a in 0..3 {
                min_pos[a] = min_pos[a].min(p[a]);
                max_pos[a] = max_pos[a].max(p[a]);
            }
        }

        // Buffer views for this primitive.
        let bv_pos = buffer_views.len() as u32;
        buffer_views.push(json!({
            "buffer": 0, "byteOffset": pos_off, "byteLength": pos_size, "target": 34962
        }));
        let bv_norm = buffer_views.len() as u32;
        buffer_views.push(json!({
            "buffer": 0, "byteOffset": norm_off, "byteLength": norm_size, "target": 34962
        }));
        let bv_uv = buffer_views.len() as u32;
        buffer_views.push(json!({
            "buffer": 0, "byteOffset": uv_off, "byteLength": uv_size, "target": 34962
        }));
        let bv_idx = buffer_views.len() as u32;
        buffer_views.push(json!({
            "buffer": 0, "byteOffset": idx_off, "byteLength": idx_size, "target": 34963
        }));

        // Accessors.
        let acc_pos = accessors.len() as u32;
        accessors.push(json!({
            "bufferView": bv_pos, "componentType": 5126, "count": vcount, "type": "VEC3",
            "min": [min_pos[0], min_pos[1], min_pos[2]],
            "max": [max_pos[0], max_pos[1], max_pos[2]]
        }));
        let acc_norm = accessors.len() as u32;
        accessors.push(json!({
            "bufferView": bv_norm, "componentType": 5126, "count": vcount, "type": "VEC3"
        }));
        let acc_uv = accessors.len() as u32;
        accessors.push(json!({
            "bufferView": bv_uv, "componentType": 5126, "count": vcount, "type": "VEC2"
        }));
        let acc_idx = accessors.len() as u32;
        accessors.push(json!({
            "bufferView": bv_idx, "componentType": 5125, "count": icount, "type": "SCALAR"
        }));

        // Mesh with one primitive per material group.
        meshes_json.push(json!({
            "primitives": [{
                "attributes": {
                    "POSITION": acc_pos,
                    "NORMAL": acc_norm,
                    "TEXCOORD_0": acc_uv
                },
                "indices": acc_idx,
                "material": pi
            }],
            "name": format!("mat_{}", pi)
        }));

        // Material with PBR preset.
        let preset = pbr_preset(&prim.material_name);
        let (roughness, metallic, emissive_rgb, emissive_strength) = preset
            .map(|p| (p.roughness, p.metallic, p.emissive, p.emissive_strength))
            .unwrap_or((0.85, 0.0, [0.0; 3], 1.0));

        materials_json.push(json!({
            "name": prim.material_name,
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": 0, "texCoord": 0 },
                "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
                "metallicFactor": metallic,
                "roughnessFactor": roughness
            },
            "emissiveFactor": [emissive_rgb[0], emissive_rgb[1], emissive_rgb[2]],
            "emissiveStrength": emissive_strength,
            "doubleSided": true
        }));
    }

    drop(bin);

    // ── Write atlas PNG ──────────────────────────────────────────────────
    image::save_buffer(
        &atlas_path,
        &tiles.atlas.pixels,
        tiles.atlas.width,
        tiles.atlas.height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|err| ExportError::Io(std::io::Error::other(err)))?;

    // ── Write glTF JSON ──────────────────────────────────────────────────
    let total_bin_size = byte_offset;
    let gltf = json!({
        "asset": {
            "version": "2.0",
            "generator": "Project Bedrock"
        },
        "scene": 0,
        "scenes": [{
            "nodes": [0]
        }],
        "nodes": [{
            "mesh": 0
        }],
        "meshes": [{
            "primitives": (0..meshes_json.len()).flat_map(|i| {
                meshes_json[i]["primitives"].as_array().unwrap().clone()
            }).collect::<Vec<_>>()
        }],
        "accessors": accessors,
        "bufferViews": buffer_views,
        "buffers": [{
            "uri": bin_name,
            "byteLength": total_bin_size
        }],
        "materials": materials_json,
        "textures": [{
            "sampler": 0,
            "source": 0
        }],
        "images": [{
            "uri": atlas_name
        }],
        "samplers": [{
            "magFilter": 9729,
            "minFilter": 9729
        }]
    });

    let gltf_file = std::fs::File::create(gltf_path)?;
    serde_json::to_writer_pretty(gltf_file, &gltf)
        .map_err(|err| ExportError::Io(std::io::Error::other(err)))?;

    Ok(ExportStats {
        obj_path: gltf_path.to_path_buf(),
        mtl_path: PathBuf::new(), // glTF uses embedded materials
        blocks,
        faces: total_faces,
        materials: primitives.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bedrock_parser::jar_textures::JarTextureLoader;
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

    fn test_chunk() -> Chunk {
        let mut indices = vec![0u16; 4096];
        indices[0] = 1;
        indices[1] = 1;
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

    fn test_tileset(names: &[String]) -> FaceAwareTileSet {
        FaceAwareTileSet::build(names.iter().cloned(), &JarTextureLoader::empty())
    }

    #[test]
    fn exports_two_stone_blocks() {
        let chunk = test_chunk();
        let region = ExportRegion {
            min: [0, 0, 0],
            max: [16, 16, 16],
        };
        let dir = std::env::temp_dir().join("bedrock-gltf-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gltf_path = dir.join("test.gltf");

        let block_names: Vec<String> = chunk.block_names().into_iter().map(str::to_owned).collect();
        let tiles = test_tileset(&block_names);
        let stats = export_gltf(&[chunk], &region, &gltf_path, &tiles).unwrap();

        assert!(
            stats.blocks >= 2,
            "expected ≥2 blocks, got {}",
            stats.blocks
        );
        assert!(stats.faces > 0, "expected faces");
        assert!(stats.materials >= 1, "expected materials");

        // Verify files exist
        assert!(gltf_path.exists(), "gltf file missing");
        assert!(gltf_path.with_extension("bin").exists(), "bin file missing");
        assert!(
            gltf_path.with_extension("atlas.png").exists(),
            "atlas missing"
        );

        // Verify glTF JSON is parseable and has expected structure
        let text = std::fs::read_to_string(&gltf_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["asset"]["version"], "2.0");
        assert!(parsed["accessors"].as_array().unwrap().len() >= 4);
        assert!(parsed["bufferViews"].as_array().unwrap().len() >= 4);
        assert!(!parsed["materials"].as_array().unwrap().is_empty());
        assert!(!parsed["textures"].as_array().unwrap().is_empty());
        assert!(!parsed["images"].as_array().unwrap().is_empty());

        eprintln!(
            "glTF export: {} blocks, {} faces, {} materials",
            stats.blocks, stats.faces, stats.materials
        );
        eprintln!(
            "Accessors: {}",
            parsed["accessors"].as_array().unwrap().len()
        );
        eprintln!(
            "BufferViews: {}",
            parsed["bufferViews"].as_array().unwrap().len()
        );
        eprintln!(
            "Materials: {}",
            parsed["materials"].as_array().unwrap().len()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gltf_has_pbr_parameters() {
        let chunk = test_chunk();
        let region = ExportRegion {
            min: [0, 0, 0],
            max: [2, 1, 1],
        };
        let dir = std::env::temp_dir().join("bedrock-gltf-pbr");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gltf_path = dir.join("pbr.gltf");

        let block_names: Vec<String> = chunk.block_names().into_iter().map(str::to_owned).collect();
        let tiles = test_tileset(&block_names);
        let _stats = export_gltf(&[chunk], &region, &gltf_path, &tiles).unwrap();

        let text = std::fs::read_to_string(&gltf_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mat = &parsed["materials"][0];
        assert!(mat["pbrMetallicRoughness"]["roughnessFactor"]
            .as_f64()
            .is_some());
        assert!(mat["pbrMetallicRoughness"]["metallicFactor"]
            .as_f64()
            .is_some());
        assert!(mat["pbrMetallicRoughness"]["baseColorTexture"]["index"]
            .as_u64()
            .is_some());
        assert_eq!(mat["doubleSided"], true);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
