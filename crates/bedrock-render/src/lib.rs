//! # bedrock-render
//!
//! GPU viewport rendering for Project Bedrock, built on wgpu (via egui-wgpu).
//!
//! Hard boundary: this crate renders parsed world data for navigation. It
//! never parses worlds (it consumes [`bedrock_parser::chunk::Chunk`]) and
//! never exports.
//!
//! The scene renders into an offscreen texture with its own depth buffer in
//! the callback's `prepare` step; `paint` then blits that texture into the
//! egui render pass, which has no depth attachment of its own.

pub mod math;
pub mod mesh;

use egui_wgpu::wgpu;
use egui_wgpu::{CallbackResources, CallbackTrait, RenderState, ScreenDescriptor};
use math::Camera;
use mesh::ChunkMesh;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Clear color of the 3D scene (matches the UI's dark background).
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.043,
    g: 0.047,
    b: 0.059,
    a: 1.0,
};

/// Debug visualisation toggles passed from the app to the renderer each frame.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct DebugState {
    /// Overlay FPS + vertex/triangle counts on the viewport.
    pub show_stats: bool,
    /// Draw wireframe boxes around every loaded chunk.
    pub show_chunk_borders: bool,
    /// Overlay mesh edges as a wireframe.
    pub show_wireframe: bool,
}

/// A chunk-aligned axis-aligned box for border rendering.
#[derive(Debug, Clone, Copy)]
pub struct ChunkBorder {
    /// Minimum corner in world block coordinates.
    pub min: [i32; 3],
    /// Maximum corner (exclusive) in world block coordinates.
    pub max: [i32; 3],
}

/// CPU-side scene state shared between the UI thread and the paint callback.
/// The app writes camera/input/pending meshes; the callback reads them in
/// `prepare`. Wrapped in `Arc<Mutex<…>>` — see [`ViewportRenderer::shared`].
#[derive(Default)]
pub struct SharedScene {
    /// Orbit camera, updated by viewport input each frame.
    pub camera: Option<Camera>,
    /// Newly meshed chunks waiting to be uploaded to the GPU.
    /// Stored as a map so the renderer can update individual chunks.
    pub pending_chunks: Option<Vec<mesh::ChunkMesh>>,
    /// Updated atlas texture to upload (if any).
    pub pending_atlas: Option<mesh::AtlasPixels>,
    /// `(vertices, triangles)` of all chunks currently on the GPU.
    pub mesh_stats: (usize, usize),
    /// Desired offscreen size in physical pixels (viewport rect), updated by
    /// the UI each frame. Zero means "hidden".
    pub desired_size: [u32; 2],
    /// Export region box to draw as a wireframe, `(min, max)` in world
    /// block coordinates. `None` hides the box.
    pub region: Option<([f32; 3], [f32; 3])>,
    /// Debug visualisation flags.
    pub debug: DebugState,
    /// Chunk border boxes for wireframe rendering.
    pub chunk_borders: Vec<ChunkBorder>,
    /// Player's position in world coordinates (for "Snap to Player").
    pub player_pos: Option<[f32; 3]>,
}

impl SharedScene {
    /// Wrap a fresh scene in its shared pointer.
    pub fn new_shared() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::default()))
    }
}

/// WGSL for the world mesh: lambert-lit colored cubes with depth testing.
const MESH_SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
};

@group(1) @binding(0)
var atlas_texture: texture_2d<f32>;
@group(1) @binding(1)
var atlas_sampler: sampler;

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = uniforms.view_proj * vec4<f32>(in.pos, 1.0);
    out.uv = in.uv;
    out.normal = in.normal;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas_texture, atlas_sampler, in.uv);
    // Discard near-invisible pixels (cutouts like leaves, flowers, doors).
    if texel.a < 0.1 { discard; }
    let light = normalize(vec3<f32>(0.5, 1.0, 0.3));
    let brightness = 0.55 + 0.45 * max(dot(in.normal, light), 0.0);
    return vec4<f32>(texel.rgb * brightness, texel.a);
}
"#;

/// WGSL for the export-region wireframe: position-only lines in a constant
/// color, reusing the mesh uniform (group 0, binding 0).
const LINE_SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return uniforms.view_proj * vec4<f32>(pos, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.55, 0.95, 0.35, 1.0);
}
"#;

/// WGSL for chunk-border lines: light-blue constant color.
const CHUNK_BORDER_SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return uniforms.view_proj * vec4<f32>(pos, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.3, 0.6, 1.0, 0.8);
}
"#;

/// WGSL for mesh wireframe overlay: orange lines.
const WIREFRAME_SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return uniforms.view_proj * vec4<f32>(pos, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.65, 0.0, 0.6);
}
"#;

/// WGSL that blits the offscreen scene texture into the egui pass.
const BLIT_SHADER: &str = r#"
@group(0) @binding(0)
var scene_texture: texture_2d<f32>;
@group(0) @binding(1)
var scene_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0)
    );
    var out: VsOut;
    out.position = vec4<f32>(positions[index], 0.0, 1.0);
    // Flip V: NDC +Y is up, but texture v=0 is the top row.
    out.uv = vec2<f32>(positions[index].x * 0.5 + 0.5, 0.5 - positions[index].y * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let color = textureSample(scene_texture, scene_sampler, in.uv);
    return vec4<f32>(color.rgb, 1.0);
}
"#;

/// A chunk mesh cached on the GPU with its world-space bounds for frustum
/// culling.
struct CachedChunk {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    #[allow(dead_code)]
    bounds_min: [f32; 3],
    #[allow(dead_code)]
    bounds_max: [f32; 3],
}

/// GPU resources for the viewport, stored in egui's callback resources.
pub struct ViewportRenderer {
    shared: Arc<Mutex<SharedScene>>,
    mesh_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    line_buffer: wgpu::Buffer,
    chunk_border_pipeline: wgpu::RenderPipeline,
    chunk_border_buffer: Option<wgpu::Buffer>,
    chunk_border_count: u32,
    wireframe_pipeline: wgpu::RenderPipeline,
    wireframe_index_buffer: Option<wgpu::Buffer>,
    wireframe_index_count: u32,
    atlas_bind_group_layout: wgpu::BindGroupLayout,
    atlas_bind_group: Option<wgpu::BindGroup>,
    atlas_sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    mesh_bind_group: wgpu::BindGroup,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_bind_group: Option<wgpu::BindGroup>,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    color_view: Option<wgpu::TextureView>,
    depth_view: Option<wgpu::TextureView>,
    size: [u32; 2],
    /// Per-chunk GPU buffers for frustum-culled drawing.
    chunk_cache: HashMap<mesh::ChunkId, CachedChunk>,
    /// Combined vertex buffer (all chunks concatenated) used by the wireframe
    /// overlay, whose indices reference vertices by global offset.
    combined_vertex_buffer: Option<wgpu::Buffer>,
}

impl ViewportRenderer {
    /// Create the viewport pipelines on egui's wgpu device.
    pub fn new(render_state: &RenderState, shared: Arc<Mutex<SharedScene>>) -> Self {
        let device = &render_state.device;
        let format = render_state.target_format;

        // --- Mesh pipeline (offscreen, with depth) ---
        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bedrock/mesh shader"),
            source: wgpu::ShaderSource::Wgsl(MESH_SHADER.into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bedrock/mesh uniforms"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mesh_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("bedrock/mesh bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let mesh_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bedrock/mesh bind group"),
            layout: &mesh_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let atlas_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("bedrock/atlas bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bedrock/atlas sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let mesh_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bedrock/mesh pipeline layout"),
            bind_group_layouts: &[
                Some(&mesh_bind_group_layout),
                Some(&atlas_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bedrock/mesh pipeline"),
            layout: Some(&mesh_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<mesh::Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // No blending. The fragment shader already discards texels
                    // below an alpha threshold, which is how Minecraft draws
                    // cutout blocks — leaves, plants, rails — and unlike
                    // blending it does not care what order geometry arrives
                    // in. Blending on top of that made every partly
                    // transparent texel composite against whatever chunk
                    // happened to be drawn before it, smearing the terrain
                    // into vertical streaks that shifted as the camera moved.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // Reverse-Z: nearer fragments have HIGHER depth values,
                // so we use Greater (pass the fragment closest to the camera).
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- Region wireframe pipeline (lines, always on top) ---
        let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bedrock/line shader"),
            source: wgpu::ShaderSource::Wgsl(LINE_SHADER.into()),
        });
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bedrock/line pipeline"),
            layout: Some(&mesh_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &line_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &line_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let line_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bedrock/region wireframe"),
            size: 24 * 12,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- Chunk border pipeline (line list, light blue) ---
        let chunk_border_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bedrock/chunk border shader"),
            source: wgpu::ShaderSource::Wgsl(CHUNK_BORDER_SHADER.into()),
        });
        let chunk_border_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bedrock/chunk border pipeline"),
                layout: Some(&mesh_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &chunk_border_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 12,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &chunk_border_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineList,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        // --- Wireframe pipeline (line list, orange) ---
        let wireframe_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bedrock/wireframe shader"),
            source: wgpu::ShaderSource::Wgsl(WIREFRAME_SHADER.into()),
        });
        let wireframe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bedrock/wireframe pipeline"),
            layout: Some(&mesh_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &wireframe_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &wireframe_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- Blit pipeline (into the egui pass) ---
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bedrock/blit shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });
        let blit_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("bedrock/blit bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bedrock/blit sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bedrock/blit pipeline layout"),
            bind_group_layouts: &[Some(&blit_bind_group_layout)],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bedrock/blit pipeline"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        tracing::info!("Viewport renderer pipelines created (mesh + blit)");
        Self {
            shared,
            mesh_pipeline,
            blit_pipeline,
            line_pipeline,
            line_buffer,
            chunk_border_pipeline,
            chunk_border_buffer: None,
            chunk_border_count: 0,
            wireframe_pipeline,
            wireframe_index_buffer: None,
            wireframe_index_count: 0,
            atlas_bind_group_layout,
            atlas_bind_group: None,
            atlas_sampler,
            uniform_buffer,
            mesh_bind_group,
            blit_bind_group_layout,
            blit_bind_group: None,
            sampler,
            format,
            color_view: None,
            depth_view: None,
            size: [0, 0],
            chunk_cache: HashMap::new(),
            combined_vertex_buffer: None,
        }
    }

    /// (Re)create the offscreen color/depth targets and the blit bind group.
    fn resize_targets(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
    ) {
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bedrock/offscreen color"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bedrock/offscreen depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        self.blit_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bedrock/blit bind group"),
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
        self.color_view = Some(color_view);
        self.depth_view = Some(depth_view);
        self.size = size;
    }

    /// Upload chunk border line vertices for all loaded chunks.
    fn upload_chunk_borders(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        borders: &[ChunkBorder],
    ) {
        // Each chunk → 12 edges × 2 vertices = 24 vertices.
        let mut verts: Vec<[f32; 3]> = Vec::with_capacity(borders.len() * 24);
        for b in borders {
            let [x0, y0, z0] = [b.min[0] as f32, b.min[1] as f32, b.min[2] as f32];
            let [x1, y1, z1] = [b.max[0] as f32, b.max[1] as f32, b.max[2] as f32];
            // Bottom ring
            verts.extend_from_slice(&[
                [x0, y0, z0],
                [x1, y0, z0],
                [x1, y0, z0],
                [x1, y0, z1],
                [x1, y0, z1],
                [x0, y0, z1],
                [x0, y0, z1],
                [x0, y0, z0],
                // Top ring
                [x0, y1, z0],
                [x1, y1, z0],
                [x1, y1, z0],
                [x1, y1, z1],
                [x1, y1, z1],
                [x0, y1, z1],
                [x0, y1, z1],
                [x0, y1, z0],
                // Pillars
                [x0, y0, z0],
                [x0, y1, z0],
                [x1, y0, z0],
                [x1, y1, z0],
                [x1, y0, z1],
                [x1, y1, z1],
                [x0, y0, z1],
                [x0, y1, z1],
            ]);
        }
        self.chunk_border_count = verts.len() as u32;
        if verts.is_empty() {
            self.chunk_border_buffer = None;
            return;
        }
        let capacity = verts.len().next_power_of_two();
        let buf = self.chunk_border_buffer.get_or_insert_with(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bedrock/chunk borders"),
                size: (capacity * 12) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        if (capacity * 12) as u64 > buf.size() {
            *buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bedrock/chunk borders"),
                size: (capacity * 12) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(buf, 0, bytemuck::cast_slice(&verts));
    }

    /// Upload a texture atlas to the GPU. Destroys and replaces the old one.
    fn upload_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &mesh::AtlasPixels,
    ) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bedrock/atlas"),
            size: wgpu::Extent3d {
                width: atlas.width,
                height: atlas.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas.width * 4),
                rows_per_image: Some(atlas.height),
            },
            wgpu::Extent3d {
                width: atlas.width,
                height: atlas.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.atlas_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bedrock/atlas bind group"),
            layout: &self.atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.atlas_sampler),
                },
            ],
        }));
    }

    /// Process pending chunk meshes: upsert each chunk's GPU buffers, remove
    /// stale entries, then rebuild the combined vertex/wireframe buffers.
    fn update_chunk_cache(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        meshes: Vec<ChunkMesh>,
    ) {
        let mut seen: HashMap<mesh::ChunkId, bool> = HashMap::new();
        // Build combined vertex data + wireframe indices.
        let mut combined_verts: Vec<mesh::Vertex> = Vec::new();
        let mut wire_indices: Vec<u32> = Vec::new();
        let mut base_vertex: u32 = 0;

        for chunk_mesh in meshes {
            let key = chunk_mesh.id;
            seen.insert(key, true);

            // Per-chunk GPU buffers for frustum-culled drawing.
            let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("bedrock/chunk/{:?} v", key)),
                size: (chunk_mesh.vertices.len() * size_of::<mesh::Vertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let ibuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("bedrock/chunk/{:?} i", key)),
                size: (chunk_mesh.indices.len() * size_of::<u32>()) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&vbuf, 0, bytemuck::cast_slice(&chunk_mesh.vertices));
            queue.write_buffer(&ibuf, 0, bytemuck::cast_slice(&chunk_mesh.indices));

            // Append to combined vertex buffer (for wireframe overlay).
            combined_verts.extend_from_slice(&chunk_mesh.vertices);

            // Generate wireframe edges from this chunk's indices.
            for tri in chunk_mesh.indices.chunks(3) {
                if tri.len() < 3 {
                    continue;
                }
                let i0 = tri[0] + base_vertex;
                let i1 = tri[1] + base_vertex;
                let i2 = tri[2] + base_vertex;
                wire_indices.extend_from_slice(&[i0, i1, i1, i2, i2, i0]);
            }
            base_vertex += chunk_mesh.vertices.len() as u32;

            self.chunk_cache.insert(
                key,
                CachedChunk {
                    vertex_buffer: vbuf,
                    index_buffer: ibuf,
                    index_count: chunk_mesh.indices.len() as u32,
                    bounds_min: chunk_mesh.bounds_min.map(|v| v as f32),
                    bounds_max: chunk_mesh.bounds_max.map(|v| v as f32),
                },
            );
        }
        // Evict chunks that are no longer present.
        self.chunk_cache.retain(|key, _| seen.contains_key(key));

        // Upload combined vertex + wireframe buffers (for wireframe overlay).
        // If the combined data exceeds the GPU's max buffer size, skip both
        // — per-chunk drawing still works.
        const MAX_COMBINED_BUF: u64 = 256 * 1024 * 1024; // 256 MiB
        let cv_size = (combined_verts.len() * size_of::<mesh::Vertex>()) as u64;
        let wf_size = (wire_indices.len() * size_of::<u32>()) as u64;
        if cv_size > MAX_COMBINED_BUF || wf_size > MAX_COMBINED_BUF || combined_verts.is_empty() {
            self.combined_vertex_buffer = None;
            self.wireframe_index_buffer = None;
            self.wireframe_index_count = 0;
        } else {
            let cv_buf = self.combined_vertex_buffer.get_or_insert_with(|| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bedrock/combined vertices"),
                    size: cv_size,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            });
            if cv_size > cv_buf.size() {
                *cv_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bedrock/combined vertices"),
                    size: cv_size,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            queue.write_buffer(cv_buf, 0, bytemuck::cast_slice(&combined_verts));

            let wf_buf = self.wireframe_index_buffer.get_or_insert_with(|| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bedrock/wireframe indices"),
                    size: wf_size,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            });
            if wf_size > wf_buf.size() {
                *wf_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bedrock/wireframe indices"),
                    size: wf_size,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            queue.write_buffer(wf_buf, 0, bytemuck::cast_slice(&wire_indices));
            self.wireframe_index_count = wire_indices.len() as u32;
        }
    }

    /// Extract the six frustum planes from a view-projection matrix.
    /// Returns `[left, right, bottom, top, near, far]` as `(nx, ny, nz, d)`
    /// where the plane equation is `nx*x + ny*y + nz*z + d = 0`.
    ///
    /// The matrix is stored **column-major** (`mvp[col][row]`). The Gribb/
    /// Hartmann algorithm assumes row-major access, so we extract rows first.
    ///
    /// For the **Reverse-Z** projection with wgpu's `[0, 1]` depth range:
    /// - Near plane (z_view = -near) maps to clip.z/clip.w = 1.0, i.e. clip.z = clip.w
    ///   → plane = row3 - row2
    /// - Far plane (z_view = -far) maps to clip.z/clip.w = 0.0, i.e. clip.z = 0
    ///   → plane = row2
    /// - Left/right/bottom/top use the standard Gribb formulas (unchanged by Reverse-Z).
    #[allow(dead_code)]
    fn extract_frustum_planes(mvp: &[[f32; 4]; 4]) -> [[f32; 4]; 6] {
        // Extract rows from column-major storage: mvp[col][row]
        let r0 = [mvp[0][0], mvp[1][0], mvp[2][0], mvp[3][0]];
        let r1 = [mvp[0][1], mvp[1][1], mvp[2][1], mvp[3][1]];
        let r2 = [mvp[0][2], mvp[1][2], mvp[2][2], mvp[3][2]];
        let r3 = [mvp[0][3], mvp[1][3], mvp[2][3], mvp[3][3]];

        let mut planes = [[0.0f32; 4]; 6];
        // Four side planes (same for any depth convention).
        planes[0] = Self::vec4_add(r3, r0); // Left
        planes[1] = Self::vec4_sub(r3, r0); // Right
        planes[2] = Self::vec4_add(r3, r1); // Bottom
        planes[3] = Self::vec4_sub(r3, r1); // Top
                                            // Reverse-Z [0,1] near/far planes.
        planes[4] = Self::vec4_sub(r3, r2); // Near  (clip.z = clip.w)
        planes[5] = r2; // Far   (clip.z = 0)

        // Normalise each plane (scale so the normal is unit length).
        for p in planes.iter_mut() {
            let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            if len > 1e-8 {
                p[0] /= len;
                p[1] /= len;
                p[2] /= len;
                p[3] /= len;
            }
        }
        planes
    }

    /// Element-wise 4-vector addition.
    #[allow(dead_code)]
    fn vec4_add(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
    }

    /// Element-wise 4-vector subtraction.
    #[allow(dead_code)]
    fn vec4_sub(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
    }

    /// Test an AABB against the six frustum planes. Returns `true` if the box
    /// is at least partially inside the frustum (or entirely inside).
    #[allow(dead_code)]
    fn aabb_in_frustum(min: [f32; 3], max: [f32; 3], planes: &[[f32; 4]; 6]) -> bool {
        for &p in planes {
            // Find the p-vertex (most negative along the plane normal).
            let px = if p[0] >= 0.0 { min[0] } else { max[0] };
            let py = if p[1] >= 0.0 { min[1] } else { max[1] };
            let pz = if p[2] >= 0.0 { min[2] } else { max[2] };
            // If the p-vertex is behind the plane, the whole box is outside.
            if p[0] * px + p[1] * py + p[2] * pz + p[3] < 0.0 {
                return false;
            }
        }
        true
    }
}

/// The 24 line vertices (12 edges) of an axis-aligned box.
fn region_edges(min: [f32; 3], max: [f32; 3]) -> [[f32; 3]; 24] {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let c = |x, y, z| [x, y, z];
    [
        // Bottom square.
        c(x0, y0, z0),
        c(x1, y0, z0),
        c(x1, y0, z0),
        c(x1, y0, z1),
        c(x1, y0, z1),
        c(x0, y0, z1),
        c(x0, y0, z1),
        c(x0, y0, z0),
        // Top square.
        c(x0, y1, z0),
        c(x1, y1, z0),
        c(x1, y1, z0),
        c(x1, y1, z1),
        c(x1, y1, z1),
        c(x0, y1, z1),
        c(x0, y1, z1),
        c(x0, y1, z0),
        // Verticals.
        c(x0, y0, z0),
        c(x0, y1, z0),
        c(x1, y0, z0),
        c(x1, y1, z0),
        c(x1, y0, z1),
        c(x1, y1, z1),
        c(x0, y0, z1),
        c(x0, y1, z1),
    ]
}

/// egui paint callback that renders the world scene, then blits it.
struct ViewportCallback;

impl CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(renderer) = callback_resources.get_mut::<ViewportRenderer>() else {
            return Vec::new();
        };

        // Pull UI-side state.
        let (camera, pending_chunks, pending_atlas, size, region, debug, chunk_borders) = {
            let mut scene = renderer.shared.lock().expect("scene lock poisoned");
            (
                scene.camera.unwrap_or_default(),
                scene.pending_chunks.take(),
                scene.pending_atlas.take(),
                scene.desired_size,
                scene.region,
                scene.debug,
                scene.chunk_borders.clone(),
            )
        };

        if size[0] > 0 && size[1] > 0 && size != renderer.size {
            renderer.resize_targets(device, size, renderer.format);
        }

        // Update chunk cache if new meshes arrived.
        if let Some(meshes) = pending_chunks {
            let verts: usize = meshes.iter().map(|m| m.vertices.len()).sum();
            let tris: usize = meshes.iter().map(|m| m.triangle_count()).sum();
            renderer.update_chunk_cache(device, queue, meshes);
            renderer
                .shared
                .lock()
                .expect("scene lock poisoned")
                .mesh_stats = (verts, tris);
        }

        // Upload new atlas if provided.
        if let Some(atlas) = pending_atlas {
            renderer.upload_atlas(device, queue, &atlas);
        }

        if debug.show_chunk_borders {
            renderer.upload_chunk_borders(device, queue, &chunk_borders);
        }

        let (Some(color_view), Some(depth_view)) = (&renderer.color_view, &renderer.depth_view)
        else {
            return Vec::new();
        };

        // Scene pass: clear, then draw meshes with depth testing and frustum
        // culling.
        let aspect = renderer.size[0] as f32 / renderer.size[1].max(1) as f32;
        let view_proj = camera.view_proj(aspect);
        queue.write_buffer(
            &renderer.uniform_buffer,
            0,
            bytemuck::cast_slice(&[view_proj]),
        );
        if let Some((min, max)) = region {
            queue.write_buffer(
                &renderer.line_buffer,
                0,
                bytemuck::cast_slice(&region_edges(min, max)),
            );
        }

        {
            let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bedrock/scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Draw every cached chunk (frustum culling temporarily disabled).
            if let Some(atlas) = &renderer.atlas_bind_group {
                pass.set_pipeline(&renderer.mesh_pipeline);
                pass.set_bind_group(0, &renderer.mesh_bind_group, &[]);
                pass.set_bind_group(1, atlas, &[]);
                for cached in renderer.chunk_cache.values() {
                    pass.set_vertex_buffer(0, cached.vertex_buffer.slice(..));
                    pass.set_index_buffer(cached.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..cached.index_count, 0, 0..1);
                }
            }

            // Export region wireframe (always-on-top overlay).
            if region.is_some() {
                pass.set_pipeline(&renderer.line_pipeline);
                pass.set_bind_group(0, &renderer.mesh_bind_group, &[]);
                pass.set_vertex_buffer(0, renderer.line_buffer.slice(..));
                pass.draw(0..24, 0..1);
            }

            // Chunk borders (light-blue overlay, always on top).
            if debug.show_chunk_borders && renderer.chunk_border_count > 0 {
                if let Some(buf) = &renderer.chunk_border_buffer {
                    pass.set_pipeline(&renderer.chunk_border_pipeline);
                    pass.set_bind_group(0, &renderer.mesh_bind_group, &[]);
                    pass.set_vertex_buffer(0, buf.slice(..));
                    pass.draw(0..renderer.chunk_border_count, 0..1);
                }
            }

            // Wireframe overlay (orange, depth-tested).
            // Uses the combined vertex buffer (all chunks concatenated) and
            // the wireframe index buffer (edge indices with global offsets).
            if debug.show_wireframe && renderer.wireframe_index_count > 0 {
                if let (Some(ref cv_buf), Some(ref wf_ib)) = (
                    &renderer.combined_vertex_buffer,
                    &renderer.wireframe_index_buffer,
                ) {
                    pass.set_pipeline(&renderer.wireframe_pipeline);
                    pass.set_bind_group(0, &renderer.mesh_bind_group, &[]);
                    pass.set_vertex_buffer(0, cv_buf.slice(..));
                    pass.set_index_buffer(wf_ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..renderer.wireframe_index_count, 0, 0..1);
                }
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(renderer) = callback_resources.get::<ViewportRenderer>() else {
            return;
        };
        let Some(blit_bind_group) = &renderer.blit_bind_group else {
            return;
        };
        let ppp = info.pixels_per_point;
        let vp = info.viewport;
        let clip = info.clip_rect;

        let x = (vp.min.x * ppp).round().max(0.0);
        let y = (vp.min.y * ppp).round().max(0.0);
        let w = (vp.width() * ppp).round().max(1.0);
        let h = (vp.height() * ppp).round().max(1.0);
        render_pass.set_viewport(x, y, w, h, 0.0, 1.0);

        let [screen_w, screen_h] = info.screen_size_px;
        let cx = ((clip.min.x * ppp).round().max(0.0) as u32).min(screen_w);
        let cy = ((clip.min.y * ppp).round().max(0.0) as u32).min(screen_h);
        let cw = ((clip.width() * ppp).round().max(0.0) as u32)
            .min(screen_w - cx)
            .max(1);
        let ch = ((clip.height() * ppp).round().max(0.0) as u32)
            .min(screen_h - cy)
            .max(1);
        render_pass.set_scissor_rect(cx, cy, cw, ch);

        render_pass.set_pipeline(&renderer.blit_pipeline);
        render_pass.set_bind_group(0, blit_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// Paint the GPU viewport into all available space of `ui`, handling orbit /
/// pan / zoom input against the shared scene.
pub fn show_viewport(ui: &mut egui::Ui, shared: &Arc<Mutex<SharedScene>>) {
    let (rect, response) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }

    let mut interacting = false;
    let show_stats;
    let vert_count;
    let tri_count;
    {
        let mut scene = shared.lock().expect("scene lock poisoned");
        let ppp = ui.ctx().pixels_per_point();
        scene.desired_size = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        let camera = scene.camera.get_or_insert_with(Camera::default);

        let delta = response.drag_delta();
        if response.dragged_by(egui::PointerButton::Primary) {
            camera.orbit(delta.x, delta.y);
            interacting = true;
        } else if response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
        {
            camera.pan(delta.x, delta.y);
            interacting = true;
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                camera.zoom(scroll);
                interacting = true;
            }
        }

        show_stats = scene.debug.show_stats;
        vert_count = scene.mesh_stats.0;
        tri_count = scene.mesh_stats.1;

        if scene.mesh_stats.0 == 0 && scene.pending_chunks.is_none() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Open a world from the World Browser to explore it",
                egui::FontId::proportional(14.0),
                egui::Color32::from_gray(150),
            );
        }
    }

    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
        rect,
        ViewportCallback,
    ));

    // ── Stats overlay ────────────────────────────────────────────────────
    if show_stats {
        let painter = ui.painter();
        let bg = egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(8.0, 8.0),
            egui::vec2(260.0, 80.0),
        );
        painter.rect_filled(bg, 4.0, egui::Color32::from_black_alpha(160));

        let text_color = egui::Color32::from_rgb(200, 220, 255);
        let mono = egui::FontId::monospace(12.0);
        let mut y = bg.top() + 8.0;
        let left = bg.left() + 8.0;

        // FPS (approximate from repaint rate)
        let fps = 1.0 / ui.input(|i| i.unstable_dt).max(1e-6);
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            format!("FPS:  {fps:.1}"),
            mono.clone(),
            text_color,
        );
        y += 16.0;

        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            format!("Verts: {}", vert_count),
            mono.clone(),
            text_color,
        );
        y += 16.0;

        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            format!("Tris:  {}", tri_count),
            mono.clone(),
            text_color,
        );
        y += 16.0;

        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            "F1: snap to player  |  F3: stats  |  F4: chunks  |  F5: wireframe",
            egui::FontId::proportional(10.0),
            egui::Color32::from_gray(140),
        );
    }

    if interacting {
        ui.ctx().request_repaint();
    }
}
