# chunkforge-core

A dependency-light Rust port of the [ChunkForge](../app) webapp's Minecraft
save parser. It reads Minecraft Java Edition **Anvil region files** (`.mca`,
1.18+ chunk format), decodes chunks, culls interior blocks, and returns
exterior block positions grouped by block type.

## Quick start

```toml
[dependencies]
chunkforge-core = { path = "chunkforge-core" }
```

```rust
use chunkforge_core::{parse_region_paths, parse_region_bytes};

// Merge several region files into one world:
let world = parse_region_paths(&["r.0.0.mca", "r.0.-1.mca"])?;
println!(
    "{} blocks ({} exterior), bounds {:?}, DataVersion {:?}",
    world.total_blocks, world.exterior_blocks, world.bounds, world.data_version
);
for (name, positions) in &world.blocks_by_type {
    println!("{name}: {} blocks, first at {:?}", positions.len(), positions[0]);
}

// Or parse an in-memory buffer:
let bytes = std::fs::read("r.0.0.mca")?;
let world = parse_region_bytes(&bytes, "r.0.0.mca")?;
# Ok::<(), chunkforge_core::ParseError>(())
```

Block appearance lookup (colors, jar texture paths, occlusion flag), ported
from the app's 350-entry table:

```rust
use chunkforge_core::appearance;
let a = appearance("minecraft:stone");
assert!(a.opaque);          // full cube -> occludes neighbors
assert!(!appearance("minecraft:torch").opaque); // torches never occlude
```

## Feature notes

* **1.18+ chunk format** (`sections[]` + `block_states` palette/bit-packed
  long array). Pre-1.18 chunks (`Level.Sections`) are detected, counted and
  skipped; an all-legacy input fails with `Pre118Format` and the exact TS
  message.
* **Opacity-driven exterior culling** — a block is kept iff at least one of
  its 6 neighbors is absent or non-opaque; edges of the parsed region count
  as absent. Only full opaque cubes occlude (torches/plants/glass/slabs…
  never do).
* **Corruption tolerant** — a bad chunk is skipped and counted
  (`corrupt_chunks`), never fatal. LZ4-compressed chunks (type 4) are
  skipped and counted (`skipped_lz4_chunks`).
* **Section-arena storage** — one `Vec<u16>` arena of 4096-cell sections
  keyed numerically; the design that cut the TS parser's peak memory ~3.5×
  (1639 MB → ~470 MB on a 23.4M-block region).
* **Dependencies**: `flate2` (rust_backend) + `byteorder`. No `unsafe`.
* Progress callbacks are not built in — run the parse on your own worker
  thread and report per-file completion (see the crate docs).

## Docs

* **[PARSING.md](PARSING.md)** — the full format walkthrough: region layout,
  NBT tags, bit-packing rules with a worked example, culling, the
  section-arena design, DataVersion table, and TS→Rust porting gotchas.
* API docs: `cargo doc --open`.

## Tests

```
cargo test
```

Unit tests (NBT roundtrip, bit-unpack edge cases at width 4 and ≥5, header
parsing, culling) plus integration tests against real fixtures with exact
expected numbers (e.g. MC 1.20.6 region: 1,062,650 total / 123,956 exterior
blocks, DataVersion 3839).
