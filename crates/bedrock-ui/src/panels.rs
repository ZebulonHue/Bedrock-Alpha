//! The individual dock panels. Pure presentation — each panel receives plain
//! data and edits it in place. No business logic lives here.

use crate::log::{Level, LogBuffer};
use bedrock_export::obj::ExportRegion;
use bedrock_render::SharedScene;
use bedrock_settings::{DebugSettings, ExportFormat, ExportPreferences, ExportPreset};
use egui::{Color32, RichText, ScrollArea};
use std::sync::{Arc, Mutex};

/// Central GPU viewport (rendered by `bedrock-render`).
pub fn viewport(ui: &mut egui::Ui, scene: &Arc<Mutex<SharedScene>>) {
    bedrock_render::show_viewport(ui, scene);
}

/// What the 2D Overview panel needs: the top-down map texture and the
/// world coordinates of its top-left pixel.
#[derive(Clone, Copy)]
pub struct OverviewData<'a> {
    /// One-pixel-per-block color map of the loaded area.
    pub texture: &'a egui::TextureHandle,
    /// World `[x, z]` of the image's top-left pixel.
    pub origin: [i32; 2],
}

/// 2D overview: top-down map of the loaded world with the export region
/// drawn as a rectangle.
pub fn overview(ui: &mut egui::Ui, data: Option<OverviewData>, region: &ExportRegion) {
    let Some(data) = data else {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.weak("Open a world to see its map.");
        });
        return;
    };

    let [width, height] = data.texture.size();
    let (outer, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let scale = (outer.width() / width as f32)
        .min(outer.height() / height as f32)
        .max(0.01);
    let rect = egui::Rect::from_center_size(
        outer.center(),
        egui::vec2(width as f32 * scale, height as f32 * scale),
    );
    let painter = ui.painter();
    painter.image(
        data.texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );

    // Export region rectangle (world x/z → screen).
    let to_screen = |wx: i32, wz: i32| {
        egui::pos2(
            rect.left() + (wx - data.origin[0]) as f32 * scale,
            rect.top() + (wz - data.origin[1]) as f32 * scale,
        )
    };
    let region_rect = egui::Rect::from_two_pos(
        to_screen(region.min[0], region.min[2]),
        to_screen(region.max[0], region.max[2]),
    );
    painter.rect_stroke(
        region_rect,
        0.0,
        egui::Stroke::new(2.0, Color32::from_rgb(140, 242, 89)),
        egui::StrokeKind::Outside,
    );
}

/// Properties placeholder — region selection arrives in Phase 5.
pub fn properties(ui: &mut egui::Ui) {
    ui.heading("Properties");
    ui.separator();
    ui.weak("Nothing selected.");
    ui.weak("Region selection arrives in Phase 5.");
}

/// Export preferences and region panel — fully wired to the export system.
pub fn export_settings(
    ui: &mut egui::Ui,
    prefs: &mut ExportPreferences,
    region: &mut ExportRegion,
    world_loaded: bool,
    export_requested: &mut bool,
) {
    ui.heading("Export Settings");
    ui.add_space(4.0);

    egui::ComboBox::from_label("Preset")
        .selected_text(prefs.preset.label())
        .show_ui(ui, |ui| {
            for preset in ExportPreset::ALL {
                ui.selectable_value(&mut prefs.preset, preset, preset.label());
            }
        });

    egui::ComboBox::from_label("Format")
        .selected_text(prefs.format.label())
        .show_ui(ui, |ui| {
            for format in ExportFormat::ALL {
                if format.is_supported() {
                    ui.selectable_value(&mut prefs.format, format, format.label());
                } else {
                    ui.add_enabled(false, egui::Button::selectable(false, format.label()))
                        .on_disabled_hover_text("Planned for a future release");
                }
            }
        });

    ui.add_space(4.0);
    ui.label("Output directory");
    ui.add(
        egui::TextEdit::singleline(&mut prefs.output_dir)
            .hint_text("Documents\\Project Bedrock")
            .desired_width(f32::INFINITY),
    );

    ui.add_space(8.0);
    ui.label("Region (world block coordinates)");
    ui.horizontal(|ui| {
        ui.weak("min");
        for (axis, label) in ["X", "Y", "Z"].iter().enumerate() {
            ui.add(egui::DragValue::new(&mut region.min[axis]).prefix(format!("{label} ")));
        }
    });
    ui.horizontal(|ui| {
        ui.weak("max");
        for (axis, label) in ["X", "Y", "Z"].iter().enumerate() {
            ui.add(egui::DragValue::new(&mut region.max[axis]).prefix(format!("{label} ")));
        }
    });
    let size = [
        region.max[0] - region.min[0],
        region.max[1] - region.min[1],
        region.max[2] - region.min[2],
    ];
    if size.iter().all(|s| *s > 0) {
        ui.weak(format!("{} × {} × {} blocks", size[0], size[1], size[2]));
    } else {
        ui.colored_label(
            Color32::from_rgb(224, 108, 117),
            "Invalid region: max must be above min on every axis",
        );
    }

    ui.add_space(8.0);
    let valid = world_loaded && size.iter().all(|s| *s > 0);
    ui.add_enabled(
        valid,
        egui::Button::new(format!("Export {}", prefs.format.label()))
            .min_size(egui::vec2(ui.available_width(), 30.0)),
    )
    .on_disabled_hover_text(if world_loaded {
        "Fix the region bounds first"
    } else {
        "Open a world first"
    })
    .clicked()
    .then(|| *export_requested = true);
    ui.add_space(4.0);
    match prefs.format {
        ExportFormat::Obj => {
            ui.weak(
                "Writes a .obj + .mtl with one material per block type, centered on the origin.",
            );
        }
        ExportFormat::Gltf => {
            ui.weak("Writes .gltf + .bin + atlas.png with PBR materials (roughness, metallic, emissive).");
        }
        ExportFormat::Fbx | ExportFormat::Usd => {
            ui.weak("Planned for a future release.");
        }
    }
}

/// Output Log panel — the human-readable application log.
pub fn output_log(ui: &mut egui::Ui, log: &LogBuffer, auto_scroll: &mut bool) {
    let lines = log.lines();
    ui.horizontal(|ui| {
        ui.weak(format!("{} entries", lines.len()));
        ui.separator();
        ui.checkbox(auto_scroll, "Auto-scroll");
        if ui.button("Clear").clicked() {
            log.clear();
        }
    });
    ui.separator();

    let mut scroll = ScrollArea::vertical().auto_shrink([false, false]);
    if *auto_scroll {
        scroll = scroll.stick_to_bottom(true);
    }
    scroll.show(ui, |ui| {
        for (level, line) in &lines {
            ui.colored_label(
                level_color(*level),
                RichText::new(line).monospace().size(12.0),
            );
        }
    });
}

/// Debug visualisation settings panel.
pub fn debug_settings(ui: &mut egui::Ui, debug: &mut DebugSettings) {
    ui.heading("Debug Visualizations");
    ui.separator();

    ui.label("Overlays");
    ui.checkbox(&mut debug.show_stats, "Stats (FPS, verts, tris)");
    ui.checkbox(&mut debug.show_chunk_borders, "Chunk borders (light blue)");
    ui.checkbox(&mut debug.show_wireframe, "Wireframe overlay (orange)");

    ui.add_space(8.0);
    ui.separator();
    ui.weak("Keyboard shortcuts:");
    ui.weak("F1 — snap camera to player position");
    ui.weak("F3 — toggle stats overlay");
    ui.weak("F4 — toggle chunk borders");
    ui.weak("F5 — toggle wireframe");
    ui.add_space(4.0);
    ui.weak("Debug overlays never modify export output.");
}

/// Color for a log level, tuned for both dark and light themes.
fn level_color(level: Level) -> Color32 {
    match level {
        Level::Error => Color32::from_rgb(224, 108, 117),
        Level::Warn => Color32::from_rgb(229, 192, 123),
        Level::Info => Color32::from_rgb(152, 195, 121),
        Level::Debug => Color32::from_gray(150),
        Level::Trace => Color32::from_gray(110),
    }
}
