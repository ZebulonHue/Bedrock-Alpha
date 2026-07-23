//! Visual theme: dark-first, rounded corners, comfortable spacing, and a
//! grass-green accent taken from the Minecraft atlas — subtle, never childish.

use bedrock_settings::Theme;
use egui::{Color32, CornerRadius, Stroke, Style, Visuals};

/// Atlas-inspired grass green used for accents and selection.
const ACCENT: Color32 = Color32::from_rgb(124, 189, 66);

/// Apply a theme to the egui context.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    match theme {
        Theme::Dark => {
            ctx.set_style_of(egui::Theme::Dark, styled(Visuals::dark(), true));
            ctx.set_theme(egui::ThemePreference::Dark);
        }
        Theme::Light => {
            ctx.set_style_of(egui::Theme::Light, styled(Visuals::light(), false));
            ctx.set_theme(egui::ThemePreference::Light);
        }
    }
}

/// Shared styling on top of an egui base visuals set.
fn styled(mut visuals: Visuals, dark: bool) -> Style {
    if dark {
        visuals.window_fill = Color32::from_rgb(24, 27, 33);
        visuals.panel_fill = Color32::from_rgb(21, 23, 28);
        visuals.extreme_bg_color = Color32::from_rgb(14, 15, 19);
        visuals.faint_bg_color = Color32::from_rgb(32, 36, 44);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 42, 51);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(46, 51, 62);
        visuals.widgets.active.bg_fill = Color32::from_rgb(54, 60, 73);
    }

    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;

    let radius = CornerRadius::same(6);
    visuals.widgets.noninteractive.corner_radius = radius;
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.open.corner_radius = radius;

    let mut style = Style {
        visuals,
        ..Default::default()
    };
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12);
    style
}
