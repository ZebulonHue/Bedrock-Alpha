use crate::block_shape::BlockQuad;
use crate::json_model::ModelElement;
use std::collections::HashMap;

/// Convert Vanilla JSON elements into block quads.
pub fn generate_quads_from_elements<'a>(
    wx: i32,
    wy: i32,
    wz: i32,
    elements_data: &[(ModelElement, i32, i32, HashMap<String, String>)],
    lookup: &impl Fn(i32, i32, i32) -> Option<&'a str>,
) -> Vec<BlockQuad> {
    let mut quads = Vec::new();

    let offset_x = wx as f64;
    let offset_y = wy as f64;
    let offset_z = wz as f64;

    for (el, rot_x, rot_y, textures) in elements_data {
        let rot_x = *rot_x;
        let rot_y = *rot_y;
        let min_x = el.from[0] / 16.0;
        let min_y = el.from[1] / 16.0;
        let min_z = el.from[2] / 16.0;
        let max_x = el.to[0] / 16.0;
        let max_y = el.to[1] / 16.0;
        let max_z = el.to[2] / 16.0;

        // 6 faces: top, bottom, east, west, south, north
        // face mapping:
        let faces_def = [
            (
                "up",
                [0, 1, 0],
                0,
                [
                    [min_x, max_y, min_z],
                    [max_x, max_y, min_z],
                    [max_x, max_y, max_z],
                    [min_x, max_y, max_z],
                ],
            ),
            (
                "down",
                [0, -1, 0],
                1,
                [
                    [min_x, min_y, min_z],
                    [min_x, min_y, max_z],
                    [max_x, min_y, max_z],
                    [max_x, min_y, min_z],
                ],
            ),
            (
                "east",
                [1, 0, 0],
                2,
                [
                    [max_x, min_y, min_z],
                    [max_x, min_y, max_z],
                    [max_x, max_y, max_z],
                    [max_x, max_y, min_z],
                ],
            ),
            (
                "west",
                [-1, 0, 0],
                3,
                [
                    [min_x, min_y, min_z],
                    [min_x, max_y, min_z],
                    [min_x, max_y, max_z],
                    [min_x, min_y, max_z],
                ],
            ),
            (
                "south",
                [0, 0, 1],
                4,
                [
                    [min_x, min_y, max_z],
                    [min_x, max_y, max_z],
                    [max_x, max_y, max_z],
                    [max_x, min_y, max_z],
                ],
            ),
            (
                "north",
                [0, 0, -1],
                5,
                [
                    [min_x, min_y, min_z],
                    [max_x, min_y, min_z],
                    [max_x, max_y, min_z],
                    [min_x, max_y, min_z],
                ],
            ),
        ];

        for (face_name, normal, face_idx, mut corners) in faces_def {
            if let Some(model_face) = el.faces.get(face_name) {
                // Determine if this face is culled by a neighboring block
                if let Some(cullface) = &model_face.cullface {
                    let mut cull_normal = normal;
                    match cullface.as_str() {
                        "up" => cull_normal = [0, 1, 0],
                        "down" => cull_normal = [0, -1, 0],
                        "east" => cull_normal = [1, 0, 0],
                        "west" => cull_normal = [-1, 0, 0],
                        "south" => cull_normal = [0, 0, 1],
                        "north" => cull_normal = [0, 0, -1],
                        _ => {}
                    }
                    let nx = wx + cull_normal[0];
                    let ny = wy + cull_normal[1];
                    let nz = wz + cull_normal[2];

                    if let Some(neighbor) = lookup(nx, ny, nz) {
                        if chunkforge_core::appearance(neighbor).opaque {
                            continue;
                        }
                    }
                }

                // Apply element rotation (e.g. 45 degrees around Y)
                if let Some(el_rot) = &el.rotation {
                    let origin_x = el_rot.origin[0] / 16.0;
                    let origin_y = el_rot.origin[1] / 16.0;
                    let origin_z = el_rot.origin[2] / 16.0;
                    let angle_rad = el_rot.angle.to_radians();
                    let (sin_a, cos_a) = angle_rad.sin_cos();

                    for c in &mut corners {
                        let mut cx = c[0] - origin_x;
                        let mut cy = c[1] - origin_y;
                        let mut cz = c[2] - origin_z;

                        match el_rot.axis.as_str() {
                            "y" => {
                                let nx = cx * cos_a + cz * sin_a;
                                let nz = -cx * sin_a + cz * cos_a;
                                cx = nx;
                                cz = nz;
                            }
                            "x" => {
                                let ny = cy * cos_a - cz * sin_a;
                                let nz = cy * sin_a + cz * cos_a;
                                cy = ny;
                                cz = nz;
                            }
                            "z" => {
                                let nx = cx * cos_a - cy * sin_a;
                                let ny = cx * sin_a + cy * cos_a;
                                cx = nx;
                                cy = ny;
                            }
                            _ => {}
                        }

                        c[0] = cx + origin_x;
                        c[1] = cy + origin_y;
                        c[2] = cz + origin_z;
                    }
                }

                // Apply variant rotation (rot_x, rot_y) in 90 degree increments
                // Variant rotation originates from the center (0.5, 0.5, 0.5)
                let mut rotated_normal = normal;
                if rot_x != 0 || rot_y != 0 {
                    for c in &mut corners {
                        let mut cx = c[0] - 0.5;
                        let mut cy = c[1] - 0.5;
                        let mut cz = c[2] - 0.5;

                        if rot_x != 0 {
                            let angle = (rot_x as f64).to_radians();
                            let (sin_a, cos_a) = angle.sin_cos();
                            let ny = cy * cos_a - cz * sin_a;
                            let nz = cy * sin_a + cz * cos_a;
                            cy = ny;
                            cz = nz;
                        }

                        if rot_y != 0 {
                            let angle = (rot_y as f64).to_radians();
                            let (sin_a, cos_a) = angle.sin_cos();
                            let nx = cx * cos_a + cz * sin_a;
                            let nz = -cx * sin_a + cz * cos_a;
                            cx = nx;
                            cz = nz;
                        }

                        c[0] = cx + 0.5;
                        c[1] = cy + 0.5;
                        c[2] = cz + 0.5;
                    }

                    let mut nx = rotated_normal[0] as f64;
                    let mut ny = rotated_normal[1] as f64;
                    let mut nz = rotated_normal[2] as f64;
                    if rot_x != 0 {
                        let angle = (rot_x as f64).to_radians();
                        let (sin_a, cos_a) = angle.sin_cos();
                        let new_y = ny * cos_a - nz * sin_a;
                        let new_z = ny * sin_a + nz * cos_a;
                        ny = new_y;
                        nz = new_z;
                    }
                    if rot_y != 0 {
                        let angle = (rot_y as f64).to_radians();
                        let (sin_a, cos_a) = angle.sin_cos();
                        let new_x = nx * cos_a + nz * sin_a;
                        let new_z = -nx * sin_a + nz * cos_a;
                        nx = new_x;
                        nz = new_z;
                    }
                    rotated_normal = [nx.round() as i32, ny.round() as i32, nz.round() as i32];
                }

                // Resolve texture variable
                let mut tex = model_face.texture.clone();
                if tex.starts_with('#') {
                    if let Some(t) = textures.get(&tex[1..]) {
                        tex = t.clone();
                    }
                }

                // Add translation offset
                for c in &mut corners {
                    c[0] += offset_x;
                    c[1] += offset_y;
                    c[2] += offset_z;
                }

                quads.push(BlockQuad {
                    corners,
                    face_idx,
                    normal: rotated_normal,
                    texture: Some(tex),
                });
            }
        }
    }

    quads
}
