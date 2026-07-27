//! Project Bedrock — application entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod background_media;
mod loader;

use app::BedrockApp;
use bedrock_settings::{log_path, Settings};
use bedrock_ui::log::{self, LogBuffer};

fn main() -> eframe::Result<()> {
    let log_buffer = LogBuffer::new(500);
    log::init(log_buffer.clone(), &log_path());
    tracing::info!("Project Bedrock v{} starting", env!("CARGO_PKG_VERSION"));

    let settings = Settings::load();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Project Bedrock")
            .with_app_id("project-bedrock")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0])
            .with_icon(load_icon()),
        renderer: eframe::Renderer::Wgpu,
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "Project Bedrock",
        native_options,
        Box::new(move |cc| Ok(Box::new(BedrockApp::new(cc, settings, log_buffer)))),
    )
}

/// Decode the bundled application icon for the window and taskbar.
fn load_icon() -> egui::IconData {
    let bytes = include_bytes!("../../../assets/icon.png");
    let image = image::load_from_memory(bytes).expect("bundled assets/icon.png must be valid");
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    }
}
