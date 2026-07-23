//! Simplified Java world loading using chunkforge-core.
//!
//! Replaces the complex Mineways-based geometry system with a simple
//! approach: parse `.mca` files via chunkforge-core, store every exterior
//! block position, and render them as simple coloured cubes with
//! opacity-driven face culling.

use std::collections::HashMap;

/// Exterior block positions grouped by block name.
pub struct ExteriorWorld {
    /// All exterior blocks: position → block name.
    pub blocks: HashMap<(i32, i32, i32), String>,
    /// Block names in first-seen order.
    pub names: Vec<String>,
}

impl ExteriorWorld {
    /// Load a Java world from `.mca` file paths using chunkforge-core.
    pub fn load(paths: &[impl AsRef<std::path::Path>]) -> Result<Self, String> {
        let parsed = chunkforge_core::parse_region_paths(paths).map_err(|e| e.to_string())?;
        Ok(Self::from_parsed(parsed))
    }

    /// Convert a chunkforge-core `ParsedWorld` into our simple format.
    pub fn from_parsed(world: chunkforge_core::ParsedWorld) -> Self {
        let mut blocks = HashMap::new();
        let mut names = Vec::new();
        for (name, positions) in &world.blocks_by_type {
            names.push(name.clone());
            for &[x, y, z] in positions {
                blocks.insert((x, y, z), name.clone());
            }
        }
        ExteriorWorld { blocks, names }
    }

    /// Get the block name at a world position.
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> Option<&str> {
        self.blocks.get(&(x, y, z)).map(|s| s.as_str())
    }

    /// Number of exterior blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

/// Re-export chunkforge-core's appearance lookup for culling.
pub use chunkforge_core::appearance;

pub use chunkforge_core::BlockAppearance;
