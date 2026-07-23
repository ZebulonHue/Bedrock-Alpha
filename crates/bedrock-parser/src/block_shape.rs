//! Simplified block geometry: all blocks render as simple cubes.
//!
//! Face culling uses chunkforge-core's appearance-table opacity rather
//! than Mineways shape flags.  No slabs, stairs, fences, panes, or other
//! complex shapes — just cubes.

use crate::java_simple::appearance;

/// One quad face emitted for a block.
#[derive(Debug, Clone)]
pub struct BlockQuad {
    /// 4 corners in world space, CW from outside.
    pub corners: [[f64; 3]; 4],
    /// Face index 0..5 for UV look-up (top/bottom/east/west/south/north).
    pub face_idx: usize,
    /// Normal direction as a face-index offset.
    pub normal: [i32; 3],
    /// Optional override texture name (None = use face-based UV from block).
    pub texture: Option<String>,
}

/// Get the quads for a block.  All blocks are full cubes; visible faces
/// are determined solely by neighbour opacity (chunkforge-core's table).
pub fn block_quads<'a>(
    wx: i32,
    wy: i32,
    wz: i32,
    _block_name: &str,
    lookup: &impl Fn(i32, i32, i32) -> Option<&'a str>,
) -> Vec<BlockQuad> {
    let mut quads = Vec::with_capacity(6);
    for (face_idx, (normal, corners)) in CUBE_FACES.iter().enumerate() {
        let nx = wx + normal[0];
        let ny = wy + normal[1];
        let nz = wz + normal[2];
        if is_occluded(nx, ny, nz, lookup) {
            continue;
        }
        let offset = [wx as f64, wy as f64, wz as f64];
        quads.push(BlockQuad {
            corners: [
                add3(corners[0], offset),
                add3(corners[1], offset),
                add3(corners[2], offset),
                add3(corners[3], offset),
            ],
            face_idx,
            normal: *normal,
            texture: None,
        });
    }
    quads
}

/// A face at `(x, y, z)` is occluded iff the neighbour block exists AND
/// chunkforge-core reports it as opaque.
fn is_occluded<'a>(
    x: i32,
    y: i32,
    z: i32,
    lookup: &impl Fn(i32, i32, i32) -> Option<&'a str>,
) -> bool {
    match lookup(x, y, z) {
        Some(name) => appearance(name).opaque,
        None => false,
    }
}

fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The 6 faces of a unit cube, CW winding when viewed from outside.
/// Index order: 0=+Y(top), 1=-Y(bottom), 2=+X(east), 3=-X(west),
/// 4=+Z(south), 5=-Z(north).
pub const CUBE_FACES: [([i32; 3], [[f64; 3]; 4]); 6] = [
    (
        [0, 1, 0],
        [
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
    ),
    (
        [0, -1, 0],
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
    ),
    (
        [1, 0, 0],
        [
            [1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
        ],
    ),
    (
        [-1, 0, 0],
        [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ],
    ),
    (
        [0, 0, 1],
        [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ],
    ),
    (
        [0, 0, -1],
        [
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ],
    ),
];
