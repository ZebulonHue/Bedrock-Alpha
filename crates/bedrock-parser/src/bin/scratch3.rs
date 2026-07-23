fn main() {
    let worlds = bedrock_parser::detect::detect_worlds();
    println!("Found {} worlds", worlds.len());
}
