//! Print each face's UV rectangle for a block, so a model's declared
//! coordinates can be checked against what the exporter actually emits.
use std::collections::HashMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let block = args.next().unwrap_or_else(|| "oak_stairs".into());
    let mut props = HashMap::new();
    for pair in args {
        if let Some((k, v)) = pair.split_once('=') {
            props.insert(k.to_string(), v.to_string());
        }
    }

    let quads = bedrock_parser::block_shape::block_quads_stated(0, 0, 0, &block, &props, &|_, _, _| None);
    println!("{block} {props:?} -> {} quads", quads.len());
    for axis in 0..3 {
        let lo = quads.iter().flat_map(|q| q.corners.iter()).map(|c| c[axis]).fold(f64::INFINITY, f64::min);
        let hi = quads.iter().flat_map(|q| q.corners.iter()).map(|c| c[axis]).fold(f64::NEG_INFINITY, f64::max);
        println!("  extent {} : {lo:.3} .. {hi:.3}  (centre {:.3})", ["x", "y", "z"][axis], (lo + hi) / 2.0);
    }
    for q in &quads {
        let uv = bedrock_parser::block_shape::face_uv_corners(q);
        let umin = uv.iter().map(|c| c[0]).fold(f32::INFINITY, f32::min);
        let umax = uv.iter().map(|c| c[0]).fold(f32::NEG_INFINITY, f32::max);
        let vmin = uv.iter().map(|c| c[1]).fold(f32::INFINITY, f32::min);
        let vmax = uv.iter().map(|c| c[1]).fold(f32::NEG_INFINITY, f32::max);
        let ys: Vec<f64> = q.corners.iter().map(|c| c[1]).collect();
        println!(
            "  face {} n{:?} y {:.2}..{:.2}  tex {:<28} u {:.3}..{:.3}  v {:.3}..{:.3}",
            q.face_idx,
            q.normal,
            ys.iter().cloned().fold(f64::INFINITY, f64::min),
            ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            q.texture.clone().unwrap_or_default(),
            umin, umax, vmin, vmax
        );
    }
}
