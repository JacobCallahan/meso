/*
 * Headless wgpu compute context shared by the nowcast kernels.
 *
 * The context is created lazily once per process and is independent of GTK and
 * meso-render. All kernels use plain f32 storage buffers so no texture-format
 * features are required.
 */

use anyhow::{anyhow, Context, Result};
use std::sync::OnceLock;
use wgpu::util::DeviceExt;

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_name: String,
    pub advect: wgpu::ComputePipeline,
    pub flow: FlowKernels,
}

pub struct FlowKernels {
    pub downsample: wgpu::ComputePipeline,
    pub upsample_flow: wgpu::ComputePipeline,
    pub lk_terms: wgpu::ComputePipeline,
    pub blur_h: wgpu::ComputePipeline,
    pub blur_v: wgpu::ComputePipeline,
    pub lk_solve: wgpu::ComputePipeline,
    pub diffuse_init: wgpu::ComputePipeline,
    pub diffuse_step: wgpu::ComputePipeline,
    pub diffuse_apply: wgpu::ComputePipeline,
}

static GPU: OnceLock<std::result::Result<Gpu, String>> = OnceLock::new();

impl Gpu {
    /// Shared GPU context; fails with a descriptive error if no adapter is available.
    pub fn get() -> Result<&'static Gpu> {
        GPU.get_or_init(|| Gpu::init().map_err(|e| format!("{e:#}")))
            .as_ref()
            .map_err(|e| anyhow!("GPU nowcast unavailable: {e}"))
    }

    fn init() -> Result<Gpu> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .context("No wgpu adapter found")?;
        let info = adapter.get_info();
        tracing::info!("nowcast wgpu adapter: {} ({:?})", info.name, info.backend);

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("meso nowcast device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .context("Failed to create wgpu device")?;

        let pipeline = |module: &wgpu::ShaderModule, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module,
                entry_point: entry,
                compilation_options: Default::default(),
                cache: None,
            })
        };

        let adv_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("advect_diffuse"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/advect_diffuse.wgsl").into()),
        });
        let flow_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/flow.wgsl").into()),
        });

        let advect = pipeline(&adv_mod, "main");
        let flow = FlowKernels {
            downsample: pipeline(&flow_mod, "downsample"),
            upsample_flow: pipeline(&flow_mod, "upsample_flow"),
            lk_terms: pipeline(&flow_mod, "lk_terms"),
            blur_h: pipeline(&flow_mod, "blur_h"),
            blur_v: pipeline(&flow_mod, "blur_v"),
            lk_solve: pipeline(&flow_mod, "lk_solve"),
            diffuse_init: pipeline(&flow_mod, "diffuse_init"),
            diffuse_step: pipeline(&flow_mod, "diffuse_step"),
            diffuse_apply: pipeline(&flow_mod, "diffuse_apply"),
        };

        Ok(Gpu {
            device,
            queue,
            adapter_name: info.name,
            advect,
            flow,
        })
    }

    pub fn storage_from(&self, data: &[f32]) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            })
    }

    pub fn storage_empty(&self, floats: usize) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (floats.max(1) * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn uniform<T: bytemuck::Pod>(&self, value: &T) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::bytes_of(value),
                usage: wgpu::BufferUsages::UNIFORM,
            })
    }

    /// Record one compute pass over a `w` x `h` domain (16x16 workgroups).
    /// `bindings` are (binding index, buffer) pairs matching the pipeline's layout.
    pub fn dispatch_2d(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        bindings: &[(u32, &wgpu::Buffer)],
        w: usize,
        h: usize,
    ) {
        let entries: Vec<wgpu::BindGroupEntry> = bindings
            .iter()
            .map(|(binding, buf)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: buf.as_entire_binding(),
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(w.div_ceil(16) as u32, h.div_ceil(16) as u32, 1);
    }

    pub fn submit(&self, encoder: wgpu::CommandEncoder) {
        self.queue.submit(Some(encoder.finish()));
    }

    pub fn encoder(&self) -> wgpu::CommandEncoder {
        self.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None })
    }

    /// Blocking readback of the first `floats` values of a storage buffer.
    pub fn read(&self, buf: &wgpu::Buffer, floats: usize) -> Result<Vec<f32>> {
        let bytes = (floats * 4) as u64;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.encoder();
        enc.copy_buffer_to_buffer(buf, 0, &staging, 0, bytes);
        self.submit(enc);

        let (tx, rx) = std::sync::mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .context("GPU readback channel closed")?
            .context("GPU readback failed")?;
        let out = bytemuck::cast_slice(&staging.slice(..).get_mapped_range()).to_vec();
        staging.unmap();
        Ok(out)
    }
}
