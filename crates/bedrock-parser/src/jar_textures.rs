//! Load real Minecraft block textures from the Java Edition client `.jar`.
//!
//! The Minecraft `.jar` is a standard ZIP archive. Block textures live under
//! `assets/minecraft/textures/block/<name>.png`. This module finds the latest
//! installed version, opens the JAR, and extracts every block texture PNG into
//! an in-memory map keyed by texture name (without the path prefix or `.png`
//! suffix, e.g. `"grass_block_top"`).
//!
//! When no JAR is found the loader falls back gracefully — callers always check
//! the return value and fall back to procedural textures.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const BLOCK_PREFIX: &str = "assets/minecraft/textures/block/";

/// In-memory store of block texture PNGs, keyed by texture name.
///
/// Example key: `"grass_block_top"` (no path prefix, no `.png`).
pub struct JarTextureLoader {
    /// Raw PNG bytes per texture name.
    textures: HashMap<String, Vec<u8>>,
    /// Which Minecraft version the textures came from (informational).
    pub version: String,
}

impl JarTextureLoader {
    /// An empty loader: every lookup misses, so atlases fall back to
    /// procedural tiles. Used when no real textures are available.
    pub fn empty() -> Self {
        Self {
            textures: HashMap::new(),
            version: "none".to_owned(),
        }
    }

    /// Load vanilla textures from the best available source, in order:
    ///
    /// 1. an installed Java Edition client JAR ([`Self::auto_detect`]),
    /// 2. a previously downloaded JAR in the app cache,
    /// 3. a one-time download of the latest release client JAR from
    ///    Mojang's official CDN (piston-meta / piston-data).
    pub fn load() -> Result<Self, String> {
        if let Ok(loader) = Self::auto_detect() {
            return Ok(loader);
        }
        let cache = cache_jar_path()?;
        if !cache.exists() {
            tracing::info!(
                "No local Minecraft installation found — downloading vanilla \
                 textures from Mojang (one-time, ~25 MB)…"
            );
            download_client_jar(&cache)?;
        }
        let loader = Self::from_jar(&cache, "latest release".to_owned())?;
        tracing::info!(
            "Loaded {} block textures from the cached vanilla client JAR",
            loader.textures.len()
        );
        Ok(loader)
    }

    /// Attempt to find and load the Minecraft Java Edition client JAR.
    ///
    /// Searches `%APPDATA%\.minecraft\versions\` on Windows (or the equivalent
    /// on other platforms), picks the most recently played version from
    /// `launcher_profiles.json`, and extracts all block texture PNGs.
    ///
    /// Returns `Err` with a human-readable message if no JAR can be found or
    /// if the extraction fails.
    pub fn auto_detect() -> Result<Self, String> {
        let mc_dir = minecraft_dir()?;
        let version = detect_version(&mc_dir)?;
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
        let loader = Self::from_jar(&jar_path, version)?;
        tracing::info!(
            "Loaded {} block textures from Minecraft {}",
            loader.textures.len(),
            loader.version
        );
        Ok(loader)
    }

    /// Load block textures from a specific JAR path.
    pub fn from_jar(jar_path: &Path, version: String) -> Result<Self, String> {
        let file = File::open(jar_path)
            .map_err(|e| format!("Cannot open JAR {}: {e}", jar_path.display()))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Cannot read JAR as ZIP: {e}"))?;

        let mut textures: HashMap<String, Vec<u8>> = HashMap::new();

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().to_owned();
            if !name.starts_with(BLOCK_PREFIX) || !name.ends_with(".png") {
                continue;
            }
            // Strip prefix and .png suffix to get the texture name.
            let texture_name = &name[BLOCK_PREFIX.len()..name.len() - 4];
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| format!("Cannot read texture {name}: {e}"))?;
            textures.insert(texture_name.to_owned(), bytes);
        }

        if textures.is_empty() {
            return Err(format!(
                "No block textures found in JAR {}",
                jar_path.display()
            ));
        }
        Ok(Self { textures, version })
    }

    /// Raw PNG bytes for a texture name, e.g. `"grass_block_top"`.
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        if let Some(bytes) = self.textures.get(name) {
            return Some(bytes.as_slice());
        }
        // Aliases for name variations across Minecraft versions
        let alias = match name {
            "short_grass" => "grass",
            "grass" => "short_grass",
            "dirt_path" => "grass_path",
            "grass_path" => "dirt_path",
            "water_still" => "water",
            "water" => "water_still",
            "water_flow" => "water_flow",
            _ => return None,
        };
        self.textures.get(alias).map(Vec::as_slice)
    }

    /// Number of textures loaded.
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// True when no textures were loaded.
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

/// Path of the cached downloaded client JAR, creating the cache directory.
pub fn cache_jar_path() -> Result<PathBuf, String> {
    let dir = dirs::cache_dir()
        .ok_or_else(|| "no OS cache directory available".to_string())?
        .join("Project Bedrock");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("cannot create cache directory {}: {e}", dir.display()))?;
    Ok(dir.join("vanilla-client.jar"))
}

/// Mojang's version manifest: lists every release and where to download it.
const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// User-Agent string sent to Mojang's API and CDN. Required — Mojang blocks
/// requests that omit or use a generic UA.
const MOJANG_USER_AGENT: &str = "ProjectBedrock/0.1.0";

/// Download the latest release client JAR from Mojang's CDN into `dest`
/// (atomically, via a temporary file).
pub fn download_client_jar(dest: &Path) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(300))
        .user_agent(MOJANG_USER_AGENT)
        .build();
    let get_json = |url: &str| -> Result<serde_json::Value, String> {
        agent
            .get(url)
            .call()
            .map_err(|e| format!("GET {url}: {e}"))?
            .into_json()
            .map_err(|e| format!("bad JSON from {url}: {e}"))
    };

    let manifest = get_json(MANIFEST_URL)?;
    let release = manifest
        .get("latest")
        .and_then(|l| l.get("release"))
        .and_then(|r| r.as_str())
        .ok_or_else(|| "version manifest has no latest.release".to_string())?;
    let version_url = manifest
        .get("versions")
        .and_then(|v| v.as_array())
        .and_then(|versions| {
            versions
                .iter()
                .find(|v| v.get("id").and_then(|id| id.as_str()) == Some(release))
        })
        .and_then(|v| v.get("url"))
        .and_then(|u| u.as_str())
        .ok_or_else(|| format!("version manifest has no entry for {release}"))?;

    let version_json = get_json(version_url)?;
    let client_url = version_json
        .get("downloads")
        .and_then(|d| d.get("client"))
        .and_then(|c| c.get("url"))
        .and_then(|u| u.as_str())
        .ok_or_else(|| format!("no client download URL for {release}"))?;

    tracing::info!("Downloading Minecraft {release} client JAR…");
    let response = agent
        .get(client_url)
        .call()
        .map_err(|e| format!("GET {client_url}: {e}"))?;
    let tmp = dest.with_extension("jar.tmp");
    let mut file =
        File::create(&tmp).map_err(|e| format!("cannot create {}: {e}", tmp.display()))?;
    std::io::copy(&mut response.into_reader(), &mut file)
        .map_err(|e| format!("download failed: {e}"))?;
    std::fs::rename(&tmp, dest)
        .map_err(|e| format!("cannot move {} into place: {e}", tmp.display()))?;
    tracing::info!("Cached vanilla client JAR at {}", dest.display());
    Ok(())
}

/// Return the platform-appropriate `.minecraft` directory.
pub fn minecraft_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| "APPDATA environment variable not set".to_string())?;
        let dir = PathBuf::from(appdata).join(".minecraft");
        if dir.is_dir() {
            return Ok(dir);
        }
        Err(format!(
            ".minecraft directory not found at {}",
            dir.display()
        ))
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("Cannot find home directory")?;
        let dir = home.join("Library/Application Support/minecraft");
        if dir.is_dir() {
            return Ok(dir);
        }
        Err(format!(
            "minecraft directory not found at {}",
            dir.display()
        ))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let home = dirs::home_dir().ok_or("Cannot find home directory")?;
        let dir = home.join(".minecraft");
        if dir.is_dir() {
            return Ok(dir);
        }
        Err(format!(
            ".minecraft directory not found at {}",
            dir.display()
        ))
    }
}

/// Determine the Minecraft version to use.
///
/// Reads `launcher_profiles.json` to find the most recently used profile's
/// version. Falls back to scanning the `versions/` directory and picking the
/// newest-looking folder.
pub fn detect_version(mc_dir: &Path) -> Result<String, String> {
    // Try launcher_profiles.json first.
    if let Ok(version) = version_from_launcher_profiles(mc_dir) {
        return Ok(version);
    }
    // Fallback: scan versions/ directory.
    version_from_versions_dir(mc_dir)
}

fn version_from_launcher_profiles(mc_dir: &Path) -> Result<String, String> {
    let profiles_path = mc_dir.join("launcher_profiles.json");
    let content = std::fs::read_to_string(&profiles_path)
        .map_err(|e| format!("Cannot read launcher_profiles.json: {e}"))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Bad JSON in launcher_profiles: {e}"))?;

    // Find the selected profile's lastVersionId.
    if let Some(profiles) = json.get("profiles").and_then(|p| p.as_object()) {
        // Find the profile with the latest lastUsed timestamp.
        let mut best: Option<(&str, &str)> = None; // (lastUsed, version)
        for (_key, profile) in profiles {
            let Some(version_id) = profile.get("lastVersionId").and_then(|v| v.as_str()) else {
                continue;
            };
            // Skip modded/snapshot profiles — prefer release.
            // A release version looks like "1.21.1" (digits and dots only).
            let is_release = version_id.chars().all(|c| c.is_ascii_digit() || c == '.');
            if !is_release {
                continue;
            }
            let last_used = profile
                .get("lastUsed")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if best.is_none() || last_used > best.unwrap().0 {
                best = Some((last_used, version_id));
            }
        }
        if let Some((_, version)) = best {
            let jar_path = mc_dir
                .join("versions")
                .join(version)
                .join(format!("{version}.jar"));
            if jar_path.exists() {
                return Ok(version.to_owned());
            }
        }
    }
    Err("No suitable release profile found in launcher_profiles.json".to_string())
}

fn version_from_versions_dir(mc_dir: &Path) -> Result<String, String> {
    let versions_dir = mc_dir.join("versions");
    let entries = std::fs::read_dir(&versions_dir)
        .map_err(|e| format!("Cannot read versions directory: {e}"))?;

    let mut candidates: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Only release versions: all digits and dots (e.g. "1.21.1").
        if name.chars().all(|c| c.is_ascii_digit() || c == '.') {
            let jar = entry.path().join(format!("{name}.jar"));
            if jar.exists() {
                candidates.push(name);
            }
        }
    }

    if candidates.is_empty() {
        return Err(format!(
            "No release versions found in {}",
            versions_dir.display()
        ));
    }

    // Sort by version number (semver-ish: split by '.', compare numerically).
    candidates.sort_by(|a, b| compare_versions(a, b));
    Ok(candidates.last().cloned().unwrap())
}

/// Compare two version strings numerically, component-by-component.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u32> { s.split('.').filter_map(|c| c.parse().ok()).collect() };
    parse(a).cmp(&parse(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_works() {
        use std::cmp::Ordering;
        assert_eq!(compare_versions("1.21.1", "1.20.4"), Ordering::Greater);
        assert_eq!(compare_versions("1.8", "1.21"), Ordering::Less);
        assert_eq!(compare_versions("1.21.1", "1.21.1"), Ordering::Equal);
    }

    #[test]
    fn texture_name_strip() {
        // Verify our prefix/suffix stripping logic.
        let full = "assets/minecraft/textures/block/grass_block_top.png";
        assert!(full.starts_with(BLOCK_PREFIX));
        assert!(full.ends_with(".png"));
        let name = &full[BLOCK_PREFIX.len()..full.len() - 4];
        assert_eq!(name, "grass_block_top");
    }
}
