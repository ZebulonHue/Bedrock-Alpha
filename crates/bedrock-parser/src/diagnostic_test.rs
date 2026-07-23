#[test]
fn test_diagnostic_elements() {
    let assets = crate::assets_extractor::VanillaAssets::load().unwrap();
    let resolver = crate::json_model::BlockModelResolver::new(&assets);

    let mut map = std::collections::HashMap::new();
    map.insert("facing".to_string(), "east".to_string());
    map.insert("half".to_string(), "bottom".to_string());
    map.insert("shape".to_string(), "straight".to_string());

    let mut wheat_map = std::collections::HashMap::new();
    wheat_map.insert("age".to_string(), "7".to_string());

    let states = vec![
        crate::chunk::BlockState {
            name: "minecraft:sculk_sensor".into(),
            properties: std::collections::HashMap::new(),
        },
        crate::chunk::BlockState {
            name: "minecraft:oak_stairs".into(),
            properties: map.clone(),
        },
        crate::chunk::BlockState {
            name: "minecraft:wheat".into(),
            properties: wheat_map,
        },
        crate::chunk::BlockState {
            name: "minecraft:water".into(),
            properties: std::collections::HashMap::new(),
        },
    ];

    for state in &states {
        let els = resolver.get_elements_for_state(state);
        println!("State: {}", state.name);
        println!("  elements count: {}", els.len());
        let quads =
            crate::json_geometry::generate_quads_from_elements(0, 0, 0, &els, &|_, _, _| None);
        for (i, q) in quads.iter().enumerate() {
            println!("  quad {}: face_idx {}, tex {:?}", i, q.face_idx, q.texture);
        }
    }
}
#[test]
fn test_diagnostic_bedrock_states() {
    // This is an interactive diagnostic — only runs when a known Bedrock
    // world folder is present. The path is a local development artifact.
    let candidate_paths = [
            "C:/Users/zebby/AppData/Local/Packages/Microsoft.MinecraftUWP_8wekyb3d8bbwe/LocalState/games/com.mojang/minecraftWorlds/Rainbow",
            "D:/project-bedrock-patched-v5/Rainbow",
        ];
    let path = candidate_paths
        .iter()
        .find(|p| std::path::Path::new(p).exists());
    let Some(path) = path else {
        eprintln!("Skipping bedrock state diagnostic — no Bedrock world found at known paths");
        return;
    };
    let w = crate::bedrock::BedrockWorld::open(path).unwrap();
    let chunks = w.chunks_near(2, -17, 0).unwrap();
    for c in &chunks {
        for y in 0..128 {
            for x in 0..16 {
                for z in 0..16 {
                    if let Some(pal) = c.block_state_at(x, y, z) {
                        if pal.name.contains("stairs")
                            || pal.name.contains("wheat")
                            || pal.name.contains("bed")
                        {
                            println!("BEDROCK STATE: {:?}", pal);
                            break;
                        }
                    }
                }
            }
        }
    }
}
