//! What "Open Folder…" finds in a given directory.
//!
//! Mirrors the World Browser's manual-open rules so a folder that will not
//! show up in the app can be diagnosed without clicking through the GUI.
//!
//! Run: `cargo run -p bedrock-parser --example open_folder -- "<path>"`

use bedrock_parser::detect::{open_bedrock_world, open_java_world};
use std::path::PathBuf;

fn main() {
    let Some(arg) = std::env::args().nth(1) else {
        eprintln!("usage: open_folder <folder>");
        std::process::exit(2);
    };
    let folder = PathBuf::from(arg);
    println!("Looking in: {}", folder.display());

    let describe = |path: &PathBuf| -> Option<String> {
        if path.join("db").is_dir() {
            match open_bedrock_world(path) {
                Ok(w) => Some(format!("Bedrock  '{}'  {} bytes", w.name, w.size_bytes)),
                Err(e) => Some(format!("Bedrock  FAILED: {e}")),
            }
        } else if path.join("level.dat").is_file() || path.join("region").is_dir() {
            match open_java_world(path) {
                Ok(w) => Some(format!("Java     '{}'  {} bytes", w.name, w.size_bytes)),
                Err(e) => Some(format!("Java     FAILED: {e}")),
            }
        } else {
            None
        }
    };

    if let Some(found) = describe(&folder) {
        println!("  itself -> {found}");
        return;
    }

    let mut any = false;
    if let Ok(entries) = std::fs::read_dir(&folder) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if !path.is_dir() {
                continue;
            }
            let label = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match describe(&path) {
                Some(found) => {
                    any = true;
                    println!("  {label:<20} -> {found}");
                }
                None => println!("  {label:<20} -> not a world folder"),
            }
        }
    }
    if !any {
        println!("  nothing openable here");
    }
}
