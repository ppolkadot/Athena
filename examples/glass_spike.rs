#[path = "glass_spike/bench.rs"]
mod bench;
#[path = "glass_spike/gpu.rs"]
mod gpu;
#[path = "glass_spike/math.rs"]
mod math;
#[path = "glass_spike/wallpaper.rs"]
mod wallpaper;

use std::time::Instant;

use eframe::egui::{
    self, Align2, Area, Color32, CornerRadius, FontId, Id, Key, LayerId, Order, Pos2, Rect,
    RichText, Stroke, StrokeKind, TextEdit, Vec2, ViewportCommand,
};

use crate::bench::{BenchStage, BenchmarkRunner};
use crate::gpu::{AdapterMetadata, GlassPanelCallback, GpuTelemetry, init_glass_resources};
use crate::math::{
    EffectiveSurface, GlassGateInput, SurfaceMode, fill_uv_rect, is_supported_target_format,
    resolve_effective_surface,
};
use crate::wallpaper::{EguiWallpaperTexture, WallpaperSource, generate_wallpaper};

const PANEL_WIDTH_PT: f32 = 460.0;
const PANEL_HEIGHT_PT: f32 = 340.0;
const PANEL_CORNER_RADIUS_PT: f32 = 14.0;

#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
struct CliOptions {
    auto_bench: bool,
    exit_after_bench: bool,
    window_size_pt: [f32; 2],
    simulated_scale: Option<f32>,
    offset_panel: bool,
    light_theme: bool,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            auto_bench: false,
            exit_after_bench: false,
            window_size_pt: [1280.0, 820.0],
            simulated_scale: None,
            offset_panel: false,
            light_theme: false,
        }
    }
}

fn parse_cli_options() -> CliOptions {
    let mut opts = CliOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--auto-bench" => opts.auto_bench = true,
            "--exit-after-bench" => opts.exit_after_bench = true,
            "--offset" => opts.offset_panel = true,
            "--light" => opts.light_theme = true,
            "--size" => {
                if let Some(val) = args.next()
                    && let Some((w, h)) = val.split_once('x')
                    && let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>())
                    && w > 0.0
                    && h > 0.0
                {
                    opts.window_size_pt = [w, h];
                }
            }
            "--scale" => {
                if let Some(val) = args.next()
                    && let Ok(scale) = val.parse::<f32>()
                    && scale.is_finite()
                    && scale > 0.0
                {
                    opts.simulated_scale = Some(scale);
                }
            }
            _ => {}
        }
    }
    opts
}

#[derive(Debug, Clone, Copy)]
struct SpikeTheme {
    panel_rgb: [u8; 3],
    tint_alpha: u8,
    border: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    toolbar_bg: Color32,
}

impl SpikeTheme {
    const DARK: Self = Self {
        panel_rgb: [0x0A, 0x0C, 0x10],
        tint_alpha: 218,
        border: Color32::from_rgb(0x23, 0x2A, 0x36),
        text: Color32::from_rgb(0xEC, 0xEE, 0xF4),
        muted: Color32::from_rgb(0x9E, 0xA8, 0xBA),
        accent: Color32::from_rgb(0x6C, 0x8C, 0xFF),
        toolbar_bg: Color32::from_rgba_premultiplied(10, 12, 16, 235),
    };

    const LIGHT: Self = Self {
        panel_rgb: [0xFF, 0xFF, 0xFF],
        tint_alpha: 218,
        border: Color32::from_rgb(0xD2, 0xD7, 0xE2),
        text: Color32::from_rgb(0x14, 0x18, 0x21),
        muted: Color32::from_rgb(0x4A, 0x52, 0x62),
        accent: Color32::from_rgb(0x3F, 0x55, 0xC6),
        toolbar_bg: Color32::from_rgba_premultiplied(245, 247, 251, 240),
    };

    fn solid_color(self) -> Color32 {
        Color32::from_rgb(self.panel_rgb[0], self.panel_rgb[1], self.panel_rgb[2])
    }

    fn tint_color(self) -> Color32 {
        Color32::from_rgba_unmultiplied(
            self.panel_rgb[0],
            self.panel_rgb[1],
            self.panel_rgb[2],
            self.tint_alpha,
        )
    }

    fn tint_gamma_f32(self) -> [f32; 4] {
        [
            f32::from(self.panel_rgb[0]) / 255.0,
            f32::from(self.panel_rgb[1]) / 255.0,
            f32::from(self.panel_rgb[2]) / 255.0,
            f32::from(self.tint_alpha) / 255.0,
        ]
    }

    fn border_gamma_f32(self) -> [f32; 4] {
        [
            f32::from(self.border.r()) / 255.0,
            f32::from(self.border.g()) / 255.0,
            f32::from(self.border.b()) / 255.0,
            0.9,
        ]
    }
}

#[allow(clippy::struct_excessive_bools)]
struct AthenaGlassSpikeApp {
    requested_mode: SurfaceMode,
    forced_unavailable: bool,
    offset_panel: bool,
    dark_theme: bool,
    simulated_scale: Option<f32>,
    wallpaper: WallpaperSource,
    egui_wallpaper: EguiWallpaperTexture,
    input_text: String,
    adapter: Option<AdapterMetadata>,
    telemetry: Option<GpuTelemetry>,
    bench: BenchmarkRunner,
    auto_bench_pending: bool,
    exit_after_bench: bool,
    printed_report: bool,
    last_frame_start: Option<Instant>,
    total_decorative_repaints: u64,
}

impl AthenaGlassSpikeApp {
    fn new(cc: &eframe::CreationContext<'_>, opts: CliOptions) -> Self {
        let (adapter, telemetry) = if let Some(render_state) = &cc.wgpu_render_state {
            let meta = AdapterMetadata::from_render_state(render_state);
            let tel = init_glass_resources(render_state);
            (Some(meta), Some(tel))
        } else {
            (None, None)
        };

        if let Some(scale) = opts.simulated_scale {
            cc.egui_ctx.set_pixels_per_point(scale);
        }

        Self {
            requested_mode: SurfaceMode::Glass,
            forced_unavailable: false,
            offset_panel: opts.offset_panel,
            dark_theme: !opts.light_theme,
            simulated_scale: opts.simulated_scale,
            wallpaper: generate_wallpaper(1),
            egui_wallpaper: EguiWallpaperTexture::new(),
            input_text: "Athena sample note 1234.56".to_owned(),
            adapter,
            telemetry,
            bench: BenchmarkRunner::new(),
            auto_bench_pending: opts.auto_bench,
            exit_after_bench: opts.exit_after_bench,
            printed_report: false,
            last_frame_start: None,
            total_decorative_repaints: 0,
        }
    }

    fn theme(&self) -> SpikeTheme {
        if self.dark_theme {
            SpikeTheme::DARK
        } else {
            SpikeTheme::LIGHT
        }
    }

    fn active_mode(&self) -> SurfaceMode {
        self.bench.current_mode().unwrap_or(self.requested_mode)
    }

    fn evaluate_surface(&self, zero_area: bool) -> EffectiveSurface {
        let gpu_snapshot = self
            .telemetry
            .as_ref()
            .map(GpuTelemetry::snapshot)
            .unwrap_or_default();
        let (has_wgpu_state, is_software, target_supported, unsupported_fmt) =
            if let Some(adapter) = &self.adapter {
                let supported = is_supported_target_format(adapter.target_format);
                (
                    true,
                    adapter.is_software_adapter(),
                    supported,
                    (!supported).then_some(adapter.target_format),
                )
            } else {
                (false, false, false, None)
            };

        resolve_effective_surface(
            GlassGateInput {
                requested: self.active_mode(),
                has_wgpu_state,
                is_software_adapter: is_software,
                resources_ready: gpu_snapshot.resources_ready,
                forced_unavailable: self.forced_unavailable,
                target_format_supported: target_supported,
                gpu_validation_failed: gpu_snapshot.validation_error.is_some(),
                zero_area,
            },
            unsupported_fmt,
        )
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.bench.is_running() {
            return;
        }
        let text_focused = ctx.egui_wants_keyboard_input();
        ctx.input(|i| {
            let alt = i.modifiers.alt;
            if (!text_focused && i.key_pressed(Key::Num1)) || (alt && i.key_pressed(Key::Num1)) {
                self.requested_mode = SurfaceMode::Solid;
            }
            if (!text_focused && i.key_pressed(Key::Num2)) || (alt && i.key_pressed(Key::Num2)) {
                self.requested_mode = SurfaceMode::Tint;
            }
            if (!text_focused && i.key_pressed(Key::Num3)) || (alt && i.key_pressed(Key::Num3)) {
                self.requested_mode = SurfaceMode::Glass;
            }
            if !text_focused && i.key_pressed(Key::U) {
                self.forced_unavailable = !self.forced_unavailable;
            }
            if !text_focused && i.key_pressed(Key::O) {
                self.offset_panel = !self.offset_panel;
            }
            if !text_focused && i.key_pressed(Key::T) {
                self.dark_theme = !self.dark_theme;
            }
            if !text_focused && i.key_pressed(Key::W) {
                self.wallpaper = generate_wallpaper(self.wallpaper.generation + 1);
            }
            if !text_focused && i.key_pressed(Key::B) {
                let gpu = self
                    .telemetry
                    .as_ref()
                    .map(GpuTelemetry::snapshot)
                    .unwrap_or_default();
                self.bench.start(&gpu, self.simulated_scale);
            }
        });
    }

    fn draw_background_wallpaper(&mut self, ctx: &egui::Context, screen_rect: Rect) {
        let tex_id = self.egui_wallpaper.ensure_uploaded(ctx, &self.wallpaper);
        if let Some(crop) = fill_uv_rect(
            self.wallpaper.size_px,
            [screen_rect.width(), screen_rect.height()],
        ) {
            let uv = Rect::from_min_max(
                Pos2::new(crop.min[0], crop.min[1]),
                Pos2::new(crop.max[0], crop.max[1]),
            );
            let bg_painter = ctx.layer_painter(LayerId::background());
            bg_painter.image(tex_id, screen_rect, uv, Color32::WHITE);
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn draw_toolbar(&mut self, ui: &mut egui::Ui, theme: SpikeTheme) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("spike_toolbar")
            .frame(
                egui::Frame::new()
                    .fill(theme.toolbar_bg)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .stroke(Stroke::new(1.0, theme.border)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Athena Glass Spike")
                            .strong()
                            .color(theme.text),
                    );
                    ui.separator();

                    let bench_running = self.bench.is_running();
                    for mode in [SurfaceMode::Solid, SurfaceMode::Tint, SurfaceMode::Glass] {
                        let active = self.active_mode() == mode;
                        let shortcut = match mode {
                            SurfaceMode::Solid => "1",
                            SurfaceMode::Tint => "2",
                            SurfaceMode::Glass => "3",
                        };
                        let label = format!("{} [{shortcut}]", mode.label());
                        if ui
                            .add_enabled(!bench_running, egui::Button::selectable(active, label))
                            .clicked()
                        {
                            self.requested_mode = mode;
                        }
                    }

                    ui.separator();
                    ui.add_enabled_ui(!bench_running, |ui| {
                        ui.checkbox(&mut self.forced_unavailable, "Force Unavailable [U]");
                        ui.checkbox(&mut self.offset_panel, "Offset Panel [O]");
                        if ui
                            .button(if self.dark_theme {
                                "Theme: Dark [T]"
                            } else {
                                "Theme: Light [T]"
                            })
                            .clicked()
                        {
                            self.dark_theme = !self.dark_theme;
                        }
                        if ui.button("Swap Wallpaper [W]").clicked() {
                            self.wallpaper = generate_wallpaper(self.wallpaper.generation + 1);
                        }
                        if ui.button("Run Benchmark [B]").clicked() {
                            let gpu = self
                                .telemetry
                                .as_ref()
                                .map(GpuTelemetry::snapshot)
                                .unwrap_or_default();
                            self.bench.start(&gpu, self.simulated_scale);
                        }
                    });

                    ui.separator();
                    ui.label(RichText::new("Scale:").color(theme.muted));
                    for (label, scale_opt) in [
                        ("Native", None),
                        ("1.0x", Some(1.0)),
                        ("1.25x", Some(1.25)),
                        ("1.5x", Some(1.5)),
                        ("2.0x", Some(2.0)),
                    ] {
                        let selected = self.simulated_scale == scale_opt;
                        if ui
                            .add_enabled(!bench_running, egui::Button::selectable(selected, label))
                            .clicked()
                        {
                            self.simulated_scale = scale_opt;
                            if let Some(scale) = scale_opt {
                                ctx.set_pixels_per_point(scale);
                            } else {
                                let native = ctx
                                    .input(|i| i.viewport().native_pixels_per_point)
                                    .unwrap_or(1.0);
                                ctx.set_pixels_per_point(native);
                            }
                        }
                    }
                });
            });
    }

    fn draw_panel(
        &mut self,
        ctx: &egui::Context,
        screen_rect: Rect,
        theme: SpikeTheme,
        effective: EffectiveSurface,
    ) {
        let center = screen_rect.center();
        let offset = if self.offset_panel {
            Vec2::new(112.0, 68.0)
        } else {
            Vec2::ZERO
        };
        let panel_size = Vec2::new(
            PANEL_WIDTH_PT.min((screen_rect.width() - 24.0).max(0.0)),
            PANEL_HEIGHT_PT.min((screen_rect.height() - 80.0).max(0.0)),
        );
        let panel_rect = Rect::from_center_size(center + offset, panel_size);

        self.paint_panel_surface(ctx, panel_rect, theme, effective);
        self.draw_panel_contents(ctx, panel_rect, theme, effective);
    }

    fn paint_tint_underlay(
        panel_bg_painter: &egui::Painter,
        panel_rect: Rect,
        corner: CornerRadius,
        theme: SpikeTheme,
    ) {
        panel_bg_painter.rect_filled(panel_rect, corner, theme.tint_color());
        panel_bg_painter.rect_stroke(
            panel_rect,
            corner,
            Stroke::new(1.0, theme.border),
            StrokeKind::Inside,
        );
    }

    fn paint_panel_surface(
        &self,
        ctx: &egui::Context,
        panel_rect: Rect,
        theme: SpikeTheme,
        effective: EffectiveSurface,
    ) {
        let panel_bg_painter = ctx.layer_painter(LayerId::new(
            Order::Background,
            Id::unique("glass_spike_panel_surface"),
        ));
        let corner = CornerRadius::same(14);
        match effective {
            EffectiveSurface::Solid { .. } => {
                panel_bg_painter.rect_filled(panel_rect, corner, theme.solid_color());
                panel_bg_painter.rect_stroke(
                    panel_rect,
                    corner,
                    Stroke::new(1.0, theme.border),
                    StrokeKind::Inside,
                );
            }
            EffectiveSurface::Tint => {
                Self::paint_tint_underlay(&panel_bg_painter, panel_rect, corner, theme);
            }
            EffectiveSurface::Glass => {
                // Spike-only Tint underlay: `fs_glass` outputs premultiplied alpha = 1.0 inside
                // the rounded panel, replacing this underlay cleanly in normal frames. If a
                // post-UI `wgpu` validation error causes `GlassPanelCallback::paint` to skip on
                // this frame, text keeps a WCAG-AA tinted backdrop until the next frame switches
                // to `EffectiveSurface::Solid { fallback: Some(FallbackReason::ValidationError) }`.
                Self::paint_tint_underlay(&panel_bg_painter, panel_rect, corner, theme);
                if let Some(telemetry) = &self.telemetry {
                    let callback = eframe::egui_wgpu::Callback::new_paint_callback(
                        panel_rect,
                        GlassPanelCallback {
                            wallpaper: self.wallpaper.clone(),
                            panel_rect_pt: [
                                panel_rect.min.x,
                                panel_rect.min.y,
                                panel_rect.max.x,
                                panel_rect.max.y,
                            ],
                            corner_radius_pt: PANEL_CORNER_RADIUS_PT,
                            border_width_pt: 1.0,
                            tint_gamma: theme.tint_gamma_f32(),
                            border_gamma: theme.border_gamma_f32(),
                            force_reblur: self.bench.should_force_reblur(),
                            telemetry: telemetry.clone(),
                            egui_ctx: Some(ctx.clone()),
                        },
                    );
                    panel_bg_painter.add(callback);
                }
            }
        }
    }

    fn draw_panel_contents(
        &mut self,
        ctx: &egui::Context,
        panel_rect: Rect,
        theme: SpikeTheme,
        effective: EffectiveSurface,
    ) {
        let gpu_stats = self
            .telemetry
            .as_ref()
            .map(GpuTelemetry::snapshot)
            .unwrap_or_default();
        let adapter_line = self.adapter.as_ref().map_or_else(
            || "Adapter: none (no wgpu_render_state)".to_owned(),
            |a| {
                format!(
                    "Adapter: {} ({}, {:?}) | Format: {:?}",
                    a.name, a.backend, a.device_type, a.target_format
                )
            },
        );
        let fallback_line = effective
            .fallback_reason()
            .map_or_else(|| "none".to_owned(), math::FallbackReason::describe);

        Area::new(Id::unique("glass_spike_panel_content"))
            .order(Order::Middle)
            .fixed_pos(panel_rect.min)
            .show(ctx, |ui| {
                ui.set_min_size(panel_rect.size());
                ui.set_max_size(panel_rect.size());
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(20))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Athena Surface & Typography Check")
                                .font(FontId::proportional(19.0))
                                .strong()
                                .color(theme.text),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(
                                "Verify sharp text, contrast, border clipping, and global wallpaper sampling alignment across Solid, Tint, and Glass.",
                            )
                            .color(theme.muted),
                        );
                        ui.add_space(10.0);

                        ui.label(
                            RichText::new(format!(
                                "Requested: {}   |   Effective: {}",
                                self.active_mode().label(),
                                effective.label()
                            ))
                            .strong()
                            .color(theme.text),
                        );
                        ui.label(
                            RichText::new(format!("Fallback reason: {fallback_line}"))
                                .color(theme.muted),
                        );
                        ui.label(RichText::new(adapter_line).small().color(theme.muted));
                        ui.label(
                            RichText::new(format!(
                                "Cache: src_alloc={} src_up={} blur_targets={} blur_runs={} egui_alloc={} egui_partial={} bench_repaints={}",
                                gpu_stats.source_allocations,
                                gpu_stats.source_uploads,
                                gpu_stats.blur_target_creations,
                                gpu_stats.blur_runs,
                                self.egui_wallpaper.stats.allocations,
                                self.egui_wallpaper.stats.partial_updates,
                                self.total_decorative_repaints
                            ))
                            .small()
                            .color(theme.muted),
                        );

                        self.draw_panel_input_section(ui, panel_rect.width(), theme);

                        if let Some(hint) = self.bench.frame_counter_hint() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(format!("Benchmark status: {hint}"))
                                    .strong()
                                    .color(theme.accent),
                            );
                        }
                    });
            });
    }

    fn draw_panel_input_section(&mut self, ui: &mut egui::Ui, panel_width: f32, theme: SpikeTheme) {
        ui.add_space(10.0);
        ui.label(
            RichText::new("Persistent draft input (Tab/click to focus):")
                .small()
                .color(theme.text),
        );
        let edit_resp = ui.add_sized(
            [panel_width - 40.0, 28.0],
            TextEdit::singleline(&mut self.input_text),
        );
        let focus_text = if edit_resp.has_focus() {
            "Focus affordance: TextEdit FOCUSED (draft preserved)"
        } else {
            "Focus affordance: TextEdit idle (draft preserved)"
        };
        let focus_color = if edit_resp.has_focus() {
            theme.accent
        } else {
            theme.muted
        };
        ui.label(RichText::new(focus_text).small().color(focus_color));
    }

    fn draw_report_overlay(&self, ctx: &egui::Context, theme: SpikeTheme) {
        let Some(report) = &self.bench.report else {
            return;
        };
        Area::new(Id::unique("glass_spike_report_overlay"))
            .order(Order::Foreground)
            .anchor(Align2::LEFT_BOTTOM, Vec2::new(12.0, -12.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme.toolbar_bg)
                    .corner_radius(CornerRadius::same(10))
                    .stroke(Stroke::new(1.0, theme.border))
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_max_width(640.0);
                        ui.set_max_height(220.0);
                        ui.label(
                            RichText::new("Benchmark Complete (also printed to stdout)")
                                .strong()
                                .color(theme.accent),
                        );
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.monospace(report.format_text());
                        });
                    });
            });
    }
}

impl eframe::App for AthenaGlassSpikeApp {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let frame_start = Instant::now();
        let frame_interval_ms = self
            .last_frame_start
            .map(|prev| frame_start.duration_since(prev).as_secs_f64() * 1000.0);
        self.last_frame_start = Some(frame_start);

        if self.auto_bench_pending {
            self.auto_bench_pending = false;
            let gpu = self
                .telemetry
                .as_ref()
                .map(GpuTelemetry::snapshot)
                .unwrap_or_default();
            self.bench.start(&gpu, self.simulated_scale);
        }

        self.handle_shortcuts(&ctx);

        let theme = self.theme();
        let screen_rect = ctx.content_rect();
        let ppp = ctx.pixels_per_point();
        let client_px = [
            (screen_rect.width() * ppp).round().max(0.0) as u32,
            (screen_rect.height() * ppp).round().max(0.0) as u32,
        ];
        let native_dpi = ctx
            .input(|i| i.viewport().native_pixels_per_point)
            .unwrap_or(ppp);
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let zero_area = client_px[0] == 0 || client_px[1] == 0;

        self.draw_background_wallpaper(&ctx, screen_rect);
        self.draw_toolbar(ui, theme);

        let effective = self.evaluate_surface(zero_area);
        self.draw_panel(&ctx, screen_rect, theme, effective);
        self.draw_report_overlay(&ctx, theme);

        let ui_cpu_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
        let gpu_snapshot = self
            .telemetry
            .as_ref()
            .map(GpuTelemetry::snapshot)
            .unwrap_or_default();

        if let Some(adapter) = &self.adapter {
            let next_repaint = self.bench.on_frame_end(
                frame_interval_ms,
                ui_cpu_ms,
                &gpu_snapshot,
                self.egui_wallpaper.stats,
                effective.label(),
                client_px,
                native_dpi,
                focused,
                adapter,
            );
            if let Some(delay) = next_repaint {
                ctx.request_repaint_after(delay);
                if matches!(
                    self.bench.stage(),
                    BenchStage::ColdEntry
                        | BenchStage::ColdCapture
                        | BenchStage::Warmup
                        | BenchStage::Recorded
                ) {
                    self.total_decorative_repaints += 1;
                }
            }
        }

        if let Some(report) = &self.bench.report
            && !self.printed_report
        {
            self.printed_report = true;
            println!("{}", report.format_text());
            if self.exit_after_bench {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    let opts = parse_cli_options();
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Athena S1/A1 Glass Spike")
            .with_inner_size(opts.window_size_pt)
            .with_min_inner_size([320.0, 240.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Athena S1/A1 Glass Spike",
        native_options,
        Box::new(move |cc| Ok(Box::new(AthenaGlassSpikeApp::new(cc, opts)))),
    )
}
