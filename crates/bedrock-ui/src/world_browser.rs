//! World Browser panel: detected Minecraft worlds as cards with thumbnails.
//!
//! Scanning runs on a background thread so the UI never freezes; thumbnails
//! are decoded lazily and cached as GPU textures.

use bedrock_parser::detect::{detect_worlds, open_java_world, WorldSummary};
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::thread;

/// State of the World Browser: scan lifecycle plus the thumbnail cache.
#[derive(Default)]
pub struct WorldBrowserState {
    worlds: Vec<WorldSummary>,
    scan: Option<Receiver<Vec<WorldSummary>>>,
    textures: HashMap<PathBuf, Option<TextureHandle>>,
    selected: Option<usize>,
}

impl WorldBrowserState {
    /// Start a background scan. No-op while a scan is already running.
    pub fn start_scan(&mut self) {
        if self.scan.is_some() {
            return;
        }
        let (tx, rx) = channel();
        thread::spawn(move || {
            let _ = tx.send(detect_worlds());
        });
        self.scan = Some(rx);
    }

    /// Collect finished scan results. Keeps repainting while a scan runs.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.scan else { return };
        match rx.try_recv() {
            Ok(worlds) => {
                tracing::info!("Detected {} Minecraft world(s)", worlds.len());
                self.worlds = worlds;
                self.scan = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint(),
            Err(TryRecvError::Disconnected) => {
                tracing::warn!("World scan thread disconnected");
                self.scan = None;
            }
        }
    }

    /// The detected worlds (empty until the first scan finishes).
    pub fn worlds(&self) -> &[WorldSummary] {
        &self.worlds
    }

    /// Take the world the user clicked, if any (consumed once).
    pub fn take_selected(&mut self) -> Option<WorldSummary> {
        self.selected
            .take()
            .and_then(|index| self.worlds.get(index).cloned())
    }

    /// Add a world folder chosen manually by the user (for installs the
    /// auto-scan can't see, e.g. third-party launchers), and select it so
    /// it opens immediately.
    ///
    /// Forgiving about what gets picked: if `folder` is not itself a world
    /// but contains world folders (e.g. a `saves` directory), every world
    /// inside is added instead. Returns how many worlds were added.
    pub fn add_manual_world(&mut self, folder: PathBuf) -> Result<usize, String> {
        if folder.join("level.dat").is_file() {
            self.add_java_world(folder)?;
            self.selected = Some(self.worlds.len() - 1);
            return Ok(1);
        }

        // Maybe the user picked a `saves` folder — look one level down.
        let mut added = 0;
        if let Ok(entries) = std::fs::read_dir(&folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() || !path.join("level.dat").is_file() {
                    continue;
                }
                match self.add_java_world(path) {
                    Ok(()) => added += 1,
                    Err(err) => tracing::warn!("Skipping world folder: {err}"),
                }
            }
        }
        if added > 0 {
            // Open the first world found in the folder.
            self.selected = Some(self.worlds.len() - added);
            return Ok(added);
        }

        // Last resort: try the folder itself as a Java world (handles
        // standalone DIM folders that have `region/` but no `level.dat`).
        if let Ok(()) = self.add_java_world(folder.clone()) {
            self.selected = Some(self.worlds.len() - 1);
            return Ok(1);
        }

        if folder.join("levelname.txt").is_file() || folder.join("db").is_dir() {
            return Err(
                "that looks like a Bedrock Edition world — Bedrock support is not implemented yet"
                    .to_owned(),
            );
        }
        Err(format!(
            "no level.dat or .mca files found in '{}' — pick a world folder containing \
             level.dat or region files (.mca)",
            folder.display()
        ))
    }

    /// Read `level.dat` in `folder` and append the world to the list.
    fn add_java_world(&mut self, folder: PathBuf) -> Result<(), String> {
        let summary = open_java_world(&folder)?;
        self.worlds.push(summary);
        Ok(())
    }
}

/// Draw the World Browser panel.
pub fn world_browser(ui: &mut egui::Ui, state: &mut WorldBrowserState) {
    ui.horizontal(|ui| {
        ui.heading("Worlds");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let scanning = state.scan.is_some();
            let label = if scanning {
                "Scanning…"
            } else {
                "Detect Worlds"
            };
            if ui
                .add_enabled(!scanning, egui::Button::new(label))
                .clicked()
            {
                state.start_scan();
            }
            if ui
                .button("Open Folder…")
                .on_hover_text(
                    "Open a world folder from any location (e.g. a third-party launcher)",
                )
                .clicked()
            {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("Open a Java Edition world folder")
                    .pick_folder()
                {
                    match state.add_manual_world(folder) {
                        Ok(count) => tracing::info!("Added {count} world(s) from folder"),
                        Err(message) => tracing::error!("Could not open world: {message}"),
                    }
                }
            }
        });
    });
    ui.separator();

    if state.worlds.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            if state.scan.is_some() {
                ui.weak("Scanning for Minecraft worlds…");
            } else {
                ui.weak("No Minecraft worlds detected.");
                ui.weak("Java and Bedrock installs are scanned automatically.");
            }
        });
        return;
    }

    let WorldBrowserState {
        worlds,
        textures,
        selected,
        ..
    } = state;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, world) in worlds.iter().enumerate() {
                if world_card(ui, textures, world).clicked() {
                    *selected = Some(index);
                }
                ui.add_space(6.0);
            }
        });
}

/// One world card: thumbnail, name, edition, size, last played. Returns the
/// card's click response so the panel can record a selection.
fn world_card(
    ui: &mut egui::Ui,
    textures: &mut HashMap<PathBuf, Option<TextureHandle>>,
    world: &WorldSummary,
) -> egui::Response {
    let frame = egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            match world
                .icon
                .as_ref()
                .and_then(|path| load_texture(ui, textures, path))
            {
                Some(texture) => {
                    ui.add(
                        egui::Image::new(&texture)
                            .fit_to_exact_size(egui::vec2(48.0, 48.0))
                            .corner_radius(4),
                    );
                }
                None => {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 4, Color32::from_rgb(38, 42, 51));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        world
                            .edition
                            .label()
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_string(),
                        egui::FontId::proportional(20.0),
                        Color32::from_rgb(124, 189, 66),
                    );
                }
            }
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.strong(&world.name);
                ui.horizontal(|ui| {
                    ui.weak(world.edition.label());
                    if let Some(data_version) = world.data_version {
                        ui.weak(format!("· data v{data_version}"));
                    }
                });
                ui.weak(format!(
                    "{} · {}",
                    human_size(world.size_bytes),
                    format_last_played(world.last_played_ms)
                ));
            });
        });
    });
    frame
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Click to open this world")
}

/// Decode an image file and cache it as a GPU texture (`None` marks a
/// decode failure so we only try once per file).
fn load_texture(
    ui: &egui::Ui,
    cache: &mut HashMap<PathBuf, Option<TextureHandle>>,
    path: &Path,
) -> Option<TextureHandle> {
    if let Some(entry) = cache.get(path) {
        return entry.clone();
    }
    let loaded = (|| {
        let rgba = image::open(path).ok()?.to_rgba8();
        let (width, height) = rgba.dimensions();
        let image = ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
        Some(
            ui.ctx()
                .load_texture(path.display().to_string(), image, TextureOptions::LINEAR),
        )
    })();
    cache.insert(path.to_path_buf(), loaded.clone());
    loaded
}

/// Human-readable byte size (B/KB/MB/GB).
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// Format an epoch-milliseconds timestamp as `YYYY-MM-DD`.
fn format_last_played(ms: Option<i64>) -> String {
    match ms {
        Some(ms) if ms > 0 => {
            let (year, month, day) = civil_from_days(ms / 86_400_000);
            format!("{year:04}-{month:02}-{day:02}")
        }
        _ => "never played".to_owned(),
    }
}

/// Days-from-epoch to (year, month, day) — Howard Hinnant's algorithm,
/// avoiding a chrono dependency for one label.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_scales() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 1_700_000_000 s = 2023-11-14 (UTC).
        assert_eq!(civil_from_days(1_700_000_000 / 86_400), (2023, 11, 14));
    }

    #[test]
    fn last_played_formats_and_handles_absent() {
        assert_eq!(format_last_played(None), "never played");
        assert_eq!(format_last_played(Some(0)), "never played");
        assert_eq!(format_last_played(Some(1_700_000_000_000)), "2023-11-14");
    }
}
