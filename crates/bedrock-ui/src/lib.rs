//! # bedrock-ui
//!
//! The Project Bedrock UI layer: theme, dockable panel layout, the panels
//! themselves, floating windows, and the log view.
//!
//! Hard boundary: pure presentation. This crate never parses worlds, never
//! talks to the GPU directly, and never contains business logic — panels
//! receive plain data (`&mut ExportPreferences`, `&LogBuffer`, …) and edit it
//! in place.

pub mod dock;
pub mod log;
pub mod panels;
pub mod theme;
pub mod windows;
pub mod world_browser;
