//! Why a block state produces no prototype geometry.
//!
//! `write_block_prototypes` drops any state whose model yields no quads, and
//! the importer then falls back to the OBJ's atlas cube for that block. This
//! reports, for a set of states taken from a real export manifest, how many
//! quads each one builds — so "no prototype" can be traced to the model
//! lookup rather than guessed at.
//!
//! Run: `cargo run -p bedrock-parser --example audit_prototypes`

use bedrock_parser::block_shape::{block_quads_stated, prototype_stem};
use std::collections::HashMap;

fn props(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn main() {
    // States lifted verbatim from a manifest whose prototypes were missing,
    // plus two that were written, as a control.
    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("bell", vec![("attachment", "floor"), ("facing", "north"), ("powered", "false")]),
        ("lectern", vec![("facing", "west"), ("has_book", "false"), ("powered", "false")]),
        ("conduit", vec![("waterlogged", "false")]),
        ("ender_chest", vec![("facing", "south"), ("waterlogged", "false")]),
        ("trapped_chest", vec![("facing", "south"), ("type", "single"), ("waterlogged", "false")]),
        ("chiseled_bookshelf", vec![("facing", "west"), ("slot_0_occupied", "false")]),
        ("oak_shelf", vec![("facing", "west"), ("powered", "false"), ("side_chain", "unconnected")]),
        ("stonecutter", vec![("facing", "west")]),
        ("lever", vec![("face", "floor"), ("facing", "east"), ("powered", "false")]),
        ("sea_pickle", vec![("pickles", "1"), ("waterlogged", "false")]),
        ("campfire", vec![("facing", "east"), ("lit", "true"), ("signal_fire", "false")]),
        ("red_bed", vec![("facing", "west"), ("occupied", "false"), ("part", "head")]),
        ("crimson_fungus", vec![]),
        ("warped_fungus", vec![]),
        ("waxed_lightning_rod", vec![("facing", "up"), ("powered", "false")]),
        ("sulfur_slab", vec![("type", "bottom"), ("waterlogged", "false")]),
        ("acacia_fence", vec![("east", "false"), ("north", "false"), ("south", "true"), ("west", "false")]),
        ("beacon", vec![]),
        ("slime_block", vec![]),
        ("cauldron", vec![]),
        // Controls: these did get prototypes.
        ("torch", vec![]),
        ("ladder", vec![("facing", "north")]),
        ("oak_slab", vec![("type", "bottom")]),
    ];

    let nothing_adjacent = |_: i32, _: i32, _: i32| -> Option<&str> { None };

    println!("{:<24} {:<48} {:>6}", "block", "prototype stem", "quads");
    println!("{}", "-".repeat(82));
    for (block, pairs) in cases {
        let p = props(&pairs);
        let stem = prototype_stem(block, &p);
        let quads = block_quads_stated(0, 0, 0, &format!("minecraft:{block}"), &p, &nothing_adjacent);
        println!(
            "{:<24} {:<48} {:>6}{}",
            block,
            stem,
            quads.len(),
            if quads.is_empty() { "   <- no prototype written" } else { "" }
        );
    }
}
