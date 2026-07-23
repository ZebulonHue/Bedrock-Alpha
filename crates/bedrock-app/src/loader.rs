//! World loading: reads region files around the player's last position and
//! meshes the chunks for the viewport. Also provides streaming support for
//! dynamically loading/unloading chunks as the camera moves.
//!
//! The initial load runs on a worker thread so the UI never freezes; results
//! travel back over an mpsc channel. Streaming runs on the UI thread (fast).

use bedrock_parser::bedrock::BedrockWorld;
use bedrock_parser::blocks::{block_color, is_air};
use bedrock_parser::chunk::Chunk;
use bedrock_parser::detect::{Edition, WorldSummary};
use bedrock_parser::jar_textures::JarTextureLoader;
use bedrock_parser::java_simple::ExteriorWorld;
use bedrock_parser::region::RegionFile;
use bedrock_parser::texture::FaceAwareTileSet;
use bedrock_parser::world::World;
use bedrock_render::math::Camera;
use bedrock_render::mesh::{chunks_to_meshes, mesh_exterior_world, AtlasPixels, ChunkMesh};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// A top-down, one-pixel-per-block color map of the loaded chunks.
pub struct OverviewImage {
    /// RGBA pixels, row-major, one pixel per block column.
    pub rgba: Vec<u8>,
    /// Width in pixels (blocks).
    pub width: usize,
    /// Height in pixels (blocks).
    pub height: usize,
    /// World X of the leftmost pixel column.
    pub origin_x: i32,
    /// World Z of the topmost pixel row.
    pub origin_z: i32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming support
// ─────────────────────────────────────────────────────────────────────────────

/// A handle to an open Minecraft world, kept alive for on-demand chunk loading.
pub enum WorldHandle {
    /// Java Edition (region files).
    Java(World),
    /// Bedrock Edition (LevelDB).
    Bedrock(Box<BedrockWorld>),
}

/// An actively loaded world with streaming support.
pub struct ActiveWorld {
    /// Display name.
    pub name: String,
    /// The open world handle for loading additional chunks.
    pub handle: WorldHandle,
    /// All chunks currently in memory, keyed by `(chunk_x, chunk_z)`.
    pub chunk_map: HashMap<(i32, i32), Chunk>,
    /// Chunk coordinates that are currently meshed and uploaded to the GPU.
    pub loaded_chunks: HashSet<(i32, i32)>,
    /// The tileset used for meshing (shared by all chunks).
    pub tiles: FaceAwareTileSet,
    /// Atlas pixels (shared by all chunks, uploaded once to the GPU).
    pub atlas: AtlasPixels,
    /// Player's last position, for "Snap to Player".
    pub player_pos: Option<[f64; 3]>,
    /// Top-down color map for the 2D Overview panel.
    pub overview: Option<OverviewImage>,
    /// Initial camera framing.
    pub camera: Camera,
}

/// Open a world and return an `ActiveWorld` with chunks around the player
/// position loaded and meshed. This is the streaming-capable replacement for
/// the initial load path.
pub fn load_active_world(
    summary: &WorldSummary,
    radius_chunks: i32,
) -> Result<(ActiveWorld, Vec<ChunkMesh>), String> {
    match summary.edition {
        Edition::Java => load_java_simple(summary),
        Edition::Bedrock => load_bedrock_active(summary, radius_chunks),
    }
}

fn load_bedrock_active(
    summary: &WorldSummary,
    radius_chunks: i32,
) -> Result<(ActiveWorld, Vec<ChunkMesh>), String> {
    let world = BedrockWorld::open(summary.folder.clone()).map_err(|e| e.to_string())?;
    let player_pos = world.player_pos();
    let (center_x, center_z) = match player_pos {
        Some([x, _, z]) => ((x / 16.0).floor() as i32, (z / 16.0).floor() as i32),
        None => (0, 0),
    };
    tracing::info!(
        "Loading Bedrock world '{}' around chunk ({center_x}, {center_z}) with radius {radius_chunks}",
        summary.name
    );
    let chunks = world
        .chunks_near(center_x, center_z, radius_chunks)
        .map_err(|e| e.to_string())?;
    finish_active_loading(
        &summary.name,
        WorldHandle::Bedrock(Box::new(world)),
        chunks,
        (center_x, center_z),
        player_pos,
    )
}

/// Extract which dimension a region file belongs to from its path.
#[allow(dead_code)]
fn dim_from_path(path: &Path) -> i32 {
    for ancestor in path.ancestors() {
        if let Some(name) = ancestor.file_name().and_then(|n| n.to_str()) {
            if let Some(rest) = name.strip_prefix("DIM") {
                if let Ok(dim) = rest.parse::<i32>() {
                    return dim;
                }
            }
        }
    }
    0
}

#[allow(dead_code)]
fn load_java_active(
    summary: &WorldSummary,
    radius_chunks: i32,
) -> Result<(ActiveWorld, Vec<ChunkMesh>), String> {
    let world = World::open(summary.folder.clone());
    // level.dat may be absent for standalone dimension folders (e.g. DIM1).
    let (center_x, center_z, player_pos) = match world.level_meta() {
        Ok(meta) => {
            let (cx, cz) = match meta.player_pos {
                Some([x, _, z]) => ((x / 16.0).floor() as i32, (z / 16.0).floor() as i32),
                None => (0, 0),
            };
            (cx, cz, meta.player_pos)
        }
        Err(_) => {
            tracing::info!("No level.dat — using chunk (0,0) as loading center");
            (0, 0, None)
        }
    };
    tracing::info!(
        "Loading Java world '{}' around chunk ({center_x}, {center_z}) with radius {radius_chunks}",
        summary.name
    );

    let min_rx = (center_x - radius_chunks).div_euclid(32);
    let max_rx = (center_x + radius_chunks).div_euclid(32);
    let min_rz = (center_z - radius_chunks).div_euclid(32);
    let max_rz = (center_z + radius_chunks).div_euclid(32);

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut per_dim: HashMap<i32, usize> = HashMap::new();
    for (rx, rz, path) in world.regions() {
        if rx < min_rx || rx > max_rx || rz < min_rz || rz > max_rz {
            continue;
        }
        let dim = dim_from_path(&path);
        let Ok(mut region) = RegionFile::open(&path) else {
            continue;
        };
        let mut dim_count = 0usize;
        for (lx, lz) in region.present_chunks() {
            let cx = rx * 32 + i32::from(lx);
            let cz = rz * 32 + i32::from(lz);
            if (cx - center_x).abs() > radius_chunks || (cz - center_z).abs() > radius_chunks {
                continue;
            }
            if let Some(Ok(nbt)) = region.chunk_nbt(lx, lz) {
                if let Ok(chunk) = Chunk::from_nbt(&nbt) {
                    chunks.push(chunk);
                    dim_count += 1;
                }
            }
        }
        *per_dim.entry(dim).or_insert(0) += dim_count;
    }
    // Log per-dimension loading summary.
    let dim_desc: Vec<String> = per_dim
        .iter()
        .map(|(d, c)| {
            let label = match *d {
                0 => "Overworld".into(),
                1 => "End".into(),
                -1 => "Nether".into(),
                k => format!("DIM{k}"),
            };
            format!("{label}: {c} chunks")
        })
        .collect();
    tracing::info!(
        "Loaded {} chunks from dimension(s): {}",
        chunks.len(),
        dim_desc.join(", ")
    );
    finish_active_loading(
        &summary.name,
        WorldHandle::Java(world),
        chunks,
        (center_x, center_z),
        player_pos,
    )
}

/// Load a Java world using chunkforge-core's simple approach.
/// All blocks render as cubes with opacity-based face culling.
pub fn load_java_simple(summary: &WorldSummary) -> Result<(ActiveWorld, Vec<ChunkMesh>), String> {
    let paths: Vec<std::path::PathBuf> = find_mca_files(&summary.folder);
    if paths.is_empty() {
        return Err(format!(
            "No .mca files found in '{}'",
            summary.folder.display()
        ));
    }

    let world = ExteriorWorld::load(&paths)?;
    if world.is_empty() {
        return Err("No exterior blocks found — world is empty".into());
    }

    // Build procedural tile set (no JAR loading).
    let block_names: Vec<String> = world.names.clone();
    let tiles = FaceAwareTileSet::build(block_names, &JarTextureLoader::empty());

    let chunk_meshes = mesh_exterior_world(&world, &tiles);

    let atlas = AtlasPixels {
        rgba: tiles.atlas.pixels.clone(),
        width: tiles.atlas.width,
        height: tiles.atlas.height,
    };

    let total_verts: usize = chunk_meshes.iter().map(|m| m.vertices.len()).sum();
    let total_tris: usize = chunk_meshes.iter().map(|m| m.triangle_count()).sum();
    tracing::info!(
        "Meshed {} blocks: {} vertices, {} triangles",
        world.len(),
        total_verts,
        total_tris,
    );

    // Center camera on the world bounds.
    let world_center = world_center(&world);
    let camera = Camera {
        target: [world_center[0], world_center[1], world_center[2]],
        ..Camera::default()
    };

    let active = ActiveWorld {
        name: summary.name.clone(),
        handle: WorldHandle::Java(World::open(summary.folder.clone())),
        chunk_map: HashMap::new(),
        loaded_chunks: HashSet::new(),
        tiles,
        atlas,
        player_pos: None,
        overview: None,
        camera,
    };

    Ok((active, chunk_meshes))
}

/// Recursively find all `.mca` files under a folder.
fn find_mca_files(folder: &Path) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(find_mca_files(&path));
            } else if path.extension().is_some_and(|x| x == "mca") {
                results.push(path);
            }
        }
    }
    results
}

/// Compute the center of an ExteriorWorld (average of bounds).
fn world_center(world: &ExteriorWorld) -> [f32; 3] {
    let mut min = [i32::MAX; 3];
    let mut max = [i32::MIN; 3];
    for &(x, y, z) in world.blocks.keys() {
        min[0] = min[0].min(x);
        max[0] = max[0].max(x);
        min[1] = min[1].min(y);
        max[1] = max[1].max(y);
        min[2] = min[2].min(z);
        max[2] = max[2].max(z);
    }
    [
        (min[0] + max[0]) as f32 / 2.0,
        (min[1] + max[1]) as f32 / 2.0,
        (min[2] + max[2]) as f32 / 2.0,
    ]
}

/// Shared tail: build tileset, mesh, overview, camera, wrap in ActiveWorld.
fn finish_active_loading(
    name: &str,
    handle: WorldHandle,
    chunks: Vec<Chunk>,
    (center_x, center_z): (i32, i32),
    player_pos: Option<[f64; 3]>,
) -> Result<(ActiveWorld, Vec<ChunkMesh>), String> {
    if chunks.is_empty() {
        return Err(format!(
            "No generated chunks found near chunk ({center_x}, {center_z}) — \
             try exploring that area in Minecraft first"
        ));
    }

    let block_names: Vec<String> = chunks
        .iter()
        .flat_map(|c| c.block_names().into_iter().map(str::to_owned))
        .collect();

    let tiles = match JarTextureLoader::load() {
        Ok(loader) => {
            tracing::info!(
                "Using real textures from Minecraft {} ({} textures)",
                loader.version,
                loader.len()
            );
            FaceAwareTileSet::build(block_names, &loader)
        }
        Err(reason) => {
            tracing::warn!(
                "No Minecraft textures available — using procedural tiles. Reason: {reason}"
            );
            FaceAwareTileSet::build(block_names, &JarTextureLoader::empty())
        }
    };

    let chunk_meshes = chunks_to_meshes(&chunks, &tiles);
    let atlas = AtlasPixels {
        rgba: tiles.atlas.pixels.clone(),
        width: tiles.atlas.width,
        height: tiles.atlas.height,
    };

    let total_verts: usize = chunk_meshes.iter().map(|m| m.vertices.len()).sum();
    let total_tris: usize = chunk_meshes.iter().map(|m| m.triangle_count()).sum();
    let overview = build_overview(&chunks);
    tracing::info!(
        "Meshed {} chunks: {} vertices, {} triangles",
        chunks.len(),
        total_verts,
        total_tris
    );

    let center_y = player_pos.map(|p| p[1] as f32).unwrap_or(70.0);
    let camera = Camera {
        target: [
            (center_x * 16) as f32 + 8.0,
            center_y,
            (center_z * 16) as f32 + 8.0,
        ],
        ..Camera::default()
    };

    let chunk_map: HashMap<(i32, i32), Chunk> =
        chunks.into_iter().map(|c| ((c.x, c.z), c)).collect();
    let loaded_chunks: HashSet<(i32, i32)> = chunk_map.keys().copied().collect();

    let world = ActiveWorld {
        name: name.to_owned(),
        handle,
        chunk_map,
        loaded_chunks,
        tiles,
        atlas,
        player_pos,
        overview,
        camera,
    };
    Ok((world, chunk_meshes))
}

/// Load a single chunk from disk by its chunk coordinates. Returns `None` if
/// the chunk does not exist or fails to decode.
pub fn load_one_chunk(handle: &WorldHandle, cx: i32, cz: i32) -> Option<Chunk> {
    match handle {
        WorldHandle::Java(world) => load_one_java_chunk(world, cx, cz),
        WorldHandle::Bedrock(world) => load_one_bedrock_chunk(world, cx, cz),
    }
}

fn load_one_java_chunk(world: &World, cx: i32, cz: i32) -> Option<Chunk> {
    let rx = cx.div_euclid(32);
    let rz = cz.div_euclid(32);
    let lx = (cx.rem_euclid(32)) as u8;
    let lz = (cz.rem_euclid(32)) as u8;
    // A chunk may live in any dimension folder (overworld `region/` or
    // `DIM<n>/region/`); try each region file matching the coordinates.
    for (region_rx, region_rz, path) in world.regions() {
        if region_rx != rx || region_rz != rz {
            continue;
        }
        let Ok(mut region) = RegionFile::open(&path) else {
            continue;
        };
        if let Some(Ok(nbt)) = region.chunk_nbt(lx, lz) {
            if let Ok(chunk) = Chunk::from_nbt(&nbt) {
                return Some(chunk);
            }
        }
    }
    None
}

fn load_one_bedrock_chunk(world: &BedrockWorld, cx: i32, cz: i32) -> Option<Chunk> {
    // BedrockWorld::chunks_near loads a batch. We call it with radius 0 to
    // load a single chunk. This is slightly wasteful (scans keys) but
    // simple and fast enough for streaming.
    let chunks = world.chunks_near(cx, cz, 0).ok()?;
    // The returned chunks should include our target.
    chunks.into_iter().find(|c| c.x == cx && c.z == cz)
}

// ─────────────────────────────────────────────────────────────────────────────
// Overview image
// ─────────────────────────────────────────────────────────────────────────────

/// Build the top-down overview image: each pixel is the color of the
/// highest non-air block in that column.
fn build_overview(chunks: &[Chunk]) -> Option<OverviewImage> {
    let min_cx = chunks.iter().map(|c| c.x).min()?;
    let max_cx = chunks.iter().map(|c| c.x).max()?;
    let min_cz = chunks.iter().map(|c| c.z).min()?;
    let max_cz = chunks.iter().map(|c| c.z).max()?;
    let width = (max_cx - min_cx + 1) as usize * 16;
    let height = (max_cz - min_cz + 1) as usize * 16;

    let mut rgba = vec![0u8; width * height * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[11, 12, 15, 255]); // match the UI background
    }
    for chunk in chunks {
        let Some((min_y, max_y)) = chunk.y_range() else {
            continue;
        };
        for z in 0..16usize {
            for x in 0..16usize {
                for y in (min_y..max_y).rev() {
                    match chunk.block_at(x, y, z) {
                        Some(name) if !is_air(name) => {
                            let [r, g, b] = block_color(name);
                            let px = (chunk.x - min_cx) as usize * 16 + x;
                            let pz = (chunk.z - min_cz) as usize * 16 + z;
                            let offset = (pz * width + px) * 4;
                            rgba[offset..offset + 4].copy_from_slice(&[
                                (r * 255.0) as u8,
                                (g * 255.0) as u8,
                                (b * 255.0) as u8,
                                255,
                            ]);
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Some(OverviewImage {
        rgba,
        width,
        height,
        origin_x: min_cx * 16,
        origin_z: min_cz * 16,
    })
}
