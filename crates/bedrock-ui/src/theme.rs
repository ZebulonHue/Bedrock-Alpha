//! Visual theme.
//!
//! Colours are sampled from the app's design mock-up rather than invented, so
//! the built app matches the artwork instead of drifting from it. The palette
//! is a near-neutral dark green-grey — Minecraft's dirt-and-grass world read
//! at very low saturation — with plain white as the accent. Colour is left to
//! the pixel art and the world itself.

use bedrock_settings::Theme;
use egui::{Color32, CornerRadius, Stroke, Style, Visuals};

// ── Palette, sampled from UI-layout_Mockup.png ──────────────────────────────

/// Main content area behind the viewport and panels.
const BG_APP: Color32 = Color32::from_rgb(0x22, 0x23, 0x21);
/// Left sidebar, a touch darker than the content it sits beside.
const BG_SIDEBAR: Color32 = Color32::from_rgb(0x1E, 0x20, 0x1E);
/// Inset fields — search boxes, text entry, log backgrounds.
const BG_INSET: Color32 = Color32::from_rgb(0x18, 0x1A, 0x18);
/// Raised surfaces: cards, menus, dialogs.
const BG_CARD: Color32 = Color32::from_rgb(0x2E, 0x2F, 0x2B);
/// Hover state for a card or row.
const BG_HOVER: Color32 = Color32::from_rgb(0x38, 0x39, 0x34);
/// Pressed / open state.
const BG_ACTIVE: Color32 = Color32::from_rgb(0x44, 0x45, 0x3F);

/// Primary text.
const TEXT: Color32 = Color32::from_rgb(0xF9, 0xF9, 0xF9);
/// Section labels and secondary text.
const TEXT_DIM: Color32 = Color32::from_rgb(0x9A, 0x9C, 0x96);

/// Selection and emphasis. The mock-up underlines the active tab and fills the
/// selected sidebar row in near-white rather than a colour.
const ACCENT: Color32 = Color32::from_rgb(0xE8, 0xE8, 0xE8);
/// Grass green, kept for the export region box and other world-space markers
/// where it has to read against terrain rather than against the UI.
pub const WORLD_ACCENT: Color32 = Color32::from_rgb(0x7C, 0xBD, 0x42);

/// Secondary text colour, for callers laying out their own labels.
pub const MUTED: Color32 = TEXT_DIM;
/// Sidebar fill, for panels that paint their own background.
pub const SIDEBAR: Color32 = BG_SIDEBAR;
/// Raised-surface fill, for cards drawn by hand.
pub const CARD: Color32 = BG_CARD;

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
        visuals.window_fill = BG_CARD;
        visuals.panel_fill = BG_APP;
        visuals.extreme_bg_color = BG_INSET;
        visuals.faint_bg_color = BG_SIDEBAR;

        visuals.widgets.noninteractive.bg_fill = BG_APP;
        visuals.widgets.inactive.bg_fill = BG_CARD;
        visuals.widgets.hovered.bg_fill = BG_HOVER;
        visuals.widgets.active.bg_fill = BG_ACTIVE;
        visuals.widgets.open.bg_fill = BG_ACTIVE;

        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);

        // The mock-up has almost no visible borders; surfaces separate by
        // their fill alone. Keeping faint strokes would read as busy beside it.
        let hairline = Stroke::new(1.0, Color32::from_rgb(0x33, 0x35, 0x31));
        visuals.widgets.noninteractive.bg_stroke = hairline;
        visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        visuals.widgets.hovered.bg_stroke = Stroke::NONE;
        visuals.widgets.active.bg_stroke = Stroke::NONE;
        visuals.window_stroke = hairline;

        visuals.panel_fill = BG_APP;
        visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(120),
        };
    }

    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.22);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;

    // Softer than before: the mock-up's cards and fields are gently rounded,
    // and the window itself more so.
    let radius = CornerRadius::same(8);
    visuals.widgets.noninteractive.corner_radius = radius;
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.open.corner_radius = radius;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);

    let mut style = Style {
        visuals,
        ..Default::default()
    };
    // Roomier than the default: the mock-up breathes, with generous gaps
    // between sidebar rows and around content.
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.window_margin = egui::Margin::same(14);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.indent = 18.0;
    style
}
