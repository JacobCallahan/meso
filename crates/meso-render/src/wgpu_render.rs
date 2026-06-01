/*
 * wgpu GPU rendering backend.
 *
 * Full wgpu pipeline for radar rendering and map overlays.
 */

use anyhow::{Context as AnyhowContext, Result};
use wgpu::util::DeviceExt;
use wgpu::*;

use crate::frame::RenderedImage;
use crate::geometry::QuadBuffer;
use crate::viewport::Viewport;
use meso_data::map_data::MapData;
use meso_data::radar::color_palette::ColorPalette;

// ── CPU-side line uniform ─────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LineUniformsData {
    color: [f32; 4],
}

// ── WGSL shaders ─────────────────────────────────────────────────────────────

const SHADER_WGSL: &str = r#"
struct VertexIn {
    @location(0) pos_km: vec2<f32>,
    @location(1) level: f32,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) level: f32,
};

struct ViewportUniforms {
    center_latlon: vec2<f32>,
    site_origin_latlon: vec2<f32>,
    zoom: f32,
    range_km: f32,
    screen_size: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: ViewportUniforms;

// ── Radar Shaders ─────────────────────────────────────────────────────────────

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;

    // Equirectangular projection relative to center
    let center_lat_rad = radians(uniforms.center_latlon.x);
    let dx_site_km = radians(uniforms.site_origin_latlon.y - uniforms.center_latlon.y) * 6371.0 * cos(center_lat_rad);
    let dy_site_km = radians(uniforms.site_origin_latlon.x - uniforms.center_latlon.x) * 6371.0;

    let min_dim = min(uniforms.screen_size.x, uniforms.screen_size.y);
    let ppkm = (min_dim / 2.0) / uniforms.range_km * uniforms.zoom;

    let dx_km_from_center = dx_site_km + in.pos_km.x;
    let dy_km_from_center = dy_site_km + in.pos_km.y;

    // Convert pixel coords to NDC
    let ndc_x = (dx_km_from_center * ppkm * 2.0) / uniforms.screen_size.x;
    let ndc_y = (dy_km_from_center * ppkm * 2.0) / uniforms.screen_size.y;

    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.level = in.level;
    return out;
}

@group(0) @binding(1) var palette_texture: texture_1d<f32>;
@group(0) @binding(2) var palette_sampler: sampler;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if (in.level == 0.0) {
        return vec4<f32>(18.0 / 255.0, 18.0 / 255.0, 18.0 / 255.0, 1.0);
    }
    // Perform palette color lookup
    let color = textureSample(palette_texture, palette_sampler, in.level / 255.0);
    return color;
}

// ── Line Shaders (Map Overlays) ───────────────────────────────────────────────

struct LineVertexIn {
    @location(0) latlon: vec2<f32>,
};

struct LineUniforms {
    color: vec4<f32>,
};

@group(1) @binding(0) var<uniform> line_uniforms: LineUniforms;

@vertex
fn vs_line(in: LineVertexIn) -> VertexOut {
    var out: VertexOut;

    let center_lat_rad = radians(uniforms.center_latlon.x);
    let dx_km = radians(in.latlon.y - uniforms.center_latlon.y) * 6371.0 * cos(center_lat_rad);
    let dy_km = radians(in.latlon.x - uniforms.center_latlon.x) * 6371.0;

    let min_dim = min(uniforms.screen_size.x, uniforms.screen_size.y);
    let ppkm = (min_dim / 2.0) / uniforms.range_km * uniforms.zoom;

    let ndc_x = (dx_km * ppkm * 2.0) / uniforms.screen_size.x;
    let ndc_y = (dy_km * ppkm * 2.0) / uniforms.screen_size.y;

    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.level = 1.0;
    return out;
}

@fragment
fn fs_line(in: VertexOut) -> @location(0) vec4<f32> {
    return line_uniforms.color;
}
"#;

// ── WgpuContext ───────────────────────────────────────────────────────────────

/// Persistent GPU context. Create once at app startup; reuse across renders.
#[derive(Clone)]
pub struct WgpuContext {
    pub device: std::sync::Arc<Device>,
    pub queue: std::sync::Arc<Queue>,
    pub adapter_info: AdapterInfo,
}

impl WgpuContext {
    /// Initialize the wgpu context. Returns an error if no suitable adapter
    /// is found.
    pub async fn new() -> Result<Self> {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .context("No wgpu adapter found")?;

        let adapter_info = adapter.get_info();
        tracing::info!(
            "wgpu adapter: {} ({:?})",
            adapter_info.name,
            adapter_info.backend
        );

        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor {
                    label: Some("meso radar device"),
                    required_features: Features::empty(),
                    required_limits: Limits::downlevel_webgl2_defaults(),
                    memory_hints: MemoryHints::Performance,
                },
                None,
            )
            .await
            .context("Failed to create wgpu device")?;

        Ok(WgpuContext {
            device: std::sync::Arc::new(device),
            queue: std::sync::Arc::new(queue),
            adapter_info,
        })
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_info.name
    }
}

// ── Uniform Structs ───────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewportUniforms {
    pub center_latlon: [f32; 2],
    pub site_origin_latlon: [f32; 2],
    pub zoom: f32,
    pub range_km: f32,
    pub screen_size: [f32; 2],
}

// ── GPU Resource wrappers ─────────────────────────────────────────────────────

pub struct WgpuPalette {
    pub texture: Texture,
    pub view: TextureView,
    pub sampler: Sampler,
}

impl WgpuPalette {
    pub fn new(device: &Device, queue: &Queue, palette: &ColorPalette) -> Self {
        let size = Extent3d {
            width: 256,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("palette_texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D1,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());

        // Fill palette colors
        let mut pixels = Vec::with_capacity(256 * 4);
        for lvl in 0..=255 {
            let (r, g, b) = palette.color(lvl);
            pixels.push(r);
            pixels.push(g);
            pixels.push(b);
            pixels.push(255);
        }
        queue.write_texture(
            ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &pixels,
            ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(256 * 4),
                rows_per_image: None,
            },
            size,
        );

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("palette_sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            ..Default::default()
        });

        WgpuPalette {
            texture,
            view,
            sampler,
        }
    }
}

pub struct WgpuRadarGeometry {
    pub pos_buf: Buffer,
    pub level_buf: Buffer,
    pub index_buf: Buffer,
    pub index_count: u32,
}

impl WgpuRadarGeometry {
    pub fn new(device: &Device, quads: &QuadBuffer) -> Self {
        let pos_data: &[u8] = bytemuck::cast_slice(&quads.positions);
        let pos_buf = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("radar_pos_buf"),
            contents: pos_data,
            usage: BufferUsages::VERTEX,
        });

        // Convert u8 bin levels to f32 for the shader (`@location(1) level: f32`).
        let levels_f32: Vec<f32> = quads.levels.iter().map(|&l| l as f32).collect();
        let level_buf = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("radar_level_buf"),
            contents: bytemuck::cast_slice(&levels_f32),
            usage: BufferUsages::VERTEX,
        });

        let num_quads = quads.quad_count as u32;
        let indices: Vec<u32> = (0..num_quads)
            .flat_map(|q| {
                let base = q * 4;
                [base, base + 1, base + 2, base, base + 2, base + 3]
            })
            .collect();
        let index_buf = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("radar_idx_buf"),
            contents: bytemuck::cast_slice(&indices),
            usage: BufferUsages::INDEX,
        });

        WgpuRadarGeometry {
            pos_buf,
            level_buf,
            index_buf,
            index_count: num_quads * 6,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WgpuRadarFrameMetadata {
    pub index_start: u32,
    pub index_count: u32,
    pub base_vertex: i32,
}

pub struct WgpuRadarAnimation {
    pub pos_buf: Buffer,
    pub level_buf: Buffer,
    pub index_buf: Buffer,
    pub frames: Vec<WgpuRadarFrameMetadata>,
}

impl WgpuRadarAnimation {
    pub fn new(device: &Device, frame_quads: &[QuadBuffer]) -> Self {
        let mut all_positions = Vec::new();
        let mut all_levels = Vec::new();
        let mut all_indices = Vec::new();
        let mut frames = Vec::new();

        let mut current_vertex_offset = 0i32;
        let mut current_index_offset = 0u32;

        for quads in frame_quads {
            let num_quads = quads.quad_count as u32;
            let index_count = num_quads * 6;

            all_positions.extend_from_slice(&quads.positions);
            all_levels.extend(quads.levels.iter().map(|&l| l as f32));

            let indices: Vec<u32> = (0..num_quads)
                .flat_map(|q| {
                    let base = q * 4;
                    [base, base + 1, base + 2, base, base + 2, base + 3]
                })
                .collect();
            all_indices.extend_from_slice(&indices);

            frames.push(WgpuRadarFrameMetadata {
                index_start: current_index_offset,
                index_count,
                base_vertex: current_vertex_offset,
            });

            current_vertex_offset += (quads.quad_count * 4) as i32;
            current_index_offset += index_count;
        }

        let pos_buf = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("anim_pos_buf"),
            contents: bytemuck::cast_slice(&all_positions),
            usage: BufferUsages::VERTEX,
        });

        let level_buf = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("anim_level_buf"),
            contents: bytemuck::cast_slice(&all_levels),
            usage: BufferUsages::VERTEX,
        });

        let index_buf = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("anim_idx_buf"),
            contents: bytemuck::cast_slice(&all_indices),
            usage: BufferUsages::INDEX,
        });

        WgpuRadarAnimation {
            pos_buf,
            level_buf,
            index_buf,
            frames,
        }
    }
}

pub struct WgpuMapGeometry {
    pub counties_buf: Buffer,
    pub counties_count: u32,
    pub states_buf: Buffer,
    pub states_count: u32,
    pub lakes_buf: Buffer,
    pub lakes_count: u32,
    pub roads_buf: Buffer,
    pub roads_count: u32,
}

impl WgpuMapGeometry {
    pub fn new(device: &Device, map: &MapData) -> Self {
        let seg_to_verts = |segs: &[meso_data::map_data::GeoSegment]| -> (Buffer, u32) {
            let mut verts = Vec::with_capacity(segs.len() * 4);
            for seg in segs {
                verts.push(seg.lat1);
                verts.push(seg.lon1);
                verts.push(seg.lat2);
                verts.push(seg.lon2);
            }
            let buf = device.create_buffer_init(&util::BufferInitDescriptor {
                label: Some("map_layer_buf"),
                contents: bytemuck::cast_slice(&verts),
                usage: BufferUsages::VERTEX,
            });
            (buf, (segs.len() * 2) as u32)
        };

        let (counties_buf, counties_count) = seg_to_verts(&map.counties);
        let (states_buf, states_count) = seg_to_verts(&map.states);
        let (lakes_buf, lakes_count) = seg_to_verts(&map.lakes);
        let (roads_buf, roads_count) = seg_to_verts(&map.roads_major);

        WgpuMapGeometry {
            counties_buf,
            counties_count,
            states_buf,
            states_count,
            lakes_buf,
            lakes_count,
            roads_buf,
            roads_count,
        }
    }
}

// ── Offscreen render pipeline ─────────────────────────────────────────────────

pub struct RadarPipeline {
    pub pipeline: RenderPipeline,
    pub line_pipeline: RenderPipeline,
    pub uniform_buf: Buffer,
    pub radar_bind_group_layout: BindGroupLayout,
    pub line_bind_group_layout: BindGroupLayout,
}

impl RadarPipeline {
    pub fn new(ctx: &WgpuContext, format: TextureFormat) -> Self {
        let shader = ctx.device.create_shader_module(ShaderModuleDescriptor {
            label: Some("combined_radar_shader"),
            source: ShaderSource::Wgsl(SHADER_WGSL.into()),
        });

        let uniform_buf = ctx.device.create_buffer(&BufferDescriptor {
            label: Some("viewport_uniforms"),
            size: 32, // 32 bytes ViewportUniforms
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let radar_bgl = ctx
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("radar_bgl"),
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        visibility: ShaderStages::VERTEX,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Texture {
                            sample_type: TextureSampleType::Float { filterable: false },
                            view_dimension: TextureViewDimension::D1,
                            multisampled: false,
                        },
                        count: None,
                    },
                    BindGroupLayoutEntry {
                        binding: 2,
                        visibility: ShaderStages::FRAGMENT,
                        ty: BindingType::Sampler(SamplerBindingType::NonFiltering),
                        count: None,
                    },
                ],
            });

        let line_bgl = ctx
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("line_bgl"),
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Radar quads only need bind group 0 (uniforms + palette).
        // Line overlays need both 0 and 1 (uniforms + per-layer color).
        // Use separate layouts so the radar pass doesn't need a dummy BG1.
        let radar_only_layout = ctx
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("radar_only_pl_layout"),
                bind_group_layouts: &[&radar_bgl],
                push_constant_ranges: &[],
            });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("radar_pl_layout"),
                bind_group_layouts: &[&radar_bgl, &line_bgl],
                push_constant_ranges: &[],
            });

        // Pipeline for radar colored quads
        let pipeline = ctx
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("radar_quads_pipeline"),
                layout: Some(&radar_only_layout),
                vertex: VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[
                        // Position: 2 × f32 (km relative to origin)
                        VertexBufferLayout {
                            array_stride: 8,
                            step_mode: VertexStepMode::Vertex,
                            attributes: &vertex_attr_array![0 => Float32x2],
                        },
                        // Level: 1 × f32 (bin index, converted from u8 on upload)
                        VertexBufferLayout {
                            array_stride: 4,
                            step_mode: VertexStepMode::Vertex,
                            attributes: &vertex_attr_array![1 => Float32],
                        },
                    ],
                    compilation_options: PipelineCompilationOptions::default(),
                },
                fragment: Some(FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[Some(ColorTargetState {
                        format,
                        blend: Some(BlendState::ALPHA_BLENDING),
                        write_mask: ColorWrites::ALL,
                    })],
                    compilation_options: PipelineCompilationOptions::default(),
                }),
                primitive: PrimitiveState {
                    topology: PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        // Pipeline for lines (state, county, road layers)
        let line_pipeline = ctx
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("lines_pipeline"),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &shader,
                    entry_point: "vs_line",
                    buffers: &[VertexBufferLayout {
                        array_stride: 8,
                        step_mode: VertexStepMode::Vertex,
                        attributes: &vertex_attr_array![0 => Float32x2],
                    }],
                    compilation_options: PipelineCompilationOptions::default(),
                },
                fragment: Some(FragmentState {
                    module: &shader,
                    entry_point: "fs_line",
                    targets: &[Some(ColorTargetState {
                        format,
                        blend: Some(BlendState::ALPHA_BLENDING),
                        write_mask: ColorWrites::ALL,
                    })],
                    compilation_options: PipelineCompilationOptions::default(),
                }),
                primitive: PrimitiveState {
                    topology: PrimitiveTopology::LineList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        RadarPipeline {
            pipeline,
            line_pipeline,
            uniform_buf,
            radar_bind_group_layout: radar_bgl,
            line_bind_group_layout: line_bgl,
        }
    }
}

// ── Offscreen render ──────────────────────────────────────────────────────────

/// Render a radar frame to an offscreen RGBA texture and read it back.
/// If `map_geo` is provided the county, state, lake, and (optionally) road
/// overlay layers are composited on top of the radar quads.
pub fn render_offscreen(
    ctx: &WgpuContext,
    pipeline: &RadarPipeline,
    quads: &QuadBuffer,
    palette: &ColorPalette,
    viewport: &Viewport,
    map_geo: Option<&WgpuMapGeometry>,
    show_roads: bool,
) -> Result<RenderedImage> {
    let geometry = WgpuRadarGeometry::new(&ctx.device, quads);
    render_offscreen_from_geo(
        ctx, pipeline, &geometry, palette, viewport, map_geo, show_roads,
    )
}

/// Like `render_offscreen` but reuses a pre-built `WgpuRadarGeometry` instead
/// of building it from a `QuadBuffer`. Call this on viewport-only changes
/// (pan/zoom) where the radar data has not changed.
pub fn render_offscreen_from_geo(
    ctx: &WgpuContext,
    pipeline: &RadarPipeline,
    geo: &WgpuRadarGeometry,
    palette: &ColorPalette,
    viewport: &Viewport,
    map_geo: Option<&WgpuMapGeometry>,
    show_roads: bool,
) -> Result<RenderedImage> {
    let w = viewport.width;
    let h = viewport.height;
    let wgpu_palette = WgpuPalette::new(&ctx.device, &ctx.queue, palette);
    let uniforms = make_viewport_uniforms(viewport);
    let uniform_buf = ctx.device.create_buffer_init(&util::BufferInitDescriptor {
        label: Some("viewport_uniforms_per_render"),
        contents: bytemuck::cast_slice(&[uniforms]),
        usage: BufferUsages::UNIFORM,
    });
    let bind_group = make_radar_bind_group(ctx, pipeline, &wgpu_palette, &uniform_buf);
    let (texture, texture_view, readback_buf, bytes_per_row) = make_offscreen_texture(ctx, w, h);
    let mut encoder = ctx
        .device
        .create_command_encoder(&CommandEncoderDescriptor {
            label: Some("radar_cached_geo_encoder"),
        });
    {
        let mut rpass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("radar_cached_geo_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &texture_view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::BLACK),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rpass.set_pipeline(&pipeline.pipeline);
        rpass.set_bind_group(0, &bind_group, &[]);
        rpass.set_vertex_buffer(0, geo.pos_buf.slice(..));
        rpass.set_vertex_buffer(1, geo.level_buf.slice(..));
        rpass.set_index_buffer(geo.index_buf.slice(..), IndexFormat::Uint32);
        if geo.index_count > 0 {
            rpass.draw_indexed(0..geo.index_count, 0, 0..1);
        }
    }
    if let Some(gmap) = map_geo {
        encode_map_overlays(
            &mut encoder,
            ctx,
            pipeline,
            &texture_view,
            &bind_group,
            gmap,
            show_roads,
        );
    }
    readback_pixels(ctx, encoder, &texture, &readback_buf, bytes_per_row, w, h)
}

pub struct OffscreenMapOverlay<'a> {
    pub map_geo: Option<&'a WgpuMapGeometry>,
    pub show_roads: bool,
}

/// Render one frame of a pre-baked animation to an offscreen RGBA texture.
/// Uses the combined GPU buffers in `anim` rather than re-uploading per frame.
pub fn render_offscreen_frame(
    ctx: &WgpuContext,
    pipeline: &RadarPipeline,
    anim: &WgpuRadarAnimation,
    frame_idx: usize,
    palette: &ColorPalette,
    viewport: &Viewport,
    overlays: OffscreenMapOverlay<'_>,
) -> Result<RenderedImage> {
    let meta = anim
        .frames
        .get(frame_idx)
        .with_context(|| format!("animation frame index {frame_idx} out of range"))?;

    let w = viewport.width;
    let h = viewport.height;

    let wgpu_palette = WgpuPalette::new(&ctx.device, &ctx.queue, palette);

    let uniforms = make_viewport_uniforms(viewport);
    let uniform_buf = ctx.device.create_buffer_init(&util::BufferInitDescriptor {
        label: Some("viewport_uniforms_per_anim_frame"),
        contents: bytemuck::cast_slice(&[uniforms]),
        usage: BufferUsages::UNIFORM,
    });

    let bind_group = make_radar_bind_group(ctx, pipeline, &wgpu_palette, &uniform_buf);
    let (texture, texture_view, readback_buf, bytes_per_row) = make_offscreen_texture(ctx, w, h);

    let mut encoder = ctx
        .device
        .create_command_encoder(&CommandEncoderDescriptor {
            label: Some("anim_render_encoder"),
        });

    {
        let mut rpass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("anim_radar_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &texture_view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::BLACK),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rpass.set_pipeline(&pipeline.pipeline);
        rpass.set_bind_group(0, &bind_group, &[]);
        rpass.set_vertex_buffer(0, anim.pos_buf.slice(..));
        rpass.set_vertex_buffer(1, anim.level_buf.slice(..));
        rpass.set_index_buffer(anim.index_buf.slice(..), IndexFormat::Uint32);
        if meta.index_count > 0 {
            rpass.draw_indexed(
                meta.index_start..meta.index_start + meta.index_count,
                meta.base_vertex,
                0..1,
            );
        }
    }

    if let Some(geo) = overlays.map_geo {
        encode_map_overlays(
            &mut encoder,
            ctx,
            pipeline,
            &texture_view,
            &bind_group,
            geo,
            overlays.show_roads,
        );
    }

    readback_pixels(ctx, encoder, &texture, &readback_buf, bytes_per_row, w, h)
}

// ── Private helpers ───────────────────────────────────────────────────────────

pub fn make_viewport_uniforms(viewport: &Viewport) -> ViewportUniforms {
    ViewportUniforms {
        center_latlon: [viewport.center.lat as f32, viewport.center.lon as f32],
        site_origin_latlon: [
            viewport.site_origin.lat as f32,
            viewport.site_origin.lon as f32,
        ],
        zoom: viewport.zoom as f32,
        range_km: viewport.range_km as f32,
        screen_size: [viewport.width as f32, viewport.height as f32],
    }
}

pub fn make_radar_bind_group(
    ctx: &WgpuContext,
    pipeline: &RadarPipeline,
    palette: &WgpuPalette,
    uniform_buf: &Buffer,
) -> BindGroup {
    ctx.device.create_bind_group(&BindGroupDescriptor {
        label: Some("radar_bind_group"),
        layout: &pipeline.radar_bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(&palette.view),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(&palette.sampler),
            },
        ],
    })
}

pub fn make_offscreen_texture(
    ctx: &WgpuContext,
    w: u32,
    h: u32,
) -> (Texture, TextureView, Buffer, u32) {
    let texture = ctx.device.create_texture(&TextureDescriptor {
        label: Some("radar_offscreen_texture"),
        size: Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let texture_view = texture.create_view(&TextureViewDescriptor::default());
    let bytes_per_row = align_to(w * 4, COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback_buf = ctx.device.create_buffer(&BufferDescriptor {
        label: Some("readback_buf"),
        size: (bytes_per_row * h) as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (texture, texture_view, readback_buf, bytes_per_row)
}

/// Append line render passes for all enabled map overlay layers.
/// Must be called after the radar quad pass, before `encoder.finish()`.
/// Uses `LoadOp::Load` to composite on top of existing radar output.
pub fn encode_map_overlays(
    encoder: &mut CommandEncoder,
    ctx: &WgpuContext,
    pipeline: &RadarPipeline,
    texture_view: &TextureView,
    radar_bind_group: &BindGroup,
    geo: &WgpuMapGeometry,
    show_roads: bool,
) {
    // Layer definitions: (color_rgba, vertex_buffer, vertex_count)
    // Colors match the Cairo reference implementation in cairo_render.rs.
    let mut layers: Vec<([f32; 4], &Buffer, u32)> = vec![
        (
            [0.45, 0.45, 0.45, 0.85],
            &geo.counties_buf,
            geo.counties_count,
        ),
        ([0.85, 0.85, 0.85, 0.95], &geo.states_buf, geo.states_count),
        ([0.40, 0.60, 0.90, 0.85], &geo.lakes_buf, geo.lakes_count),
    ];
    if show_roads {
        layers.push(([0.96, 0.96, 0.96, 0.85], &geo.roads_buf, geo.roads_count));
    }

    // Build per-layer color uniform buffers and bind groups up front so all
    // GPU resources stay alive for the duration of this encoder scope.
    let line_resources: Vec<(Buffer, BindGroup)> = layers
        .iter()
        .map(|(color, _, _)| {
            let cbuf = ctx.device.create_buffer_init(&util::BufferInitDescriptor {
                label: Some("line_color_buf"),
                contents: bytemuck::cast_slice(&[LineUniformsData { color: *color }]),
                usage: BufferUsages::UNIFORM,
            });
            let bg = ctx.device.create_bind_group(&BindGroupDescriptor {
                label: Some("line_bind_group"),
                layout: &pipeline.line_bind_group_layout,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: cbuf.as_entire_binding(),
                }],
            });
            (cbuf, bg)
        })
        .collect();

    for ((_, bg), (_, vbuf, count)) in line_resources.iter().zip(layers.iter()) {
        if *count == 0 {
            continue;
        }
        let mut rpass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("line_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: texture_view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rpass.set_pipeline(&pipeline.line_pipeline);
        rpass.set_bind_group(0, radar_bind_group, &[]);
        rpass.set_bind_group(1, bg, &[]);
        rpass.set_vertex_buffer(0, vbuf.slice(..));
        rpass.draw(0..*count, 0..1);
    }
    // line_resources kept alive until end of scope; buffers freed after submit.
}

pub fn readback_pixels(
    ctx: &WgpuContext,
    encoder: CommandEncoder,
    texture: &Texture,
    readback_buf: &Buffer,
    bytes_per_row: u32,
    w: u32,
    h: u32,
) -> Result<RenderedImage> {
    let mut encoder = encoder;
    encoder.copy_texture_to_buffer(
        ImageCopyTexture {
            texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        ImageCopyBuffer {
            buffer: readback_buf,
            layout: ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h),
            },
        },
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );

    ctx.queue.submit([encoder.finish()]);

    let slice = readback_buf.slice(..);
    slice.map_async(MapMode::Read, |_| {});
    ctx.device.poll(Maintain::Wait);

    let raw = slice.get_mapped_range();
    let mut out = RenderedImage::new(w, h);
    for row in 0..h {
        let src_start = (row * bytes_per_row) as usize;
        let dst_start = (row * w * 4) as usize;
        let src = &raw[src_start..src_start + (w * 4) as usize];
        out.data[dst_start..dst_start + (w * 4) as usize].copy_from_slice(src);
    }
    drop(raw);
    readback_buf.unmap();

    Ok(out)
}

#[inline]
fn align_to(val: u32, align: u32) -> u32 {
    (val + align - 1) & !(align - 1)
}
