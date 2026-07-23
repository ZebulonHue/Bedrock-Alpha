# PARSING.md — how chunkforge-core reads a Minecraft save

This is the end-to-end logic doc for the crate: what bytes we read, in what
order, and why. It is written for the crate user who wants to *understand*,
not just call. Everything here matches the TypeScript parser in the
ChunkForge webapp (`src/lib/mc/`), which is the source of truth this crate
ports.

Pipeline overview:

```
.mca bytes
  → region header scan (locate chunk records, count corrupt ones)
  → per chunk: inflate (gzip/zlib/none; LZ4 = skip+count)
  → NBT parse (big-endian, all 12 tags, bounds-checked)
  → section decode (palette + bit-packed long array) into the section arena
  → opacity-driven exterior culling (two-phase)
  → ParsedWorld { blocks_by_type, totals, bounds, counters }
```

---

## 1. Region file layout (Anvil)

A region file stores up to 32×32 chunks in a fixed grid. All integers in the
header are **big-endian**.

```
offset  size    contents
------  ------  ---------------------------------------------------
0       4096    1024-entry location table, 4 bytes per entry:
                 [ sector offset (3 bytes BE) | sector count (1 byte) ]
4096    4096    1024-entry timestamp table (4 bytes BE each; read but unused)
8192    ...     chunk records, one per present chunk, sector-aligned
```

* Entry index `i = x + z*32` gives the chunk's position `(x, z)` *within the
  region* (0..31, 0..31). A region file named `r.<rx>.<rz>.mca` covers chunk
  columns `rx*32 .. rx*32+31`, `rz*32 .. rz*32+31` — though in practice we
  take each chunk's world coords from its NBT (`xPos`/`zPos`), falling back
  to the in-region index only when those tags are missing (kept TS behavior).
* 1 sector = 4096 bytes. Entry `(offset=0, count=0)` means "no chunk here".
* Each **chunk record**:

```
offset  size    contents
------  ------  ---------------------------------------------------
+0      4       length L (BE) — INCLUDES the compression byte
+4      1       compression type: 1=gzip, 2=zlib, 3=uncompressed, 4=LZ4
+5      L-1     (compressed) NBT payload
```

Header-scan rules (each is deliberate — a corrupt chunk must never stall the
file):

* `sector_offset == 0 || sector_count == 0` → no chunk, skip.
* record start + 5 bytes past end of file → count **corrupt**, skip.
* `length < 1` → skip **without** counting corrupt (kept TS quirk).
* record overruns end of file → count **corrupt**, skip.

## 2. Compression

Types 1 (gzip) and 2 (zlib) are inflated with `flate2` (miniz_oxide backend);
type 3 is used as-is. **LZ4 (type 4) is not supported** — seen in some
1.20.5+ worlds. Such chunks are *skipped and counted*
(`skipped_lz4_chunks`), never fatal; the TS app has the same limitation
(fflate cannot inflate LZ4). If LZ4 chunks leave you with zero decodable
chunks, the parse fails with `NoChunksDecoded`.

## 3. NBT (Named Binary Tag)

Chunk payloads are NBT documents: one named root **compound**. All numbers
are **big-endian**. The 12 tag types:

| id | name        | payload                                              |
|---:|:------------|:-----------------------------------------------------|
|  0 | End         | (none — terminates a compound)                        |
|  1 | Byte        | i8                                                    |
|  2 | Short       | i16 BE                                                |
|  3 | Int         | i32 BE                                                |
|  4 | Long        | i64 BE                                                |
|  5 | Float       | f32 BE                                                |
|  6 | Double      | f64 BE                                                |
|  7 | ByteArray   | i32 length + bytes                                    |
|  8 | String      | u16 length + UTF-8 (decoded lossy, like TextDecoder)  |
|  9 | List        | u8 subtype + i32 count + count payloads of subtype    |
| 10 | Compound    | repeated { u8 tag, String name, payload } until End   |
| 11 | IntArray    | i32 length + i32s                                     |
| 12 | LongArray   | i32 length + i64s                                     |

Robustness rules (the TS reader gets these "for free" via `RangeError`; Rust
needs them explicit):

* **Bounds-check every read** — any overrun is `CorruptNbt`.
* Negative array/list lengths are rejected.
* Length prefixes are validated against the remaining bytes *before*
  allocating (each element costs ≥1 byte), so a hostile length can never
  trigger a giant allocation.
* Nesting depth is capped (512) so a hostile file cannot overflow the stack.
  Vanilla chunk NBT is ~10 levels deep.
* An empty list (`count == 0`) never validates its subtype — TS behavior.

## 4. Chunk format versions, and how we detect them

We support the **1.18+ layout**:

```
root compound
├── DataVersion : Int            (e.g. 3465 = 1.20.1, 3839 = 1.20.6)
├── xPos, zPos  : Int            (chunk world coords)
├── yPos        : Int
└── sections    : List<Compound>
     ├── Y            : Byte     (section index; world y = Y*16 + local y)
     └── block_states : Compound
          ├── palette : List<Compound { Name: String, Properties?: Compound }>
          └── data    : LongArray (bit-packed palette indices; optional)
```

Pre-1.18 chunks instead carry `Level.Sections` (capital S, nested one level
down) and have no root `sections` list. Detection: if the root has no
`sections` list, the chunk is **legacy** — counted (`legacy_chunks`) and
skipped, never fatal... unless EVERY chunk in the parse is legacy, which
fails with the exact TS error:

```
This save uses the pre-1.18 chunk format, which is not supported yet.
```

DataVersion cheat sheet (Java Edition):

| DataVersion | version | DataVersion | version |
|---:|:--------|---:|:--------|
| 2860 | 1.18   | 3463 | 1.20    |
| 2975 | 1.18.2 | 3465 | 1.20.1  |
| 3105 | 1.19   | 3578 | 1.20.2  |
| 3120 | 1.19.1 | 3700 | 1.20.4  |
| 3218 | 1.19.3 | 3837 | 1.20.5  |
| 3337 | 1.19.4 | 3839 | 1.20.6  |
|      |        | 3953 | 1.21    |
|      |        | 3955 | 1.21.1  |

The world reports the **max** DataVersion seen.

## 5. Sections, palettes and bit-packing

A section is a 16×16×16 block cube (4096 cells). Cell order is YZX:
`index = (y << 8) | (z << 4) | x`, world `y = section.Y * 16 + localY`.

Each section has a **palette** (distinct block states in the section) and a
**data** long array holding one palette index per cell, bit-packed.

* Bits per index: `bits = max(4, ceil(log2(palette.len())))`.
* Indices are packed **LSB-first** into 64-bit longs.
* **The no-long-crossing rule**: an index NEVER straddles two longs. Each
  long holds exactly `floor(64/bits)` indices; any leftover high bits are
  unused padding.

### Worked example: palette of 13 → 4 bits → 16 per long

`ceil(log2(13)) = 4`, so `bits = max(4, 4) = 4` and each long carries
`floor(64/4) = 16` cells. A full section needs `4096 / 16 = 256` longs.
Cell 0 sits in bits 0..3 of long 0, cell 1 in bits 4..7, …, cell 15 in bits
60..63 of long 0, cell 16 starts fresh at bits 0..3 of long 1:

```
long 0:  [idx15 idx14 ... idx1 idx0]   <- 4 bits each, cell 0 in the LOW bits
long 1:  [idx31 idx30 ... idx17 idx16]
...
long 255:[idx4095   ...      idx4080]
```

Extraction in Rust is a plain `u64` shift+mask — no BigInt gymnastics needed
(this is one place the port is *simpler* than the TS, which juggles 32-bit
halves to avoid BigInt in the hot loop):

```rust
let idx = ((word >> (k * bits)) & ((1u64 << bits) - 1)) as usize;
```

The no-long-crossing rule is exactly what makes this safe: `k*bits + bits ≤ 64`
always holds, so the shift never overflows and no index is ever split across
`data[li]`/`data[li+1]`. (Our unit tests set the padding bits of every long
to garbage to prove they are never read.)

Edge cases, all matching the TS parser:

* **No `data` array** → the whole section is `palette[0]` (homogeneous
  section). If that's air, the section is skipped entirely.
* **Truncated `data`** (fewer longs than 4096 indices need) → the missing
  tail is treated as air. Don't invent blocks.
* **Palette index out of range** (corrupt file) → that cell is air.
* Palette entries that are air (`minecraft:air`, `cave_air`, `void_air`,
  with or without namespace) or malformed (no `Name`) are dropped; if a
  section has no non-air palette entries it is skipped.

## 6. Exterior culling

We don't want every block — only the **surface** of the world. A block is
kept iff at least one of its 6 face neighbors (±x, ±y, ±z) is **absent or
non-opaque**:

* Air cells and missing sections are absent.
* Opacity comes from `appearance(name).opaque`, which is `true` ONLY for
  full opaque cubes (stone, dirt, ores, planks, wool…). Glass, ice, leaves,
  slabs, stairs, fences, rails, crops, torches… are all non-occluding.
  Unknown blocks default to opaque unless a substring hint
  (`slab|stair|fence|glass|leaves|torch|…`) says otherwise; the 350-entry
  table always wins over the hints (so `snow_block` and `packed_ice` stay
  opaque even though they contain "snow"/"ice").
* **Region/file edges count as absent** — a block on the boundary of what
  was loaded is always kept. We can't see past the edge, so we assume the
  face is visible rather than risk dropping a real surface block.

### The torch tale (why opacity must be strict)

An early version of the TS table accidentally had `torch` marked opaque. A
torch sitting on a stone floor then "blocked" the top face of the stone
beneath it — and since every other face of that stone was covered by other
opaque blocks, the stone was culled as interior. Torch-lit floors and cave
walls silently lost blocks wherever a torch was attached. The fix — and the
standing rule — is: **only full opaque cubes occlude**. Torches, fire,
plants, glass, leaves, slabs, stairs, fences, rails never do. When in doubt,
mark `opaque: false`: a wrongly non-opaque block merely keeps a few extra
blocks; a wrongly opaque one *deletes visible geometry*.

### Two-phase cull (memory discipline)

* **Pass 1** — scan the section arena; every exterior cell is packed as
  `(local_index << 16) | (name_id + 1)` into one `Vec<u32>` (~4 bytes per
  exterior block instead of ~80 for an `[x, y, z]` tuple). Per-name exact
  counts are tallied alongside.
* The big section arena is then **dropped** — *before* the output tuples are
  allocated. Building tuples while the arena is still live is what used to
  blow peak RSS on dense regions.
* **Pass 2** — each type's `Vec<[i32; 3]>` is allocated at its exact
  pre-counted size and filled by unpacking the `u32`s (section key → world
  base coords + local index → offsets).

## 7. The section-arena storage design

The naive design — one hash map entry per block — costs ~50+ bytes per block
and millions of tiny allocations. Instead, blocks live in **one growable
`Vec<u16>` arena of 4096-cell sections**:

* `HashMap<section_key, slot>` maps a numeric section key to a 4096-cell
  slot; a cell holds `name_id + 1` (`u16`), so 0 means air/absent.
* Air-only sections are never allocated. Block names are interned once per
  palette into small integer ids.
* Section keys are pure arithmetic:
  `key = ((chunkX + 2^21) * 2^22 + (chunkZ + 2^21)) * 64 + (sectionY + 32)`.
  Chunk coords are bounded by the ±30M world border (±1.875M chunks < 2^21),
  so keys stay below 2^50 and neighbor sections are key ±1 (y), ±64 (z),
  ±2^28 (x) — the culler's neighbor lookups are index math, not hashing.
* One big backing store (instead of thousands of 8 KB arrays) avoids
  per-array overhead and lets the OS reclaim the whole arena promptly after
  culling.

This is the design that cut peak memory **from 1639 MB to ~470 MB on a
23.4M-block region** (~3.5×) in the TS parser — we keep it deliberately.

Because cells are `u16`s, the name table is bounded at **65,534** distinct
names (vanilla + any modpack stays in the low thousands; the guard only
trips on a deliberately hostile file, which then fails the parse with an
error instead of wrapping ids).

## 8. Error model & judgment calls

* `Truncated` — buffer smaller than the 8 KiB header.
* `CorruptNbt(String)` — NBT that fails bounds/type checks. Per chunk this
  is a skip+count, not a failure.
* `Pre118Format` — every chunk (in a file that has chunks) uses the
  pre-1.18 layout. Exact message above. In a multi-file parse, an
  all-legacy sibling file still fails the whole parse (TS behavior).
* `NoChunksDecoded` — chunks were found but none could be decoded (all
  corrupt or LZ4). Applied to single-buffer parses too (the TS only does
  this check for multi-file), and to merged totals for multi-file parses:
  a corrupt-only sibling is **tolerated** when another file decodes.
* `Io` — `parse_region_paths` couldn't read a file.

Deliberate deviations from the TS (documented bugs we did not reproduce):

* `bounds` is `Option`: an empty world (no exterior blocks) reports `None`.
  The TS merges fake `[0,0,0]` bounds for empty parts — a latent bug.
* Output ordering is deterministic: block types appear in first-seen
  (intern) order; multi-file merges keep first-file-first ordering; section
  iteration follows insertion order, like the TS `Map`.

Kept TS quirks:

* `length < 1` chunk records are skipped without counting corrupt.
* Chunk world coords come from NBT `xPos`/`zPos`, falling back to the
  in-region index when absent.
* Duplicate sections (corrupt files) overwrite only their non-air cells.

## 9. Coordinates

Y-up, 1 block = 1 unit. Positions are block **min-corners** — the block at
`[x, y, z]` occupies `[x, x+1) × [y, y+1) × [z, z+1)`, so its center is at
`(x+0.5, y+0.5, z+0.5)`. Bounds in `ParsedWorld` are inclusive min/max over
exterior blocks only.

## 10. Porting gotchas (TS → Rust)

Things that bit us — or would have — in the port:

* **BigInt pitfalls are moot.** The TS splits longs into 32-bit halves only
  to avoid BigInt math in the hot loop. In Rust, `data[li] as u64` and plain
  shift/mask is both simpler and exact. Do mind the `as u64` cast: shifting
  an `i64` by ≥ its width is UB-adjacent (panics in debug, wraps in
  release); we always shift a `u64` by an amount `< 64` (guaranteed by the
  no-long-crossing rule: `per_long * bits ≤ 64`).
* **`bits > 30` corrupt-file path**: TS falls back to exact BigInt
  extraction. In Rust, guard `per_long == 0` (bits ≥ 65 → TS would compute
  `i % 0 = NaN` and stop) and saturate the mask at `bits ≥ 64`.
* **`ceil(log2(n))` without floats**: `(n-1).ilog2() + 1` for `n ≥ 2`;
  float `log2` can round the exact powers wrong on some inputs.
* **JS `%` is signed**: `((h % 360) + 360) % 360` in the color hash is
  exactly `i32::rem_euclid(360)` in Rust.
* **`Math.round` ≠ `as i64`**: JS rounds half up; `f64::round()` rounds half
  away from zero — identical for the positive values the color math
  produces.
* **Bounds-check every NBT read**, and validate length prefixes against the
  remaining bytes *before* `Vec::with_capacity` — a hostile length in TS
  throws a catchable `RangeError`; in Rust it can abort the process.
* **UTF-16 vs UTF-8 in the name hash**: `charCodeAt` iterates UTF-16 code
  units; `str::encode_utf16()` matches it (block names are ASCII in
  practice, but modded names need it).
* **Recursion depth**: TS NBT compounds recurse too, but a stack overflow
  there is a catchable exception; in Rust it's an abort. Cap the depth.
* **The 65,534 name-id bound** comes from `name_id + 1` in `u16` cells —
  exceeding it must error, not wrap.
* **Determinism**: Rust's `HashMap` iteration order is random; keep
  insertion-order side vectors (names list, section order) anywhere output
  ordering matters.
