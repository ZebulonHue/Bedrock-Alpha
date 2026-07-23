fn main() {
    let py = bedrock_blender::addon::generate_addon_py(&bedrock_blender::addon::AddonOptions::default());
    std::fs::write("d:/project-bedrock-patched-v5/project_bedrock_import_tools.py", py).unwrap();
    println!("Successfully generated d:/project-bedrock-patched-v5/project_bedrock_import_tools.py!");
}
