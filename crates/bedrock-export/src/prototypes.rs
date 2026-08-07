//! Per-block-type prototype meshes, for importers that place one real mesh
//! per block instead of drawing a textured cube from a shared atlas.
//!
//! A shared atlas has to be right about two independent things at once: which
//! tile a block maps to, and where that tile's edges are. Both have been a
//! steady source of bugs — a swatch table generated from a different Mineways
//! build silently pointed blocks at unrelated pictures, and UVs sampled on a
//! tile boundary bled the neighbouring tile in as a thin line along every
//! block edge.
//!
//! A prototype sidesteps the whole class: one small OBJ per block type holding
//! that block's true geometry at the origin, textured from named PNGs pulled
//! straight out of the player's own client JAR. Nothing is shared, so nothing
//! can bleed, and a texture is found by name rather than by position in a
//! table that can drift.
//!
//! The importer instances these at the positions in `blocks.json`, which is
//! the same mechanism that already places MCprep assets.

use bedrock_parser::block_shape;
use bedrock_parser::jar_textures::JarTextureLoader;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufWriter, Write};
use std::path::Path;

/// What [`write_block_prototypes`] produced.
#[derive(Debug, Default)]
pub struct PrototypeStats {
    /// Block types that got a prototype mesh.
    pub written: usize,
    /// Distinct texture PNGs extracted from the JAR.
    pub textures: usize,
    /// Block types whose geometry could not be built.
    pub skipped: Vec<String>,
}

/// Write `<export>.prototypes/<block>.obj` + `.mtl` and
/// `<export>.prototypes/textures/*.png` for every block id in `blocks`.
///
/// Geometry comes from the same per-block model data the main exporter uses,
/// evaluated at the origin with nothing adjacent, so every face is present —
/// an instanced prototype is placed wherever the block occurs and cannot know
/// in advance which of its faces will be hidden.
/// Write one prototype per distinct *block state*, not per block type.
///
/// `blocks` maps a prototype file stem to the state it should be built from;
/// build it with [`bedrock_parser::block_shape::prototype_stem`] so the names
/// match what the importer looks for.
pub fn write_block_prototypes(
    obj_path: &Path,
    blocks: &BTreeMap<String, (String, BTreeMap<String, String>)>,
) -> PrototypeStats {
    let mut stats = PrototypeStats::default();

    // Named after the OBJ, exactly like the `blocks.json` beside it, and for
    // the same reason: one shared `prototypes/` folder holds whichever export
    // wrote last, so importing an older export finds only the stems the newer
    // one happened to need. Every block state missing from that set silently
    // falls back to the OBJ's atlas cube -- which is how a bed, a bell and a
    // conduit end up as featureless boxes while a bed of another colour, from
    // the export that did run last, comes through correctly.
    let dir = obj_path.with_extension("prototypes");
    if dir.parent().is_none() {
        return stats;
    }
    let texture_dir = dir.join("textures");
    if std::fs::create_dir_all(&texture_dir).is_err() {
        tracing::warn!("could not create {}", texture_dir.display());
        return stats;
    }

    // Clear what a previous run left here. Writing only the files this export
    // needs leaves the rest untouched and years out of date: a re-export after
    // a texture change kept `stonecutter_saw` frozen on frame one, because the
    // run that wrote it predated animation support and nothing overwrote it.
    // Only the generated file types go, so anything a user parked in the
    // folder survives.
    clear_generated(&dir, &["obj", "mtl"]);
    clear_generated(&texture_dir, &["png", "json"]);

    let loader = match JarTextureLoader::load() {
        Ok(loader) => loader,
        Err(err) => {
            tracing::warn!("no client JAR available for prototype textures: {err}");
            return stats;
        }
    };

    let mut extracted: BTreeSet<String> = BTreeSet::new();
    let mut animations: BTreeMap<String, bedrock_parser::texture_animation::TextureAnimation> =
        BTreeMap::new();

    for (stem, (block, properties)) in blocks {
        // No neighbours: every face of the prototype must be drawn.
        let nothing_adjacent = |_: i32, _: i32, _: i32| -> Option<&str> { None };
        let props: std::collections::HashMap<String, String> = properties
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let quads = block_shape::block_quads_stated(
            0,
            0,
            0,
            &format!("minecraft:{block}"),
            &props,
            &nothing_adjacent,
        );
        if quads.is_empty() {
            stats.skipped.push(stem.clone());
            continue;
        }

        // Several vanilla models stack coincident elements: `grass_block` is
        // literally two 0..16 cubes, the second carrying only the tinted side
        // overlay. The game draws those in a defined order; a mesh renderer
        // has no such rule and two faces at identical coordinates z-fight into
        // hard stripes. Push each repeat of a face a hair further out along
        // its own normal so the overlay always wins, by less than a millimetre
        // at Minecraft scale.
        let quads = separate_coincident_faces(quads);

        // Group faces by the texture they use — that becomes one material each.
        let mut by_texture: BTreeMap<String, Vec<&block_shape::BlockQuad>> = BTreeMap::new();
        for quad in &quads {
            let texture = quad.texture.clone().unwrap_or_else(|| block.clone());
            by_texture.entry(texture).or_default().push(quad);
        }

        let obj_file = dir.join(format!("{stem}.obj"));
        let mtl_file = dir.join(format!("{stem}.mtl"));
        let Ok(obj) = std::fs::File::create(&obj_file) else {
            stats.skipped.push(stem.clone());
            continue;
        };
        let mut obj = BufWriter::new(obj);
        let _ = writeln!(obj, "# Project Bedrock prototype: {stem}");
        let _ = writeln!(obj, "mtllib {stem}.mtl");

        // Centre the prototype on its own origin so an instance lands at the
        // block's centre rather than its corner.
        let mut vertex_base = 1usize;
        let mut wrote_any = false;
        let mut materials: Vec<(String, bool)> = Vec::new();

        for (texture, group) in &by_texture {
            let cutout = extract_texture(
                &loader,
                texture,
                &texture_dir,
                &mut extracted,
                &mut animations,
            );
            materials.push((texture.clone(), cutout));

            let _ = writeln!(obj, "o {stem}_{texture}");
            let _ = writeln!(obj, "usemtl {texture}");
            for quad in group {
                for corner in &quad.corners {
                    let _ = writeln!(
                        obj,
                        "v {:.4} {:.4} {:.4}",
                        corner[0] - 0.5,
                        corner[1] - 0.5,
                        corner[2] - 0.5
                    );
                }
                // The prototype's texture is the block's own PNG, so each face
                // spans the whole image rather than a slice of an atlas.
                //
                // An animated one is a strip of frames, though, and a face
                // that spans the whole image would show all of them stacked
                // at once. Narrow it to the first frame here, in the mesh, so
                // the OBJ is right on its own -- opened in anything, with the
                // add-on's animation pass disabled, or if that pass fails. The
                // animation then only slides this window down the strip.
                let uvs = block_shape::face_uv_corners(quad);
                let frames = animations.get(texture).map_or(1, |a| a.frame_count);
                for uv in &uvs {
                    let v = 1.0 - uv[1];
                    let v = (v + f32::from(u16::try_from(frames - 1).unwrap_or(0)))
                        / frames as f32;
                    let _ = writeln!(obj, "vt {:.4} {v:.4}", uv[0]);
                }
                let _ = writeln!(
                    obj,
                    "f {}/{} {}/{} {}/{} {}/{}",
                    vertex_base,
                    vertex_base,
                    vertex_base + 1,
                    vertex_base + 1,
                    vertex_base + 2,
                    vertex_base + 2,
                    vertex_base + 3,
                    vertex_base + 3
                );
                vertex_base += 4;
                wrote_any = true;
            }
        }
        let _ = obj.flush();

        if !wrote_any {
            let _ = std::fs::remove_file(&obj_file);
            stats.skipped.push(stem.clone());
            continue;
        }

        if let Ok(mtl) = std::fs::File::create(&mtl_file) {
            let mut mtl = BufWriter::new(mtl);
            for (texture, cutout) in &materials {
                let _ = writeln!(mtl, "\nnewmtl {texture}");
                let _ = writeln!(mtl, "Ka 0.0000 0.0000 0.0000");
                let _ = writeln!(mtl, "Kd 1.0000 1.0000 1.0000");
                let _ = writeln!(mtl, "Ks 0.0000 0.0000 0.0000");
                let _ = writeln!(mtl, "Ns 0");
                // Fluids and glass are see-through as a whole surface rather
                // than per-pixel, which no amount of cutout alpha expresses:
                // without a dissolve they read as solid slabs, and an ocean
                // covering most of a world hides everything under it.
                let dissolve = translucency(texture);
                let _ = writeln!(
                    mtl,
                    "illum {}",
                    if *cutout || dissolve.is_some() { 4 } else { 2 }
                );
                let _ = writeln!(mtl, "map_Kd textures/{texture}.png");
                if let Some(d) = dissolve {
                    let _ = writeln!(mtl, "d {d:.3}");
                    let _ = writeln!(mtl, "Tr {:.3}", 1.0 - d);
                } else if *cutout {
                    let _ = writeln!(mtl, "map_d textures/{texture}.png");
                }
            }
            let _ = mtl.flush();
        }
        stats.written += 1;
    }

    stats.textures = extracted.len();

    // Every animated texture the JAR ships, not just the ones these prototypes
    // reference. The library's assets carry their own copies of the vanilla
    // textures -- the torch flame, fire, lava -- and they are named after the
    // same ones, so a full table is what lets the addon animate those too.
    for name in loader.meta_names() {
        if animations.contains_key(name) {
            continue;
        }
        let Some(png) = loader.get(name) else { continue };
        if let Some(anim) = read_animation(&loader, name, png) {
            animations.insert(name.to_owned(), anim);
        }
    }

    // One sidecar for the whole export rather than one per texture: the addon
    // reads it once and looks every image up, including the library's own
    // assets, which are named after the same vanilla textures.
    if let Ok(json) = serde_json::to_string_pretty(&animations) {
        let _ = std::fs::write(texture_dir.join("animations.json"), json);
    }
    tracing::info!("{} animated prototype textures", animations.len());

    stats
}

/// Offset repeats of the same face outward so coincident geometry can't z-fight.
///
/// Only exact duplicates are moved, and only the second and later ones, so a
/// model with no stacked elements comes through byte-identical.
fn separate_coincident_faces(quads: Vec<block_shape::BlockQuad>) -> Vec<block_shape::BlockQuad> {
    /// One tenth of a millimetre at Minecraft scale: below any texel, well
    /// above the depth precision a renderer needs to order two faces.
    const NUDGE: f64 = 0.001;

    let key = |quad: &block_shape::BlockQuad| -> Vec<i64> {
        quad.corners
            .iter()
            .flatten()
            .map(|v| (v * 4096.0).round() as i64)
            .collect()
    };

    let mut seen: BTreeMap<Vec<i64>, u32> = BTreeMap::new();
    let mut out = Vec::with_capacity(quads.len());
    for mut quad in quads {
        let repeats = seen.entry(key(&quad)).or_insert(0);
        if *repeats > 0 {
            let offset = NUDGE * f64::from(*repeats);
            for corner in &mut quad.corners {
                for (axis, value) in corner.iter_mut().enumerate() {
                    *value += offset * f64::from(quad.normal[axis]);
                }
            }
        }
        *repeats += 1;
        out.push(quad);
    }
    out
}

/// Whole-surface opacity for materials the game draws translucent, as an MTL
/// `d` (dissolve) value, or `None` for the fully opaque majority.
fn translucency(texture: &str) -> Option<f32> {
    match texture {
        "water" | "water_still" | "water_flow" | "water_overlay" => Some(0.55),
        "ice" | "frosted_ice" => Some(0.75),
        "glass" | "tinted_glass" => Some(0.35),
        _ => None,
    }
}

/// Copy one texture out of the JAR, returning whether it has real cutout alpha.
///
/// The JAR stores biome-coloured textures — grass tops, most leaves, vines,
/// stems — as grayscale masks that the game multiplies by a colormap value at
/// draw time. Writing those out untouched is what makes canopies and grass
/// render as white or grey patches, so the same tint the atlas builder bakes
/// in is applied here.
fn extract_texture(
    loader: &JarTextureLoader,
    name: &str,
    dir: &Path,
    seen: &mut BTreeSet<String>,
    animations: &mut BTreeMap<String, bedrock_parser::texture_animation::TextureAnimation>,
) -> bool {
    // Track the name the texture was actually found under. A block whose model
    // has no explicit texture asks for its own id -- `water` -- and resolves
    // through the prefix fallback to `water_still`. Tint has to be looked up
    // against the resolved name: `water` is in no tint list, so keying on the
    // request left the ocean as raw grayscale.
    let found = loader
        .get(name)
        .map(|bytes| (name.to_owned(), bytes.to_vec()))
        .or_else(|| {
            loader
                .find_prefixed(&format!("{name}_"))
                .map(|(resolved, bytes)| (resolved.to_owned(), bytes.to_vec()))
        });
    let Some((resolved, png)) = found else {
        return false;
    };

    // Animated textures ship as a vertical strip of square frames, and the
    // whole strip goes out: a face's 0..1 V range then covers every frame at
    // once, which the addon narrows to one row and keys over time. Cropping to
    // frame 1 here is what used to leave fire, magma and portals frozen.
    if let Some(anim) = read_animation(loader, &resolved, &png) {
        animations.insert(name.to_owned(), anim);
    }

    if seen.insert(name.to_owned()) {
        let tint = bedrock_parser::texture::biome_tint(&resolved)
            .or_else(|| bedrock_parser::texture::biome_tint(name));
        let tinted = tint.and_then(|tint| tint_png(&png, tint));
        let _ = std::fs::write(
            dir.join(format!("{name}.png")),
            tinted.as_deref().unwrap_or(&png),
        );
    }
    has_alpha(&png)
}

/// Delete this exporter's own output from a directory, leaving anything else.
///
/// Matching on extension rather than emptying the folder means a stray note or
/// a hand-made override someone dropped in is not destroyed by a re-export.
fn clear_generated(dir: &Path, extensions: &[&str]) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let matches = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| extensions.iter().any(|want| e.eq_ignore_ascii_case(want)));
        if matches && path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Read a texture's animation, if it has one.
///
/// Both halves are needed: the sidecar names the order and timing, the PNG
/// says how many rows the strip actually has. A sidecar without a matching
/// strip (or the other way round) is not an animation.
fn read_animation(
    loader: &JarTextureLoader,
    resolved: &str,
    png: &[u8],
) -> Option<bedrock_parser::texture_animation::TextureAnimation> {
    let mcmeta = loader.meta(resolved)?;
    let image = image::load_from_memory(png).ok()?;
    let frames =
        bedrock_parser::texture_animation::strip_frame_count(image.width(), image.height())?;
    bedrock_parser::texture_animation::parse_mcmeta(mcmeta, frames)
}

/// Multiply a PNG's RGB by `tint`, leaving alpha alone, and re-encode it.
///
/// Alpha has to survive untouched: leaves and the grass side overlay are
/// cutout textures whose transparency the MTL references through `map_d`.
fn tint_png(png: &[u8], tint: [u8; 3]) -> Option<Vec<u8>> {
    let mut rgba = image::load_from_memory(png).ok()?.to_rgba8();
    for pixel in rgba.pixels_mut() {
        for channel in 0..3 {
            pixel.0[channel] =
                (u16::from(pixel.0[channel]) * u16::from(tint[channel]) / 255) as u8;
        }
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

/// True when a PNG is meaningfully see-through, so its material needs `map_d`.
fn has_alpha(png: &[u8]) -> bool {
    let Ok(image) = image::load_from_memory(png) else {
        return false;
    };
    let rgba = image.to_rgba8();
    let (mut clear, mut total) = (0usize, 0usize);
    for pixel in rgba.pixels() {
        total += 1;
        if pixel.0[3] < 128 {
            clear += 1;
        }
    }
    total > 0 && clear * 10 > total
}
