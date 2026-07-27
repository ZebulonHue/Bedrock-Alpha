# Project Bedrock

Load a Minecraft world, pick an area, and get it into Blender looking like the
game.

Reads Java and Bedrock saves directly, shows the world in a 3D viewport, and
exports a selected region as OBJ or glTF. The included Blender add-on imports
that export and rebuilds it with real block geometry and MCprep materials
rather than a field of textured cubes.

## Running the app

**Windows:** double-click `run.bat`.

The first run builds the app and takes a few minutes; afterwards it starts
straight away. If Rust is not installed it will say so —
[rustup.rs](https://rustup.rs) is the one-click installer.

Any platform, from a terminal:

```bash
cargo run --release
```

The app is not shipped as a prebuilt `.exe` on purpose: that would add tens of
megabytes to every clone and be out of date the moment a line changed. It is
built from source on your machine instead.

### Using it

1. **Detect Worlds** finds your Minecraft saves automatically.
2. Open one — it loads and appears in the viewport.
3. Fly with `W A S D`, `Space` up, `Shift` down. Drag with the left mouse
   button to look around, the scroll wheel to change how fast you fly, and
   `Ctrl` + scroll to zoom.
4. In the **2D Overview** tab, **right-drag** to box out the area to export.
5. Set the output folder in **Export Settings** and press **Export**.

## Importing into Blender

Install `project_bedrock_import_tools.py` through
**Edit → Preferences → Add-ons → Install…**, then enable *Project Bedrock
Import Tools*.

Import with **File → Import → Import Project Bedrock Export** and pick the
`.obj` (or `.glb`/`.gltf`) the app wrote. The add-on finds the manifest and
prototype meshes sitting beside it on its own.

[MCprep](https://github.com/Moo-Ack-Productions/MCprep) is optional but
recommended — the add-on uses its assets and materials where they exist, and
falls back to geometry built from your own Minecraft install where they do
not.

### What an export is made of

| file | what it is |
| --- | --- |
| `<world>.obj` | the terrain mesh |
| `<world>.mtl` + `<world>.atlas.png` | its materials and texture atlas |
| `<world>.blocks.json` | where every non-cube block sits, and in what state |
| `prototypes/` | one small mesh per block state, textured from your game files |

The manifest and prototypes are what let the add-on place a real fence, stair
or plant at each position instead of a cube. Keep them beside the `.obj`.

## Layout

```
run.bat                          build and launch (Windows)
project_bedrock_import_tools.py  the Blender add-on
crates/                          the app
  bedrock-app/                     window, panels, export flow
  bedrock-ui/                      docked panel widgets
  bedrock-render/                  GPU viewport and chunk meshing
  bedrock-parser/                  save reading, block models, textures
  bedrock-export/                  OBJ and glTF writers
  bedrock-settings/                saved preferences
tools/                           generators for the block data tables
docs/                            design and reference notes
```

Block geometry and texture tables under `crates/bedrock-parser/src/` are
generated from the game's own files by the scripts in `tools/` — edit those
rather than the generated `.rs` files, which get overwritten.

## Building and testing

```bash
cargo build --release
cargo test
```

Some tests read textures from an installed Minecraft client and skip
themselves if one is not present.
