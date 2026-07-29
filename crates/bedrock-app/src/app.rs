//! Application shell: menu bar, status bar, dock area, and persistence.
//! Includes chunk streaming: dynamically loading/unloading chunks as the
//! camera moves, enabling arbitrarily large worlds.

use crate::loader::{load_active_world, load_chunks_for_region, load_one_chunk, ActiveWorld};
use bedrock_export::gltf::export_gltf;
use bedrock_export::obj::{export_obj_with_options, ExportOptions, ExportRegion, ExportStats};
use bedrock_parser::chunk::Chunk;
use bedrock_parser::detect::WorldSummary;
use bedrock_ui::dock::{self, Panel};
use bedrock_parser::mineways::build_mineways_tileset;
use bedrock_render::mesh::{chunks_to_meshes, ChunkMesh};
use bedrock_render::{ChunkBorder, SharedScene};
use bedrock_settings::ExportFormat;
use bedrock_settings::{layout_path, Settings, Theme};

/// Top-level sections in the sidebar.
///
/// Replaces the draggable dock. The design puts navigation in a fixed left
/// rail with one thing on screen at a time, which is far easier to find your
/// way around than an arrangement of panels the user has to assemble first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavSection {
    Home,
    Worlds,
    Exports,
    Settings,
}

impl NavSection {
    const ALL: [NavSection; 4] = [
        NavSection::Home,
        NavSection::Worlds,
        NavSection::Exports,
        NavSection::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            NavSection::Home => "Home",
            NavSection::Worlds => "Worlds",
            NavSection::Exports => "Exports",
            NavSection::Settings => "Settings",
        }
    }

    fn heading(self) -> &'static str {
        match self {
            NavSection::Home => "Project Bedrock",
            NavSection::Worlds => "Worlds",
            NavSection::Exports => "Exports",
            NavSection::Settings => "Settings",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            NavSection::Home => "Prepare for an adventure of limitless possibilities",
            NavSection::Worlds => "Every save found on this machine",
            NavSection::Exports => "Choose an area and send it to Blender",
            NavSection::Settings => "Preferences and diagnostics",
        }
    }

    fn tabs(self) -> &'static [&'static str] {
        match self {
            NavSection::Home => &["Explore", "Map", "Details", "Log"],
            NavSection::Worlds => &["Browse"],
            NavSection::Exports => &["Region", "Log"],
            NavSection::Settings => &["General", "Background", "Debug", "Log"],
        }
    }
}

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
    /// Which sidebar section is showing.
    nav: NavSection,
    /// Which tab within that section is showing.
    nav_tab: usize,
    /// Sidebar search box contents.
    search: String,
    /// Cube mark at the top of the sidebar.
    logo_tex: Option<egui::TextureHandle>,
    /// Running background video/audio playback, if configured and ffmpeg is
    /// available. `None` covers "not configured", "ffmpeg missing", and
    /// "failed to start" alike -- the UI does not need to tell those apart.
    background: Option<crate::background_media::BackgroundMedia>,
    /// (path, audio-on) this session's `background` was started with, so a
    /// change in Settings can be noticed and acted on once, not re-triggered
    /// every frame. Blur is deliberately absent: it no longer restarts
    /// anything.
    background_started_for: Option<(std::path::PathBuf, bool)>,
    background_tex: Option<egui::TextureHandle>,
    /// Creeper mark for the current-world row.
    creeper_tex: Option<egui::TextureHandle>,
    overview_tex: Option<egui::TextureHandle>,
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
            nav: NavSection::Home,
            nav_tab: 0,
            search: String::new(),
            logo_tex: None,
            background: None,
            background_started_for: None,
            background_tex: None,
            creeper_tex: None,
            overview_tex: None,
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

    /// Reconcile the running background video/audio against Settings, pull
    /// any new frame, and paint it on egui's `Background` layer -- beneath
    /// every panel, so it is only visible where a panel lets it through.
    fn update_background_media(&mut self, ctx: &egui::Context) {
        // Only the file and the audio toggle can force a restart. Blur used
        // to be part of this key, which is why changing it froze the picture
        // and jumped the audio to a new random point: it tore down and
        // respawned both ffmpeg processes. It is now applied per frame on the
        // decode thread instead.
        let wanted = self
            .settings
            .background_media_path
            .clone()
            .map(|p| (p, self.settings.background_media_audio));

        if wanted != self.background_started_for {
            tracing::info!(
                "background media restart: {:?} -> {:?}",
                self.background_started_for, wanted
            );
            // Old ffmpeg children are killed by `BackgroundMedia::drop`, so
            // simply replacing the value here is enough to stop them.
            self.background = wanted.as_ref().and_then(|(path, audio)| {
                crate::background_media::BackgroundMedia::start(
                    path,
                    *audio,
                    self.settings.background_media_blur,
                    self.settings.background_media_volume,
                )
            });
            self.background_started_for = wanted;
            // Recorded even on failure (ffmpeg missing, bad file): otherwise
            // a failed start would retry, and log, every single frame.
        }

        let Some(background) = self.background.as_mut() else {
            return;
        };

        // egui repaints on demand: with no input it simply stops calling this,
        // and a video that only advances when the mouse moves is not a video.
        // Nothing drained the frame channel either, so the decode thread
        // filled its buffer and blocked after a couple of frames -- which is
        // why the background appeared frozen or absent rather than merely
        // stuttering. Asking for a repaint at the decode frame rate is what
        // makes it play on its own.
        ctx.request_repaint_after(std::time::Duration::from_millis(
            (1000 / crate::background_media::FRAME_FPS.max(1)) as u64,
        ));
        // Volume, unlike blur or the file, changes live -- no restart.
        background.set_volume(self.settings.background_media_volume);
        background.set_blur(self.settings.background_media_blur);
        let frame_arrived = if let Some(frame) = background.latest_frame() {
            // Size comes from the frame, not a constant: blurred frames are
            // computed at a reduced size, so it varies with the setting.
            let (w, h) = (frame.width, frame.height);
            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &frame.rgba);
            match &mut self.background_tex {
                Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.background_tex =
                        Some(ctx.load_texture("ui/background_media", image, egui::TextureOptions::LINEAR));
                }
            }
            true
        } else {
            false
        };
        let Some(tex) = &self.background_tex else {
            return;
        };

        let screen = ctx.content_rect();
        let [tw, th] = tex.size();
        let tex_aspect = tw as f32 / th.max(1) as f32;
        let screen_aspect = (screen.width() / screen.height()).max(1e-3);

        // Cover-fit: crop rather than letterbox. This is an ambient backdrop,
        // not content to see in full, and it is already blurred, so cropping
        // a margin off any edge costs nothing visible.
        let uv = if screen_aspect > tex_aspect {
            let crop = (1.0 - tex_aspect / screen_aspect) * 0.5;
            egui::Rect::from_min_max(egui::pos2(0.0, crop), egui::pos2(1.0, 1.0 - crop))
        } else {
            let crop = (1.0 - screen_aspect / tex_aspect) * 0.5;
            egui::Rect::from_min_max(egui::pos2(crop, 0.0), egui::pos2(1.0 - crop, 1.0))
        };

        ctx.layer_painter(egui::LayerId::background())
            .image(tex.id(), screen, uv, egui::Color32::WHITE);
        let _ = frame_arrived;
    }

    /// Panel-fill alpha for the sidebar/content backdrops: fully opaque when
    /// no background video is running (identical to before this feature
    /// existed), otherwise driven by the opacity setting. Floored well above
    /// zero so text stays legible at any slider position.
    fn panel_veil_alpha(&self) -> u8 {
        if self.background.is_none() {
            return 255;
        }
        let t = self.settings.background_media_opacity.clamp(0.0, 1.0);
        // Range widened from 255..=125 (49% see-through at most) to 255..=45
        // (82%): verified by direct instrumentation that frames, the screen
        // rect, and this alpha value were all already correct end-to-end at
        // max opacity, yet at 125 the effect read as "nothing changed" --
        // dark blurred video under an already-dark panel just wasn't a
        // visible enough shift. Floored at 45, not 0, so panel text stays
        // legible at every setting rather than the UI vanishing entirely.
        (255.0 - t * 210.0).round() as u8
    }

    /// Background media settings: pick a local video, toggle its audio, set
    /// how much it shows through the UI.
    fn background_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(
                "Plays a local video file, blurred, behind the app, with a random \
                 start point. The file stays wherever it already is on disk -- only \
                 its path is remembered.",
            )
            .color(bedrock_ui::theme::MUTED)
            .size(12.0),
        );
        ui.add_space(10.0);

        if !crate::background_media::ffmpeg_available() {
            ui.colored_label(
                egui::Color32::from_rgb(224, 176, 90),
                "ffmpeg was not found on PATH, so this is unavailable. \
                 Install it (e.g. `winget install ffmpeg`) and restart the app.",
            );
            ui.add_space(8.0);
        }

        ui.horizontal(|ui| {
            let current = self
                .settings
                .background_media_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "No file selected".to_owned());
            ui.label(current);
            if ui.button("Browse...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Video", &["webm", "mp4", "mkv", "mov", "avi"])
                    .pick_file()
                {
                    self.settings.background_media_path = Some(path);
                    self.settings.save();
                }
            }
            if self.settings.background_media_path.is_some() && ui.button("Clear").clicked() {
                self.settings.background_media_path = None;
                self.settings.save();
            }
        });

        ui.add_space(10.0);
        if ui
            .checkbox(&mut self.settings.background_media_audio, "Play audio")
            .changed()
        {
            self.settings.save();
        }
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label("How much shows through the UI:");
            if ui
                .add(egui::Slider::new(&mut self.settings.background_media_opacity, 0.0..=1.0).show_value(false))
                .changed()
            {
                self.settings.save();
            }
        });

        ui.horizontal(|ui| {
            ui.label("Volume:");
            // Applied continuously in `update_background_media`, not only on
            // release: this is a `Sink::set_volume` call, not a process
            // restart, so there is no cost to updating it every frame the
            // slider is moving.
            if ui
                .add(egui::Slider::new(&mut self.settings.background_media_volume, 0.0..=1.0).show_value(false))
                .changed()
            {
                self.settings.save();
            }
        });

        ui.horizontal(|ui| {
            ui.label("Blur:");
            // Continuous, like volume: blur is applied per frame on the decode
            // thread now, so dragging costs nothing and takes effect at once.
            if ui
                .add(egui::Slider::new(&mut self.settings.background_media_blur, 0.0..=30.0).show_value(false))
                .changed()
            {
                self.settings.save();
            }
        });
    }

    /// Load a bundled PNG as a crisp, never-filtered texture.
    fn ui_texture(
        ctx: &egui::Context,
        slot: &mut Option<egui::TextureHandle>,
        name: &'static str,
        bytes: &'static [u8],
        options: egui::TextureOptions,
    ) -> egui::TextureHandle {
        slot.get_or_insert_with(|| {
            let image = image::load_from_memory(bytes)
                .expect("bundled UI art must be valid")
                .to_rgba8();
            let (w, h) = image.dimensions();
            ctx.load_texture(
                name,
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &image),
                options,
            )
        })
        .clone()
    }

    /// Left navigation rail, following the design mock-up.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Linear, not nearest: this is smooth flat-shaded vector-style art
        // (see tools/generate_icon.py), displayed well below its 256px source
        // size, unlike the pixel-art creeper below it. Nearest
        // filtering would turn its clean anti-aliased edges jagged.
        let logo = Self::ui_texture(
            &ctx,
            &mut self.logo_tex,
            "ui/cube_logo",
            include_bytes!("../../../assets/ui/cube_logo.png"),
            egui::TextureOptions::LINEAR,
        );
        let creeper = Self::ui_texture(
            &ctx,
            &mut self.creeper_tex,
            "ui/creeper",
            include_bytes!("../../../assets/ui/creeper.png"),
            egui::TextureOptions::NEAREST,
        );

        let veil = self.panel_veil_alpha();
        egui::Panel::left("nav_rail")
            .exact_size(196.0)
            .frame(
                egui::Frame::NONE
                    .fill(bedrock_ui::theme::SIDEBAR.gamma_multiply_u8(veil))
                    .inner_margin(egui::Margin::symmetric(12, 14)),
            )
            .show(ui, |ui| {
                ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(56.0, 56.0)));
                ui.add_space(14.0);

                egui::Frame::NONE
                    .fill(ui.visuals().extreme_bg_color)
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Search...")
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY),
                        );
                    });

                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new("Navigate")
                        .size(10.0)
                        .color(bedrock_ui::theme::MUTED),
                );
                ui.add_space(6.0);

                for section in NavSection::ALL {
                    if Self::nav_row(ui, section.label(), None, self.nav == section) {
                        self.nav = section;
                        self.nav_tab = 0;
                    }
                }

                ui.add_space(18.0);
                ui.label(
                    egui::RichText::new("My Topics")
                        .size(10.0)
                        .color(bedrock_ui::theme::MUTED),
                );
                ui.add_space(6.0);
                let world = self
                    .current_world
                    .clone()
                    .unwrap_or_else(|| "No world open".to_owned());
                Self::nav_row(ui, &world, Some(&creeper), self.current_world.is_some());
            });
    }

    /// One sidebar row. Returns true when clicked.
    ///
    /// The selected row inverts -- pale fill, dark text -- which is how the
    /// mock-up marks it, rather than by a coloured accent.
    fn nav_row(
        ui: &mut egui::Ui,
        label: &str,
        icon: Option<&egui::TextureHandle>,
        selected: bool,
    ) -> bool {
        let height = 30.0;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::click(),
        );
        let painter = ui.painter_at(rect);
        if selected {
            painter.rect_filled(rect, egui::CornerRadius::same(7), bedrock_ui::theme::TEXT);
        } else if response.hovered() {
            painter.rect_filled(rect, egui::CornerRadius::same(7), bedrock_ui::theme::CARD);
        }
        let fg = if selected {
            bedrock_ui::theme::SIDEBAR
        } else {
            bedrock_ui::theme::TEXT
        };
        let mut x = rect.left() + 9.0;
        if let Some(icon) = icon {
            let size = 20.0;
            let icon_rect = egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - size * 0.5),
                egui::vec2(size, size),
            );
            painter.image(
                icon.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            x += size + 8.0;
        }
        painter.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            fg,
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response.clicked()
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
                        let (stats, over_budget) = {
                            let scene = self.scene.lock().expect("scene lock poisoned");
                            (scene.mesh_stats, scene.chunks_over_budget)
                        };
                        if stats.1 > 0 {
                            ui.separator();
                            ui.weak(format!("{} triangles", human_count(stats.1)));
                        }
                        // A partly drawn world otherwise looks like a failed
                        // load. Say so, and say what to do about it.
                        if over_budget > 0 {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(format!(
                                    "⚠ {} chunk(s) over GPU budget",
                                    human_count(over_budget)
                                ))
                                .color(egui::Color32::from_rgb(0xE0, 0xA0, 0x30)),
                            )
                            .on_hover_text(
                                "The loaded world needs more GPU memory than the \
                                 viewport will allocate, so the chunks furthest \
                                 from the camera are not drawn. Lower the load \
                                 radius in Settings, or move the camera to bring \
                                 a different area into the budget.",
                            );
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
        self.update_background_media(&ctx);
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

        self.sidebar(ui);

        let veil = self.panel_veil_alpha();
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(ui.visuals().panel_fill.gamma_multiply_u8(veil)))
            .show(ui, |ui| {
            let section = self.nav;

            // Heading block, matching the mock-up: a large title, a quiet
            // subtitle, then the tab row underlined beneath it.
            ui.add_space(18.0);
            ui.label(
                egui::RichText::new(section.heading())
                    .size(40.0)
                    .strong()
                    .color(bedrock_ui::theme::TEXT),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(section.subtitle())
                    .size(13.0)
                    .color(bedrock_ui::theme::MUTED),
            );
            ui.add_space(16.0);

            let tabs = section.tabs();
            if self.nav_tab >= tabs.len() {
                self.nav_tab = 0;
            }
            ui.horizontal(|ui| {
                for (i, name) in tabs.iter().enumerate() {
                    let selected = i == self.nav_tab;
                    let colour = if selected {
                        bedrock_ui::theme::TEXT
                    } else {
                        bedrock_ui::theme::MUTED
                    };
                    let text = egui::RichText::new(*name).size(14.0).color(colour);
                    let text = if selected { text.strong() } else { text };
                    let response = ui
                        .add(egui::Label::new(text).sense(egui::Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        self.nav_tab = i;
                    }
                    // Underline the active tab rather than boxing it, as the
                    // mock-up does.
                    if selected {
                        let r = response.rect;
                        ui.painter().line_segment(
                            [
                                egui::pos2(r.left(), r.bottom() + 5.0),
                                egui::pos2(r.right(), r.bottom() + 5.0),
                            ],
                            egui::Stroke::new(2.0, bedrock_ui::theme::TEXT),
                        );
                    }
                    ui.add_space(12.0);
                }
            });
            ui.add_space(14.0);
            ui.separator();
            ui.add_space(8.0);

            let tab = tabs[self.nav_tab];
            let overview = self.overview_tex.as_ref().map(|texture| {
                bedrock_ui::panels::OverviewData {
                    texture,
                    origin: self.overview_origin,
                }
            });
            match (section, tab) {
                (NavSection::Home, "Explore") => {
                    if self.active_world.is_some() {
                        bedrock_ui::panels::viewport(ui, &self.scene);
                    } else {
                        // The viewport's own paint callback clears to a solid
                        // colour and blits it with no blending -- correct once
                        // a world is loaded, but with nothing loaded it was
                        // an opaque rectangle sitting on top of the background
                        // video and the translucent panel fill behind it,
                        // hiding both completely regardless of the blur or
                        // opacity sliders. Skipping the 3D paint callback
                        // entirely here, rather than trying to make it
                        // draw transparently, is what lets that show through.
                        ui.allocate_space(ui.available_size());
                        ui.put(
                            egui::Rect::from_center_size(
                                ui.max_rect().center(),
                                egui::vec2(320.0, 40.0),
                            ),
                            egui::Label::new(
                                egui::RichText::new("Open a world to see it here")
                                    .color(bedrock_ui::theme::MUTED)
                                    .size(14.0),
                            ),
                        );
                    }
                }
                (NavSection::Home, "Map") => {
                    bedrock_ui::panels::overview(ui, overview, &mut self.export_region);
                }
                (NavSection::Exports, "Region") => {
                    // Settings and map on the left, the 3D view on the right.
                    // Choosing an export region from coordinate boxes alone is
                    // working blind: the viewport already draws the selection
                    // as a box in the world, so the only thing missing was
                    // having it on screen while the numbers are being typed.
                    let world_loaded = self.active_world.is_some();
                    egui::Panel::left("export_tools")
                        .resizable(true)
                        .default_size(360.0)
                        .size_range(300.0..=560.0)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                            right: 12,
                            ..Default::default()
                        }))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    bedrock_ui::panels::export_settings(
                                        ui,
                                        &mut self.settings.export,
                                        &mut self.export_region,
                                        world_loaded,
                                        &mut self.export_requested,
                                    );
                                    ui.add_space(8.0);
                                    bedrock_ui::panels::overview(
                                        ui,
                                        overview,
                                        &mut self.export_region,
                                    );
                                });
                        });
                    if world_loaded {
                        bedrock_ui::panels::viewport(ui, &self.scene);
                    } else {
                        // Same reason as Home > Explore: with no world the 3D
                        // paint callback is an opaque rectangle over the
                        // background, so skip it rather than draw a black box.
                        ui.allocate_space(ui.available_size());
                        ui.put(
                            egui::Rect::from_center_size(
                                ui.max_rect().center(),
                                egui::vec2(320.0, 40.0),
                            ),
                            egui::Label::new(
                                egui::RichText::new("Open a world to see the area you are exporting")
                                    .color(bedrock_ui::theme::MUTED)
                                    .size(14.0),
                            ),
                        );
                    }
                }
                (NavSection::Home, "Details") => bedrock_ui::panels::properties(ui),
                (NavSection::Worlds, _) => {
                    bedrock_ui::world_browser::world_browser(ui, &mut self.world_browser);
                }
                (NavSection::Settings, "General") => {
                    bedrock_ui::panels::export_settings(
                        ui,
                        &mut self.settings.export,
                        &mut self.export_region,
                        self.active_world.is_some(),
                        &mut self.export_requested,
                    );
                }
                (NavSection::Settings, "Background") => self.background_settings(ui),
                (NavSection::Settings, "Debug") => {
                    bedrock_ui::panels::debug_settings(ui, &mut self.settings.debug);
                }
                _ => {
                    bedrock_ui::panels::output_log(ui, &self.log, &mut self.auto_scroll_log);
                }
            }
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
