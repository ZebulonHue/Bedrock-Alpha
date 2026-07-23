//! Creates a synthetic Java Edition world in the standard saves folder so
//! the full app pipeline (detect → open → render → export) can be tested on
//! machines without Minecraft installed.
//!
//! Run with: `cargo run --example create_test_world -p bedrock-parser`
//!
//! The world is an 8×8-chunk area of simple terrain (stone/dirt/grass) with
//! a small stone-brick house, written as one Anvil region file plus a
//! `level.dat` with the player standing next to the house.

use flate2::write::GzEncoder;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

const SECTOR_BYTES: usize = 4096;
const WORLD_NAME: &str = "Bedrock Test World";

#[derive(Serialize)]
struct LevelDat {
    #[serde(rename = "Data")]
    data: DataTag,
}

#[derive(Serialize)]
struct DataTag {
    #[serde(rename = "LevelName")]
    level_name: String,
    #[serde(rename = "LastPlayed")]
    last_played: i64,
    #[serde(rename = "Version")]
    version: VersionTag,
    #[serde(rename = "Player")]
    player: PlayerTag,
}

#[derive(Serialize)]
struct VersionTag {
    #[serde(rename = "Id")]
    id: i32,
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Serialize)]
struct PlayerTag {
    #[serde(rename = "Pos")]
    pos: Vec<f64>,
}

#[derive(Serialize)]
struct ChunkNbt {
    #[serde(rename = "xPos")]
    x: i32,
    #[serde(rename = "zPos")]
    z: i32,
    sections: Vec<SectionNbt>,
}

#[derive(Serialize)]
struct SectionNbt {
    #[serde(rename = "Y")]
    y: i8,
    block_states: BlockStates,
}

#[derive(Serialize)]
struct BlockStates {
    palette: Vec<PaletteEntry>,
    data: fastnbt::LongArray,
}

#[derive(Serialize)]
struct PaletteEntry {
    #[serde(rename = "Name")]
    name: String,
}

fn main() {
    let saves = dirs::config_dir()
        .expect("no config dir")
        .join(".minecraft")
        .join("saves");
    let world_dir = saves.join(WORLD_NAME);
    std::fs::create_dir_all(world_dir.join("region")).expect("create world dir");

    write_level_dat(&world_dir.join("level.dat"));
    write_region(&world_dir.join("region").join("r.0.0.mca"));

    println!("Created test world at {}", world_dir.display());
    println!("Restart Project Bedrock and click '{WORLD_NAME}' in the World Browser.");
}

fn write_level_dat(path: &PathBuf) {
    let level = LevelDat {
        data: DataTag {
            level_name: WORLD_NAME.to_owned(),
            last_played: 1_700_000_000_000,
            version: VersionTag {
                id: 4189,
                name: "1.21.4".to_owned(),
            },
            player: PlayerTag {
                pos: vec![12.5, 63.0, 12.5],
            },
        },
    };
    let nbt = fastnbt::to_bytes(&level).expect("serialize level.dat");
    let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&nbt).expect("gzip level.dat");
    std::fs::write(path, encoder.finish().expect("finish gzip")).expect("write level.dat");
}

/// Terrain generator: returns the block at world (x, y, z).
fn block_at(x: i32, y: i32, z: i32) -> &'static str {
    // House: floor 8..13 × 8..13, walls up to y 66, at world x/z 8..13.
    let in_house = (8..13).contains(&x) && (8..13).contains(&z);
    if in_house && y == 62 {
        return "minecraft:oak_planks";
    }
    if in_house && (63..67).contains(&y) {
        let wall = x == 8 || x == 12 || z == 8 || z == 12;
        if wall {
            // Door gap on the south side, glass windows on the others.
            if z == 8 && x == 10 && y <= 64 {
                return "minecraft:air";
            }
            if y == 64 && ((x == 8 || x == 12) && (9..12).contains(&z) || z == 12 && x == 10) {
                return "minecraft:glass";
            }
            return "minecraft:stone_bricks";
        }
        if y == 66 && x == 10 && z == 10 {
            return "minecraft:glowstone";
        }
        return "minecraft:air";
    }
    if in_house && y > 62 {
        return "minecraft:air";
    }
    // A tree at (4, 4).
    if x == 4 && z == 4 && (63..67).contains(&y) {
        return "minecraft:oak_log";
    }
    if (2..7).contains(&x) && (2..7).contains(&z) && (65..68).contains(&y) {
        return "minecraft:oak_leaves";
    }
    match y {
        48..=59 => "minecraft:stone",
        60..=61 => "minecraft:dirt",
        62 => "minecraft:grass_block",
        _ => "minecraft:air",
    }
}

/// Build the NBT for one chunk (modern 1.18+ layout).
fn chunk_nbt(cx: i32, cz: i32) -> ChunkNbt {
    let mut sections = Vec::new();
    for section_y in [3i8, 4] {
        let mut palette: Vec<String> = Vec::new();
        let mut palette_ids: HashMap<&str, u16> = HashMap::new();
        let mut indices = vec![0u16; 4096];
        for y in 0..16i32 {
            for z in 0..16i32 {
                for x in 0..16i32 {
                    let name = block_at(cx * 16 + x, i32::from(section_y) * 16 + y, cz * 16 + z);
                    let id = *palette_ids.entry(name).or_insert_with(|| {
                        palette.push(name.to_owned());
                        (palette.len() - 1) as u16
                    });
                    indices[(y as usize) << 8 | (z as usize) << 4 | x as usize] = id;
                }
            }
        }
        let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(4) as usize;
        let per_long = 64 / bits;
        let mut data = vec![0i64; 4096_usize.div_ceil(per_long)];
        for (i, &index) in indices.iter().enumerate() {
            data[i / per_long] |= i64::from(index) << ((i % per_long) * bits);
        }
        sections.push(SectionNbt {
            y: section_y,
            block_states: BlockStates {
                palette: palette
                    .into_iter()
                    .map(|name| PaletteEntry { name })
                    .collect(),
                data: fastnbt::LongArray::new(data),
            },
        });
    }
    ChunkNbt {
        x: cx,
        z: cz,
        sections,
    }
}

/// Write an 8×8-chunk Anvil region file.
fn write_region(path: &PathBuf) {
    let mut file = vec![0u8; 2 * SECTOR_BYTES];
    let mut next_sector = 2usize;
    for cz in 0..8u8 {
        for cx in 0..8u8 {
            let nbt = fastnbt::to_bytes(&chunk_nbt(i32::from(cx), i32::from(cz)))
                .expect("serialize chunk");
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(&nbt).expect("zlib chunk");
            let payload = encoder.finish().expect("finish zlib");

            let length = (payload.len() + 1) as u32;
            let sectors = (4 + length as usize).div_ceil(SECTOR_BYTES);
            let index = 4 * (usize::from(cx) + usize::from(cz) * 32);
            file[index] = ((next_sector >> 16) & 0xFF) as u8;
            file[index + 1] = ((next_sector >> 8) & 0xFF) as u8;
            file[index + 2] = (next_sector & 0xFF) as u8;
            file[index + 3] = sectors as u8;

            let start = next_sector * SECTOR_BYTES;
            file.resize(start + sectors * SECTOR_BYTES, 0);
            file[start..start + 4].copy_from_slice(&length.to_be_bytes());
            file[start + 4] = 2; // zlib
            file[start + 5..start + 5 + payload.len()].copy_from_slice(&payload);
            next_sector += sectors;
        }
    }
    std::fs::write(path, file).expect("write region file");
}
