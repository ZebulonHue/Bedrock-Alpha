//! # bedrock-blender — Phase 6
//!
//! Blender-side pipeline: import conventions, MCPrep compatibility, origin
//! and material cleanup, collection organisation, and the "Project Bedrock
//! Blender Tools" add-on generator for a one-click workflow.
//!
//! Hard boundary: no UI, no world parsing in this crate.
//!
//! # Modules
//!
//! - [`material`] — Material naming conventions, PBR presets for common
//!   Minecraft blocks.
//! - [`collection`] — Collection hierarchy definitions for organising
//!   imported meshes.
//! - [`addon`] — Generator that produces a complete Blender Python add-on
//!   for one-click import of Project Bedrock exports.

pub mod addon;
pub mod collection;
pub mod material;
