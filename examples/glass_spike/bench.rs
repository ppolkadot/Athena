use std::fmt::Write as _;
#[cfg(target_os = "windows")]
use std::process::Command;
use std::time::{Duration, Instant};

use crate::gpu::{AdapterMetadata, GpuCounters};
use crate::math::{SurfaceMode, percentile_ms};
use crate::wallpaper::EguiTextureStats;

pub const WARMUP_FRAMES: usize = 60;
pub const RECORDED_FRAMES: usize = 300;
pub const TARGET_60HZ_INTERVAL: Duration = Duration::from_nanos(16_666_667);
pub const IDLE_CHECK_DURATION: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
pub struct MemorySample {
    pub working_set_mib: f64,
    pub private_mib: f64,
}

#[derive(Debug, Clone)]
pub struct ColdModeSample {
    pub frame_interval_ms: Option<f64>,
    pub ui_cpu_ms: f64,
    pub prepare_cpu_ms: f64,
    pub uploads_delta: u64,
    pub target_creations_delta: u64,
    pub blur_runs_delta: u64,
}

#[derive(Debug, Clone)]
pub struct ModeBenchResult {
    pub mode: SurfaceMode,
    pub effective_label: String,
    pub client_px: [u32; 2],
    pub native_dpi: f32,
    pub simulated_scale: Option<f32>,
    pub cold: ColdModeSample,
    pub sample_count: usize,
    pub frame_interval_p50_ms: f64,
    pub frame_interval_p95_ms: f64,
    pub frame_interval_min_ms: f64,
    pub frame_interval_max_ms: f64,
    pub ui_cpu_p50_ms: f64,
    pub ui_cpu_p95_ms: f64,
    pub prepare_cpu_p50_ms: f64,
    pub prepare_cpu_p95_ms: f64,
    pub warm_uploads_delta: u64,
    pub warm_target_creations_delta: u64,
    pub warm_blur_runs_delta: u64,
    pub egui_allocations_delta: u64,
    pub egui_partial_updates_delta: u64,
    pub gpu_blur_ms: Option<f64>,
    pub memory: Option<MemorySample>,
}

#[derive(Debug, Clone)]
pub struct IdleCheckResult {
    pub duration_s: f64,
    pub decorative_repaint_requests: u64,
    pub wakeup_timer_requests: u64,
    pub frames_observed: u64,
    pub focused: bool,
}

#[derive(Debug, Clone)]
pub struct BenchmarkReport {
    pub os_info: String,
    pub adapter: AdapterMetadata,
    pub modes: Vec<ModeBenchResult>,
    pub idle: IdleCheckResult,
}

impl BenchmarkReport {
    #[must_use]
    pub fn format_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "=== Athena Glass Spike Benchmark Report ===");
        let _ = writeln!(out, "OS: {}", self.os_info);
        let _ = writeln!(
            out,
            "Adapter: {} | Backend: {} | DeviceType: {:?} | Driver: {} ({})",
            self.adapter.name,
            self.adapter.backend,
            self.adapter.device_type,
            self.adapter.driver,
            self.adapter.driver_info
        );
        let _ = writeln!(
            out,
            "Target format: {:?} (is_srgb={}) | GPU timestamp query: {}",
            self.adapter.target_format,
            self.adapter.target_format.is_srgb(),
            if self.adapter.timestamp_query_supported {
                "supported"
            } else {
                "unavailable (TIMESTAMP_QUERY not enabled on default eframe device; main render pass timing also unexposed by egui-wgpu::CallbackTrait)"
            }
        );
        let _ = writeln!(
            out,
            "Schedule: {WARMUP_FRAMES} warmup + {RECORDED_FRAMES} recorded frames per mode at 60 Hz (16.67 ms target)"
        );
        let _ = writeln!(
            out,
            "Timing definitions: frame_interval = wall-clock delta between App::ui entries (includes UI, wgpu submit, present/v-sync, and 60 Hz sleep jitter); ui_cpu = App::ui body only; prepare_cpu = CallbackTrait::prepare body only."
        );
        for m in &self.modes {
            Self::format_mode_text(&mut out, m);
        }
        let _ = writeln!(
            out,
            "\n--- Stationary Idle Check ({:.2} s) ---",
            self.idle.duration_s
        );
        let _ = writeln!(
            out,
            "Focused: {} | Decorative repaint requests: {} | Completion wakeup timers: {} | Frames observed during idle: {}",
            self.idle.focused,
            self.idle.decorative_repaint_requests,
            self.idle.wakeup_timer_requests,
            self.idle.frames_observed
        );
        out
    }

    fn format_mode_text(out: &mut String, m: &ModeBenchResult) {
        let _ = writeln!(
            out,
            "\n--- Mode: {} ({}) ---",
            m.mode.label(),
            m.effective_label
        );
        let _ = writeln!(
            out,
            "Client pixels: {}x{} | Native DPI scale: {:.2} | Simulated scale: {}",
            m.client_px[0],
            m.client_px[1],
            m.native_dpi,
            m.simulated_scale
                .map_or_else(|| "none (native)".to_owned(), |s| format!("{s:.2}"))
        );
        let _ = writeln!(
            out,
            "Cold entry frame: interval={}, ui_cpu={:.3} ms, prepare_cpu={:.3} ms, uploads=+{}, target_creations=+{}, blur_runs=+{}",
            m.cold
                .frame_interval_ms
                .map_or_else(|| "n/a".to_owned(), |v| format!("{v:.3} ms")),
            m.cold.ui_cpu_ms,
            m.cold.prepare_cpu_ms,
            m.cold.uploads_delta,
            m.cold.target_creations_delta,
            m.cold.blur_runs_delta
        );
        let _ = writeln!(
            out,
            "Warm steady-state ({} samples): frame_interval p50={:.3} ms, p95={:.3} ms (min={:.3}, max={:.3})",
            m.sample_count,
            m.frame_interval_p50_ms,
            m.frame_interval_p95_ms,
            m.frame_interval_min_ms,
            m.frame_interval_max_ms
        );
        let _ = writeln!(
            out,
            "CPU work: ui_cpu p50={:.3} ms, p95={:.3} ms | prepare_cpu p50={:.3} ms, p95={:.3} ms | GPU pass: {}",
            m.ui_cpu_p50_ms,
            m.ui_cpu_p95_ms,
            m.prepare_cpu_p50_ms,
            m.prepare_cpu_p95_ms,
            m.gpu_blur_ms
                .map_or_else(|| "unavailable".to_owned(), |v| format!("{v:.3} ms"))
        );
        let _ = writeln!(
            out,
            "Warm cache deltas: gpu_uploads=+{}, target_creations=+{}, blur_runs=+{}, egui_allocs=+{}, egui_partial=+{}",
            m.warm_uploads_delta,
            m.warm_target_creations_delta,
            m.warm_blur_runs_delta,
            m.egui_allocations_delta,
            m.egui_partial_updates_delta
        );
        match m.memory {
            Some(mem) => {
                let _ = writeln!(
                    out,
                    "Process memory (PowerShell Get-Process after mode): working_set={:.1} MiB, private={:.1} MiB",
                    mem.working_set_mib, mem.private_mib
                );
            }
            None => {
                let _ = writeln!(out, "Process memory: unmeasured");
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchStage {
    Idle,
    ColdEntry,
    ColdCapture,
    Warmup,
    Recorded,
    IdleCheck,
    Done,
}

struct ActiveModeState {
    mode_index: usize,
    stage: BenchStage,
    frames_in_stage: usize,
    cold_start_gpu: GpuCounters,
    cold_ui_cpu_ms: f64,
    cold_sample: Option<ColdModeSample>,
    warm_start_gpu: GpuCounters,
    warm_start_egui: EguiTextureStats,
    frame_intervals_ms: Vec<f64>,
    ui_cpu_ms: Vec<f64>,
    prepare_cpu_ms: Vec<f64>,
    effective_label: String,
    client_px: [u32; 2],
    native_dpi: f32,
}

pub struct BenchmarkRunner {
    modes: [SurfaceMode; 3],
    active: Option<ActiveModeState>,
    completed_modes: Vec<ModeBenchResult>,
    idle_started_at: Option<Instant>,
    idle_wakeup_requests: u64,
    idle_decorative_requests: u64,
    idle_frames_observed: u64,
    idle_focused: bool,
    simulated_scale: Option<f32>,
    pub report: Option<BenchmarkReport>,
}

impl BenchmarkRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modes: [SurfaceMode::Solid, SurfaceMode::Tint, SurfaceMode::Glass],
            active: None,
            completed_modes: Vec::new(),
            idle_started_at: None,
            idle_wakeup_requests: 0,
            idle_decorative_requests: 0,
            idle_frames_observed: 0,
            idle_focused: true,
            simulated_scale: None,
            report: None,
        }
    }

    pub fn start(&mut self, gpu: &GpuCounters, simulated_scale: Option<f32>) {
        self.completed_modes.clear();
        self.idle_started_at = None;
        self.idle_wakeup_requests = 0;
        self.idle_decorative_requests = 0;
        self.idle_frames_observed = 0;
        self.idle_focused = true;
        self.simulated_scale = simulated_scale;
        self.report = None;
        self.active = Some(Self::begin_mode(0, gpu));
    }

    fn begin_mode(mode_index: usize, gpu: &GpuCounters) -> ActiveModeState {
        ActiveModeState {
            mode_index,
            stage: BenchStage::ColdEntry,
            frames_in_stage: 0,
            cold_start_gpu: gpu.clone(),
            cold_ui_cpu_ms: 0.0,
            cold_sample: None,
            warm_start_gpu: gpu.clone(),
            warm_start_egui: EguiTextureStats::default(),
            frame_intervals_ms: Vec::with_capacity(RECORDED_FRAMES),
            ui_cpu_ms: Vec::with_capacity(RECORDED_FRAMES),
            prepare_cpu_ms: Vec::with_capacity(RECORDED_FRAMES),
            effective_label: String::new(),
            client_px: [0, 0],
            native_dpi: 1.0,
        }
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.active.is_some() || self.idle_started_at.is_some()
    }

    #[must_use]
    pub fn stage(&self) -> BenchStage {
        if let Some(active) = &self.active {
            active.stage
        } else if self.idle_started_at.is_some() {
            BenchStage::IdleCheck
        } else if self.report.is_some() {
            BenchStage::Done
        } else {
            BenchStage::Idle
        }
    }

    #[must_use]
    pub fn current_mode(&self) -> Option<SurfaceMode> {
        self.active
            .as_ref()
            .map(|active| self.modes[active.mode_index])
    }

    #[must_use]
    pub fn should_force_reblur(&self) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.stage == BenchStage::ColdEntry
                && self.modes[active.mode_index] == SurfaceMode::Glass
        })
    }

    #[must_use]
    pub fn frame_counter_hint(&self) -> Option<String> {
        if let Some(active) = &self.active {
            let mode = self.modes[active.mode_index].label();
            let stage = match active.stage {
                BenchStage::ColdEntry | BenchStage::ColdCapture => "cold".to_owned(),
                BenchStage::Warmup => {
                    format!("warmup {}/{}", active.frames_in_stage, WARMUP_FRAMES)
                }
                BenchStage::Recorded => {
                    format!("rec {}/{}", active.frames_in_stage, RECORDED_FRAMES)
                }
                _ => String::new(),
            };
            Some(format!("{mode} ({stage})"))
        } else {
            self.idle_started_at.map(|started| {
                format!(
                    "Idle check {:.1}/{:.0}s",
                    started.elapsed().as_secs_f64(),
                    IDLE_CHECK_DURATION.as_secs_f64()
                )
            })
        }
    }

    /// Returns `Some(delay)` when the benchmark owns a synthetic repaint schedule for this frame.
    #[allow(clippy::too_many_arguments)]
    pub fn on_frame_end(
        &mut self,
        frame_interval_ms: Option<f64>,
        ui_cpu_ms: f64,
        gpu: &GpuCounters,
        egui_stats: EguiTextureStats,
        effective_label: &str,
        client_px: [u32; 2],
        native_dpi: f32,
        focused: bool,
        adapter: &AdapterMetadata,
    ) -> Option<Duration> {
        if let Some(idle_start) = self.idle_started_at {
            self.idle_frames_observed += 1;
            self.idle_focused &= focused;
            let elapsed = idle_start.elapsed();
            if elapsed >= IDLE_CHECK_DURATION {
                self.report = Some(BenchmarkReport {
                    os_info: os_description(),
                    adapter: adapter.clone(),
                    modes: self.completed_modes.clone(),
                    idle: IdleCheckResult {
                        duration_s: elapsed.as_secs_f64(),
                        decorative_repaint_requests: self.idle_decorative_requests,
                        wakeup_timer_requests: self.idle_wakeup_requests,
                        frames_observed: self.idle_frames_observed.saturating_sub(1),
                        focused: self.idle_focused,
                    },
                });
                self.idle_started_at = None;
            }
            return None;
        }

        let active = self.active.as_mut()?;
        effective_label.clone_into(&mut active.effective_label);
        active.client_px = client_px;
        active.native_dpi = native_dpi;

        match active.stage {
            BenchStage::ColdEntry => {
                // `CallbackTrait::prepare` runs after `App::ui` returns; capture its
                // counters and wall-clock frame interval on the following frame.
                active.cold_start_gpu = gpu.clone();
                active.cold_ui_cpu_ms = ui_cpu_ms;
                active.stage = BenchStage::ColdCapture;
                active.frames_in_stage = 0;
                Some(TARGET_60HZ_INTERVAL)
            }
            BenchStage::ColdCapture => {
                let prepare_ms = if self.modes[active.mode_index] == SurfaceMode::Glass {
                    gpu.last_prepare_cpu_ms
                } else {
                    0.0
                };
                active.cold_sample = Some(ColdModeSample {
                    frame_interval_ms,
                    ui_cpu_ms: active.cold_ui_cpu_ms,
                    prepare_cpu_ms: prepare_ms,
                    uploads_delta: gpu
                        .source_uploads
                        .saturating_sub(active.cold_start_gpu.source_uploads),
                    target_creations_delta: gpu
                        .blur_target_creations
                        .saturating_sub(active.cold_start_gpu.blur_target_creations),
                    blur_runs_delta: gpu
                        .blur_runs
                        .saturating_sub(active.cold_start_gpu.blur_runs),
                });
                active.stage = BenchStage::Warmup;
                active.frames_in_stage = 0;
                Some(TARGET_60HZ_INTERVAL)
            }
            BenchStage::Warmup => {
                active.frames_in_stage += 1;
                if active.frames_in_stage >= WARMUP_FRAMES {
                    active.stage = BenchStage::Recorded;
                    active.frames_in_stage = 0;
                    active.warm_start_gpu = gpu.clone();
                    active.warm_start_egui = egui_stats;
                }
                Some(TARGET_60HZ_INTERVAL)
            }
            BenchStage::Recorded => {
                self.advance_recorded_frame(frame_interval_ms, ui_cpu_ms, gpu, egui_stats, focused)
            }
            BenchStage::Idle | BenchStage::IdleCheck | BenchStage::Done => None,
        }
    }

    fn advance_recorded_frame(
        &mut self,
        frame_interval_ms: Option<f64>,
        ui_cpu_ms: f64,
        gpu: &GpuCounters,
        egui_stats: EguiTextureStats,
        focused: bool,
    ) -> Option<Duration> {
        let active = self.active.as_mut()?;
        if let Some(interval) = frame_interval_ms {
            active.frame_intervals_ms.push(interval);
        }
        active.ui_cpu_ms.push(ui_cpu_ms);
        active
            .prepare_cpu_ms
            .push(if self.modes[active.mode_index] == SurfaceMode::Glass {
                gpu.last_prepare_cpu_ms
            } else {
                0.0
            });
        active.frames_in_stage += 1;

        if active.frames_in_stage < RECORDED_FRAMES {
            return Some(TARGET_60HZ_INTERVAL);
        }

        let completed = Self::finalize_mode(
            self.modes[active.mode_index],
            active,
            gpu,
            egui_stats,
            self.simulated_scale,
        );
        let next_idx = active.mode_index + 1;
        self.completed_modes.push(completed);

        if next_idx < self.modes.len() {
            self.active = Some(Self::begin_mode(next_idx, gpu));
            Some(TARGET_60HZ_INTERVAL)
        } else {
            self.active = None;
            self.idle_started_at = Some(Instant::now());
            self.idle_wakeup_requests = 1;
            self.idle_decorative_requests = 0;
            self.idle_frames_observed = 0;
            self.idle_focused = focused;
            Some(IDLE_CHECK_DURATION)
        }
    }

    fn finalize_mode(
        mode: SurfaceMode,
        active: &ActiveModeState,
        gpu: &GpuCounters,
        egui_stats: EguiTextureStats,
        simulated_scale: Option<f32>,
    ) -> ModeBenchResult {
        let min_ms = active
            .frame_intervals_ms
            .iter()
            .copied()
            .reduce(f64::min)
            .unwrap_or(0.0);
        let max_ms = active
            .frame_intervals_ms
            .iter()
            .copied()
            .reduce(f64::max)
            .unwrap_or(0.0);
        let p50 = percentile_ms(&active.frame_intervals_ms, 0.50).unwrap_or(0.0);
        let p95 = percentile_ms(&active.frame_intervals_ms, 0.95).unwrap_or(0.0);
        let ui_p50 = percentile_ms(&active.ui_cpu_ms, 0.50).unwrap_or(0.0);
        let ui_p95 = percentile_ms(&active.ui_cpu_ms, 0.95).unwrap_or(0.0);
        let prep_p50 = percentile_ms(&active.prepare_cpu_ms, 0.50).unwrap_or(0.0);
        let prep_p95 = percentile_ms(&active.prepare_cpu_ms, 0.95).unwrap_or(0.0);

        let cold = active.cold_sample.clone().unwrap_or(ColdModeSample {
            frame_interval_ms: None,
            ui_cpu_ms: 0.0,
            prepare_cpu_ms: 0.0,
            uploads_delta: 0,
            target_creations_delta: 0,
            blur_runs_delta: 0,
        });

        ModeBenchResult {
            mode,
            effective_label: active.effective_label.clone(),
            client_px: active.client_px,
            native_dpi: active.native_dpi,
            simulated_scale,
            cold,
            sample_count: active.frame_intervals_ms.len(),
            frame_interval_p50_ms: p50,
            frame_interval_p95_ms: p95,
            frame_interval_min_ms: min_ms,
            frame_interval_max_ms: max_ms,
            ui_cpu_p50_ms: ui_p50,
            ui_cpu_p95_ms: ui_p95,
            prepare_cpu_p50_ms: prep_p50,
            prepare_cpu_p95_ms: prep_p95,
            warm_uploads_delta: gpu
                .source_uploads
                .saturating_sub(active.warm_start_gpu.source_uploads),
            warm_target_creations_delta: gpu
                .blur_target_creations
                .saturating_sub(active.warm_start_gpu.blur_target_creations),
            warm_blur_runs_delta: gpu
                .blur_runs
                .saturating_sub(active.warm_start_gpu.blur_runs),
            egui_allocations_delta: egui_stats
                .allocations
                .saturating_sub(active.warm_start_egui.allocations),
            egui_partial_updates_delta: egui_stats
                .partial_updates
                .saturating_sub(active.warm_start_egui.partial_updates),
            gpu_blur_ms: gpu.last_gpu_blur_ms,
            memory: sample_process_memory(),
        }
    }
}

#[must_use]
fn os_description() -> String {
    format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(target_os = "windows")]
#[allow(clippy::cast_precision_loss)]
fn sample_process_memory() -> Option<MemorySample> {
    let pid = std::process::id();
    let script = format!(
        "$p = Get-Process -Id {pid} -ErrorAction Stop; Write-Output \"$($p.WorkingSet64),$($p.PrivateMemorySize64)\""
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut parts = text.trim().split(',');
    let ws: u64 = parts.next()?.trim().parse().ok()?;
    let priv_mem: u64 = parts.next()?.trim().parse().ok()?;
    let mib = 1024.0 * 1024.0;
    Some(MemorySample {
        working_set_mib: ws as f64 / mib,
        private_mib: priv_mem as f64 / mib,
    })
}

#[cfg(not(target_os = "windows"))]
fn sample_process_memory() -> Option<MemorySample> {
    None
}
