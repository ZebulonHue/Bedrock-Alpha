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

    /// Every dimension in this save that actually holds region files.
    ///
    /// Covers the legacy `DIM1` (End) / `DIM-1` (Nether) / `DIM<n>` layout and
    /// the modern `dimensions/<namespace>/<name>` one, plus the top-level
    /// `region/`. Some saves keep even the overworld under `dimensions/`
    /// rather than at the top level, so neither location can be assumed.
    ///
    /// Returned in preference order, most likely to be the overworld first.
    pub fn dimensions(&self) -> Vec<Dimension> {
        let mut roots = vec![(DimensionKind::Overworld, self.folder.clone())];
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
                    if let Ok(id) = rest.parse::<i32>() {
                        roots.push((DimensionKind::Legacy(id), path));
                    }
                }
            }
        }
        if let Ok(namespaces) = std::fs::read_dir(self.folder.join("dimensions")) {
            for namespace_entry in namespaces.flatten() {
                let namespace_path = namespace_entry.path();
                if !namespace_path.is_dir() {
                    continue;
                }
                let Ok(names) = std::fs::read_dir(&namespace_path) else {
                    continue;
                };
                for name_entry in names.flatten() {
                    let name_path = name_entry.path();
                    if !name_path.is_dir() {
                        continue;
                    }
                    let label = name_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_owned();
                    roots.push((DimensionKind::Named(label), name_path));
                }
            }
        }

        let mut found: Vec<Dimension> = Vec::new();
        for (kind, root) in roots {
            let Ok(entries) = std::fs::read_dir(root.join("region")) else {
                continue;
            };
            let mut regions: Vec<(i32, i32, PathBuf)> = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some((x, z)) = region_coordinates(&path) {
                    regions.push((x, z, path));
                }
            }
            if regions.is_empty() {
                continue;
            }
            regions.sort_unstable();
            found.push(Dimension { kind, regions });
        }
        found.sort_by_key(|d| d.kind.rank());
        found
    }

    /// `(region_x, region_z, path)` for the dimension this world opens in.
    ///
    /// One dimension only. Every dimension covers the same X/Z, so returning
    /// all of them stacks the Nether and the End on top of the overworld at
    /// identical coordinates -- geometry that overlaps in space, cannot be
    /// told apart afterwards because a chunk is keyed by X/Z alone, and costs
    /// its full price to build. A save with a well-explored Nether meshed 3,293
    /// chunks for a radius that can only hold 1,369, at 42 GB of vertices.
    ///
    /// Falls through to whichever dimension does have regions, so a save whose
    /// chunks live only under `DIM1/region/` or
    /// `dimensions/minecraft/overworld/region/` still opens.
    pub fn regions(&self) -> Vec<(i32, i32, PathBuf)> {
        self.dimensions()
            .into_iter()
            .next()
            .map(|d| d.regions)
            .unwrap_or_default()
    }
}

/// Which dimension a set of region files belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimensionKind {
    /// The save's top-level `region/`.
    Overworld,
    /// A `DIM<n>` folder: 1 is the End, -1 the Nether.
    Legacy(i32),
    /// A `dimensions/<namespace>/<name>` folder, by its name.
    Named(String),
}

impl DimensionKind {
    /// Sort key placing the most overworld-like first.
    fn rank(&self) -> (u8, String) {
        match self {
            DimensionKind::Overworld => (0, String::new()),
            DimensionKind::Named(name) if name == "overworld" => (1, name.clone()),
            DimensionKind::Named(name) => (3, name.clone()),
            DimensionKind::Legacy(id) => (4, id.to_string()),
        }
    }

    /// True when this is the Nether, under either folder layout.
    pub fn is_nether(&self) -> bool {
        matches!(self, DimensionKind::Legacy(-1))
            || matches!(self, DimensionKind::Named(n) if n == "the_nether" || n == "nether")
    }

    /// True when this is the End, under either folder layout.
    pub fn is_end(&self) -> bool {
        matches!(self, DimensionKind::Legacy(1))
            || matches!(self, DimensionKind::Named(n) if n == "the_end" || n == "end")
    }

    /// True when this is the overworld, under either folder layout.
    pub fn is_overworld(&self) -> bool {
        matches!(self, DimensionKind::Overworld)
            || matches!(self, DimensionKind::Named(n) if n == "overworld")
    }

    /// Human-readable name for logs.
    pub fn label(&self) -> String {
        match self {
            DimensionKind::Overworld => "Overworld".to_owned(),
            DimensionKind::Legacy(1) => "End".to_owned(),
            DimensionKind::Legacy(-1) => "Nether".to_owned(),
            DimensionKind::Legacy(id) => format!("DIM{id}"),
            DimensionKind::Named(name) => name.clone(),
        }
    }
}

/// One dimension's region files.
#[derive(Debug, Clone)]
pub struct Dimension {
    /// Which dimension this is.
    pub kind: DimensionKind,
    /// `(region_x, region_z, path)`, sorted.
    pub regions: Vec<(i32, i32, PathBuf)>,
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

        // All three are found, the overworld first. The order of the rest
        // does not matter; only that the overworld wins.
        let labels: Vec<String> = world.dimensions().iter().map(|d| d.kind.label()).collect();
        assert_eq!(labels[0], "Overworld");
        assert_eq!(labels.len(), 3);

        // ...but opening the world uses one of them. Every dimension covers
        // the same X/Z, so loading them together stacks the Nether and the End
        // through the overworld at identical coordinates.
        let coords: Vec<(i32, i32)> = world
            .regions()
            .into_iter()
            .map(|(x, z, _)| (x, z))
            .collect();
        assert_eq!(coords, vec![(0, 0)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_without_a_top_level_region_folder_still_opens() {
        // Both real cases: an End-only save, and a save that keeps its
        // overworld under `dimensions/` instead of at the top level.
        for (sub, expect) in [
            (PathBuf::from("DIM1"), "End"),
            (PathBuf::from("dimensions").join("minecraft").join("overworld"), "overworld"),
        ] {
            let dir = std::env::temp_dir()
                .join(format!("bedrock-dim-only-{}-{expect}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join(&sub).join("region")).unwrap();
            std::fs::write(dir.join(&sub).join("region").join("r.1.1.mca"), b"").unwrap();

            let world = World::open(&dir);
            let coords: Vec<(i32, i32)> = world
                .regions()
                .into_iter()
                .map(|(x, z, _)| (x, z))
                .collect();
            assert_eq!(coords, vec![(1, 1)], "{expect} save must still open");
            assert_eq!(world.dimensions()[0].kind.label(), expect);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn regions_scans_modern_dimensions_folder() {
        let dir = std::env::temp_dir().join(format!("bedrock-dim-test2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Overworld data living under dimensions/minecraft/overworld/region,
        // as seen on a real-world save instead of the usual top-level region/.
        std::fs::create_dir_all(dir.join("dimensions").join("minecraft").join("overworld").join("region"))
            .unwrap();
        std::fs::write(
            dir.join("dimensions")
                .join("minecraft")
                .join("overworld")
                .join("region")
                .join("r.-1.0.mca"),
            b"",
        )
        .unwrap();

        let world = World::open(&dir);
        let coords: Vec<(i32, i32)> = world.regions().into_iter().map(|(x, z, _)| (x, z)).collect();

        assert_eq!(coords, vec![(-1, 0)]);
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
