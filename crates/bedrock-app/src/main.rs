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
    install_panic_logger();
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

/// Write panics to the log file before the process dies.
///
/// A release build is a `windows_subsystem = "windows"` binary with no console
/// attached, so the default panic message goes nowhere at all: the app
/// vanishes and the log simply stops mid-sentence. That is precisely the
/// report we get from testers -- "it crashes" -- with nothing to act on.
/// Chaining rather than replacing keeps the standard behaviour for anyone
/// running it from a terminal.
fn install_panic_logger() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_owned());
        tracing::error!("PANIC at {location}: {}", panic_message(info));
        previous(info);
    }));
}

/// The human-readable part of a panic payload.
fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_owned()
    }
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
