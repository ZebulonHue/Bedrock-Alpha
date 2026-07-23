# PRD — Project Bedrock

**Modern Minecraft World Exporter for Blender**

Version 2.0

> **This is not a Minecraft clone.**
> **This is a professional desktop tool for artists that happens to understand Minecraft worlds.**
> That distinction affects every architectural decision. We are not trying to
> recreate Minecraft — we're trying to make the best world extraction pipeline
> ever made.

---

## 1. Vision

Project Bedrock is a next-generation desktop application that modernizes
Minecraft world exporting for professional artists — a modern replacement for
Mineways, not a clone.

**Goals:**

- Beautiful modern desktop UI
- Native performance (WGPU / Vulkan / DirectX 12)
- Real-time Minecraft world viewer
- Java & Bedrock support
- Accurate rendering identical to Minecraft (for export preview)
- Export OBJ / glTF / USD + automatic Blender import
- Massive worlds with smooth streaming
- Modular architecture that AI can build reliably

---

## 2. Guiding Principles

- **Principle 1** — Never sacrifice export accuracy for visual effects.
  Export quality always wins.
- **Principle 2** — The renderer exists to help artists choose what to
  export. It is NOT the product.
- **Principle 3** — One-click workflows. The application should remove manual
  Blender cleanup wherever possible.
- **Principle 4** — Native desktop application first. Never feel like a web
  page wrapped inside Electron.
- **Principle 5** — Build foundations before features. Do not add ambitious
  features until the core pipeline is completely reliable.
- **Principle 6** — Every completed milestone must leave the application in a
  working, compilable state. Never begin the next feature until the previous
  one is fully functional and verified.

---

## 3. Problem Statement

### Why AI struggles with this project

It isn't because Minecraft is conceptually difficult. It's because Minecraft
has 20+ years of accumulated rendering logic that must all work together:

- chunk parsing
- NBT reading
- region loading
- block states
- models
- multipart models
- variants
- rotations
- UV generation
- biome tinting
- transparency
- connected textures
- culling
- ambient occlusion
- texture atlases

If even one stage is wrong, you get:

- floating blocks
- black gaps
- incorrect UVs
- rotated textures
- duplicated faces
- missing chunks

The solution: decompose every stage into an independent, testable system.

---

## 4. Complete Rendering Pipeline

The renderer is broken into independent stages, each with its own validation:

```
Minecraft Save
  ↓
Region Loader          — reads .mca / .mcc files
  ↓
Chunk Parser           — decompresses & validates NBT
  ↓
Section Parser         — extracts 16×16×16 column sections
  ↓
NBT Decoder            — deserialises block state palette + indices
  ↓
Palette Decoder        — unpacks bit-packed block IDs
  ↓
Block State Resolver   — resolves block name + properties
  ↓
Model Resolver         — selects blockstate variant / multipart
  ↓
Geometry Generator     — emits positioned quads with face normals
  ↓
Face Culling           — drops quads occluded by opaque neighbours
  ↓
UV Generator           — maps per-face texture coordinates
  ↓
Texture Atlas Builder  — packs texture tiles + applies biome tint
  ↓
Mesh Builder           — assembles GPU-ready vertex/index buffers
  ↓
Viewport Renderer      — draws with camera, depth, lighting
  ↓
Exporter               — writes OBJ/glTF/USD + materials + atlas
```

Every stage must be independently testable. Never skip validation.

---

## 5. Official Minecraft Asset Pipeline

The canonical data flow from block ID to rendered face:

```
Block ID
  ↓
Block State             — e.g. "minecraft:oak_stairs[facing=north,half=bottom]"
  ↓
blockstates JSON        — assets/minecraft/blockstates/<block>.json
  ↓                       (resolves variant/multipart to model + rotation)
model JSON              — assets/minecraft/models/block/<model>.json
  ↓                       (defines parent, textures, elements[])
elements[]              — cuboids with from/to, rotation, per-face textures
  ↓
faces                   — up/down/east/west/south/north per element
  ↓
uv                      — per-face UV rect or auto-generated from element bounds
  ↓
texture reference       — e.g. "#side" resolved from model's texture map
  ↓
PNG                     — assets/minecraft/textures/block/<name>.png
  ↓
GPU mesh                — uploaded to the viewport or written to export
```

**Key insight:** The JSON files don't just point to textures — they define how
the block is constructed (cuboid elements), which faces exist, how they're
rotated, and which texture is used on each face. The geometry is generated
from these definitions.

The `third_party/` directory contains the canonical reference implementations
(Mineways, jmc2obj, Chunky, PrismarineJS, Amulet Editor, Cubiomes,
MCA Selector, MCprep, minecraft-data) — study their approaches before writing
new code.

---

## 6. AI Development Strategy

Rather than asking AI to "build a Minecraft renderer":

**Per stage:**

1. Build loader
2. Validate loader
3. Build parser
4. Validate parser
5. Build geometry
6. Validate geometry
7. Build viewport
8. Validate viewport
9. Build exporter
10. Validate exporter

**Rules:**

- Never work on multiple stages simultaneously.
- Each stage must pass its own test suite before the next begins.
- Every public function is documented (`///`).
- The project must compile and pass `cargo fmt + clippy + test` after every
  completed task.
- Prefer iterative improvements over sweeping rewrites.

---

## 7. Debug Mode

The application includes developer visualizations that render alongside the
world or in an overlay. These are essential for diagnosing pipeline bugs:

- chunk borders
- section borders
- palette IDs
- face normals (arrows / lines)
- UV checker texture (replaces atlas for UV debugging)
- wireframe overlay
- hidden face visualization (highlight culled faces)
- block IDs (text labels on blocks)
- blockstate inspector (hover/pick a block to see its state)
- JSON inspector (view raw blockstate / model JSON)
- FPS counter
- draw call count
- vertex / triangle statistics
- mesh memory usage

Toggled via a developer settings panel or keyboard shortcuts. Rendering debug
overlays never modifies export output.

---

## 8. Performance Goals

| Metric | Target | Notes |
|---|---|---|
| Frame rate | 60–144 FPS | Smooth at all camera distances |
| Visible blocks | Millions | Tested against large builds |
| Chunk load | Async | Never blocks the UI thread |
| Parsing | Multi-threaded | One thread pool per CPU core |
| Mesh caching | Per-chunk GPU buffers | Only rebuild dirty chunks |
| Frustum culling | Yes | Drop off-screen chunks before draw |
| Occlusion culling | Yes | Drop hidden chunks via query |
| GPU instancing | Where appropriate | Flora, torches, repeating elements |
| Incremental rebuilds | Yes | Only update changed blocks |

---

## 9. Export Pipeline

### Formats

| Format | Status | Notes |
|---|---|---|
| OBJ | ✅ Implemented | Wavefront, with MTL + atlas PNG |
| glTF | ✅ Implemented | Modern pipeline, PBR materials with roughness/metallic/emissive, texture atlas |
| USD | ❌ Future | Universal Scene Description |
| FBX | ❌ Optional | Proprietary, lower priority |

### Export guarantees

- UVs preserved per face
- Materials exported (MTL / glTF PBR)
- Texture atlas exported as standalone PNG
- Vertex normals correct
- World coordinates preserved with centering
- Chunk-aligned geometry (seamless between chunks)
- Watertight mesh (shared vertices at block boundaries)

---

## 10. Target Users

Minecraft builders · Minecraft artists · Blender artists · Minecraft
YouTubers · Thumbnail creators · Animation creators · Map makers ·
Professional builders · Anyone needing Minecraft geometry inside Blender

---

## 11. Core Workflow

```
Open Application
  ↓
Detect Minecraft Worlds  (automatic)
  ↓
Open World
  ↓
Navigate World            (orbit / pan / zoom 3D view)
  ↓
Select Export Region      (drag resize box in 3D + 2D overview)
  ↓
Click Export              (single click, background thread)
  ↓
Open Blender
  ↓
Finished Scene Ready To Use  (correct origin, materials, UVs, atlas)
```

No manual cleanup. No fixing origins. No fixing materials. No importing
twenty different files. Everything simply works.

---

## 12. Primary Deliverables

- Native Windows Desktop Application
- Professional Installer
- Proper EXE with icons
- Settings with persistence
- Automatic Updates (future)
- Modern UI (dockable panels, dark theme)
- High-performance WGPU renderer
- Reliable export pipeline (OBJ first, more later)
- Blender import pipeline (add-on or .blend template)

---

## 13. Architecture

### Crate Map

```
bedrock-app       — Entry point, window, app-state wiring
bedrock-ui        — egui panels, dock layout, theme, log view
bedrock-render    — WGPU viewport rendering (consumes parsed data)
bedrock-parser    — Read Java/Bedrock saves, NBT, chunks, block models
bedrock-export    — Export files from parsed data (OBJ, glTF, ...)
bedrock-blender   — Blender import conventions & tooling
bedrock-cache     — Thumbnail/chunk/texture caches
bedrock-settings  — Persistent preferences
```

### Hard Boundaries

| Crate | Responsibility | Must NOT |
|---|---|---|
| `bedrock-app` | Entry point, window, app-state wiring | contain business logic |
| `bedrock-ui` | egui panels, dock layout, theme, log view | parse worlds, touch GPU |
| `bedrock-render` | WGPU viewport rendering | export, parse |
| `bedrock-parser` | Read Java/Bedrock saves, NBT, chunks | UI, GPU, export |
| `bedrock-export` | Export files from parsed data | UI, rendering |
| `bedrock-blender` | Blender import conventions & tooling | UI, parsing |
| `bedrock-cache` | Thumbnail/chunk/texture caches | UI, GPU |
| `bedrock-settings` | Persistent preferences | anything else |

- The renderer consumes parsed data; it is never responsible for export.
- The export system is never responsible for rendering.
- UI never contains business logic; systems communicate through plain data.

---

## 14. Rendering Philosophy

### What we render

- Accurate blocks with correct textures
- Correct atlas UVs (no bleeding)
- Good enough lighting for navigation
- Smooth camera controls
- Excellent performance

### What we do NOT render

- Gameplay mechanics (health, inventory, etc.)
- Entities (future, if at all)
- Particles or weather
- Skyboxes or fog (optional aesthetic)

### Dynamic Chunk Streaming

- Zoomed out → simplified distant chunks (LOD)
- Zoom in → increase chunk fidelity
- Move camera → load new chunks, unload distant chunks
- The world should always feel continuous — never an empty void

### Camera Controls

Orbit · Pan · Zoom · Focus Selection · Snap To Player · Reset Camera · Bookmarks

---

## 15. World Detection

Automatically detect: Minecraft Java saves (`.minecraft/saves`), Minecraft
Bedrock saves (UWP + GDK installs). Extract: world names, last played,
version, thumbnail, world size.

Display worlds as cards with: screenshot thumbnail, world name, edition
badge (Java/Bedrock), Minecraft version, last played, file size.

---

## 16. Export Region Selection

- Resizable 3D selection box in the viewport
- Synchronous 2D rectangle on the overview map
- Numeric coordinate controls
- Live dimensions (blocks × blocks × blocks)
- Estimated block count and polygon count
- Keyboard shortcuts for precise positioning

---

## 17. Blender Pipeline

This is the heart of the application. After export:

- **Correct origin** — scene lands at Blender's world origin
- **Correct scale** — 1 block = 1 Blender unit
- **Correct materials** — one material per block type
- **Correct UVs** — per-face texture coordinates
- **Texture atlas** — packed PNG with matching UV layout
- **Correct normals** — face-weighted vertex normals
- **No duplicates** — no duplicate materials, no broken names
- **Organised collections** — blocks grouped by chunk or region

### MCPrep Integration

Study MCprep (`third_party/MCprep/`) for material conventions. Where
appropriate, reuse compatible material naming, support expected import
options, automate cleanup — and eventually expand beyond MCprep's
capabilities.

### Future: Project Bedrock Blender Add-on

Automatic import, material cleanup, origin placement, collection creation,
texture assignment, world organisation, LOD management, one-click workflow.

---

## 18. User Interface

### Philosophy

Modern · Elegant · Minimal · Friendly · Professional

Inspired by: Blender, Discord, modern IDEs.
Not inspired by: Windows XP, legacy utility software.

### Visual Style

Rounded corners · Soft shadows · Subtle animations · Dark mode · Comfortable
spacing · Readable typography · No visual clutter

### Minecraft Identity

Subtle charm, never childish: Steve head for player location, block icons,
pickaxe icons, creeper warning icons, subtle pixel accents, atlas-inspired
colours. Everything tasteful.

### Layout

Dockable, resizable panels that remember their layout:

- **World Browser** — thumbnail grid of detected worlds
- **3D View** — GPU-accelerated viewport
- **2D Overview Map** — top-down world map with region overlay
- **Properties** — selected block / region properties
- **Export Settings** — format, preset, output path
- **Output Log** — human-readable export + debug logs

### UI Performance

- Background loading & exporting
- Never freeze the UI
- Progress bars with cancellation
- GPU-accelerated rendering
- Smart caching

### Export Presets

High Quality · Animation · Thumbnail · Low Memory · Custom

### Settings

Application Settings · Renderer Settings · Export Settings · Per World
Settings · Recent Worlds · Recent Exports

### Logging & Error Handling

- Clear, human-readable logs
- Developer logs optional
- Never crash silently
- Every error explains: what happened, why, and a possible solution

---

## 19. Plugin System (Future)

Not required for Version 1, but the architecture should support: Blender
plugins, Unity export, Unreal export, custom exporters, community extensions.

---

## 20. Development Roadmap

| Phase | What | Status |
|---|---|---|
| **Phase 1 — Foundation** | native app, project structure, UI shell, settings, dock panels, icons, build pipeline | ✅ Substantially complete |
| **Phase 2 — World Detection** | Java + Bedrock detection, world browser, thumbnails, metadata | ✅ Substantially complete |
| **Phase 3 — World Parser** | region loading, NBT decoding, chunk parsing, palette, block models, texture atlas, JAR extraction | ✅ Substantially complete |
| **Phase 4 — Renderer** | WGPU viewport, orbit camera, mesh pipeline, atlas sampling, depth buffer, blit integration | ✅ Substantially complete |
| **Phase 5 — Export** | OBJ export, glTF export, MTL materials, atlas PNG, vertex dedup, per-face UVs, region selection, PBR materials | ✅ Substantially complete |
| **Phase 6 — Blender Pipeline** | import add-on, origin correction, material cleanup, collection organisation, MCPrep compatibility | ✅ Substantially complete |
| **Phase 6b — Debug Mode** | chunk borders, wireframe, stats overlay, debug panel, keyboard shortcuts, developer visualizations | ✅ Substantially complete |
| **Phase 7 — Polish** | animations, sounds, preferences, performance tuning, bug fixing, documentation | ⚠️ Partial |

---

## 21. Future Roadmap

- LOD system for massive world navigation
- Schematic import/export (`.schem`, `.schematic`)
- Live Minecraft connection (RCON / Spectator)
- Command block visualization
- Redstone overlays
- Biome overlays
- Height maps
- Minimap
- Search blocks / entities / coordinates
- Entity rendering
- Animation support
- Lighting previews (day/night, block light)
- Ray-traced viewport
- Plugin API

---

## 22. Definition of Success

Project Bedrock succeeds when a user can:

1. Open the application.
2. Automatically detect their Minecraft worlds.
3. Select a world from a thumbnail.
4. Instantly navigate a smooth, fully streamed 3D representation of that world.
5. Snap directly to their player's last position.
6. Drag a selection box around the desired build.
7. Export with a single click.
8. Open Blender and find the scene correctly positioned, textured,
   organised, and ready for immediate creative work.

If that experience feels effortless, fast, and dependable, then Project
Bedrock has achieved its purpose.
