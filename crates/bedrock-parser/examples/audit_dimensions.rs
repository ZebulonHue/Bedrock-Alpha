//! List the dimensions a save holds, and how many region files each has.
fn main() {
    for dir in std::env::args().skip(1) {
        let world = bedrock_parser::world::World::open(std::path::PathBuf::from(&dir));
        let dims = world.dimensions();
        let name = std::path::Path::new(&dir)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parts: Vec<String> = dims
            .iter()
            .map(|d| format!("{} ({} regions)", d.kind.label(), d.regions.len()))
            .collect();
        println!("  {name:<28} opens: {:<12} all: {}",
            dims.first().map(|d| d.kind.label()).unwrap_or_default(),
            parts.join(", "));
    }
}
