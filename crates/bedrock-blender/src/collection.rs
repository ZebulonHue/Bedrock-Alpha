//! Collection hierarchy conventions for organising imported Project Bedrock
//! meshes inside Blender.
//!
//! When an export is imported, the geometry can be organised into Blender
//! collections according to different schemes. This module defines those
//! schemes and provides utility functions for naming collections.

/// Strategy for organising imported geometry into Blender collections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollectionScheme {
    /// All objects placed in a single top-level collection.
    Flat,
    /// Objects grouped by chunk coordinate (e.g. `Chunk_0_0`, `Chunk_0_1`).
    ByChunk,
    /// Objects grouped by block type (e.g. `stone`, `oak_planks`, `glass`).
    ByBlockType,
    /// Objects grouped by Y-layer (vertical slice, e.g. `Layer_4`, `Layer_5`).
    ByYLayer,
    /// Objects grouped by chunk first, then by block type within each chunk.
    #[default]
    ChunkThenType,
}

/// A level in a hierarchical collection path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionLevel {
    /// Named collection.
    Named(String),
    /// Chunk coordinate `(x, z)`.
    Chunk(i32, i32),
    /// Block type name.
    BlockType(String),
    /// Y-layer index.
    YLayer(i32),
    /// Region label.
    Region(i32, i32),
}

impl CollectionLevel {
    /// Produce the Blender collection name for this level.
    pub fn collection_name(&self) -> String {
        match self {
            CollectionLevel::Named(name) => name.clone(),
            CollectionLevel::Chunk(cx, cz) => format!("Chunk_{cx}_{cz}"),
            CollectionLevel::BlockType(name) => format!("BlockType_{name}"),
            CollectionLevel::YLayer(y) => format!("Layer_{y}"),
            CollectionLevel::Region(rx, rz) => format!("Region_{rx}_{rz}"),
        }
    }

    /// Produce a dashed identifier suitable for Python variable names.
    pub fn python_identifier(&self) -> String {
        match self {
            CollectionLevel::Named(name) => name.clone(),
            CollectionLevel::Chunk(cx, cz) => format!("chunk_{cx}_{cz}"),
            CollectionLevel::BlockType(name) => format!("block_type_{name}"),
            CollectionLevel::YLayer(y) => format!("layer_{y}"),
            CollectionLevel::Region(rx, rz) => format!("region_{rx}_{rz}"),
        }
    }
}

/// Build a collection hierarchy path for a given block.
///
/// Returns a list of collection names from top to bottom, representing the
/// nesting of collections that should contain this block's object.
///
/// # Example
///
/// ```
/// use bedrock_blender::collection::{
///     build_collection_path, CollectionScheme,
/// };
/// let path = build_collection_path(
///     CollectionScheme::ChunkThenType,
///     "stone", 0, 4, 0,
/// );
/// assert_eq!(path.len(), 3);
/// assert_eq!(path[0], "Bedrock Export");
/// assert_eq!(path[1], "Chunk_0_0");
/// assert_eq!(path[2], "BlockType_stone");
/// ```
pub fn build_collection_path(
    scheme: CollectionScheme,
    block_type: &str,
    cx: i32,
    cy: i32,
    cz: i32,
) -> Vec<String> {
    const ROOT: &str = "Bedrock Export";
    match scheme {
        CollectionScheme::Flat => vec![ROOT.to_owned()],
        CollectionScheme::ByChunk => {
            vec![
                ROOT.to_owned(),
                CollectionLevel::Chunk(cx, cz).collection_name(),
            ]
        }
        CollectionScheme::ByBlockType => {
            vec![
                ROOT.to_owned(),
                CollectionLevel::BlockType(block_type.to_string()).collection_name(),
            ]
        }
        CollectionScheme::ByYLayer => {
            vec![
                ROOT.to_owned(),
                CollectionLevel::YLayer(cy).collection_name(),
            ]
        }
        CollectionScheme::ChunkThenType => {
            vec![
                ROOT.to_owned(),
                CollectionLevel::Chunk(cx, cz).collection_name(),
                CollectionLevel::BlockType(block_type.to_string()).collection_name(),
            ]
        }
    }
}

/// Generate Python code that creates the collection hierarchy.
///
/// Returns a string of Python statements that, when executed in Blender,
/// ensure the given collections exist (creating them as needed).
pub fn generate_collection_python(path: &[String]) -> String {
    if path.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    let indent = String::new();

    // Ensure the root collection exists.
    lines.push(format!(
        r#"root = bpy.data.collections.get("{root}") or bpy.data.collections.new("{root}")"#,
        root = path[0]
    ));

    let mut parent_var = "root".to_string();
    for segment in &path[1..] {
        let var_name = segment
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .to_lowercase();

        lines.push(format!(
            r#"{indent}{var} = bpy.data.collections.get("{seg}") or bpy.data.collections.new("{seg}")"#,
            indent = indent,
            var = var_name,
            seg = segment,
        ));
        lines.push(format!(
            r#"{indent}if {var}.name not in {parent}.children:"#,
            indent = indent,
            var = var_name,
            parent = parent_var,
        ));
        lines.push(format!(
            r#"{indent}    {parent}.children.link({var})"#,
            indent = indent,
            parent = parent_var,
            var = var_name,
        ));
        parent_var = var_name;
    }

    lines.push(format!(
        r#"collection = {parent_var}"#,
        parent_var = parent_var
    ));

    lines.join("\n")
}

/// Generate Python code to link an object into a collection.
///
/// `obj_name` is the Blender object name; `collection_var` is a Python
/// variable name holding the collection.
pub fn link_object_python(obj_name: &str, collection_var: &str) -> String {
    format!(
        r#"if "{name}" in bpy.data.objects:
    obj = bpy.data.objects["{name}"]
    if obj.name not in {col}.objects:
        {col}.objects.link(obj)
        if obj.users_collection:
            for c in obj.users_collection:
                if c != {col}:
                    c.objects.unlink(obj)"#,
        name = obj_name,
        col = collection_var,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_scheme_has_single_collection() {
        let path = build_collection_path(CollectionScheme::Flat, "stone", 0, 4, 0);
        assert_eq!(path, vec!["Bedrock Export"]);
    }

    #[test]
    fn by_chunk_includes_chunk_name() {
        let path = build_collection_path(CollectionScheme::ByChunk, "stone", 2, 4, -3);
        assert_eq!(path[1], "Chunk_2_-3");
    }

    #[test]
    fn by_block_type_includes_type() {
        let path = build_collection_path(CollectionScheme::ByBlockType, "oak_planks", 0, 4, 0);
        assert_eq!(path[1], "BlockType_oak_planks");
    }

    #[test]
    fn by_y_layer_includes_layer() {
        let path = build_collection_path(CollectionScheme::ByYLayer, "stone", 0, 12, 0);
        assert_eq!(path[1], "Layer_12");
    }

    #[test]
    fn chunk_then_type_is_hierarchical() {
        let path = build_collection_path(CollectionScheme::ChunkThenType, "stone", 1, 4, 2);
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], "Bedrock Export");
        assert_eq!(path[1], "Chunk_1_2");
        assert_eq!(path[2], "BlockType_stone");
    }

    #[test]
    fn collection_level_name_chunk() {
        let level = CollectionLevel::Chunk(3, -5);
        assert_eq!(level.collection_name(), "Chunk_3_-5");
    }

    #[test]
    fn collection_level_name_block_type() {
        let level = CollectionLevel::BlockType("glass".into());
        assert_eq!(level.collection_name(), "BlockType_glass");
    }

    #[test]
    fn generate_python_produces_valid_code() {
        let path = vec![
            "Bedrock Export".into(),
            "Chunk_0_0".into(),
            "BlockType_stone".into(),
        ];
        let code = generate_collection_python(&path);
        assert!(code.contains("Bedrock Export"));
        assert!(code.contains("Chunk_0_0"));
        assert!(code.contains("BlockType_stone"));
        assert!(code.contains("bpy.data.collections.get"));
        assert!(code.contains("bpy.data.collections.new"));
        // Ends with the collection variable assignment
        assert!(code.ends_with("collection = blocktype_stone"));
    }
}
