//! Embeds the application icon into the Windows executable.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let ico = Path::new(&manifest_dir)
        .join("../../assets/icon.ico")
        .canonicalize()
        .expect("assets/icon.ico missing — run `python tools/generate_icon.py`");
    let out_dir = env::var("OUT_DIR").unwrap();
    let rc_path = Path::new(&out_dir).join("icon.rc");
    // Resource scripts accept forward slashes, avoiding escape headaches.
    let escaped = ico.display().to_string().replace('\\', "/");
    fs::write(&rc_path, format!("1 ICON \"{escaped}\"")).unwrap();
    let _ = embed_resource::compile(&rc_path, embed_resource::NONE);
}
