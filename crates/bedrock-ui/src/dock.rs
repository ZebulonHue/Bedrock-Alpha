//! Dockable panel layout: the panel tab enum, the default layout, and the
//! [`TabViewer`] that routes each tab to its panel.

use crate::log::LogBuffer;
use crate::panels;
use crate::world_browser::{self, WorldBrowserState};
use bedrock_export::obj::ExportRegion;
use bedrock_render::SharedScene;
use bedrock_settings::{DebugSettings, ExportPreferences};
use egui_dock::{DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Every dockable panel in the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Panel {
    /// Central 3D view (GPU-rendered).
    Viewport,
    /// World list with thumbnails.
    WorldBrowser,
    /// Top-down 2D map (Phase 3–4).
    OverviewMap,
    /// Details about the current selection.
    Properties,
    /// Export preset, format, and output options.
    ExportSettings,
    /// Human-readable application log.
    OutputLog,
    /// Debug visualisation settings (Phase 6b).
    DebugSettings,
}

impl Panel {
    /// Every panel, for the View menu.
    pub const ALL: [Panel; 7] = [
        Panel::Viewport,
        Panel::WorldBrowser,
        Panel::OverviewMap,
        Panel::Properties,
        Panel::ExportSettings,
        Panel::OutputLog,
        Panel::DebugSettings,
    ];

    /// Tab title.
    pub fn title(self) -> &'static str {
        match self {
            Panel::Viewport => "Viewport",
            Panel::WorldBrowser => "World Browser",
            Panel::OverviewMap => "2D Overview",
            Panel::Properties => "Properties",
            Panel::ExportSettings => "Export Settings",
            Panel::OutputLog => "Output Log",
            Panel::DebugSettings => "Debug",
        }
    }
}

/// The default Blender-style layout: browser left, viewport center,
/// properties/export right, log along the bottom.
pub fn default_layout() -> DockState<Panel> {
    let mut state = DockState::new(vec![Panel::Viewport, Panel::OverviewMap]);
    let tree = state.main_surface_mut();
    // egui_dock split semantics: `fraction` is the share kept by the OLD node
    // and the returned indices are [old, new].
    let [main, _browser] = tree.split_left(NodeIndex::root(), 0.80, vec![Panel::WorldBrowser]);
    let [center, _right] =
        tree.split_right(main, 0.74, vec![Panel::Properties, Panel::ExportSettings]);
    let [_center, _log] = tree.split_below(center, 0.75, vec![Panel::OutputLog]);
    state
}

/// True if the panel currently exists anywhere in the layout.
pub fn is_open(dock: &DockState<Panel>, panel: Panel) -> bool {
    dock.find_tab(&panel).is_some()
}

/// Toggle a panel's visibility (used by the View menu).
pub fn toggle(dock: &mut DockState<Panel>, panel: Panel) {
    if let Some(path) = dock.find_tab(&panel) {
        dock.remove_tab(path);
    } else {
        dock.add_window(vec![panel]);
    }
}

/// Everything a panel needs while rendering, bundled once per frame.
pub struct PanelContext<'a> {
    /// Export preferences, edited in place by the Export Settings panel.
    pub export: &'a mut ExportPreferences,
    /// Application log shown by the Output Log panel.
    pub log: &'a LogBuffer,
    /// Whether the Output Log follows new entries.
    pub auto_scroll_log: &'a mut bool,
    /// World detection state shown by the World Browser panel.
    pub world_browser: &'a mut WorldBrowserState,
    /// Shared 3D scene state (camera + pending meshes) for the viewport.
    pub viewport_scene: &'a Arc<Mutex<SharedScene>>,
    /// Export region bounds, edited by the Export Settings panel.
    pub export_region: &'a mut ExportRegion,
    /// Whether a world is loaded and can be exported.
    pub world_loaded: bool,
    /// Set by the Export button; the app consumes it.
    pub export_requested: &'a mut bool,
    /// Top-down map for the 2D Overview panel (None until a world loads).
    pub overview: Option<panels::OverviewData<'a>>,
    /// Debug visualisation settings.
    pub debug: &'a mut DebugSettings,
}

impl TabViewer for PanelContext<'_> {
    type Tab = Panel;

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Panel) {
        match tab {
            Panel::Viewport => panels::viewport(ui, self.viewport_scene),
            Panel::WorldBrowser => world_browser::world_browser(ui, self.world_browser),
            Panel::OverviewMap => panels::overview(ui, self.overview, self.export_region),
            Panel::Properties => panels::properties(ui),
            Panel::ExportSettings => panels::export_settings(
                ui,
                self.export,
                self.export_region,
                self.world_loaded,
                self.export_requested,
            ),
            Panel::OutputLog => panels::output_log(ui, self.log, self.auto_scroll_log),
            Panel::DebugSettings => panels::debug_settings(ui, self.debug),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_contains_expected_panels() {
        let dock = default_layout();
        for panel in Panel::ALL {
            let count = dock
                .iter_all_tabs()
                .filter(|(_, tab)| **tab == panel)
                .count();
            // DebugSettings is not in the default layout — it is opened
            // via the View menu or F3/F4/F5 shortcuts.
            if panel == Panel::DebugSettings {
                assert_eq!(count, 0, "{panel:?} should NOT appear by default");
            } else {
                assert_eq!(count, 1, "{panel:?} should appear exactly once");
            }
        }
    }

    #[test]
    fn toggle_removes_and_readds_a_panel() {
        let mut dock = default_layout();
        toggle(&mut dock, Panel::Properties);
        assert!(!is_open(&dock, Panel::Properties));
        toggle(&mut dock, Panel::Properties);
        assert!(is_open(&dock, Panel::Properties));
    }
}
