//! # bedrock-parser — Phase 2–3
//!
//! Reads Minecraft saves: Java Edition (Anvil region files, NBT) and Bedrock
//! Edition. Produces plain parsed data consumed by `bedrock-render` and
//! `bedrock-export`.
//!
//! Hard boundary: no UI, no GPU, no export logic in this crate.
//!
//! Current scope (Phase 2): world detection and `level.dat` metadata.
//! Phase 3 adds chunk/block parsing on top.

pub mod assets_extractor;
pub mod bedrock;
/// Mineways gBlockDefinitions derived data (auto-generated from blockInfo.cpp).
pub mod block_definitions;
/// Per-face texture resolution for vanilla blocks.
pub mod block_model;
/// Per-block-type geometry generation (like Mineways' gBlockDefinitions[]).
pub mod block_models;
pub mod block_shape;
pub mod block_shapes;
pub mod blocks;
pub mod chunk;
pub mod detect;
#[cfg(test)]
mod diagnostic_test;
/// Real texture extraction from the Minecraft Java Edition client JAR.
pub mod jar_textures;
/// Chunkforge-core-based Java world loading (simple, correct approach).
pub mod java_simple;
pub mod json_geometry;
pub mod json_model;
pub mod level;
/// Parser for Minecraft Bedrock Edition `.mcstructure` files.
pub mod mcstructure;
/// Mineways-compatible block-to-texture mapping (terrainExt.png atlas).
pub mod mineways;
/// Mineways tile table data (auto-generated from gTilesTable[]).
pub mod mineways_data;
pub mod nbt_le;
pub mod region;
pub mod texture;
pub mod texture_animation;
pub mod world;
