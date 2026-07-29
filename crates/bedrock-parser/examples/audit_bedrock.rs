//! How much of a Bedrock world actually decodes.
//!
//! A world can render with holes for two very different reasons: the save
//! genuinely has no chunks there, or the reader could not decode the chunks
//! that are there. This separates the two by counting what the LevelDB holds
//! against what `chunks_near` returns, and reporting the subchunk storage
//! versions found — the decoder only handles v8 and v9, so any other version
//! is a chunk that exists on disk and never reaches the viewport.
//!
//! Run: `cargo run -p bedrock-parser --example audit_bedrock -- "<world folder>"`

use bedrock_leveldb::{BedrockKey, ChunkKey, ChunkRecordTag, Db, OpenOptions, ReadOptions};
use bedrock_parser::bedrock::BedrockWorld;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn main() {
    let Some(arg) = std::env::args().nth(1) else {
        eprintln!("usage: audit_bedrock <world folder>");
        std::process::exit(2);
    };
    let folder = PathBuf::from(arg);
    println!("World: {}", folder.display());

    let db = match Db::open(
        folder.join("db"),
        OpenOptions {
            create_if_missing: false,
            ..Default::default()
        },
    ) {
        Ok(db) => db,
        Err(err) => {
            eprintln!("cannot open db: {err}");
            std::process::exit(1);
        }
    };

    // Every subchunk record in the save, by chunk column and dimension.
    let mut columns: BTreeMap<String, BTreeSet<(i32, i32)>> = BTreeMap::new();
    let mut subchunk_keys: Vec<(String, (i32, i32), bytes::Bytes)> = Vec::new();
    let keys = db
        .collect_keys_owned(ReadOptions::default())
        .expect("scan keys");
    for key in keys {
        let BedrockKey::Chunk(ck) = BedrockKey::parse(&key) else {
            continue;
        };
        if ck.tag != ChunkRecordTag::SubChunkPrefix {
            continue;
        }
        let dim = format!("{:?}", ck.dimension);
        columns
            .entry(dim.clone())
            .or_default()
            .insert((ck.coordinates.x, ck.coordinates.z));
        subchunk_keys.push((dim, (ck.coordinates.x, ck.coordinates.z), key));
    }

    println!("\nSubchunk records on disk:");
    for (dim, cols) in &columns {
        println!("  {dim:<12}: {} chunk column(s)", cols.len());
    }

    // Storage version byte of each payload: byte 0 of a SubChunkPrefix value.
    let raw: Vec<bytes::Bytes> = subchunk_keys.iter().map(|(_, _, k)| k.clone()).collect();
    let values = db.get_many_owned(raw, ReadOptions::default()).expect("read values");
    let mut versions: BTreeMap<u8, usize> = BTreeMap::new();
    for value in values.into_iter().flatten() {
        if let Some(&v) = value.first() {
            *versions.entry(v).or_default() += 1;
        }
    }
    println!("\nSubchunk storage versions (decoder supports 8 and 9):");
    for (version, count) in &versions {
        let mark = if *version == 8 || *version == 9 {
            "ok"
        } else {
            "UNSUPPORTED -> skipped"
        };
        println!("  v{version:<3} {count:>8} record(s)  {mark}");
    }

    // What the loader actually hands the viewport, over a radius wide enough
    // to cover anything the save contains.
    let overworld_columns = columns.get("Overworld").map(BTreeSet::len).unwrap_or(0);
    let world = BedrockWorld::open(folder).expect("open world");
    match world.chunks_near(0, 0, 30_000) {
        Ok(chunks) => {
            println!(
                "\nDecoded by chunks_near: {} chunk(s) of {overworld_columns} overworld column(s)",
                chunks.len()
            );
            let missing = overworld_columns.saturating_sub(chunks.len());
            if missing > 0 {
                println!("  {missing} column(s) present on disk did not decode");
            }
        }
        Err(err) => println!("\nchunks_near failed: {err}"),
    }
}
