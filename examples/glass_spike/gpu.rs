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

    if let Some(err) = pop_validation_scope(device, scope) {
        telemetry.update(|c| {
            c.resources_ready = false;
            c.validation_error = Some(err);
        });
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
        &mut self,
        queue: &wgpu::Queue,
        egui_encoder: &mut wgpu::CommandEncoder,
        crop_min: [f32; 2],
        crop_max: [f32; 2],
        blur_extent: [u32; 2],
        blur_key: (u64, [u32; 2]),
    ) {
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
            return;
        };
        {
            let mut pass_h = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            let mut pass_v = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
        self.last_blur_key = Some(blur_key);
        self.telemetry.update(|c| c.blur_runs += 1);
    }
}

impl CallbackTrait for GlassPanelCallback {
    #[allow(clippy::cast_precision_loss)]
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
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

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

        if !cache.ensure_source_texture(device, queue, &self.wallpaper) {
            let _ = pop_validation_scope(device, scope);
            self.telemetry.update(|c| c.skipped_zero_area += 1);
            return Vec::new();
        }
        cache.ensure_blur_targets(device, blur_extent);

        let blur_key = (self.wallpaper.generation, screen_px);
        if self.force_reblur || cache.last_blur_key != Some(blur_key) {
            cache.run_blur_passes(
                queue,
                egui_encoder,
                crop.min,
                crop.max,
                blur_extent,
                blur_key,
            );
        }

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

        if let Some(err) = pop_validation_scope(device, scope) {
            cache.telemetry.update(|c| {
                c.validation_error = Some(err);
                c.resources_ready = false;
            });
        }

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        cache.telemetry.update(|c| {
            c.last_prepare_cpu_ms = elapsed_ms;
        });

        Vec::new()
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
        if !snapshot.resources_ready || snapshot.validation_error.is_some() {
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
