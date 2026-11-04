//! One bounded offscreen worker. Native GPU failures never remove CPU previews.
use super::{
    PreparedMesh,
    points::PreparedPoints,
    profiling::{self, StageTiming},
    rendering::{RenderMode, RendererStatus},
};
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    time::{Duration, Instant},
};

const STARTING: u8 = 1;
const READY: u8 = 2;
const DISABLED: u8 = 3;
/// A draw the worker has started must finish within this; queue time and
/// device start-up don't count against it.
const DRAW_WAIT: Duration = Duration::from_secs(2);
/// Cold Vulkan/driver loads can take seconds; a forced-GPU caller waits this
/// long for the device before giving up on it.
const INIT_WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(50);
const MAX_VERTEX_BYTES: u64 = 192 << 20;

pub(super) enum Scene {
    Mesh(Arc<PreparedMesh>),
    Points(Arc<PreparedPoints>),
}
struct Frame {
    rgb: Vec<u8>,
    timings: Vec<StageTiming>,
}
struct Job {
    scene: Scene,
    size: (usize, usize),
    picked: Arc<OnceLock<Instant>>,
    reply: SyncSender<Result<Frame, String>>,
}
struct Shared {
    phase: AtomicU8,
    created: Instant,
    status: Mutex<RendererStatus>,
}
struct Service {
    sender: SyncSender<Job>,
    shared: Arc<Shared>,
}
static SERVICE: OnceLock<Service> = OnceLock::new();

impl Shared {
    fn fail(&self, reason: String) {
        self.phase.store(DISABLED, Ordering::Release);
        if let Ok(mut status) = self.status.lock() {
            status.state = "disabled".to_owned();
            if status.reason.is_none() {
                status.reason = Some(reason);
            }
        }
    }
}

impl Service {
    fn start(allow_software: bool) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Job>(2);
        let mut status = RendererStatus::initial();
        status.state = "initializing".to_owned();
        let shared = Arc::new(Shared {
            phase: AtomicU8::new(STARTING),
            created: Instant::now(),
            status: Mutex::new(status),
        });
        let owner = shared.clone();
        let spawn = std::thread::Builder::new()
            .name("melt-preview-gpu".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<(), String> {
                        let start = Instant::now();
                        let mut renderer = Renderer::new(&owner, allow_software)?;
                        if let Ok(mut status) = owner.status.lock() {
                            status.initialization_ms = Some(start.elapsed().as_secs_f64() * 1000.);
                            status.state = "ready".to_owned();
                        }
                        if owner
                            .phase
                            .compare_exchange(STARTING, READY, Ordering::AcqRel, Ordering::Acquire)
                            .is_err()
                        {
                            return Err("device failed during initialization".to_owned());
                        }
                        while let Ok(job) = receiver.recv() {
                            if owner.phase.load(Ordering::Acquire) == DISABLED {
                                break;
                            }
                            let _ = job.picked.set(Instant::now());
                            let (result, timings) =
                                profiling::capture(|| renderer.draw(&job.scene, job.size));
                            let result = result.map(|rgb| Frame { rgb, timings });
                            let failure = result.as_ref().err().cloned();
                            let _ = job.reply.send(result);
                            if let Some(reason) = failure {
                                return Err(reason);
                            }
                        }
                        Ok(())
                    },
                ));
                match result {
                    Ok(Err(reason)) => owner.fail(reason),
                    Err(_) => owner.fail("GPU worker panicked".to_owned()),
                    _ => {}
                }
            });
        if spawn.is_err() {
            shared.fail("cannot start GPU worker".to_owned());
        }
        Self { sender, shared }
    }
}

pub(super) fn status() -> RendererStatus {
    SERVICE
        .get()
        .and_then(|service| {
            service
                .shared
                .status
                .lock()
                .ok()
                .map(|status| status.clone())
        })
        .unwrap_or_else(RendererStatus::initial)
}

pub(super) fn draw(
    scene: Scene,
    size: (usize, usize),
    mode: RenderMode,
) -> Result<Vec<u8>, String> {
    if size.0 == 0 || size.1 == 0 || size.0 > 1920 || size.1 > 1080 {
        return Err("GPU target exceeds preview limits".to_owned());
    }
    let service = SERVICE.get_or_init(|| Service::start(mode == RenderMode::Gpu));
    admit(&service.shared, mode)?;
    let started = Instant::now();
    let stage = match &scene {
        Scene::Mesh(_) => "draw.gpu.mesh",
        Scene::Points(_) => "draw.gpu.points",
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let picked = Arc::new(OnceLock::new());
    service
        .sender
        .try_send(Job {
            scene,
            size,
            picked: picked.clone(),
            reply: sender,
        })
        .map_err(|_| "GPU queue busy or unavailable".to_owned())?;
    let frame = await_frame(&service.shared, &receiver, &picked, (DRAW_WAIT, INIT_WAIT))?;
    for timing in frame.timings {
        profiling::append(timing);
    }
    profiling::record(StageTiming {
        stage,
        elapsed_us: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    });
    Ok(frame.rgb)
}

/// Whether `mode` may use the worker now. The first caller fixes whether a
/// software adapter is allowed, so Auto also checks what was actually picked.
fn admit(shared: &Shared, mode: RenderMode) -> Result<(), String> {
    let auto = mode == RenderMode::Auto;
    match shared.phase.load(Ordering::Acquire) {
        DISABLED => Err("GPU renderer disabled".to_owned()),
        STARTING if auto => Err("GPU renderer warming in background".to_owned()),
        // Forced GPU may diagnose llvmpipe; Auto's tuned CPU path beats it.
        READY if auto && !shared.status.lock().is_ok_and(|status| status.hardware) => {
            Err("software adapter; automatic mode uses CPU".to_owned())
        }
        _ => Ok(()),
    }
}

/// Waits for a reply, disabling the worker only for a draw that hangs once
/// started or a device that never comes up. A job still queued behind a
/// hung draw ends when that draw's caller disables the worker.
fn await_frame(
    shared: &Shared,
    receiver: &Receiver<Result<Frame, String>>,
    picked: &OnceLock<Instant>,
    (draw_wait, init_wait): (Duration, Duration),
) -> Result<Frame, String> {
    loop {
        match receiver.recv_timeout(POLL) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Disconnected) => {
                shared.fail("GPU worker stopped".to_owned());
                return Err("GPU unavailable".to_owned());
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let timeout = match (picked.get(), shared.phase.load(Ordering::Acquire)) {
            (Some(start), _) if start.elapsed() > draw_wait => "GPU draw timed out",
            (None, DISABLED) => return Err("GPU renderer disabled".to_owned()),
            (None, STARTING) if shared.created.elapsed() > init_wait => {
                "GPU initialization timed out"
            }
            _ => continue,
        };
        shared.fail(timeout.to_owned());
        return Err("GPU unavailable".to_owned());
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    light: [f32; 4],
    frame: [f32; 4],
    point: [f32; 4],
    depth: [f32; 4],
}

#[derive(Clone)]
struct Target {
    size: (usize, usize),
    color: wgpu::Texture,
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    readback: wgpu::Buffer,
    row_bytes: u32,
}

impl Target {
    fn new(device: &wgpu::Device, size: (usize, usize)) -> Self {
        let extent = wgpu::Extent3d {
            width: size.0 as u32,
            height: size.1 as u32,
            depth_or_array_layers: 1,
        };
        let descriptor = wgpu::TextureDescriptor {
            label: Some("preview target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let color = device.create_texture(&descriptor);
        let view = color.create_view(&Default::default());
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("preview depth"),
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                ..descriptor
            })
            .create_view(&Default::default());
        let row_bytes = (size.0 as u32 * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview readback"),
            size: row_bytes as u64 * size.1 as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            size,
            color,
            view,
            depth,
            readback,
            row_bytes,
        }
    }
}

struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mesh: wgpu::RenderPipeline,
    points: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    capacity: u64,
    targets: Vec<Target>,
}

impl Renderer {
    fn new(shared: &Arc<Shared>, allow_software: bool) -> Result<Self, String> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        // Simple shaders need no separately distributed DXC DLL on Windows.
        descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
        let instance = wgpu::Instance::new(descriptor);
        let requested = std::env::var("MELT_PREVIEW_ADAPTER")
            .ok()
            .map(|name| name.to_ascii_lowercase());
        let mut adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        adapters.sort_by_key(|adapter| match adapter.get_info().device_type {
            wgpu::DeviceType::IntegratedGpu => 0,
            wgpu::DeviceType::DiscreteGpu => 1,
            wgpu::DeviceType::VirtualGpu => 2,
            _ => 3,
        });
        let adapter = adapters
            .into_iter()
            .find(|adapter| {
                let info = adapter.get_info();
                requested
                    .as_ref()
                    .is_none_or(|name| info.name.to_ascii_lowercase().contains(name))
                    && (allow_software
                        || matches!(
                            info.device_type,
                            wgpu::DeviceType::IntegratedGpu
                                | wgpu::DeviceType::DiscreteGpu
                                | wgpu::DeviceType::VirtualGpu
                        ))
            })
            .ok_or("no matching hardware adapter")?;
        let info = adapter.get_info();
        if let Ok(mut status) = shared.status.lock() {
            status.adapter = Some(info.name.clone());
            status.backend = Some(format!("{:?}", info.backend));
            status.device_type = Some(format!("{:?}", info.device_type));
            status.hardware = matches!(
                info.device_type,
                wgpu::DeviceType::IntegratedGpu
                    | wgpu::DeviceType::DiscreteGpu
                    | wgpu::DeviceType::VirtualGpu
            );
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Melt preview device"),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        let lost = shared.clone();
        device.set_device_lost_callback(move |_, _| lost.fail("GPU device lost".to_owned()));
        let errors = shared.clone();
        device.on_uncaptured_error(Arc::new(move |error| {
            errors.fail(format!("GPU error: {error}"))
        }));
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview bindings"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview uniforms"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("preview pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let mesh = Self::pipeline(&device, &pipeline_layout, false);
        let points = Self::pipeline(&device, &pipeline_layout, true);
        let vertices = Self::vertex_buffer(&device, 4);
        if let Some(error) = [
            pollster::block_on(memory.pop()),
            pollster::block_on(validation.pop()),
        ]
        .into_iter()
        .flatten()
        .next()
        {
            return Err(error.to_string());
        }
        Ok(Self {
            device,
            queue,
            mesh,
            points,
            uniform,
            bind_group,
            vertices,
            capacity: 4,
            targets: Vec::new(),
        })
    }

    fn vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview vertex upload"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn pipeline(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        points: bool,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview shader"),
            source: wgpu::ShaderSource::Wgsl(
                if points {
                    include_str!("gpu/points.wgsl")
                } else {
                    include_str!("gpu/mesh.wgsl")
                }
                .into(),
            ),
        });
        let attributes = if points {
            &wgpu::vertex_attr_array![0=>Float32x3][..]
        } else {
            &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3][..]
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("preview pipeline"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: if points { 12 } else { 24 },
                    step_mode: if points {
                        wgpu::VertexStepMode::Instance
                    } else {
                        wgpu::VertexStepMode::Vertex
                    },
                    attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(if points {
                    wgpu::CompareFunction::LessEqual
                } else {
                    wgpu::CompareFunction::Less
                }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    fn draw(&mut self, scene: &Scene, size: (usize, usize)) -> Result<Vec<u8>, String> {
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let mut uniforms = Uniforms {
            mvp: [[0.; 4]; 4],
            light: [0.; 4],
            frame: [
                size.0 as f32,
                size.1 as f32,
                (size.0 / 640).clamp(1, 3) as f32,
                0.,
            ],
            point: [0.; 4],
            depth: [0.; 4],
        };
        let (bytes, count, points) = match scene {
            Scene::Mesh(mesh) => {
                uniforms.mvp = mesh.mvp.into_col_arrays();
                uniforms.light = [mesh.light_dir.x, mesh.light_dir.y, mesh.light_dir.z, 0.];
                (
                    bytemuck::cast_slice(mesh.vertices.as_slice()),
                    mesh.vertices.len() as u32,
                    false,
                )
            }
            Scene::Points(points) => {
                uniforms.point = [points.scale, points.ox, points.oy, 0.];
                uniforms.depth = [
                    points.min[0],
                    points.min[1],
                    points.min[2],
                    (points.max[2] - points.min[2]).max(0.001),
                ];
                (
                    bytemuck::cast_slice(points.projected.as_slice()),
                    points.projected.len() as u32,
                    true,
                )
            }
        };
        if bytes.len() as u64 > MAX_VERTEX_BYTES || bytes.is_empty() {
            return Err("GPU geometry exceeds upload budget".to_owned());
        }
        {
            let _span = profiling::span("gpu.upload");
            if bytes.len() as u64 > self.capacity {
                self.capacity = (bytes.len() as u64)
                    .next_power_of_two()
                    .min(MAX_VERTEX_BYTES);
                self.vertices = Self::vertex_buffer(&self.device, self.capacity);
            }
            self.queue.write_buffer(&self.vertices, 0, bytes);
            self.queue
                .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));
        }
        if !self.targets.iter().any(|target| target.size == size) {
            if self.targets.len() == 2 {
                self.targets.remove(0);
            }
            self.targets.push(Target::new(&self.device, size));
        }
        let target = self
            .targets
            .iter()
            .find(|target| target.size == size)
            .ok_or("missing render target")?
            .clone();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 10. / 255.,
                            g: 40. / 255.,
                            b: 58. / 255.,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(if points { &self.points } else { &self.mesh });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..bytes.len() as u64));
            if points {
                pass.draw(0..6, 0..count);
            } else {
                pass.draw(0..count, 0..1);
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.row_bytes),
                    rows_per_image: Some(size.1 as u32),
                },
            },
            wgpu::Extent3d {
                width: size.0 as u32,
                height: size.1 as u32,
                depth_or_array_layers: 1,
            },
        );
        {
            let _span = profiling::span("gpu.submit_wait");
            let submission = self.queue.submit([encoder.finish()]);
            let (sender, receiver) = mpsc::sync_channel(1);
            target
                .readback
                .map_async(wgpu::MapMode::Read, .., move |result| {
                    let _ = sender.send(result);
                });
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(1)),
                })
                .map_err(|error| error.to_string())?;
            receiver
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| "GPU readback callback timed out")?
                .map_err(|error| error.to_string())?;
        }
        let rgb = {
            let _span = profiling::span("gpu.readback");
            let mapped = target
                .readback
                .get_mapped_range(..)
                .map_err(|error| error.to_string())?;
            let mut rgb = Vec::with_capacity(size.0 * size.1 * 3);
            for row in mapped.chunks(target.row_bytes as usize).take(size.1) {
                for rgba in row[..size.0 * 4].as_chunks::<4>().0 {
                    rgb.extend_from_slice(&rgba[..3]);
                }
            }
            rgb
        };
        target.readback.unmap();
        if let Some(error) = [
            pollster::block_on(memory.pop()),
            pollster::block_on(validation.pop()),
        ]
        .into_iter()
        .flatten()
        .next()
        {
            return Err(error.to_string());
        }
        Ok(rgb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangles() -> Vec<super::super::Triangle> {
        let [a, b, c, d] = [
            [-30., -20., 0.],
            [60., -20., 0.],
            [0., 40., 0.],
            [10., 5., 75.],
        ];
        vec![[a, c, b], [a, b, d], [b, c, d], [c, a, d]]
    }

    fn assert_similar(cpu: &[u8], gpu: &[u8]) {
        assert_eq!(cpu.len(), gpu.len());
        let changed = cpu
            .as_chunks::<3>()
            .0
            .iter()
            .zip(gpu.as_chunks::<3>().0.iter())
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 3))
            .count();
        // Allow edge coverage/rounding differences across native backends.
        assert!(
            changed * 1000 < cpu.len() / 3,
            "more than 0.1% materially different pixels: {changed}"
        );
        assert!(gpu.as_chunks::<3>().0.iter().any(|p| *p != [10, 40, 58]));
    }

    fn shared(phase: u8) -> Shared {
        Shared {
            phase: AtomicU8::new(phase),
            created: Instant::now(),
            status: Mutex::new(RendererStatus::initial()),
        }
    }

    fn frame() -> Result<Frame, String> {
        Ok(Frame {
            rgb: vec![1, 2, 3],
            timings: Vec::new(),
        })
    }

    #[test]
    fn automatic_mode_never_uses_a_software_adapter() {
        for (hardware, mode, admitted) in [
            (false, RenderMode::Auto, false),
            (false, RenderMode::Gpu, true),
            (true, RenderMode::Auto, true),
            (true, RenderMode::Gpu, true),
        ] {
            let shared = shared(READY);
            shared.status.lock().unwrap().hardware = hardware;
            assert_eq!(
                admit(&shared, mode).is_ok(),
                admitted,
                "hardware {hardware}, {mode:?}"
            );
        }
        assert!(admit(&shared(STARTING), RenderMode::Auto).is_err());
        assert!(admit(&shared(DISABLED), RenderMode::Gpu).is_err());
    }

    /// Slow device start-up or a busy queue must not turn a healthy GPU off
    /// for the session; only a draw that hangs after pickup does.
    #[test]
    fn times_out_only_draws_that_hang_after_pickup() {
        let limits = (Duration::from_millis(100), Duration::from_millis(400));
        for (name, phase, pick, reply_after, ok, disabled) in [
            (
                "queued past the draw limit",
                READY,
                false,
                Some(250),
                true,
                false,
            ),
            (
                "warming past the draw limit",
                STARTING,
                false,
                Some(250),
                true,
                false,
            ),
            ("drawn within the limit", READY, true, Some(20), true, false),
            ("hangs after pickup", READY, true, None, false, true),
            ("device never comes up", STARTING, false, None, false, true),
            (
                "worker disabled while queued",
                DISABLED,
                false,
                None,
                false,
                true,
            ),
        ] {
            let shared = shared(phase);
            let picked = OnceLock::new();
            if pick {
                picked.set(Instant::now()).unwrap();
            }
            let (sender, receiver) = mpsc::sync_channel(1);
            let started = Instant::now();
            let (result, elapsed) = std::thread::scope(|scope| {
                if let Some(ms) = reply_after {
                    scope.spawn(move || {
                        std::thread::sleep(Duration::from_millis(ms));
                        let _ = sender.send(frame());
                    });
                } else {
                    // Keep the sender alive so the wait can only end by timing out.
                    scope.spawn(move || {
                        std::thread::sleep(Duration::from_millis(600));
                        drop(sender);
                    });
                }
                let result = await_frame(&shared, &receiver, &picked, limits);
                (result, started.elapsed())
            });
            assert_eq!(result.is_ok(), ok, "{name}: {:?}", result.err());
            assert_eq!(
                shared.phase.load(Ordering::Acquire) == DISABLED,
                disabled,
                "{name}"
            );
            assert!(elapsed < Duration::from_millis(550), "{name}: {elapsed:?}");
        }
    }

    /// Run explicitly on a real adapter; ordinary CI exercises the CPU path.
    #[test]
    #[ignore = "requires a native hardware GPU; select with MELT_PREVIEW_ADAPTER"]
    fn hardware_fidelity_reuse_concurrency_and_device_loss() {
        let shared = Arc::new(Shared {
            phase: AtomicU8::new(STARTING),
            created: Instant::now(),
            status: Mutex::new(RendererStatus::initial()),
        });
        let mut renderer = Renderer::new(&shared, false).expect("hardware adapter");
        assert!(shared.status.lock().unwrap().hardware);
        let cloud: Vec<_> = (0..513)
            .map(|i| {
                [
                    (i * 37 % 101) as f32,
                    (i * 19 % 97) as f32,
                    (i * 71 % 89) as f32,
                ]
            })
            .collect();
        for size in [(640, 360), (1920, 1080), (641, 359)] {
            let mesh = Arc::new(super::super::prepare_mesh(&triangles(), size).unwrap());
            let cpu = super::super::draw_mesh_cpu(&mesh, size);
            assert_similar(&cpu, &renderer.draw(&Scene::Mesh(mesh), size).unwrap());
            let points = Arc::new(super::super::points::prepare(&cloud, size).unwrap());
            let cpu = super::super::points::draw_cpu(&points, size);
            assert_similar(
                &cpu,
                &renderer.draw(&Scene::Points(points.clone()), size).unwrap(),
            );
            let capacity = renderer.capacity;
            renderer.draw(&Scene::Points(points), size).unwrap();
            assert_eq!(renderer.capacity, capacity);
            assert!(renderer.targets.len() <= 2);
        }
        drop(renderer);

        let size = (640, 360);
        let scene = Scene::Mesh(Arc::new(
            super::super::prepare_mesh(&triangles(), size).unwrap(),
        ));
        draw(scene, size, RenderMode::Gpu).expect("worker GPU draw");
        assert!(status().hardware);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let mesh = super::super::prepare_mesh(&triangles(), size).unwrap();
                    let expected = super::super::draw_mesh_cpu(&mesh, size);
                    let actual = super::super::rendering::with_mode(RenderMode::Gpu, || {
                        super::super::rendering::mesh(mesh, size)
                    });
                    assert_similar(&expected, &actual);
                });
            }
        });
        let service = SERVICE.get().unwrap();
        service.shared.fail("simulated device loss".to_owned());
        let mesh = super::super::prepare_mesh(&triangles(), size).unwrap();
        let expected = super::super::draw_mesh_cpu(&mesh, size);
        let (actual, stages) = profiling::capture(|| {
            super::super::rendering::with_mode(RenderMode::Gpu, || {
                super::super::rendering::mesh(mesh, size)
            })
        });
        assert_eq!(actual, expected);
        assert!(stages.iter().any(|stage| stage.stage == "draw.cpu.mesh"));
        assert_eq!(status().state, "disabled");
    }
}
