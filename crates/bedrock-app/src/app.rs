//! Application shell: menu bar, status bar, dock area, and persistence.
//! Includes chunk streaming: dynamically loading/unloading chunks as the
//! camera moves, enabling arbitrarily large worlds.

use crate::loader::{load_active_world, load_chunks_for_region, load_one_chunk, ActiveWorld};
use bedrock_export::gltf::export_gltf;
use bedrock_export::obj::{export_obj_with_options, ExportOptions, ExportRegion, ExportStats};
use bedrock_parser::chunk::Chunk;
use bedrock_parser::detect::WorldSummary;
use bedrock_parser::mineways::build_mineways_tileset;
use bedrock_render::mesh::{chunks_to_meshes, ChunkMesh};
use bedrock_render::{ChunkBorder, SharedScene};
use bedrock_settings::ExportFormat;
use bedrock_settings::{layout_path, Settings, Theme};
use bedrock_ui::dock::{self, Panel, PanelContext};

/// Alias for the channel type returned by the initial world-loading thread.
type WorldLoadResult = Result<(ActiveWorld, Vec<ChunkMesh>), String>;
use bedrock_ui::log::LogBuffer;
use bedrock_ui::world_browser::WorldBrowserState;
use bedrock_ui::{theme, windows};
use egui_dock::DockState;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

/// How far the camera must move (in blocks) before we trigger a streaming
/// update. Set to 8 blocks (half a chunk) to avoid re-streaming on tiny
/// movements while keeping up with the camera.
const STREAMING_THRESHOLD: i32 = 8;

/// The Project Bedrock application.
pub struct BedrockApp {
    settings: Settings,
    dock: DockState<Panel>,
    log: LogBuffer,
    auto_scroll_log: bool,
    show_settings: bool,
    show_about: bool,
    applied_theme: Theme,
    fps: FpsCounter,
    world_browser: WorldBrowserState,
    scene: Arc<Mutex<SharedScene>>,
    world_rx: Option<Receiver<WorldLoadResult>>,
    current_world: Option<String>,
    loading_world: Option<String>,
    /// Active world with streaming support (replaces `loaded_chunks`).
    active_world: Option<ActiveWorld>,
    /// Last camera chunk position (for streaming throttle).
    last_stream_cx: i32,
    last_stream_cz: i32,
    overview_tex: Option<egui::TextureHandle>,
    /// Pixel-art strip drawn along the bottom of the window. Loaded once and
    /// kept, since it never changes.
    banner_tex: Option<egui::TextureHandle>,
    overview_origin: [i32; 2],
    export_region: ExportRegion,
    export_requested: bool,
    export_rx: Option<Receiver<Result<ExportStats, String>>>,
    exporting: bool,
}

impl BedrockApp {
    /// Build the application: apply the theme, hand the wgpu render state to
    /// the viewport renderer, restore the saved dock layout (if any), and
    /// kick off the first world scan.
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings, log: LogBuffer) -> Self {
        theme::apply(&cc.egui_ctx, settings.theme);

        let scene = SharedScene::new_shared();
        if let Some(render_state) = cc.wgpu_render_state.as_ref() {
            render_state.renderer.write().callback_resources.insert(
                bedrock_render::ViewportRenderer::new(render_state, Arc::clone(&scene)),
            );
            tracing::info!("WGPU render state acquired — viewport renderer initialized");
        } else {
            tracing::warn!("No WGPU render state available — the viewport will be empty");
        }

        let dock = restore_layout().unwrap_or_else(dock::default_layout);

        let mut app = Self {
            applied_theme: settings.theme,
            settings,
            dock,
            log,
            auto_scroll_log: true,
            show_settings: false,
            show_about: false,
            fps: FpsCounter::default(),
            world_browser: WorldBrowserState::default(),
            scene,
            world_rx: None,
            current_world: None,
            loading_world: None,
            active_world: None,
            last_stream_cx: i32::MAX,
            last_stream_cz: i32::MAX,
            overview_tex: None,
            banner_tex: None,
            overview_origin: [0; 2],
            export_region: ExportRegion {
                min: [0; 3],
                max: [0; 3],
            },
            export_requested: false,
            export_rx: None,
            exporting: false,
        };
        app.world_browser.start_scan();
        app
    }

    /// Kick off a background load for the given world.
    fn start_world_load(&mut self, summary: WorldSummary) {
        if self.loading_world.is_some() {
            tracing::warn!("A world is already loading — wait for it to finish");
            return;
        }
        tracing::info!("Opening world '{}'…", summary.name);
        self.loading_world = Some(summary.name.clone());
        let (tx, rx) = channel();
        let radius_chunks = self.settings.load_radius_chunks;
        thread::spawn(move || {
            let _ = tx.send(load_active_world(&summary, radius_chunks));
        });
        self.world_rx = Some(rx);
    }

    /// Collect a finished world load: hand the mesh to the viewport, frame
    /// the camera, store the ActiveWorld, and update the status bar.
    fn poll_world_load(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.world_rx else { return };
        match rx.try_recv() {
            Ok(Ok((active_world, chunk_meshes))) => {
                self.export_region = chunks_bounds_from_hash(&active_world.chunk_map);
                self.overview_tex = active_world.overview.as_ref().map(|overview| {
                    self.overview_origin = [overview.origin_x, overview.origin_z];
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [overview.width, overview.height],
                        &overview.rgba,
                    );
                    ctx.load_texture("overview", image, egui::TextureOptions::NEAREST)
                });
                let chunk_count = active_world.loaded_chunks.len();
                let name = active_world.name.clone();

                // Hand meshes + atlas to the GPU.
                {
                    let mut scene = self.scene.lock().expect("scene lock poisoned");
                    scene.camera = Some(active_world.camera);
                    scene.pending_chunks = Some(chunk_meshes);
                    scene.pending_atlas = Some(active_world.atlas.clone());
                    scene.player_pos = active_world
                        .player_pos
                        .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]);
                }

                // Store the active world for streaming and export.
                self.active_world = Some(active_world);

                tracing::info!(
                    "World '{}' ready — {chunk_count} chunks loaded. Streaming enabled — \
                     chunks load dynamically as you move.",
                    name
                );
                self.current_world = Some(name);
                self.loading_world = None;
                self.world_rx = None;
                ctx.request_repaint();
            }
            Ok(Err(message)) => {
                tracing::error!("Could not open world: {message}");
                self.loading_world = None;
                self.world_rx = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint(),
            Err(TryRecvError::Disconnected) => {
                tracing::error!("World loader thread disconnected");
                self.loading_world = None;
                self.world_rx = None;
            }
        }
    }

    /// Streaming update: check camera position, load/unload chunks as needed.
    ///
    /// Runs every frame but only does work when the camera moves more than
    /// [`STREAMING_THRESHOLD`] blocks from the last streaming position.
    fn update_streaming(&mut self) {
        let Some(ref mut aw) = self.active_world else {
            return;
        };

        // Read camera target position.
        let camera_target = self
            .scene
            .lock()
            .expect("scene lock poisoned")
            .camera
            .map(|c| c.target)
            .unwrap_or([8.0, 70.0, 8.0]);

        let center_cx = (camera_target[0] / 16.0).floor() as i32;
        let center_cz = (camera_target[2] / 16.0).floor() as i32;

        // Throttle: skip if camera hasn't moved enough.
        let dx = (center_cx - self.last_stream_cx).abs();
        let dz = (center_cz - self.last_stream_cz).abs();
        if dx.max(dz) * 16 < STREAMING_THRESHOLD {
            return;
        }

        let radius = self.settings.load_radius_chunks;

        // Compute needed chunk coords.
        let needed = compute_needed_chunks(center_cx, center_cz, radius);

        // Diff: chunks to add vs remove.
        let loaded = &aw.loaded_chunks;
        let to_add: Vec<(i32, i32)> = needed.difference(loaded).copied().collect();
        let to_remove: Vec<(i32, i32)> = loaded.difference(&needed).copied().collect();

        if to_add.is_empty() && to_remove.is_empty() {
            self.last_stream_cx = center_cx;
            self.last_stream_cz = center_cz;
            return;
        }

        // ── Load new chunks from disk ──────────────────────────────────
        let mut new_chunks: Vec<(i32, i32, Chunk)> = Vec::new();
        for &(cx, cz) in &to_add {
            if let Some(chunk) = load_one_chunk(&aw.handle, cx, cz) {
                new_chunks.push((cx, cz, chunk));
            }
        }

        if new_chunks.is_empty() && to_remove.is_empty() {
            // Nothing new to do — just update the tracking.
            self.last_stream_cx = center_cx;
            self.last_stream_cz = center_cz;
            return;
        }

        // Insert new chunks into the map.
        for &(cx, cz, ref chunk) in &new_chunks {
            aw.chunk_map.insert((cx, cz), chunk.clone());
            aw.loaded_chunks.insert((cx, cz));
        }

        // ── Remove old chunks ──────────────────────────────────────────
        for &key in &to_remove {
            aw.chunk_map.remove(&key);
            aw.loaded_chunks.remove(&key);
        }

        // ── Send updated mesh list to GPU ──────────────────────────────
        // For simplicity, we re-mesh ALL currently loaded chunks. This
        // ensures correct cross-chunk face culling for existing chunks
        // that gained new neighbours. The meshing is fast (parallel).
        let all_chunks: Vec<Chunk> = aw.chunk_map.values().cloned().collect();
        let all_meshes = chunks_to_meshes(&all_chunks, &aw.tiles);

        let total_verts: usize = all_meshes.iter().map(|m| m.vertices.len()).sum();
        let total_tris: usize = all_meshes.iter().map(|m| m.triangle_count()).sum();

        {
            let mut scene = self.scene.lock().expect("scene lock poisoned");
            scene.pending_chunks = Some(all_meshes);
        }

        tracing::debug!(
            "Streaming: +{} -{} chunks at ({center_cx}, {center_cz}) — \
             {} chunks, {} verts, {} tris",
            new_chunks.len(),
            to_remove.len(),
            aw.loaded_chunks.len(),
            human_count(total_verts),
            human_count(total_tris),
        );

        self.last_stream_cx = center_cx;
        self.last_stream_cz = center_cz;
    }

    /// Kick off a background export of the current region.
    fn start_export(&mut self) {
        if self.exporting {
            tracing::warn!("An export is already running — wait for it to finish");
            return;
        }
        let Some(ref aw) = self.active_world else {
            tracing::warn!("No world loaded — nothing to export");
            return;
        };
        let region = self.export_region;
        // Read the region's chunks from disk rather than from whatever the
        // streaming loader happens to have resident: `chunk_map` is empty for
        // Java worlds and only camera-local for Bedrock, so exporting from it
        // silently produced "no blocks to export" for any real selection.
        let mut chunks: Vec<Chunk> = load_chunks_for_region(
            &aw.handle,
            region.min[0],
            region.min[2],
            region.max[0],
            region.max[2],
        );
        if chunks.is_empty() {
            chunks = aw.chunk_map.values().cloned().collect();
        }
        let out_dir = if self.settings.export.output_dir.is_empty() {
            dirs::document_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("Project Bedrock")
        } else {
            std::path::PathBuf::from(&self.settings.export.output_dir)
        };
        if let Err(err) = std::fs::create_dir_all(&out_dir) {
            tracing::error!(
                "Cannot create output directory {}: {err}",
                out_dir.display()
            );
            return;
        }
        let world_name = self
            .current_world
            .as_deref()
            .unwrap_or("world")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim()
            .replace(' ', "_");
        let ext = match self.settings.export.format {
            ExportFormat::Obj => "obj",
            ExportFormat::Gltf => "gltf",
            _ => {
                tracing::error!(
                    "Unsupported export format {:?}",
                    self.settings.export.format
                );
                return;
            }
        };
        let export_path = out_dir.join(format!("{world_name}.{ext}"));
        tracing::info!("Exporting region to {}…", export_path.display());
        let format = self.settings.export.format;
        self.exporting = true;
        let (tx, rx) = channel();
        thread::spawn(move || {
            // Use Mineways' terrainExt.png atlas for correct per-face UVs.
            let texture_keys: Vec<String> = chunks.iter().flat_map(|c| c.texture_keys()).collect();
            let tiles = build_mineways_tileset(&texture_keys);
            let result = match format {
                ExportFormat::Obj => export_obj_with_options(
                    &chunks,
                    &region,
                    &export_path,
                    &tiles,
                    // The importer needs this to place real 3D assets.
                    &ExportOptions {
                        write_block_manifest: true,
                        write_prototypes: true,
                    },
                )
                .map_err(|err| err.to_string()),
                ExportFormat::Gltf => export_gltf(&chunks, &region, &export_path, &tiles)
                    .map_err(|err| err.to_string()),
                _ => Err(format!("Unsupported export format: {format:?}")),
            };
            let _ = tx.send(result);
        });
        self.export_rx = Some(rx);
    }

    /// Collect a finished export and report it.
    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.export_rx else { return };
        match rx.try_recv() {
            Ok(Ok(stats)) => {
                tracing::info!(
                    "Export complete: {} blocks, {} faces, {} materials → {}",
                    stats.blocks,
                    stats.faces,
                    stats.materials,
                    stats.obj_path.display()
                );
                self.exporting = false;
                self.export_rx = None;
                ctx.request_repaint();
            }
            Ok(Err(message)) => {
                tracing::error!("Export failed: {message}");
                self.exporting = false;
                self.export_rx = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint(),
            Err(TryRecvError::Disconnected) => {
                tracing::error!("Export thread disconnected");
                self.exporting = false;
                self.export_rx = None;
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open World…").clicked() {
                        if !dock::is_open(&self.dock, Panel::WorldBrowser) {
                            dock::toggle(&mut self.dock, Panel::WorldBrowser);
                        }
                        ui.close();
                    }
                    ui.menu_button("Recent Worlds", |ui| {
                        if self.settings.recent_worlds.is_empty() {
                            ui.add_enabled(false, egui::Button::new("No recent worlds"));
                        }
                        for _world in &self.settings.recent_worlds {
                            // Phase 2+ populates this list.
                        }
                    });
                    ui.separator();
                    if ui.button("Settings…").clicked() {
                        self.show_settings = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("View", |ui| {
                    for panel in Panel::ALL {
                        let mut open = dock::is_open(&self.dock, panel);
                        if ui.checkbox(&mut open, panel.title()).changed() {
                            dock::toggle(&mut self.dock, panel);
                        }
                    }
                    ui.separator();
                    if ui.button("Reset Layout").clicked() {
                        self.dock = dock::default_layout();
                        ui.close();
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About Project Bedrock").clicked() {
                        self.show_about = true;
                        ui.close();
                    }
                });
            });
        });
    }

    /// Pixel-art grass strip along the bottom edge.
    ///
    /// Sits above the status bar so the readable text stays at the very edge
    /// where it is expected, with the art as a base the window rests on. The
    /// strip is drawn at its own pixel scale and tiled horizontally rather
    /// than stretched -- stretching pixel art to an arbitrary window width
    /// blurs it into mush, which is the one thing this style cannot survive.
    fn banner(&mut self, ui: &mut egui::Ui) {
        let texture = self.banner_tex.get_or_insert_with(|| {
            let bytes = include_bytes!("../../../assets/ui/grass_banner.png");
            let image = image::load_from_memory(bytes)
                .expect("bundled assets/ui/grass_banner.png must be valid")
                .to_rgba8();
            let (w, h) = image.dimensions();
            ui.ctx().load_texture(
                "ui/grass_banner",
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &image),
                // Nearest-neighbour: this is pixel art and must stay crisp.
                egui::TextureOptions::NEAREST,
            )
        });

        let [tex_w, tex_h] = texture.size();
        let height = 72.0;
        let scale = height / tex_h as f32;
        let tile_w = tex_w as f32 * scale;

        egui::Panel::bottom("art_banner")
            .exact_size(height)
            .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let painter = ui.painter_at(rect);
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                let mut x = rect.left();
                while x < rect.right() {
                    let tile = egui::Rect::from_min_size(
                        egui::pos2(x, rect.top()),
                        egui::vec2(tile_w.min(rect.right() - x), height),
                    );
                    // Trim the UV with the tile so a partial tile at the right
                    // edge is cropped rather than squashed.
                    let frac = tile.width() / tile_w;
                    let uv = egui::Rect::from_min_max(
                        uv.min,
                        egui::pos2(uv.min.x + (uv.max.x - uv.min.x) * frac, uv.max.y),
                    );
                    painter.image(texture.id(), tile, uv, egui::Color32::WHITE);
                    x += tile_w;
                }
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar")
            .exact_size(26.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.weak(format!("Project Bedrock v{}", env!("CARGO_PKG_VERSION")));
                    ui.separator();
                    if let Some(loading) = &self.loading_world {
                        ui.weak(format!("Loading '{loading}'…"));
                    } else if let Some(world) = &self.current_world {
                        ui.weak(format!("World: {world}"));
                        let stats = self.scene.lock().expect("scene lock poisoned").mesh_stats;
                        if stats.1 > 0 {
                            ui.separator();
                            ui.weak(format!("{} triangles", human_count(stats.1)));
                        }
                    } else {
                        ui.weak("No world loaded");
                    }
                    let world_count = self.world_browser.worlds().len();
                    if world_count > 0 {
                        ui.separator();
                        ui.weak(format!(
                            "{world_count} world{} detected",
                            if world_count == 1 { "" } else { "s" }
                        ));
                    }
                    if self.settings.show_fps {
                        ui.separator();
                        ui.weak(format!("{:.0} FPS", self.fps.fps()));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak("WGPU");
                    });
                });
            });
    }
}

impl eframe::App for BedrockApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.fps.tick();
        self.world_browser.poll(&ctx);
        self.poll_world_load(&ctx);
        self.poll_export(&ctx);

        // ── Start world load when user selects one ───────────────────
        if let Some(summary) = self.world_browser.take_selected() {
            self.start_world_load(summary);
        }

        // ── Trigger export if requested ─────────────────────────────
        if self.export_requested {
            self.export_requested = false;
            self.start_export();
        }

        // ── Streaming: load/unload chunks based on camera position ──
        self.update_streaming();

        // ── Sync viewport state ─────────────────────────────────────
        {
            let mut scene = self.scene.lock().expect("scene lock poisoned");
            scene.region = self.active_world.is_some().then(|| {
                (
                    self.export_region.min.map(|v| v as f32),
                    self.export_region.max.map(|v| v as f32),
                )
            });
            scene.debug.show_stats = self.settings.debug.show_stats;
            scene.debug.show_chunk_borders = self.settings.debug.show_chunk_borders;
            scene.debug.show_wireframe = self.settings.debug.show_wireframe;

            // Rebuild chunk borders from the active world.
            if let Some(ref aw) = self.active_world {
                let mut borders: Vec<ChunkBorder> = Vec::with_capacity(aw.loaded_chunks.len());
                for &(cx, cz) in &aw.loaded_chunks {
                    if let Some(chunk) = aw.chunk_map.get(&(cx, cz)) {
                        if let Some((min_y, max_y)) = chunk.y_range() {
                            borders.push(ChunkBorder {
                                min: [cx * 16, min_y, cz * 16],
                                max: [cx * 16 + 16, max_y, cz * 16 + 16],
                            });
                        }
                    }
                }
                scene.chunk_borders = borders;
            }
        }

        // ── Keyboard shortcuts ─────────────────────────────────────────
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F3)) {
            self.settings.debug.show_stats = !self.settings.debug.show_stats;
            self.settings.save();
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F4)) {
            self.settings.debug.show_chunk_borders = !self.settings.debug.show_chunk_borders;
            self.settings.save();
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
            self.settings.debug.show_wireframe = !self.settings.debug.show_wireframe;
            self.settings.save();
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            let mut scene = self.scene.lock().expect("scene lock poisoned");
            if let (Some(player_pos), Some(camera)) = (scene.player_pos, scene.camera.as_mut()) {
                camera.target = player_pos;
                tracing::info!("Snapped camera to player position {:?}", player_pos);
            }
        }

        // ── WASD fly camera ────────────────────────────────────────────
        {
            let mut scene = self.scene.lock().expect("scene lock poisoned");
            if let Some(camera) = scene.camera.as_mut() {
                let mut fwd = 0.0f32;
                let mut right = 0.0f32;
                let mut up = 0.0f32;
                ui.input(|i| {
                    if i.key_down(egui::Key::W) {
                        fwd += 1.0;
                    }
                    if i.key_down(egui::Key::S) {
                        fwd -= 1.0;
                    }
                    if i.key_down(egui::Key::D) {
                        right += 1.0;
                    }
                    if i.key_down(egui::Key::A) {
                        right -= 1.0;
                    }
                    if i.key_down(egui::Key::Space) {
                        up += 1.0;
                    }
                    if i.modifiers.shift {
                        up -= 1.0;
                    }
                });
                if fwd != 0.0 || right != 0.0 || up != 0.0 {
                    camera.fly(fwd, right, up);
                }
            }
        }

        if self.applied_theme != self.settings.theme {
            theme::apply(&ctx, self.settings.theme);
            self.applied_theme = self.settings.theme;
        }

        self.menu_bar(ui);
        self.status_bar(ui);
        self.banner(ui);

        egui::CentralPanel::default().show(ui, |ui| {
            let style = egui_dock::Style::from_egui(ui.style().as_ref());
            let mut viewer = PanelContext {
                export: &mut self.settings.export,
                log: &self.log,
                auto_scroll_log: &mut self.auto_scroll_log,
                world_browser: &mut self.world_browser,
                viewport_scene: &self.scene,
                export_region: &mut self.export_region,
                world_loaded: self.active_world.is_some(),
                export_requested: &mut self.export_requested,
                overview: self.overview_tex.as_ref().map(|texture| {
                    bedrock_ui::panels::OverviewData {
                        texture,
                        origin: self.overview_origin,
                    }
                }),
                debug: &mut self.settings.debug,
            };
            egui_dock::DockArea::new(&mut self.dock)
                .style(style)
                .show_inside(ui, &mut viewer);
        });

        if self.show_settings
            && windows::settings(&ctx, &mut self.settings, &mut self.show_settings)
        {
            self.dock = dock::default_layout();
        }
        if self.show_about {
            windows::about(&ctx, &mut self.show_about);
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if let Ok(json) = serde_json::to_string_pretty(&self.dock) {
            let _ = std::fs::write(layout_path(), json);
        }
        self.settings.save();
    }
}

/// Compute the set of chunk coordinates within `radius` chunks of
/// `(center_cx, center_cz)` (a square of side `2*radius + 1`).
fn compute_needed_chunks(center_cx: i32, center_cz: i32, radius: i32) -> HashSet<(i32, i32)> {
    let mut needed = HashSet::with_capacity((radius as usize * 2 + 1).pow(2));
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            needed.insert((center_cx + dx, center_cz + dz));
        }
    }
    needed
}

/// Block-coordinate bounds covering all chunks in a HashMap.
fn chunks_bounds_from_hash(chunk_map: &HashMap<(i32, i32), Chunk>) -> ExportRegion {
    let mut min = [i32::MAX; 3];
    let mut max = [i32::MIN; 3];
    for chunk in chunk_map.values() {
        min[0] = min[0].min(chunk.x * 16);
        max[0] = max[0].max(chunk.x * 16 + 16);
        min[2] = min[2].min(chunk.z * 16);
        max[2] = max[2].max(chunk.z * 16 + 16);
        if let Some((lo, hi)) = chunk.y_range() {
            min[1] = min[1].min(lo);
            max[1] = max[1].max(hi);
        }
    }
    if min[0] > max[0] {
        ExportRegion {
            min: [0; 3],
            max: [0; 3],
        }
    } else {
        ExportRegion { min, max }
    }
}

/// Format a count with thousands separators (e.g. `1,234,567`).
fn human_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Load a previously saved dock layout, rejecting empty or invalid ones.
fn restore_layout() -> Option<DockState<Panel>> {
    let text = std::fs::read_to_string(layout_path()).ok()?;
    let dock: DockState<Panel> = serde_json::from_str(&text).ok()?;
    (dock.iter_all_tabs().count() > 0).then_some(dock)
}

/// Smoothed frames-per-second estimator for the status bar.
struct FpsCounter {
    last: Instant,
    smoothed_dt: f32,
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self {
            last: Instant::now(),
            smoothed_dt: 1.0 / 60.0,
        }
    }
}

impl FpsCounter {
    fn tick(&mut self) {
        let dt = self.last.elapsed().as_secs_f32();
        self.last = Instant::now();
        self.smoothed_dt = self.smoothed_dt * 0.9 + dt * 0.1;
    }

    fn fps(&self) -> f32 {
        1.0 / self.smoothed_dt.max(1e-4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bedrock_parser::chunk::{BlockState, Chunk, SectionData};

    /// A minimal chunk at the given chunk coordinates with a single section
    /// at world Y=0 (so `y_range()` returns `(0, 16)`).
    fn stub_chunk(cx: i32, cz: i32) -> Chunk {
        Chunk::from_sections(
            cx,
            cz,
            vec![SectionData {
                y: 0,
                palette: vec![BlockState::new("minecraft:stone")],
                indices: vec![],
            }],
        )
    }

    #[test]
    fn needed_chunks_radius_zero_is_just_the_center() {
        let needed = compute_needed_chunks(10, -3, 0);
        assert_eq!(needed.len(), 1);
        assert!(needed.contains(&(10, -3)));
    }

    #[test]
    fn needed_chunks_radius_one_is_a_3x3_square() {
        let needed = compute_needed_chunks(0, 0, 1);
        assert_eq!(needed.len(), 9);
        for dx in -1..=1 {
            for dz in -1..=1 {
                assert!(needed.contains(&(dx, dz)), "missing ({dx}, {dz})");
            }
        }
    }

    #[test]
    fn needed_chunks_is_offset_around_center() {
        let needed = compute_needed_chunks(5, 7, 1);
        // Center + all eight neigours of chunk (5, 7).
        assert!(needed.contains(&(5, 7)));
        assert!(needed.contains(&(4, 6)));
        assert!(needed.contains(&(6, 8)));
        assert!(!needed.contains(&(0, 0)));
        assert_eq!(needed.len(), 9);
    }

    #[test]
    fn empty_chunk_map_yields_zero_bounds() {
        let empty: HashMap<(i32, i32), Chunk> = HashMap::new();
        let r = chunks_bounds_from_hash(&empty);
        assert_eq!(r.min, [0, 0, 0]);
        assert_eq!(r.max, [0, 0, 0]);
    }

    #[test]
    fn single_chunk_has_one_chunk_wide_bounds() {
        let chunk = stub_chunk(3, 4);
        let map: HashMap<(i32, i32), Chunk> = [((3, 4), chunk)].into_iter().collect();
        let r = chunks_bounds_from_hash(&map);
        // chunk (3, 4) spans blocks [48..64) on X, [64..80) on Z, [0..16) on Y.
        assert_eq!(r.min, [48, 0, 64]);
        assert_eq!(r.max, [64, 16, 80]);
    }

    #[test]
    fn bounds_span_min_and_max_of_all_chunks() {
        let chunks = [
            stub_chunk(-2, -5), // x:[-32,-16] z:[-80,-64]
            stub_chunk(1, 1),   // x:[16,32]   z:[16,32]
            stub_chunk(0, 0),   // x:[0,16]    z:[0,16]
        ];
        let map: HashMap<(i32, i32), Chunk> = chunks.into_iter().map(|c| ((c.x, c.z), c)).collect();
        let r = chunks_bounds_from_hash(&map);
        assert_eq!(r.min, [-32, 0, -80]);
        assert_eq!(r.max, [32, 16, 32]);
    }

    #[test]
    fn human_count_inserts_thousands_separators() {
        assert_eq!(human_count(0), "0");
        assert_eq!(human_count(5), "5");
        assert_eq!(human_count(123), "123");
        assert_eq!(human_count(1_000), "1,000");
        assert_eq!(human_count(1_234_567), "1,234,567");
        assert_eq!(human_count(usize::MAX), "18,446,744,073,709,551,615");
    }
}
