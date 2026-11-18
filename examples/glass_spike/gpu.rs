use std::num::NonZeroU64;
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use eframe::egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::epaint::PaintCallbackInfo;
use eframe::wgpu;

use crate::math::{fill_uv_rect, is_supported_target_format, quarter_res_extent};
use crate::wallpaper::WallpaperSource;

const SHADER_SOURCE: &str = include_str!("../shaders/wallpaper_glass.wgsl");
const BLUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

#[derive(Debug, Clone)]
pub struct AdapterMetadata {
    pub name: String,
    pub backend: String,
    pub device_type: wgpu::DeviceType,
    pub driver: String,
    pub driver_info: String,
    pub target_format: wgpu::TextureFormat,
    pub timestamp_query_supported: bool,
}

impl AdapterMetadata {
    #[must_use]
    pub fn from_render_state(state: &eframe::egui_wgpu::RenderState) -> Self {
        let info = state.adapter.get_info();
        Self {
            name: info.name,
            backend: format!("{:?}", info.backend),
            device_type: info.device_type,
            driver: info.driver,
            driver_info: info.driver_info,
            target_format: state.target_format,
            timestamp_query_supported: state
                .device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY),
        }
    }

    #[must_use]
    pub fn is_software_adapter(&self) -> bool {
        self.device_type == wgpu::DeviceType::Cpu
    }
}

#[derive(Debug, Default, Clone)]
pub struct GpuCounters {
    pub source_allocations: u64,
    pub source_uploads: u64,
    pub blur_target_creations: u64,
    pub blur_runs: u64,
    pub paint_draws: u64,
    pub skipped_zero_area: u64,
    pub last_prepare_cpu_ms: f64,
    pub last_gpu_blur_ms: Option<f64>,
    pub validation_error: Option<String>,
    pub resources_ready: bool,
}

#[derive(Debug, Clone)]
pub struct GpuTelemetry {
    inner: Arc<Mutex<GpuCounters>>,
}

impl GpuTelemetry {
    #[must_use]
    pub fn new(resources_ready: bool) -> Self {
        Self {
            inner: Arc::new(Mutex::new(GpuCounters {
                resources_ready,
                ..GpuCounters::default()
            })),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> GpuCounters {
        self.inner
            .lock()
            .map_or_else(|_| GpuCounters::default(), |guard| guard.clone())
    }

    fn update(&self, f: impl FnOnce(&mut GpuCounters)) {
        if let Ok(mut guard) = self.inner.lock() {
            f(&mut guard);
        }
    }

    /// Transitions `validation_error` from `None` to `Some(err)` and clears `resources_ready`.
    /// Returns `true` only on the initial transition so callers trigger one fallback repaint.
    #[must_use]
    pub fn record_validation_error(&self, err: String) -> bool {
        if let Ok(mut guard) = self.inner.lock() {
            guard.resources_ready = false;
            if guard.validation_error.is_none() {
                guard.validation_error = Some(err);
                return true;
            }
        }
        false
    }
}

#[derive(Clone)]
pub struct GlassPanelCallback {
    pub wallpaper: WallpaperSource,
    pub panel_rect_pt: [f32; 4],
    pub corner_radius_pt: f32,
    pub border_width_pt: f32,
    pub tint_gamma: [f32; 4],
    pub border_gamma: [f32; 4],
    pub force_reblur: bool,
    pub telemetry: GpuTelemetry,
    pub egui_ctx: Option<eframe::egui::Context>,
}

struct BlurTargets {
    extent: [u32; 2],
    _horiz_texture: wgpu::Texture,
    horiz_view: wgpu::TextureView,
    _vert_texture: wgpu::Texture,
    vert_view: wgpu::TextureView,
    horiz_read_bind_group: wgpu::BindGroup,
    final_read_bind_group: wgpu::BindGroup,
}

struct SourceTextureState {
    size_px: [u32; 2],
    uploaded_generation: u64,
    texture: wgpu::Texture,
    _view: wgpu::TextureView,
    blur_bind_group: wgpu::BindGroup,
}

pub struct GlassCacheResources {
    target_format: wgpu::TextureFormat,
    sampler: wgpu::Sampler,
    blur_bind_group_layout: wgpu::BindGroupLayout,
    blur_pipeline_h: wgpu::RenderPipeline,
    blur_pipeline_v: wgpu::RenderPipeline,
    blur_horiz_uniform_buffer: wgpu::Buffer,
    blur_vert_uniform_buffer: wgpu::Buffer,
    glass_pipeline: wgpu::RenderPipeline,
    glass_uniform_buffer: wgpu::Buffer,
    source_texture: Option<SourceTextureState>,
    blur_targets: Option<BlurTargets>,
    last_blur_key: Option<(u64, [u32; 2])>,
    telemetry: GpuTelemetry,
}

fn push_f32_bytes(out: &mut Vec<u8>, values: &[f32]) {
    for &v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn push_u32_bytes(out: &mut Vec<u8>, values: &[u32]) {
    for &v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn blur_uniform_bytes(
    crop_min_max: [f32; 4],
    texel_step: [f32; 2],
    blur_radius: f32,
    use_crop: u32,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(32);
    push_f32_bytes(&mut bytes, &crop_min_max);
    push_f32_bytes(&mut bytes, &texel_step);
    push_f32_bytes(&mut bytes, &[blur_radius]);
    push_u32_bytes(&mut bytes, &[use_crop]);
    bytes
}

#[allow(clippy::too_many_arguments)]
fn glass_uniform_bytes(
    panel_rect_px: [f32; 4],
    screen_size_px: [f32; 2],
    corner_radius_px: f32,
    target_is_srgb: u32,
    tint_gamma: [f32; 4],
    border_gamma: [f32; 4],
    border_width_px: f32,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(80);
    push_f32_bytes(&mut bytes, &panel_rect_px);
    push_f32_bytes(&mut bytes, &screen_size_px);
    push_f32_bytes(&mut bytes, &[corner_radius_px]);
    push_u32_bytes(&mut bytes, &[target_is_srgb]);
    push_f32_bytes(&mut bytes, &tint_gamma);
    push_f32_bytes(&mut bytes, &border_gamma);
    push_f32_bytes(&mut bytes, &[border_width_px, 0.0, 0.0, 0.0]);
    bytes
}

fn pop_validation_scope(device: &wgpu::Device, guard: wgpu::ErrorScopeGuard) -> Option<String> {
    let _ = device.poll(wgpu::PollType::Poll);
    let mut fut = pin!(guard.pop());
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Some(err)) => Some(err.to_string()),
        Poll::Ready(None) | Poll::Pending => None,
    }
}

fn finish_validation_scope(
    device: &wgpu::Device,
    guard: wgpu::ErrorScopeGuard,
    telemetry: &GpuTelemetry,
    repaint_ctx: Option<&eframe::egui::Context>,
) -> bool {
    if let Some(err) = pop_validation_scope(device, guard) {
        if telemetry.record_validation_error(err)
            && let Some(ctx) = repaint_ctx
        {
            ctx.request_repaint();
        }
        true
    } else {
        false
    }
}

#[must_use]
fn can_paint_glass(snapshot: &GpuCounters, has_targets: bool) -> bool {
    has_targets && snapshot.resources_ready && snapshot.validation_error.is_none()
}

pub fn init_glass_resources(render_state: &eframe::egui_wgpu::RenderState) -> GpuTelemetry {
    let target_format = render_state.target_format;
    let format_supported = is_supported_target_format(target_format);
    let telemetry = GpuTelemetry::new(false);

    if !format_supported {
        return telemetry;
    }

    let device = &render_state.device;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

    let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("athena_glass_spike_shader"),
        source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_SOURCE)),
    });

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("athena_glass_linear_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });

    let (blur_bind_group_layout, glass_bind_group_layout) = create_bind_group_layouts(device);

    let (blur_pipeline_h, blur_pipeline_v) =
        create_blur_pipelines(device, &shader_module, &blur_bind_group_layout);
    let glass_pipeline = create_glass_pipeline(
        device,
        &shader_module,
        &glass_bind_group_layout,
        target_format,
    );

    let blur_horiz_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("athena_glass_blur_h_uniforms"),
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let blur_vert_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("athena_glass_blur_v_uniforms"),
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let glass_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("athena_glass_composite_uniforms"),
        size: 80,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    if finish_validation_scope(device, scope, &telemetry, None) {
        return telemetry;
    }

    telemetry.update(|c| {
        c.resources_ready = true;
    });

    let cache = GlassCacheResources {
        target_format,
        sampler,
        blur_bind_group_layout,
        blur_pipeline_h,
        blur_pipeline_v,
        blur_horiz_uniform_buffer,
        blur_vert_uniform_buffer,
        glass_pipeline,
        glass_uniform_buffer,
        source_texture: None,
        blur_targets: None,
        last_blur_key: None,
        telemetry: telemetry.clone(),
    };
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(cache);

    telemetry
}

fn create_bind_group_layouts(
    device: &wgpu::Device,
) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
    let make_bgl = |label: &str, uniform_bytes: u64| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(uniform_bytes),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    };
    (
        make_bgl("athena_glass_blur_bgl", 32),
        make_bgl("athena_glass_composite_bgl", 80),
    )
}

fn create_blur_pipelines(
    device: &wgpu::Device,
    shader_module: &wgpu::ShaderModule,
    blur_bind_group_layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("athena_glass_blur_pipeline_layout"),
        bind_group_layouts: &[Some(blur_bind_group_layout)],
        immediate_size: 0,
    });

    let make_blur_pipeline = |label: &str, fs_entry: &str| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&blur_pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader_module,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader_module,
                entry_point: Some(fs_entry),
                targets: &[Some(wgpu::ColorTargetState {
                    format: BLUR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        })
    };

    (
        make_blur_pipeline("athena_glass_blur_h", "fs_blur_h"),
        make_blur_pipeline("athena_glass_blur_v", "fs_blur_v"),
    )
}

fn create_glass_pipeline(
    device: &wgpu::Device,
    shader_module: &wgpu::ShaderModule,
    glass_bind_group_layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let glass_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("athena_glass_composite_pipeline_layout"),
        bind_group_layouts: &[Some(glass_bind_group_layout)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("athena_glass_composite_pipeline"),
        layout: Some(&glass_pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader_module,
            entry_point: Some("vs_glass"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader_module,
            entry_point: Some("fs_glass"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl GlassCacheResources {
    fn ensure_source_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        wallpaper: &WallpaperSource,
    ) -> bool {
        let size_px = wallpaper.size_px;
        if size_px[0] == 0 || size_px[1] == 0 {
            return false;
        }
        let max_dim = device.limits().max_texture_dimension_2d;
        if size_px[0] > max_dim || size_px[1] > max_dim {
            return false;
        }

        let needs_alloc = self
            .source_texture
            .as_ref()
            .is_none_or(|state| state.size_px != size_px);

        if needs_alloc {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("athena_glass_wallpaper_src"),
                size: wgpu::Extent3d {
                    width: size_px[0],
                    height: size_px[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: BLUR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let blur_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("athena_glass_src_blur_bg"),
                layout: &self.blur_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.blur_horiz_uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.source_texture = Some(SourceTextureState {
                size_px,
                uploaded_generation: 0,
                texture,
                _view: view,
                blur_bind_group,
            });
            self.telemetry.update(|c| c.source_allocations += 1);
        }

        if let Some(src) = &mut self.source_texture
            && src.uploaded_generation != wallpaper.generation
        {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &src.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &wallpaper.rgba_bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * size_px[0]),
                    rows_per_image: Some(size_px[1]),
                },
                wgpu::Extent3d {
                    width: size_px[0],
                    height: size_px[1],
                    depth_or_array_layers: 1,
                },
            );
            src.uploaded_generation = wallpaper.generation;
            self.telemetry.update(|c| c.source_uploads += 1);
        }

        true
    }

    fn ensure_blur_targets(&mut self, device: &wgpu::Device, extent: [u32; 2]) {
        if self
            .blur_targets
            .as_ref()
            .is_some_and(|targets| targets.extent == extent)
        {
            return;
        }

        let make_target = |label: &str| {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: BLUR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            (tex, view)
        };

        let (horiz_texture, horiz_view) = make_target("athena_glass_blur_horiz");
        let (vert_texture, vert_view) = make_target("athena_glass_blur_vert");

        let horiz_read_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("athena_glass_horiz_read_bg"),
            layout: &self.blur_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.blur_vert_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&horiz_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let final_read_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("athena_glass_final_read_bg"),
            layout: &self.glass_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.glass_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&vert_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        self.blur_targets = Some(BlurTargets {
            extent,
            _horiz_texture: horiz_texture,
            horiz_view,
            _vert_texture: vert_texture,
            vert_view,
            horiz_read_bind_group,
            final_read_bind_group,
        });
        self.telemetry.update(|c| c.blur_target_creations += 1);
    }

    #[allow(clippy::cast_precision_loss)]
    fn run_blur_passes(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        crop_min: [f32; 2],
        crop_max: [f32; 2],
        blur_extent: [u32; 2],
    ) -> Option<wgpu::CommandEncoder> {
        let h_bytes = blur_uniform_bytes(
            [crop_min[0], crop_min[1], crop_max[0], crop_max[1]],
            [(crop_max[0] - crop_min[0]) / blur_extent[0] as f32, 0.0],
            1.5,
            1,
        );
        queue.write_buffer(&self.blur_horiz_uniform_buffer, 0, &h_bytes);

        let v_bytes = blur_uniform_bytes(
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 1.0 / blur_extent[1] as f32],
            1.5,
            0,
        );
        queue.write_buffer(&self.blur_vert_uniform_buffer, 0, &v_bytes);

        let (Some(src), Some(targets)) = (&self.source_texture, &self.blur_targets) else {
            return None;
        };
        let mut blur_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("athena_glass_blur_encoder"),
        });
        {
            let mut pass_h = blur_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("athena_glass_blur_h_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.horiz_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass_h.set_pipeline(&self.blur_pipeline_h);
            pass_h.set_bind_group(0, &src.blur_bind_group, &[]);
            pass_h.draw(0..3, 0..1);
        }
        {
            let mut pass_v = blur_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("athena_glass_blur_v_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.vert_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass_v.set_pipeline(&self.blur_pipeline_v);
            pass_v.set_bind_group(0, &targets.horiz_read_bind_group, &[]);
            pass_v.draw(0..3, 0..1);
        }
        Some(blur_encoder)
    }
}

impl CallbackTrait for GlassPanelCallback {
    #[allow(clippy::cast_precision_loss)]
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let start = Instant::now();
        let screen_px = screen_descriptor.size_in_pixels;
        let ppp = screen_descriptor.pixels_per_point;

        let Some(blur_extent) = quarter_res_extent(screen_px) else {
            self.telemetry.update(|c| c.skipped_zero_area += 1);
            return Vec::new();
        };

        let viewport_pt = [
            screen_px[0] as f32 / ppp.max(f32::EPSILON),
            screen_px[1] as f32 / ppp.max(f32::EPSILON),
        ];
        let Some(crop) = fill_uv_rect(self.wallpaper.size_px, viewport_pt) else {
            self.telemetry.update(|c| c.skipped_zero_area += 1);
            return Vec::new();
        };

        let Some(cache) = callback_resources.get_mut::<GlassCacheResources>() else {
            return Vec::new();
        };
        let pre_snapshot = cache.telemetry.snapshot();
        if !can_paint_glass(&pre_snapshot, true) {
            return Vec::new();
        }

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

        if !cache.ensure_source_texture(device, queue, &self.wallpaper) {
            let _ =
                finish_validation_scope(device, scope, &cache.telemetry, self.egui_ctx.as_ref());
            cache.telemetry.update(|c| c.skipped_zero_area += 1);
            return Vec::new();
        }
        cache.ensure_blur_targets(device, blur_extent);

        let blur_key = (self.wallpaper.generation, screen_px);
        let blur_cmd = if self.force_reblur || cache.last_blur_key != Some(blur_key) {
            cache
                .run_blur_passes(device, queue, crop.min, crop.max, blur_extent)
                .map(wgpu::CommandEncoder::finish)
        } else {
            None
        };

        let panel_rect_px = [
            self.panel_rect_pt[0] * ppp,
            self.panel_rect_pt[1] * ppp,
            self.panel_rect_pt[2] * ppp,
            self.panel_rect_pt[3] * ppp,
        ];
        let glass_bytes = glass_uniform_bytes(
            panel_rect_px,
            [screen_px[0] as f32, screen_px[1] as f32],
            self.corner_radius_pt * ppp,
            u32::from(cache.target_format.is_srgb()),
            self.tint_gamma,
            self.border_gamma,
            self.border_width_pt * ppp,
        );
        queue.write_buffer(&cache.glass_uniform_buffer, 0, &glass_bytes);

        let validation_failed =
            finish_validation_scope(device, scope, &cache.telemetry, self.egui_ctx.as_ref());

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        cache.telemetry.update(|c| {
            c.last_prepare_cpu_ms = elapsed_ms;
        });

        if validation_failed {
            drop(blur_cmd);
            return Vec::new();
        }

        if let Some(cmd) = blur_cmd {
            cache.last_blur_key = Some(blur_key);
            cache.telemetry.update(|c| c.blur_runs += 1);
            vec![cmd]
        } else {
            Vec::new()
        }
    }

    #[allow(clippy::cast_sign_loss)]
    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(cache) = callback_resources.get::<GlassCacheResources>() else {
            return;
        };
        let Some(targets) = &cache.blur_targets else {
            return;
        };
        let snapshot = cache.telemetry.snapshot();
        if !can_paint_glass(&snapshot, true) {
            return;
        }

        let viewport_px = info.viewport_in_pixels();
        let clip_px = info.clip_rect_in_pixels();
        let left = viewport_px.left_px.max(clip_px.left_px);
        let top = viewport_px.top_px.max(clip_px.top_px);
        let right =
            (viewport_px.left_px + viewport_px.width_px).min(clip_px.left_px + clip_px.width_px);
        let bottom =
            (viewport_px.top_px + viewport_px.height_px).min(clip_px.top_px + clip_px.height_px);
        if right <= left || bottom <= top {
            cache.telemetry.update(|c| c.skipped_zero_area += 1);
            return;
        }

        render_pass.set_scissor_rect(
            left as u32,
            top as u32,
            (right - left) as u32,
            (bottom - top) as u32,
        );
        render_pass.set_pipeline(&cache.glass_pipeline);
        render_pass.set_bind_group(0, &targets.final_read_bind_group, &[]);
        render_pass.draw(0..3, 0..1);

        cache.telemetry.update(|c| c.paint_draws += 1);
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;
    use crate::math::{
        EffectiveSurface, FallbackReason, GlassGateInput, SurfaceMode, resolve_effective_surface,
    };

    fn poll_ready<F: Future>(fut: F) -> Option<F::Output> {
        let mut pinned = pin!(fut);
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..1000 {
            if let Poll::Ready(val) = pinned.as_mut().poll(&mut cx) {
                return Some(val);
            }
            std::thread::yield_now();
        }
        None
    }

    #[test]
    fn real_wgpu_validation_error_blocks_paint_and_selects_solid_fallback() {
        let instance = wgpu::Instance::default();
        let adapter = poll_ready(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .and_then(Result::ok)
        .or_else(|| {
            poll_ready(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: true,
                ..Default::default()
            }))
            .and_then(Result::ok)
        });

        let Some(adapter) = adapter else {
            eprintln!(
                "SKIP real_wgpu_validation_error_blocks_paint_and_selects_solid_fallback: no wgpu adapter available in headless runner"
            );
            return;
        };

        let Some(Ok((device, _queue))) =
            poll_ready(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        else {
            eprintln!(
                "SKIP real_wgpu_validation_error_blocks_paint_and_selects_solid_fallback: could not acquire wgpu device"
            );
            return;
        };

        let telemetry = GpuTelemetry::new(true);
        let healthy = telemetry.snapshot();
        assert!(can_paint_glass(&healthy, true));

        // 1. Trigger a real wgpu bind-group validation error inside `ErrorFilter::Validation`.
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (_blur_bgl, glass_bgl) = create_bind_group_layouts(&device);
        let undersized_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("undersized_uniform_test"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let dummy_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy_tex_test"),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: BLUR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy_view = dummy_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let dummy_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let _invalid_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("invalid_bind_group_test"),
            layout: &glass_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: undersized_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&dummy_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&dummy_sampler),
                },
            ],
        });

        assert!(
            finish_validation_scope(&device, scope, &telemetry, None),
            "expected wgpu validation scope to capture undersized uniform binding error"
        );

        let failed = telemetry.snapshot();
        assert!(!failed.resources_ready);
        assert!(
            failed
                .validation_error
                .as_deref()
                .is_some_and(|msg| !msg.is_empty())
        );
        // On the failing frame itself, `paint` refuses to draw over the spike-only Tint underlay.
        assert!(!can_paint_glass(&failed, true));
        // On the next frame, `resolve_effective_surface` locks in Solid fallback with the error reason.
        assert_eq!(
            resolve_effective_surface(
                GlassGateInput {
                    requested: SurfaceMode::Glass,
                    has_wgpu_state: true,
                    is_software_adapter: false,
                    resources_ready: failed.resources_ready,
                    forced_unavailable: false,
                    target_format_supported: true,
                    gpu_validation_failed: failed.validation_error.is_some(),
                    zero_area: false,
                },
                None,
            ),
            EffectiveSurface::Solid {
                fallback: Some(FallbackReason::ValidationError),
            }
        );

        // 2. Also prove separate blur encoder finishing catches render-pass validation errors
        // before any command buffer would be returned to `egui-wgpu`.
        let encoder_telemetry = GpuTelemetry::new(true);
        let encoder_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut bad_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("invalid_blur_encoder_test"),
        });
        {
            // Render pass with no color/depth attachments is invalid in wgpu when finished.
            let _pass = bad_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("empty_invalid_pass"),
                color_attachments: &[],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let _discarded = bad_encoder.finish();
        assert!(
            finish_validation_scope(&device, encoder_scope, &encoder_telemetry, None),
            "expected wgpu validation scope to capture invalid encoder finish"
        );
        assert!(!can_paint_glass(&encoder_telemetry.snapshot(), true));
    }

    #[test]
    fn validation_error_transition_requests_fallback_repaint_once() {
        let telemetry = GpuTelemetry::new(true);
        assert!(can_paint_glass(&telemetry.snapshot(), true));

        // Initial None -> Some transition triggers one fallback repaint.
        assert!(telemetry.record_validation_error("first validation error".into()));
        let first = telemetry.snapshot();
        assert!(!first.resources_ready);
        assert_eq!(
            first.validation_error.as_deref(),
            Some("first validation error")
        );
        assert!(!can_paint_glass(&first, true));

        // Repeated validation errors do not trigger additional fallback repaints or overwrite root cause.
        assert!(!telemetry.record_validation_error("second error".into()));
        assert_eq!(
            telemetry.snapshot().validation_error.as_deref(),
            Some("first validation error")
        );
    }
}
