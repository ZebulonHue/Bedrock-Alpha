//! A Minecraft world on disk: level metadata plus its region files.

use crate::level::{self, LevelDatError, LevelMeta};
use std::path::{Path, PathBuf};

/// A Java Edition world folder.
pub struct World {
    folder: PathBuf,
}

impl World {
    /// Open a world folder (does not read anything yet).
    pub fn open(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
        }
    }

    /// The world folder on disk.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Read the world's `level.dat` metadata.
    pub fn level_meta(&self) -> Result<LevelMeta, LevelDatError> {
        level::read_level_dat(&self.folder.join("level.dat"))
    }

    /// `(region_x, region_z, path)` for every region file across the overworld
    /// and all dimension folders (`DIM1` = End, `DIM-1` = Nether, `DIM<n>` =
    /// custom). Scanning the dimension folders is what lets worlds whose
    /// chunks live outside the overworld — for example an End-only save whose
    /// `.mca` files are under `DIM1/region/` — actually load.
    pub fn regions(&self) -> Vec<(i32, i32, PathBuf)> {
        let mut regions = Vec::new();

        // Candidate dimension roots: the world folder (overworld) plus any
        // `DIM<n>` directory (e.g. `DIM1`, `DIM-1`).
        let mut roots = vec![self.folder.clone()];
        if let Ok(entries) = std::fs::read_dir(&self.folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if let Some(rest) = name.strip_prefix("DIM") {
                    if rest.parse::<i32>().is_ok() {
                        roots.push(path);
                    }
                }
            }
        }

        for root in roots {
            let region_dir = root.join("region");
            let Ok(entries) = std::fs::read_dir(&region_dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some((x, z)) = region_coordinates(&path) {
                    regions.push((x, z, path));
                }
            }
        }

        regions.sort_unstable();
        regions
    }
}

/// Parse `r.X.Z.mca` filenames into region coordinates.
fn region_coordinates(path: &Path) -> Option<(i32, i32)> {
    let name = path.file_name()?.to_str()?;
    let mut parts = name.strip_prefix("r.")?.strip_suffix(".mca")?.split('.');
    let x = parts.next()?.parse().ok()?;
    let z = parts.next()?.parse().ok()?;
    Some((x, z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_scans_dimension_folders() {
        let dir = std::env::temp_dir().join(format!("bedrock-dim-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Overworld region plus an End (DIM1) and Nether (DIM-1) region.
        std::fs::create_dir_all(dir.join("region")).unwrap();
        std::fs::create_dir_all(dir.join("DIM1").join("region")).unwrap();
        std::fs::create_dir_all(dir.join("DIM-1").join("region")).unwrap();
        std::fs::write(dir.join("region").join("r.0.0.mca"), b"").unwrap();
        std::fs::write(dir.join("DIM1").join("region").join("r.0.0.mca"), b"").unwrap();
        std::fs::write(dir.join("DIM-1").join("region").join("r.2.-3.mca"), b"").unwrap();
        // A non-dimension folder must be ignored.
        std::fs::create_dir_all(dir.join("playerdata")).unwrap();

        let world = World::open(&dir);
        let mut coords: Vec<(i32, i32)> = world
            .regions()
            .into_iter()
            .map(|(x, z, _)| (x, z))
            .collect();
        coords.sort_unstable();

        assert_eq!(coords, vec![(0, 0), (0, 0), (2, -3)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_region_filenames() {
        assert_eq!(
            region_coordinates(Path::new("region/r.0.0.mca")),
            Some((0, 0))
        );
        assert_eq!(region_coordinates(Path::new("r.-3.17.mca")), Some((-3, 17)));
        assert_eq!(region_coordinates(Path::new("r.0.0.mca.old")), None);
        assert_eq!(region_coordinates(Path::new("level.dat")), None);
    }
}
