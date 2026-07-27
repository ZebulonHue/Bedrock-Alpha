//! # bedrock-settings
//!
//! Persistent application settings for Project Bedrock.
//!
//! Settings are stored as TOML in the user's config directory
//! (`%APPDATA%/ProjectBedrock/settings.toml` on Windows). The dock layout and
//! the log file live next to it. Everything degrades gracefully: a missing or
//! corrupt settings file must never prevent the application from starting.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Directory that holds all Project Bedrock user data.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ProjectBedrock")
}

/// Path of the TOML settings file.
pub fn settings_path() -> PathBuf {
    config_dir().join("settings.toml")
}

/// Path of the serialized dock layout (egui_dock `DockState` as JSON).
pub fn layout_path() -> PathBuf {
    config_dir().join("layout.json")
}

/// Path of the human-readable application log.
pub fn log_path() -> PathBuf {
    config_dir().join("bedrock.log")
}

/// UI color theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Dark theme (default, matches the PRD's visual direction).
    #[default]
    Dark,
    /// Light theme.
    Light,
}

impl Theme {
    /// Every theme, for pickers.
    pub const ALL: [Theme; 2] = [Theme::Dark, Theme::Light];

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
        }
    }
}

/// Export quality preset shown in the Export Settings panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportPreset {
    /// Maximum fidelity export.
    #[default]
    HighQuality,
    /// Tuned for animation scenes.
    Animation,
    /// Tuned for thumbnails / renders.
    Thumbnail,
    /// Minimal memory footprint.
    LowMemory,
    /// User-defined.
    Custom,
}

impl ExportPreset {
    /// Every preset, for pickers.
    pub const ALL: [ExportPreset; 5] = [
        ExportPreset::HighQuality,
        ExportPreset::Animation,
        ExportPreset::Thumbnail,
        ExportPreset::LowMemory,
        ExportPreset::Custom,
    ];

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            ExportPreset::HighQuality => "High Quality",
            ExportPreset::Animation => "Animation",
            ExportPreset::Thumbnail => "Thumbnail",
            ExportPreset::LowMemory => "Low Memory",
            ExportPreset::Custom => "Custom",
        }
    }
}

/// Export file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Wavefront OBJ — the Phase 5 deliverable.
    #[default]
    Obj,
    /// FBX — future.
    Fbx,
    /// USD — future.
    Usd,
    /// glTF 2.0 — supported (PBR materials, texture atlas).
    Gltf,
}

impl ExportFormat {
    /// Every format, for pickers.
    pub const ALL: [ExportFormat; 4] = [
        ExportFormat::Obj,
        ExportFormat::Fbx,
        ExportFormat::Usd,
        ExportFormat::Gltf,
    ];

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Obj => "OBJ",
            ExportFormat::Fbx => "FBX",
            ExportFormat::Usd => "USD",
            ExportFormat::Gltf => "glTF",
        }
    }

    /// Whether the format can actually be produced today.
    pub fn is_supported(self) -> bool {
        matches!(self, ExportFormat::Obj | ExportFormat::Gltf)
    }
}

/// Preferences for the export pipeline (consumed by Phase 5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportPreferences {
    /// Selected quality preset.
    pub preset: ExportPreset,
    /// Selected output format.
    pub format: ExportFormat,
    /// Output directory chosen by the user (empty = ask at export time).
    pub output_dir: String,
}

/// Smallest allowed value for [`Settings::load_radius_chunks`].
pub const MIN_LOAD_RADIUS_CHUNKS: i32 = 4;
/// Largest allowed value for [`Settings::load_radius_chunks`]. Bounded to
/// keep worst-case memory/load time predictable until chunk streaming
/// (rather than "load everything up front") lands.
pub const MAX_LOAD_RADIUS_CHUNKS: i32 = 48;
/// Default load radius, generous enough for most worlds.
const DEFAULT_LOAD_RADIUS_CHUNKS: i32 = 24;

/// Debug visualisation toggles (Phase 6b).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DebugSettings {
    /// Overlay FPS + vertex/triangle counts on the viewport.
    pub show_stats: bool,
    /// Draw wireframe boxes around every loaded chunk.
    pub show_chunk_borders: bool,
    /// Overlay mesh edges as a wireframe.
    pub show_wireframe: bool,
    /// Highlight occluded (culled) faces.
    pub show_occluded: bool,
}

/// Root settings object, persisted as TOML.
///
/// `Eq` dropped: `background_media_opacity` is an `f32`, which cannot
/// implement it. Nothing in the app compares two whole `Settings` values for
/// exact equality.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// UI theme.
    pub theme: Theme,
    /// Show an FPS counter in the status bar.
    pub show_fps: bool,
    /// Recently opened worlds (Phase 2 will populate).
    pub recent_worlds: Vec<PathBuf>,
    /// Recent export destinations.
    pub recent_exports: Vec<PathBuf>,
    /// Export pipeline preferences.
    pub export: ExportPreferences,
    /// Radius, in chunks, loaded around the player/origin when opening a
    /// world. Clamped to `[MIN_LOAD_RADIUS_CHUNKS, MAX_LOAD_RADIUS_CHUNKS]`
    /// on load in case a hand-edited or older settings file is out of range.
    pub load_radius_chunks: i32,
    /// Debug visualisation settings (Phase 6b).
    pub debug: DebugSettings,
    /// A local video file to play, blurred, behind the UI, with its audio.
    ///
    /// Deliberately just a path, never file contents: the app has no business
    /// copying whatever the user points it at, and it means nothing about the
    /// file itself is ever written into settings.json or committed anywhere.
    /// `None` until the user sets one in Settings.
    pub background_media_path: Option<PathBuf>,
    /// Play audio from the background media. Off mutes it without unloading
    /// the video, so silencing it does not also lose the picture.
    pub background_media_audio: bool,
    /// Opacity of the background video behind the UI, 0.0 (invisible) to 1.0.
    pub background_media_opacity: f32,
    /// Background audio volume, 0.0 (silent) to 1.0.
    pub background_media_volume: f32,
    /// Gaussian blur strength (ffmpeg `gblur` sigma) applied to the video.
    /// 0 is unblurred; higher softens it further. Changing this restarts
    /// decoding, since ffmpeg bakes the blur into the frames it produces
    /// rather than it being adjustable after the fact.
    pub background_media_blur: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            show_fps: true,
            recent_worlds: Vec::new(),
            recent_exports: Vec::new(),
            export: ExportPreferences::default(),
            load_radius_chunks: DEFAULT_LOAD_RADIUS_CHUNKS,
            debug: DebugSettings::default(),
            background_media_path: None,
            background_media_audio: true,
            background_media_opacity: 0.28,
            background_media_volume: 0.6,
            background_media_blur: 14.0,
        }
    }
}

impl Settings {
    /// Load settings from the default location. Never fails.
    pub fn load() -> Self {
        Self::load_from(&settings_path())
    }

    /// Load settings from `path`, falling back to defaults on any problem.
    ///
    /// A corrupt file is renamed to `*.bak` so the user keeps evidence, and
    /// defaults are returned. This function must never panic.
    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        match toml::from_str::<Self>(&text) {
            Ok(mut settings) => {
                settings.load_radius_chunks = settings
                    .load_radius_chunks
                    .clamp(MIN_LOAD_RADIUS_CHUNKS, MAX_LOAD_RADIUS_CHUNKS);
                settings
            }
            Err(err) => {
                let backup = path.with_extension("toml.bak");
                tracing::warn!(
                    "Settings file {} is invalid ({err}); moving it to {}",
                    path.display(),
                    backup.display()
                );
                let _ = fs::rename(path, backup);
                Self::default()
            }
        }
    }

    /// Save to the default location; logs (never panics) on failure.
    pub fn save(&self) {
        if let Err(err) = self.save_to(&settings_path()) {
            tracing::error!("Failed to save settings: {err}");
        }
    }

    /// Save to `path`, creating parent directories as needed.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "bedrock-settings-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn missing_file_yields_defaults() {
        let settings = Settings::load_from(&temp_path("missing.toml"));
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.theme, Theme::Dark);
        assert!(settings.show_fps);
    }

    #[test]
    fn round_trip_preserves_values() {
        let path = temp_path("roundtrip.toml");
        let mut settings = Settings {
            theme: Theme::Light,
            export: ExportPreferences {
                preset: ExportPreset::Thumbnail,
                ..ExportPreferences::default()
            },
            ..Settings::default()
        };
        settings.export.output_dir = "D:/exports".into();
        settings.recent_worlds.push(PathBuf::from("C:/worlds/a"));
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), settings);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let path = temp_path("corrupt.toml");
        fs::write(&path, "this is [not valid toml").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("toml.bak"));
    }

    #[test]
    fn partial_file_uses_field_defaults() {
        let path = temp_path("partial.toml");
        fs::write(&path, "theme = \"light\"\n").unwrap();
        let settings = Settings::load_from(&path);
        assert_eq!(settings.theme, Theme::Light);
        assert!(settings.show_fps, "unspecified fields keep their defaults");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn debug_defaults_are_off() {
        let d = DebugSettings::default();
        assert!(!d.show_stats);
        assert!(!d.show_chunk_borders);
        assert!(!d.show_wireframe);
        assert!(!d.show_occluded);
    }

    #[test]
    fn debug_settings_round_trip() {
        let path = temp_path("debug-roundtrip.toml");
        let mut settings = Settings::default();
        settings.debug.show_stats = true;
        settings.debug.show_chunk_borders = true;
        settings.debug.show_wireframe = true;
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path);
        assert!(loaded.debug.show_stats);
        assert!(loaded.debug.show_chunk_borders);
        assert!(loaded.debug.show_wireframe);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn out_of_range_load_radius_is_clamped_on_load() {
        let path = temp_path("radius-clamp.toml");
        fs::write(&path, "load_radius_chunks = 9999\n").unwrap();
        let settings = Settings::load_from(&path);
        assert_eq!(settings.load_radius_chunks, MAX_LOAD_RADIUS_CHUNKS);
        fs::write(&path, "load_radius_chunks = -5\n").unwrap();
        let settings = Settings::load_from(&path);
        assert_eq!(settings.load_radius_chunks, MIN_LOAD_RADIUS_CHUNKS);
        let _ = fs::remove_file(&path);
    }
}
