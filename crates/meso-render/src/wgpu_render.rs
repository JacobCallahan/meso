/*
 * wgpu GPU rendering backend stub.
 *
 * Full wgpu pipeline for radar rendering.  The wgpu adapter is initialized
 * once at app startup.  If initialization fails (no compatible adapter),
 * the app falls back to cairo_render.
 *
 * Architecture:
 * - `WgpuContext`: persistent device/queue/surface state
 * - `RadarPipeline`: compiled shader pipeline for colored quads
 * - `render_frame()`: upload vertex data and issue draw call
 *
 * The rendered output is read back to CPU RAM as an RGBA texture, then
 * handed to GTK4's GtkGLArea as a texture ID — avoiding a redundant
 * readback by rendering directly into an OpenGL FBO.
 */

use anyhow::{Context as AnyhowContext, Result};
use wgpu::util::DeviceExt;
use wgpu::*;

use crate::frame::RenderedImage;
use crate::geometry::QuadBuffer;
use crate::viewport::Viewport;

// ── WGSL shaders ─────────────────────────────────────────────────────────────

const RADAR_SHADER_WGSL: &str = r#"
struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec3<f32>,
};

struct Uniforms {
    // Transform screen pixels to NDC: scale and offset
    scale:  vec2<f32>,
    offset: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // Convert pixel coords to NDC: x in [-1, 1], y flipped
    let ndc_x = in.pos.x * uniforms.scale.x + uniforms.offset.x;
    let ndc_y = in.pos.y * uniforms.scale.y + uniforms.offset.y;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
"#;

// ── WgpuContext ───────────────────────────────────────────────────────────────

/// Persistent GPU context. Create once at app startup; reuse across renders.
pub struct WgpuContext {
    pub device: Device,
    pub queue: Queue,
    adapter_info: AdapterInfo,
}

impl WgpuContext {
    /// Initialize the wgpu context.  Returns an error if no suitable adapter
    /// is found (caller should fall back to cairo).
    pub async fn new() -> Result<Self> {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::GL | Backends::VULKAN | Backends::METAL,
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::None,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .context("No wgpu adapter found — will fall back to Cairo CPU renderer")?;

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
            device,
            queue,
            adapter_info,
        })
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_info.name
    }
}

// ── Offscreen render pipeline ─────────────────────────────────────────────────

/// Compile the radar shader pipeline for a specific output texture format.
pub struct RadarPipeline {
    pipeline: RenderPipeline,
    uniform_buf: Buffer,
    uniform_bind_group: BindGroup,
}

impl RadarPipeline {
    pub fn new(ctx: &WgpuContext, format: TextureFormat) -> Self {
        let shader = ctx.device.create_shader_module(ShaderModuleDescriptor {
            label: Some("radar_shader"),
            source: ShaderSource::Wgsl(RADAR_SHADER_WGSL.into()),
        });

        let uniform_buf = ctx.device.create_buffer(&BufferDescriptor {
            label: Some("uniforms"),
            size: 16, // 4 × f32
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = ctx
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("uniform_bgl"),
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let uniform_bind_group = ctx.device.create_bind_group(&BindGroupDescriptor {
            label: Some("uniform_bg"),
            layout: &bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("radar_pl"),
                bind_group_layouts: &[&bgl],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("radar_pipeline"),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[
                        // Position buffer: 2 × f32
                        VertexBufferLayout {
                            array_stride: 8,
                            step_mode: VertexStepMode::Vertex,
                            attributes: &vertex_attr_array![0 => Float32x2],
                        },
                        // Color buffer: 4 × u8 (normalized), padded from 3-byte RGB
                        VertexBufferLayout {
                            array_stride: 4,
                            step_mode: VertexStepMode::Vertex,
                            attributes: &vertex_attr_array![1 => Unorm8x4],
                        },
                    ],
                    compilation_options: PipelineCompilationOptions::default(),
                },
                fragment: Some(FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[Some(ColorTargetState {
                        format,
                        blend: Some(BlendState::REPLACE),
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

        RadarPipeline {
            pipeline,
            uniform_buf,
            uniform_bind_group,
        }
    }
}

// ── Offscreen render ──────────────────────────────────────────────────────────

/// Render a radar quad buffer to an offscreen RGBA texture, then read back.
///
/// Returns an `RenderedImage` with RGBA pixel data.
pub fn render_offscreen(
    ctx: &WgpuContext,
    pipeline: &RadarPipeline,
    quads: &QuadBuffer,
    viewport: &Viewport,
) -> Result<RenderedImage> {
    let w = viewport.width;
    let h = viewport.height;

    // Upload uniforms: scale from pixel coords to NDC
    // NDC x = pixel_x * (2/w) - 1,  NDC y = -(pixel_y * (2/h) - 1)
    let uniforms: [f32; 4] = [2.0 / w as f32, -2.0 / h as f32, -1.0, 1.0];
    ctx.queue
        .write_buffer(&pipeline.uniform_buf, 0, bytemuck::cast_slice(&uniforms));

    // Create offscreen texture
    let texture = ctx.device.create_texture(&TextureDescriptor {
        label: Some("radar_target"),
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

    // Upload vertex data
    // Position buffer (f32)
    let pos_data: &[u8] = bytemuck::cast_slice(&quads.positions);
    let pos_buf = ctx.device.create_buffer_init(&util::BufferInitDescriptor {
        label: Some("pos_buf"),
        contents: pos_data,
        usage: BufferUsages::VERTEX,
    });

    // Color buffer (u8 RGB → pad to u8x4 for wgpu Unorm8x4)
    let color_padded: Vec<u8> = quads
        .colors
        .chunks(3)
        .flat_map(|c| [c[0], c[1], c[2], 255])
        .collect();
    let color_buf = ctx.device.create_buffer_init(&util::BufferInitDescriptor {
        label: Some("color_buf"),
        contents: &color_padded,
        usage: BufferUsages::VERTEX,
    });

    // Build index buffer: each quad (4 verts) → 2 triangles (6 indices)
    let num_quads = quads.quad_count as u32;
    let indices: Vec<u32> = (0..num_quads)
        .flat_map(|q| {
            let base = q * 4;
            [base, base + 1, base + 2, base, base + 2, base + 3]
        })
        .collect();
    let index_buf = ctx.device.create_buffer_init(&util::BufferInitDescriptor {
        label: Some("idx_buf"),
        contents: bytemuck::cast_slice(&indices),
        usage: BufferUsages::INDEX,
    });

    // Readback buffer
    let bytes_per_row = align_to(w * 4, COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback_buf = ctx.device.create_buffer(&BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * h) as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // Encode commands
    let mut encoder = ctx
        .device
        .create_command_encoder(&CommandEncoderDescriptor {
            label: Some("radar_encoder"),
        });

    {
        let mut rpass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("radar_pass"),
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
        rpass.set_bind_group(0, &pipeline.uniform_bind_group, &[]);
        rpass.set_vertex_buffer(0, pos_buf.slice(..));
        rpass.set_vertex_buffer(1, color_buf.slice(..));
        rpass.set_index_buffer(index_buf.slice(..), IndexFormat::Uint32);
        if num_quads > 0 {
            rpass.draw_indexed(0..num_quads * 6, 0, 0..1);
        }
    }

    encoder.copy_texture_to_buffer(
        ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        ImageCopyBuffer {
            buffer: &readback_buf,
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

    // Read back
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
