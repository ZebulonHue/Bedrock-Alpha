//! Rebuild `prototypes/` for an export that already exists.
//!
//! A full export re-reads every region and rewrites a multi-hundred-megabyte
//! OBJ; the prototype meshes and their textures are a few hundred kilobytes
//! derived entirely from the block manifest. Separating them makes iterating
//! on prototype geometry and texturing a two-second loop instead of a
//! multi-minute one, and keeps the manifest and the meshes in step because
//! both come from the same file.
//!
//! Usage:
//!     cargo run --release -p bedrock-export --example regen_prototypes -- <world.blocks.json>

use std::collections::BTreeMap;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let Some(manifest_path) = std::env::args().nth(1) else {
        eprintln!("usage: regen_prototypes <world.blocks.json>");
        std::process::exit(2);
    };
    let manifest_path = std::path::PathBuf::from(manifest_path);

    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("could not read {}: {err}", manifest_path.display());
            std::process::exit(1);
        }
    };
    let manifest: Manifest = match serde_json::from_str(&text) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("could not parse {}: {err}", manifest_path.display());
            std::process::exit(1);
        }
    };

    // Same rule the exporter uses: one prototype per distinct block *state*,
    // keyed by the stem the importer will look for. A fence joined north-south
    // and one joined east-west are different meshes.
    let mut representative: BTreeMap<String, (String, BTreeMap<String, String>)> = BTreeMap::new();
    for (name, variants) in &manifest.blocks {
        for variant in variants {
            let as_map: std::collections::HashMap<String, String> = variant
                .properties
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let stem = bedrock_parser::block_shape::prototype_stem(name, &as_map);
            representative
                .entry(stem)
                .or_insert_with(|| (name.clone(), variant.properties.clone()));
        }
    }

    // `write_block_prototypes` derives `prototypes/` from the OBJ's directory,
    // so hand it the OBJ path the manifest sits beside.
    let obj_path = manifest_path.with_extension("").with_extension("obj");
    let stats = bedrock_export::prototypes::write_block_prototypes(&obj_path, &representative);
    println!(
        "{} prototypes, {} textures, {} skipped -> {}",
        stats.written,
        stats.textures,
        stats.skipped.len(),
        obj_path.with_file_name("prototypes").display(),
    );
    if !stats.skipped.is_empty() {
        println!("skipped: {}", stats.skipped.join(", "));
    }
}

#[derive(serde::Deserialize)]
struct Manifest {
    blocks: BTreeMap<String, Vec<Variant>>,
}

#[derive(serde::Deserialize)]
struct Variant {
    properties: BTreeMap<String, String>,
    positions: Vec<[f32; 3]>,
}
