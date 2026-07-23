//! Extract Vanilla blockstates and models from the Minecraft client JAR.
//!
//! Uses the logic discovered in `jar_textures.rs` (and inspired by tools like
//! Minecraft-Resource-Extractor) to locate the `.minecraft` installation and
//! extract the JSON files needed for Phase 2 mesh generation.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const BLOCKSTATES_PREFIX: &str = "assets/minecraft/blockstates/";
const MODELS_PREFIX: &str = "assets/minecraft/models/";

/// Contains the raw JSON strings for blockstates and models extracted from the JAR.
pub struct VanillaAssets {
    /// e.g. "grass_block" -> "{ ... }"
    pub blockstates: HashMap<String, String>,
    /// e.g. "block/grass_block" -> "{ ... }"
    pub models: HashMap<String, String>,
    pub version: String,
}

impl VanillaAssets {
    /// Load vanilla blockstates and models from the best available source, in order:
    ///
    /// 1. an installed Java Edition client JAR
    /// 2. a previously downloaded JAR in the app cache
    /// 3. a one-time download of the latest release client JAR from Mojang's CDN
    pub fn load() -> Result<Self, String> {
        if let Ok(assets) = Self::auto_detect() {
            return Ok(assets);
        }
        let cache = crate::jar_textures::cache_jar_path()?;
        if !cache.exists() {
            tracing::info!(
                "No local Minecraft installation found — downloading vanilla \
                 client JAR from Mojang (one-time, ~25 MB)…"
            );
            crate::jar_textures::download_client_jar(&cache)?;
        }
        let assets = Self::extract(&cache, "latest release".to_owned())?;
        tracing::info!(
            "Loaded {} blockstates and {} models from the cached vanilla client JAR",
            assets.blockstates.len(),
            assets.models.len()
        );
        Ok(assets)
    }

    /// Attempt to find and load the Minecraft Java Edition client JAR.
    pub fn auto_detect() -> Result<Self, String> {
        let mc_dir = crate::jar_textures::minecraft_dir()?;
        let version = crate::jar_textures::detect_version(&mc_dir)?;
        let jar_path = mc_dir
            .join("versions")
            .join(&version)
            .join(format!("{version}.jar"));
        if !jar_path.exists() {
            return Err(format!(
                "Minecraft client JAR not found: {}",
                jar_path.display()
            ));
        }
        let assets = Self::extract(&jar_path, version)?;
        tracing::info!(
            "Loaded {} blockstates and {} models from Minecraft {}",
            assets.blockstates.len(),
            assets.models.len(),
            assets.version
        );
        Ok(assets)
    }

    /// Find the client JAR and extract all blockstates and models.
    pub fn extract(jar_path: &Path, version: String) -> Result<Self, String> {
        let file = File::open(jar_path)
            .map_err(|e| format!("Cannot open JAR {}: {e}", jar_path.display()))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Cannot read JAR as ZIP: {e}"))?;

        let mut blockstates: HashMap<String, String> = HashMap::new();
        let mut models: HashMap<String, String> = HashMap::new();

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().to_owned();

            if name.ends_with(".json") {
                if name.starts_with(BLOCKSTATES_PREFIX) {
                    let state_name = &name[BLOCKSTATES_PREFIX.len()..name.len() - 5]; // strip .json
                    let mut text = String::new();
                    if entry.read_to_string(&mut text).is_ok() {
                        blockstates.insert(state_name.to_owned(), text);
                    }
                } else if name.starts_with(MODELS_PREFIX) {
                    let model_name = &name[MODELS_PREFIX.len()..name.len() - 5]; // strip .json
                    let mut text = String::new();
                    if entry.read_to_string(&mut text).is_ok() {
                        models.insert(model_name.to_owned(), text);
                    }
                }
            }
        }

        if blockstates.is_empty() || models.is_empty() {
            return Err(format!(
                "No blockstates or models found in JAR {}",
                jar_path.display()
            ));
        }

        Ok(Self {
            blockstates,
            models,
            version,
        })
    }
}
