//! Floating windows: Settings and About.

use bedrock_settings::{Settings, Theme, MAX_LOAD_RADIUS_CHUNKS, MIN_LOAD_RADIUS_CHUNKS};

/// Show the Settings window. Returns `true` if the user asked to reset the
/// dock layout (the app applies the reset, since it owns the dock state).
pub fn settings(ctx: &egui::Context, settings: &mut Settings, open: &mut bool) -> bool {
    let mut reset_layout = false;
    egui::Window::new("Settings")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.heading("Appearance");
            egui::ComboBox::from_label("Theme")
                .selected_text(settings.theme.label())
                .show_ui(ui, |ui| {
                    for theme in Theme::ALL {
                        ui.selectable_value(&mut settings.theme, theme, theme.label());
                    }
                });
            ui.checkbox(&mut settings.show_fps, "Show FPS in the status bar");

            ui.add_space(8.0);
            ui.heading("World Loading");
            ui.add(
                egui::Slider::new(
                    &mut settings.load_radius_chunks,
                    MIN_LOAD_RADIUS_CHUNKS..=MAX_LOAD_RADIUS_CHUNKS,
                )
                .text("Load radius (chunks)"),
            );
            ui.weak(
                "Chunks loaded around the player when opening a world. Larger \
                 radii cover more of the map but use more memory and take \
                 longer to load. Takes effect the next time you open a world.",
            );

            ui.add_space(8.0);
            ui.heading("Layout");
            if ui.button("Reset panel layout").clicked() {
                reset_layout = true;
            }

            ui.add_space(8.0);
            ui.weak("Settings are saved automatically.");
        });
    reset_layout
}

/// Show the About window.
pub fn about(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("About Project Bedrock")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.heading("Project Bedrock");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(4.0);
            ui.label("A modern Minecraft world exporter for Blender.");
            ui.weak("A professional desktop tool for artists — not a Minecraft clone.");
            ui.add_space(8.0);
            ui.weak("Minecraft is a trademark of Mojang Synergies AB.");
            ui.weak("Project Bedrock is not affiliated with or endorsed by Mojang or Microsoft.");
        });
}
