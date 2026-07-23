//! World detection: finds Minecraft Java and Bedrock saves on disk and
//! summarizes each world for the World Browser.

use crate::level::{self, LevelMeta};
use std::path::{Path, PathBuf};

/// Which Minecraft edition a world belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    /// Minecraft: Java Edition (`.minecraft/saves`).
    Java,
    /// Minecraft: Bedrock Edition (Microsoft Store install).
    Bedrock,
}

impl Edition {
    /// Short label for badges.
    pub fn label(self) -> &'static str {
        match self {
            Edition::Java => "Java",
            Edition::Bedrock => "Bedrock",
        }
    }
}

/// A detected world on disk — one card in the World Browser.
#[derive(Debug, Clone)]
pub struct WorldSummary {
    /// Which edition this world belongs to.
    pub edition: Edition,
    /// The world's folder on disk.
    pub folder: PathBuf,
    /// Display name (from `level.dat` / `levelname.txt`).
    pub name: String,
    /// Thumbnail image (`icon.png` / `world_icon.jpeg`), if the world has one.
    pub icon: Option<PathBuf>,
    /// Last played, milliseconds since the Unix epoch (if known).
    pub last_played_ms: Option<i64>,
    /// Total size of the world folder in bytes.
    pub size_bytes: u64,
    /// Java data version, if known.
    pub data_version: Option<i32>,
}

/// Standard Java saves directory (`%APPDATA%/.minecraft/saves`).
pub fn java_saves_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join(".minecraft").join("saves"))
}

/// Standard Bedrock worlds directory (Microsoft Store / UWP install).
pub fn bedrock_worlds_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|p| {
        p.join("Packages")
            .join("Microsoft.MinecraftUWP_8wekyb3d8bbwe")
            .join("LocalState")
            .join("games")
            .join("com.mojang")
            .join("minecraftWorlds")
    })
}

/// Bedrock worlds directories used by the newer GDK install:
/// `%APPDATA%\Minecraft Bedrock\Users\<id>\games\com.mojang\minecraftWorlds`.
pub fn bedrock_gdk_worlds_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Some(users) = dirs::config_dir().map(|p| p.join("Minecraft Bedrock").join("Users")) else {
        return dirs;
    };
    if let Ok(entries) = std::fs::read_dir(&users) {
        for entry in entries.flatten() {
            let worlds = entry
                .path()
                .join("games")
                .join("com.mojang")
                .join("minecraftWorlds");
            if worlds.is_dir() {
                dirs.push(worlds);
            }
        }
    }
    dirs
}

/// Scan the standard locations and return every detected world,
/// most recently played first.
pub fn detect_worlds() -> Vec<WorldSummary> {
    let mut worlds = Vec::new();
    if let Some(dir) = java_saves_dir() {
        scan_java(&dir, &mut worlds);
    }
    if let Some(dir) = bedrock_worlds_dir() {
        scan_bedrock(&dir, &mut worlds);
    }
    for dir in bedrock_gdk_worlds_dirs() {
        scan_bedrock(&dir, &mut worlds);
    }
    worlds.sort_by_key(|w| std::cmp::Reverse(w.last_played_ms.unwrap_or(0)));
    worlds
}

/// Scan a Java `saves` directory, appending every valid world to `out`.
fn scan_java(saves: &Path, out: &mut Vec<WorldSummary>) {
    let Ok(entries) = std::fs::read_dir(saves) else {
        return;
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        if let Some(summary) = summarize_java(&folder) {
            out.push(summary);
        }
    }
}

/// Build a summary for a single Java world folder by reading its
/// `level.dat`. If the folder has no `level.dat` but has a `region/`
/// subfolder (e.g. a standalone `DIM1` or `DIM-1` dimension folder), it is
/// still accepted as a Java world with synthetic metadata — this lets users
/// open bare End/Nether folders.
///
/// This is the one place that knows how to turn a folder on disk into a
/// `WorldSummary` — callers (auto-scan, "Open Folder…", future CLI use) must
/// go through this rather than re-deriving the fields themselves, so behavior
/// (icon lookup, size, timestamp handling) never drifts between call sites.
///
/// Returns a human-readable error if `folder` has neither `level.dat` nor
/// a `region/` subfolder.
pub fn open_java_world(folder: &Path) -> Result<WorldSummary, String> {
    let level_dat = folder.join("level.dat");
    if level_dat.is_file() {
        let LevelMeta {
            level_name,
            last_played_ms,
            data_version,
            ..
        } = level::read_level_dat(&level_dat)
            .map_err(|err| format!("{}: {err}", level_dat.display()))?;
        let icon = folder.join("icon.png");
        return Ok(WorldSummary {
            edition: Edition::Java,
            folder: folder.to_path_buf(),
            name: level_name,
            icon: icon.is_file().then_some(icon),
            last_played_ms: Some(last_played_ms),
            size_bytes: dir_size(folder),
            data_version,
        });
    }

    // No level.dat — scan for .mca files up to one level deep.
    // Handles standalone DIM folders (DIM1, DIM-1, etc.) whether they
    // have a region/ subfolder or contain .mca files directly.
    let has_mca = || -> bool {
        // Check region/ subfolder
        if folder.join("region").is_dir()
            && std::fs::read_dir(folder.join("region"))
                .ok()
                .map(|e| {
                    e.flatten()
                        .any(|e| e.path().extension().is_some_and(|x| x == "mca"))
                })
                .unwrap_or(false)
        {
            return true;
        }
        // Check direct children
        if let Ok(entries) = std::fs::read_dir(folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // Check subfolder/region/
                    if path.join("region").is_dir()
                        && std::fs::read_dir(path.join("region"))
                            .ok()
                            .map(|e| {
                                e.flatten()
                                    .any(|e| e.path().extension().is_some_and(|x| x == "mca"))
                            })
                            .unwrap_or(false)
                    {
                        return true;
                    }
                    // Check .mca files directly in subfolder
                    if std::fs::read_dir(&path)
                        .ok()
                        .map(|e| {
                            e.flatten()
                                .any(|e| e.path().extension().is_some_and(|x| x == "mca"))
                        })
                        .unwrap_or(false)
                    {
                        return true;
                    }
                }
                // Check .mca files directly in the root
                if path.extension().is_some_and(|x| x == "mca") {
                    return true;
                }
            }
        }
        false
    };

    if has_mca() {
        let dim_name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Dimension".to_owned());
        return Ok(WorldSummary {
            edition: Edition::Java,
            folder: folder.to_path_buf(),
            name: dim_name,
            icon: None,
            last_played_ms: None,
            size_bytes: dir_size(folder),
            data_version: None,
        });
    }

    Err(format!(
        "no level.dat or .mca region files found in '{}' — pick a Java world folder",
        folder.display()
    ))
}

/// Build a summary for a single Java world folder, or `None` if it has no
/// readable `level.dat` (then it is not a usable world). Used by the silent
/// auto-scan; folders that simply aren't worlds are not logged, only ones
/// with a `level.dat` that failed to parse.
fn summarize_java(folder: &Path) -> Option<WorldSummary> {
    match open_java_world(folder) {
        Ok(summary) => Some(summary),
        Err(err) => {
            if folder.join("level.dat").is_file() {
                tracing::warn!("Skipping {}: {err}", folder.display());
            }
            None
        }
    }
}

/// Scan a Bedrock `minecraftWorlds` directory.
///
/// Bedrock detection is intentionally light for now: the name comes from
/// `levelname.txt`, last-played from the `level.dat` file's mtime. Full
/// Bedrock parsing is a later milestone.
fn scan_bedrock(worlds_dir: &Path, out: &mut Vec<WorldSummary>) {
    let Ok(entries) = std::fs::read_dir(worlds_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        let name = std::fs::read_to_string(folder.join("levelname.txt"))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                folder
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Unknown world".to_owned())
            });
        let level_dat = folder.join("level.dat");
        if !level_dat.is_file() {
            continue;
        }
        let last_played_ms = std::fs::metadata(&level_dat)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        let icon = folder.join("world_icon.jpeg");
        out.push(WorldSummary {
            edition: Edition::Bedrock,
            size_bytes: dir_size(&folder),
            folder,
            name,
            icon: icon.is_file().then_some(icon),
            last_played_ms,
            data_version: None,
        });
    }
}

/// Total size of a directory tree in bytes (best effort, ignores errors).
fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(meta) = p.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use std::io::Write;

    #[derive(Serialize)]
    struct TestLevelDat {
        #[serde(rename = "Data")]
        data: TestData,
    }

    #[derive(Serialize)]
    struct TestData {
        #[serde(rename = "LevelName")]
        level_name: String,
        #[serde(rename = "LastPlayed")]
        last_played: i64,
        #[serde(rename = "Version")]
        version: TestVersion,
    }

    #[derive(Serialize)]
    struct TestVersion {
        #[serde(rename = "Id")]
        id: i32,
    }

    /// Write a gzip-compressed NBT level.dat just like the real game does.
    fn write_fake_level_dat(world_dir: &Path, name: &str, last_played: i64, data_version: i32) {
        let nbt = fastnbt::to_bytes(&TestLevelDat {
            data: TestData {
                level_name: name.to_owned(),
                last_played,
                version: TestVersion { id: data_version },
            },
        })
        .unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&nbt).unwrap();
        std::fs::write(world_dir.join("level.dat"), encoder.finish().unwrap()).unwrap();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bedrock-detect-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn java_world_is_detected_with_metadata() {
        let saves = temp_dir("java");
        let world = saves.join("My Build");
        std::fs::create_dir_all(&world).unwrap();
        write_fake_level_dat(&world, "My Epic Build", 1_700_000_000_000, 4189);
        std::fs::write(world.join("icon.png"), b"not-really-a-png").unwrap();

        let mut worlds = Vec::new();
        scan_java(&saves, &mut worlds);

        assert_eq!(worlds.len(), 1);
        let summary = &worlds[0];
        assert_eq!(summary.edition, Edition::Java);
        assert_eq!(summary.name, "My Epic Build");
        assert_eq!(summary.last_played_ms, Some(1_700_000_000_000));
        assert_eq!(summary.data_version, Some(4189));
        assert!(summary.icon.is_some());
        assert!(summary.size_bytes > 0);

        let _ = std::fs::remove_dir_all(&saves);
    }

    #[test]
    fn folders_without_level_dat_are_skipped() {
        let saves = temp_dir("notaworld");
        std::fs::create_dir_all(saves.join("random-folder")).unwrap();

        let mut worlds = Vec::new();
        scan_java(&saves, &mut worlds);

        assert!(worlds.is_empty());
        let _ = std::fs::remove_dir_all(&saves);
    }

    #[test]
    fn corrupt_level_dat_is_skipped_not_fatal() {
        let saves = temp_dir("corrupt");
        let world = saves.join("broken");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"garbage").unwrap();

        let mut worlds = Vec::new();
        scan_java(&saves, &mut worlds);

        assert!(worlds.is_empty());
        let _ = std::fs::remove_dir_all(&saves);
    }

    #[test]
    fn bedrock_world_uses_levelname_txt() {
        let worlds_dir = temp_dir("bedrock");
        let world = worlds_dir.join("abc123=");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("levelname.txt"), "Bedrock Base\n").unwrap();
        std::fs::write(world.join("level.dat"), b"placeholder").unwrap();

        let mut worlds = Vec::new();
        scan_bedrock(&worlds_dir, &mut worlds);

        assert_eq!(worlds.len(), 1);
        assert_eq!(worlds[0].edition, Edition::Bedrock);
        assert_eq!(worlds[0].name, "Bedrock Base");
        assert!(worlds[0].last_played_ms.is_some());

        let _ = std::fs::remove_dir_all(&worlds_dir);
    }
}
