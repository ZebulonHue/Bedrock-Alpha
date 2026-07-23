//! Chunk meshing: turns decoded chunks into face-culled cube geometry.
//!
//! Faces are textured per direction (top/side/bottom) from a
//! [`FaceAwareTileSet`].

use bedrock_parser::block_shape;
use bedrock_parser::blocks::is_air;
use bedrock_parser::chunk::Chunk;
use bedrock_parser::java_simple::ExteriorWorld;
use bedrock_parser::texture::FaceAwareTileSet;
use std::collections::HashMap;

/// Unique identifier for a chunk within a world.
pub type ChunkId = (i32, i32);

/// One mesh vertex: position, face normal, atlas UV.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// World-space position (1 unit = 1 block).
    pub pos: [f32; 3],
    /// Outward face normal.
    pub normal: [f32; 3],
    /// UV inside the texture atlas.
    pub uv: [f32; 2],
}

/// Raw atlas pixels, carried with the mesh so the renderer can upload them.
#[derive(Clone)]
pub struct AtlasPixels {
    /// RGBA pixels, row-major.
    pub rgba: Vec<u8>,
    /// Atlas width in pixels.
    pub width: u32,
    /// Atlas height in pixels.
    pub height: u32,
}

/// Meshed geometry for a single chunk, including its world-space bounds.
pub struct ChunkMesh {
    /// Chunk coordinate key `(chunk_x, chunk_z)`.
    pub id: ChunkId,
    /// Vertex buffer contents.
    pub vertices: Vec<Vertex>,
    /// Triangle indices into `vertices`.
    pub indices: Vec<u32>,
    /// Minimum corner of the chunk's occupied block column (world coords).
    pub bounds_min: [i32; 3],
    /// Maximum corner (exclusive) of the chunk's occupied column.
    pub bounds_max: [i32; 3],
}

impl ChunkMesh {
    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A triangle mesh built from world chunks (kept for backward compat and
/// simple use cases). Prefer per-chunk meshing via [`chunks_to_meshes`].
#[derive(Default)]
pub struct MeshData {
    /// Vertex buffer contents.
    pub vertices: Vec<Vertex>,
    /// Triangle indices into `vertices`.
    pub indices: Vec<u32>,
    /// Texture atlas covering every block in the mesh.
    pub atlas: Option<AtlasPixels>,
}

impl MeshData {
    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Compute per-corner UV coordinates (0..1) within a face, normalised to the
/// face's bounding box so the full texture tile maps onto any rectangular face.
fn face_normalized_uvs(corners: &[[f64; 3]; 4], normal: [f32; 3]) -> [[f32; 2]; 4] {
    let (ua, va) = if normal[1] != 0.0 {
        (0usize, 2usize)
    } else if normal[0] != 0.0 {
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

/// Mesh a single chunk, returning its geometry and world-space bounds.
/// Uses `by_coord` / `lookup` for cross-chunk face culling.
///
/// The `lookup` closure should provide the block name at any world
/// coordinate — including coordinates in *neighbouring* chunks — so that
/// faces between opaque blocks are correctly dropped across chunk borders.
pub fn mesh_one_chunk<'a, F>(
    chunk: &Chunk,
    lookup: &F,
    tiles: &FaceAwareTileSet,
) -> Option<ChunkMesh>
where
    F: Fn(i32, i32, i32) -> Option<&'a str>,
{
    let (min_y, max_y) = chunk.y_range()?;
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut bounds_min = [i32::MAX; 3];
    let mut bounds_max = [i32::MIN; 3];
    let mut has_geometry = false;

    for y in min_y..max_y {
        for z in 0..16usize {
            for x in 0..16usize {
                let Some(state) = chunk.block_state_at(x, y, z) else {
                    continue;
                };
                let name = state.name.as_str();
                if is_air(name) {
                    continue;
                }
                has_geometry = true;
                let wx = chunk.x * 16 + x as i32;
                let wz = chunk.z * 16 + z as i32;
                bounds_min[0] = bounds_min[0].min(wx);
                bounds_min[1] = bounds_min[1].min(y);
                bounds_min[2] = bounds_min[2].min(wz);
                bounds_max[0] = bounds_max[0].max(wx + 1);
                bounds_max[1] = bounds_max[1].max(y + 1);
                bounds_max[2] = bounds_max[2].max(wz + 1);
                let quads = block_shape::block_quads(wx, y, wz, name, lookup);
                for quad in &quads {
                    let [u0, v0, u1, v1] = if let Some(tex) = &quad.texture {
                        tiles.tile_uv(tex)
                    } else {
                        tiles.face_uv(name, quad.face_idx)
                    };
                    let corner_uvs = face_normalized_uvs(
                        &quad.corners,
                        [
                            quad.normal[0] as f32,
                            quad.normal[1] as f32,
                            quad.normal[2] as f32,
                        ],
                    );
                    let base = vertices.len() as u32;
                    for (corner, &[fu, fv]) in quad.corners.iter().zip(corner_uvs.iter()) {
                        vertices.push(Vertex {
                            pos: [corner[0] as f32, corner[1] as f32, corner[2] as f32],
                            normal: [
                                quad.normal[0] as f32,
                                quad.normal[1] as f32,
                                quad.normal[2] as f32,
                            ],
                            uv: [u0 + fu * (u1 - u0), v0 + (1.0 - fv) * (v1 - v0)],
                        });
                    }
                    indices.extend_from_slice(&[
                        base,
                        base + 1,
                        base + 2,
                        base,
                        base + 2,
                        base + 3,
                    ]);
                }
            }
        }
    }

    if !has_geometry {
        return None;
    }
    Some(ChunkMesh {
        id: (chunk.x, chunk.z),
        vertices,
        indices,
        bounds_min,
        bounds_max,
    })
}

/// Mesh every chunk individually, returning per-chunk results.
/// Faces between opaque cubes are dropped — including across chunk borders.
///
/// For large worlds, chunks are meshed in parallel using scoped threads
/// (no external dependencies required).
pub fn chunks_to_meshes(chunks: &[Chunk], tiles: &FaceAwareTileSet) -> Vec<ChunkMesh> {
    let by_coord: HashMap<(i32, i32), &Chunk> = chunks.iter().map(|c| ((c.x, c.z), c)).collect();

    // Sequential path: avoids thread overhead for small workloads and
    // for the common case of a single-player base (few chunks).
    if chunks.len() < 16 {
        let lookup = |wx: i32, y: i32, wz: i32| -> Option<&str> {
            let cx = wx.div_euclid(16);
            let cz = wz.div_euclid(16);
            let chunk = by_coord.get(&(cx, cz))?;
            chunk.block_at(wx.rem_euclid(16) as usize, y, wz.rem_euclid(16) as usize)
        };
        return chunks
            .iter()
            .filter_map(|chunk| mesh_one_chunk(chunk, &lookup, tiles))
            .collect();
    }

    // Parallel path: divide into batches, mesh each batch in its own thread.
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let batch_size = chunks.len().div_ceil(num_threads);

    std::thread::scope(|s| {
        let mut results: Vec<std::thread::ScopedJoinHandle<'_, Vec<ChunkMesh>>> =
            Vec::with_capacity(num_threads);

        for batch in chunks.chunks(batch_size) {
            results.push(s.spawn(|| {
                // Each thread builds its own lookup closure borrowing from the
                // parent scope's `by_coord` table (read-only, safe to share).
                let lookup = |wx: i32, y: i32, wz: i32| -> Option<&str> {
                    let cx = wx.div_euclid(16);
                    let cz = wz.div_euclid(16);
                    let chunk = by_coord.get(&(cx, cz))?;
                    chunk.block_at(wx.rem_euclid(16) as usize, y, wz.rem_euclid(16) as usize)
                };
                batch
                    .iter()
                    .filter_map(|chunk| mesh_one_chunk(chunk, &lookup, tiles))
                    .collect::<Vec<_>>()
            }));
        }

        results
            .into_iter()
            .flat_map(|h| h.join())
            .flatten()
            .collect()
    })
}

/// Build one face-culled mesh from a set of chunks, textured from `tiles`.
/// Faces between opaque cubes are dropped — including across chunk borders.
pub fn mesh_chunks(chunks: &[Chunk], tiles: &FaceAwareTileSet) -> MeshData {
    let by_coord: HashMap<(i32, i32), &Chunk> = chunks.iter().map(|c| ((c.x, c.z), c)).collect();
    let lookup = |wx: i32, y: i32, wz: i32| -> Option<&str> {
        let cx = wx.div_euclid(16);
        let cz = wz.div_euclid(16);
        let chunk = by_coord.get(&(cx, cz))?;
        chunk.block_at(wx.rem_euclid(16) as usize, y, wz.rem_euclid(16) as usize)
    };

    let mut mesh = MeshData {
        atlas: Some(AtlasPixels {
            rgba: tiles.atlas.pixels.clone(),
            width: tiles.atlas.width,
            height: tiles.atlas.height,
        }),
        ..MeshData::default()
    };
    for chunk in chunks {
        let Some(chunk_mesh) = mesh_one_chunk(chunk, &lookup, tiles) else {
            continue;
        };
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend(chunk_mesh.vertices);
        mesh.indices
            .extend(chunk_mesh.indices.iter().map(|i| i + base));
    }
    mesh
}

/// Mesh an `ExteriorWorld`: generates per-chunk cube geometry with opacity
/// culling. Returns one `ChunkMesh` per non-empty chunk column.
pub fn mesh_exterior_world(world: &ExteriorWorld, tiles: &FaceAwareTileSet) -> Vec<ChunkMesh> {
    let lookup = |x: i32, y: i32, z: i32| world.block_at(x, y, z);

    // Group blocks by their chunk column.
    struct ChunkBuilder {
        vertices: Vec<Vertex>,
        indices: Vec<u32>,
        bounds_min: [i32; 3],
        bounds_max: [i32; 3],
    }
    let mut chunks: HashMap<(i32, i32), ChunkBuilder> = HashMap::new();

    for (&(x, y, z), name) in &world.blocks {
        if is_air(name) {
            continue;
        }
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let cb = chunks.entry((cx, cz)).or_insert_with(|| ChunkBuilder {
            vertices: Vec::new(),
            indices: Vec::new(),
            bounds_min: [i32::MAX; 3],
            bounds_max: [i32::MIN; 3],
        });
        cb.bounds_min[0] = cb.bounds_min[0].min(x);
        cb.bounds_min[1] = cb.bounds_min[1].min(y);
        cb.bounds_min[2] = cb.bounds_min[2].min(z);
        cb.bounds_max[0] = cb.bounds_max[0].max(x + 1);
        cb.bounds_max[1] = cb.bounds_max[1].max(y + 1);
        cb.bounds_max[2] = cb.bounds_max[2].max(z + 1);

        let quads = block_shape::block_quads(x, y, z, name, &lookup);
        for quad in &quads {
            let [u0, v0, u1, v1] = if let Some(tex) = &quad.texture {
                tiles.tile_uv(tex)
            } else {
                tiles.face_uv(name, quad.face_idx)
            };
            let corner_uvs = face_normalized_uvs(
                &quad.corners,
                [
                    quad.normal[0] as f32,
                    quad.normal[1] as f32,
                    quad.normal[2] as f32,
                ],
            );
            let base = cb.vertices.len() as u32;
            for (corner, &[fu, fv]) in quad.corners.iter().zip(corner_uvs.iter()) {
                cb.vertices.push(Vertex {
                    pos: [corner[0] as f32, corner[1] as f32, corner[2] as f32],
                    normal: [
                        quad.normal[0] as f32,
                        quad.normal[1] as f32,
                        quad.normal[2] as f32,
                    ],
                    uv: [u0 + fu * (u1 - u0), v0 + (1.0 - fv) * (v1 - v0)],
                });
            }
            cb.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    chunks
        .into_iter()
        .map(|((cx, cz), cb)| ChunkMesh {
            id: (cx, cz),
            vertices: cb.vertices,
            indices: cb.indices,
            bounds_min: cb.bounds_min,
            bounds_max: cb.bounds_max,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // Face winding is verified by block_shape's own tests.
}
