use crate::assets_extractor::VanillaAssets;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct BlockstateVariant {
    pub model: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub uvlock: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockstateMultipartCondition {
    #[serde(flatten)]
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MultipartApply {
    Single(BlockstateVariant),
    Multiple(Vec<BlockstateVariant>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockstateMultipart {
    pub when: Option<BlockstateMultipartCondition>,
    pub apply: MultipartApply,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Blockstate {
    pub variants: Option<HashMap<String, serde_json::Value>>, // Can be single variant or array of variants
    pub multipart: Option<Vec<BlockstateMultipart>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelFace {
    pub texture: String,
    pub cullface: Option<String>,
    pub uv: Option<[f64; 4]>,
    pub rotation: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelRotation {
    pub origin: [f64; 3],
    pub axis: String,
    pub angle: f64,
    pub rescale: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelElement {
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub rotation: Option<ModelRotation>,
    pub faces: HashMap<String, ModelFace>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub parent: Option<String>,
    pub textures: Option<HashMap<String, String>>,
    pub elements: Option<Vec<ModelElement>>,
}
/// A resolved element: a model element plus its rotation and texture map.
pub type ResolvedElement = (ModelElement, i32, i32, HashMap<String, String>);

/// Resolves face textures from vanilla JSON definitions.
pub struct BlockModelResolver {
    /// Resolved textures for each block name (e.g. "grass_block" -> FaceTextures)
    pub block_textures: HashMap<String, crate::block_model::FaceTextures>,
    pub parsed_models: HashMap<String, Model>,
    pub parsed_blockstates: HashMap<String, Blockstate>,
    pub element_cache: std::sync::RwLock<HashMap<String, std::sync::Arc<Vec<ResolvedElement>>>>,
}

impl BlockModelResolver {
    pub fn new(assets: &VanillaAssets) -> Self {
        let mut resolver = Self {
            block_textures: HashMap::new(),
            parsed_models: HashMap::new(),
            parsed_blockstates: HashMap::new(),
            element_cache: std::sync::RwLock::new(HashMap::new()),
        };
        resolver.build(assets);
        resolver
    }

    fn build(&mut self, assets: &VanillaAssets) {
        // Parse all models first
        for (name, json) in &assets.models {
            if let Ok(model) = serde_json::from_str::<Model>(json) {
                self.parsed_models.insert(name.clone(), model);
            }
        }

        // Parse blockstates and resolve
        for (block_name, json) in &assets.blockstates {
            if let Ok(state) = serde_json::from_str::<Blockstate>(json) {
                self.parsed_blockstates
                    .insert(block_name.clone(), state.clone());

                if let Some(variants) = &state.variants {
                    // Try to find the most "default" or upright variant
                    let preferred_variant = variants
                        .get("")
                        .or_else(|| variants.get("normal"))
                        .or_else(|| {
                            variants
                                .iter()
                                .find(|(k, _)| k.contains("axis=y"))
                                .map(|(_, v)| v)
                        })
                        .or_else(|| {
                            variants
                                .iter()
                                .find(|(k, _)| k.contains("snowy=false"))
                                .map(|(_, v)| v)
                        })
                        .or_else(|| {
                            variants
                                .iter()
                                .find(|(k, _)| k.contains("half=lower"))
                                .map(|(_, v)| v)
                        })
                        .or_else(|| {
                            variants
                                .iter()
                                .find(|(k, _)| k.contains("facing=south"))
                                .map(|(_, v)| v)
                        })
                        .or_else(|| variants.values().next());

                    if let Some(variant_val) = preferred_variant {
                        // Variants can be arrays, so take the first if it is
                        let variant_obj = if let Some(arr) = variant_val.as_array() {
                            arr.first().unwrap_or(variant_val)
                        } else {
                            variant_val
                        };

                        if let Ok(variant) =
                            serde_json::from_value::<BlockstateVariant>(variant_obj.clone())
                        {
                            let mut model_name = variant.model.clone();
                            if model_name.starts_with("minecraft:") {
                                model_name = model_name["minecraft:".len()..].to_string();
                            }

                            let face_textures =
                                Self::resolve_textures(&model_name, &self.parsed_models);
                            self.block_textures
                                .insert(block_name.clone(), face_textures);
                        }
                    }
                }
            }
        }
    }

    fn resolve_textures(
        model_name: &str,
        parsed_models: &HashMap<String, Model>,
    ) -> crate::block_model::FaceTextures {
        let mut resolved_textures = HashMap::new();

        // Traverse the parent chain
        let mut current_model_name = Some(model_name.to_string());
        let mut visited = Vec::new();

        while let Some(name) = current_model_name {
            if visited.contains(&name) {
                break;
            } // Prevent cycles
            visited.push(name.clone());

            if let Some(model) = parsed_models.get(&name) {
                if let Some(textures) = &model.textures {
                    for (k, v) in textures {
                        if !resolved_textures.contains_key(k) {
                            resolved_textures.insert(k.clone(), v.clone());
                        }
                    }
                }

                if let Some(parent) = &model.parent {
                    let mut p = parent.clone();
                    if p.starts_with("minecraft:") {
                        p = p["minecraft:".len()..].to_string();
                    }
                    current_model_name = Some(p);
                } else {
                    current_model_name = None;
                }
            } else {
                current_model_name = None;
            }
        }

        // Resolve texture variables (e.g., "#all" -> "block/stone")
        let mut final_textures = HashMap::new();
        for (k, v) in &resolved_textures {
            let mut val = v.clone();
            let mut visited_keys = Vec::new();
            while val.starts_with('#') {
                if visited_keys.contains(&val) {
                    break;
                }
                visited_keys.push(val.clone());
                if let Some(resolved) = resolved_textures.get(&val[1..]) {
                    val = resolved.clone();
                } else {
                    break; // Unresolved variable
                }
            }
            if val.starts_with("minecraft:") {
                val = val["minecraft:".len()..].to_string();
            }
            if val.starts_with("block/") {
                val = val["block/".len()..].to_string();
            }
            final_textures.insert(k.clone(), val);
        }

        let get_tex = |keys: &[&str]| -> String {
            for k in keys {
                if let Some(t) = final_textures.get(*k) {
                    return t.clone();
                }
            }

            let mut fallback = model_name.to_string();
            if fallback.starts_with("block/") {
                fallback = fallback["block/".len()..].to_string();
            }
            fallback // Fallback
        };

        crate::block_model::FaceTextures {
            top: get_tex(&["up", "top", "end", "all", "particle"]),
            bottom: get_tex(&["down", "bottom", "end", "all", "particle"]),
            south: get_tex(&["south", "side", "all", "particle"]),
            north: get_tex(&["north", "side", "all", "particle"]),
            east: get_tex(&["east", "side", "all", "particle"]),
            west: get_tex(&["west", "side", "all", "particle"]),
        }
    }

    /// Fully resolve the elements and textures for a given BlockState.
    pub fn get_elements_for_state(
        &self,
        state: &crate::chunk::BlockState,
    ) -> std::sync::Arc<Vec<ResolvedElement>> {
        let key = state.cache_key();
        if let Ok(cache) = self.element_cache.read() {
            if let Some(cached) = cache.get(&key) {
                return std::sync::Arc::clone(cached);
            }
        }

        let variants = self.match_variants(state);
        let mut resolved_geometry = Vec::new();

        for variant in variants {
            let mut model_name = variant.model;
            if model_name.starts_with("minecraft:") {
                model_name = model_name["minecraft:".len()..].to_string();
            }

            let (elements, resolved_textures) = self.resolve_model_geometry(&model_name);

            for el in elements {
                resolved_geometry.push((
                    el,
                    variant.x.unwrap_or(0),
                    variant.y.unwrap_or(0),
                    resolved_textures.clone(),
                ));
            }
        }

        let arc = std::sync::Arc::new(resolved_geometry);
        if let Ok(mut cache) = self.element_cache.write() {
            cache.insert(key, std::sync::Arc::clone(&arc));
        }
        arc
    }

    fn match_variants(&self, state: &crate::chunk::BlockState) -> Vec<BlockstateVariant> {
        let mut results = Vec::new();

        let block_name = state.short_name();
        if let Some(blockstate_json) = self.parsed_blockstates.get(block_name) {
            if let Some(variants) = &blockstate_json.variants {
                let mut best_match = None;
                let mut best_score = -1;

                for (k, v) in variants {
                    let mut score = 0;
                    let mut valid = true;
                    if !k.is_empty() && k != "normal" {
                        for prop in k.split(',') {
                            let parts: Vec<&str> = prop.split('=').collect();
                            if parts.len() == 2 {
                                let key = parts[0];
                                let val = parts[1];
                                if let Some(state_val) = state.properties.get(key) {
                                    if state_val == val {
                                        score += 10;
                                    } else {
                                        valid = false;
                                        break;
                                    }
                                } else {
                                    // Property required by variant is missing in state.
                                    // For Bedrock chunks, many default properties are missing.
                                    // We invalidate non-default looking values to ensure we pick the default variant.
                                    if val == "true"
                                        || val == "top"
                                        || val == "upper"
                                        || val == "hanging"
                                        || val == "powered"
                                    {
                                        valid = false;
                                        break;
                                    }
                                    score += 1; // minor score for matching a default
                                }
                            }
                        }
                    }
                    if valid && score > best_score {
                        best_score = score;
                        best_match = Some(v);
                    }
                }

                if let Some(variant_val) = best_match {
                    let variant_obj = if let Some(arr) = variant_val.as_array() {
                        arr.first().unwrap_or(variant_val)
                    } else {
                        variant_val
                    };
                    if let Ok(variant) =
                        serde_json::from_value::<BlockstateVariant>(variant_obj.clone())
                    {
                        results.push(variant);
                    }
                }
            }

            if let Some(multipart) = &blockstate_json.multipart {
                for part in multipart {
                    let mut matches = true;
                    if let Some(when) = &part.when {
                        for (k, v) in &when.properties {
                            // Split OR keys like "north|east|south|west" (rare but possible)
                            let mut key_matched = false;
                            for subkey in k.split('|') {
                                let state_val = state
                                    .properties
                                    .get(subkey)
                                    .map(|s| s.as_str())
                                    .unwrap_or("false");
                                if v.split('|').any(|opt| opt == state_val) {
                                    key_matched = true;
                                    break;
                                }
                            }
                            if !key_matched {
                                matches = false;
                                break;
                            }
                        }
                    }
                    if matches {
                        match &part.apply {
                            MultipartApply::Single(v) => results.push(v.clone()),
                            MultipartApply::Multiple(arr) => {
                                if let Some(v) = arr.first() {
                                    results.push(v.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        results
    }

    fn resolve_model_geometry(
        &self,
        model_name: &str,
    ) -> (Vec<ModelElement>, HashMap<String, String>) {
        let mut resolved_textures = HashMap::new();
        let mut elements = None;

        let mut current_model_name = Some(model_name.to_string());
        let mut visited = Vec::new();

        while let Some(name) = current_model_name {
            if visited.contains(&name) {
                break;
            }
            visited.push(name.clone());

            if let Some(model) = self.parsed_models.get(&name) {
                if let Some(textures) = &model.textures {
                    for (k, v) in textures {
                        if !resolved_textures.contains_key(k) {
                            resolved_textures.insert(k.clone(), v.clone());
                        }
                    }
                }

                if elements.is_none() {
                    if let Some(el) = &model.elements {
                        elements = Some(el.clone());
                    }
                }

                if let Some(parent) = &model.parent {
                    let mut p = parent.clone();
                    if p.starts_with("minecraft:") {
                        p = p["minecraft:".len()..].to_string();
                    }
                    current_model_name = Some(p);
                } else {
                    current_model_name = None;
                }
            } else {
                current_model_name = None;
            }
        }

        let mut final_textures = HashMap::new();
        for (k, v) in &resolved_textures {
            let mut val = v.clone();
            let mut visited_keys = Vec::new();
            while val.starts_with('#') {
                if visited_keys.contains(&val) {
                    break;
                }
                visited_keys.push(val.clone());
                if let Some(resolved) = resolved_textures.get(&val[1..]) {
                    val = resolved.clone();
                } else {
                    break;
                }
            }
            if val.starts_with("minecraft:") {
                val = val["minecraft:".len()..].to_string();
            }
            if val.starts_with("block/") {
                val = val["block/".len()..].to_string();
            }
            final_textures.insert(k.clone(), val);
        }

        // Add model_name fallback
        let mut fallback = model_name.to_string();
        if fallback.starts_with("block/") {
            fallback = fallback["block/".len()..].to_string();
        }
        final_textures.insert("".to_string(), fallback);

        (elements.unwrap_or_default(), final_textures)
    }

    pub fn all_textures(&self, block: &str) -> Vec<String> {
        let mut texs = Vec::new();
        if let Some(state) = self.parsed_blockstates.get(block) {
            let mut models = Vec::new();
            if let Some(variants) = &state.variants {
                for v in variants.values() {
                    let v_obj = if let Some(arr) = v.as_array() {
                        arr.first().unwrap_or(v)
                    } else {
                        v
                    };
                    if let Ok(variant) = serde_json::from_value::<BlockstateVariant>(v_obj.clone())
                    {
                        models.push(variant.model);
                    }
                }
            }
            if let Some(multipart) = &state.multipart {
                for part in multipart {
                    match &part.apply {
                        MultipartApply::Single(v) => models.push(v.model.clone()),
                        MultipartApply::Multiple(arr) => {
                            if let Some(v) = arr.first() {
                                models.push(v.model.clone());
                            }
                        }
                    }
                }
            }

            for m in models {
                let mut model_name = m;
                if model_name.starts_with("minecraft:") {
                    model_name = model_name["minecraft:".len()..].to_string();
                }
                let (_, final_textures) = self.resolve_model_geometry(&model_name);
                for tex in final_textures.values() {
                    if !texs.contains(tex) && !tex.is_empty() {
                        texs.push(tex.clone());
                    }
                }
            }
        }
        texs
    }

    pub fn face_textures(&self, block: &str) -> crate::block_model::FaceTextures {
        if let Some(ft) = self.block_textures.get(block) {
            return ft.clone();
        }
        // Fallback for unknown
        crate::block_model::FaceTextures {
            top: block.into(),
            bottom: block.into(),
            south: block.into(),
            north: block.into(),
            east: block.into(),
            west: block.into(),
        }
    }
}
