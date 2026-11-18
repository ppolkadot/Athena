# Athena Native Glass Spike (Phase S1 / A1)

> Isolated `eframe` + `egui_wgpu` + `wgpu` feasibility experiment evaluating one wallpaper-sampling blurred glass panel against `Solid` and `Tint` baselines. Shipping app code, manifests, lockfiles, and user data remain untouched.

## Scope & Architecture

- **Entrypoint**: [examples/glass_spike.rs](../../examples/glass_spike.rs)
- **Private helper modules**:
  - [examples/glass_spike/math.rs](../../examples/glass_spike/math.rs) — Aspect-fill (`cover`) UV crop math, global screen UV invariance across offsets/DPI scales, quarter-resolution extent sizing, surface/fallback resolution (`MissingWgpuState`, `SoftwareAdapter`, `ResourceInitFailed`, `ForcedUnavailable`, `UnsupportedTargetFormat`, `ValidationError`, `ZeroAreaSurface`), percentile calculator, and WCAG AA contrast check helpers.
  - [examples/glass_spike/wallpaper.rs](../../examples/glass_spike/wallpaper.rs) — Deterministic `1920x1080` high-contrast checker-gradient wallpaper generator (`generate_wallpaper`) and `egui` texture lifecycle (initial allocation + same-size `set_partial([0, 0], ...)` updates).
  - [examples/glass_spike/gpu.rs](../../examples/glass_spike/gpu.rs) — `eframe::egui_wgpu::CallbackTrait` implementation caching the source wallpaper texture and quarter-resolution horizontal/vertical Gaussian blur targets (`Rgba8UnormSrgb`) inside `CallbackResources`. Blur reruns only when `(wallpaper.generation, screen_px)` changes (or during the benchmark's explicit cold-entry probe). Scoped `wgpu::ErrorFilter::Validation` checks guard resource creation and per-frame preparation.
  - [examples/glass_spike/bench.rs](../../examples/glass_spike/bench.rs) — Finite 60 Hz benchmark runner (`60` warmup frames + `300` recorded frames per mode across `Solid`, `Tint`, and `Glass`, cold entry captured post-`prepare()`, `Get-Process` memory sampling between modes, and a `5.0 s` stationary idle check).
- **Shader**: [examples/shaders/wallpaper_glass.wgsl](../../examples/shaders/wallpaper_glass.wgsl) — Separable 9-tap Gaussian blur (`vs_fullscreen`, `fs_blur_h`, `fs_blur_v`) plus physical-pixel SDF rounded panel compositor (`vs_glass`, `fs_glass`).

### Color Encoding & Coordinate Notes

1. `egui-wgpu` on this Windows Vulkan backend renders into `Bgra8Unorm` (`is_srgb = false`) and blends in gamma space using `(One, OneMinusSrcAlpha)` premultiplied alpha.
2. The quarter-resolution blur targets use `Rgba8UnormSrgb` so bilinear filtering and Gaussian taps accumulate in linear light without washing out bright/dark checker transitions. `fs_glass` converts blurred samples back to gamma space (unless `target_format.is_srgb()` is true) before compositing the panel tint and outputting premultiplied RGB/alpha.
3. Because `PaintCallbackInfo::viewport_in_pixels()` clamps near viewport edges, `fs_glass` reads physical fragment coordinates via `@builtin(position)` against the unclamped `panel_rect_px` uniform and full `screen_size_px`, keeping global wallpaper sampling locked whether the panel is centered or offset and across `1x`–`2x` pixel scales.

## How to Run & Controls

Run interactively:

```powershell
cargo run --release --locked --example glass_spike
```

Run the automated finite benchmark and exit:

```powershell
cargo run --release --locked --example glass_spike -- --auto-bench --exit-after-bench --size 1920x1080
```

### CLI Options

| Flag | Effect |
|---|---|
| `--auto-bench` | Starts the finite 3-mode benchmark + 5 s idle check immediately after startup |
| `--exit-after-bench` | Closes the window after printing the report to stdout |
| `--size WxH` | Sets initial viewport logical inner size (default `1280x820`) |
| `--scale F` | Applies simulated `pixels_per_point` via `Context::set_pixels_per_point` (e.g. `1.0`, `1.25`, `1.5`, `2.0`) |
| `--offset` | Starts with the panel offset over high-contrast checker boundaries |
| `--light` | Starts in Light theme instead of Dark theme |

### UI & Keyboard Shortcuts

| Control | Shortcut | Behavior |
|---|---|---|
| Solid / Tint / Glass | `1` / `2` / `3` (or `Alt+1/2/3` when editing) | Switches requested surface mode while preserving `TextEdit` draft and fixed layout |
| Force Unavailable | `U` | Exercises the real fallback decision path (`ForcedUnavailable` -> `Solid` + visible reason) |
| Offset Panel | `O` | Shifts panel by `(112, 68)` pt across bright/dark checker edges |
| Theme: Dark / Light | `T` | Toggles Dark (`#0A0C10` @ 85.5%) vs Light (`#FFFFFF` @ 85.5%) theme |
| Swap Wallpaper | `W` | Generates next wallpaper generation and tests `set_partial` + blur invalidation |
| Run Benchmark | `B` | Starts finite 3-mode workload (`60` warmup + `300` recorded frames/mode + `5 s` idle) |
| Scale buttons | Toolbar buttons | Switches between `Native`, `1.0x`, `1.25x`, `1.5x`, and `2.0x` simulated scales |

## Verification & Results

### Automated Checks Executed

| Check | Command | Result | Scope / Notes |
|---|---|---|---|
| Formatting | `cargo fmt --check` | Passed | Clean on all owned files |
| Spike unit tests | `cargo test --locked --example glass_spike` | 8 passed, 0 failed | Fill UV crop, offset + 1x/2x scale invariance, zero-size rejection, all fallback/software gates, percentile math, WCAG AA worst-case tint contrast, wallpaper generation/raw bytes |
| Clippy | `cargo clippy --locked --all-targets -- -D warnings` | Passed (0 warnings) | Verified across workspace targets |
| Workspace tests | `cargo test --locked` | Passed (430 lib + 8 integration tests, 2 ignored; 0 failed) | Full crate suite untouched and green |
| Release build | `cargo build --release --locked --example glass_spike` | Passed | Built `target/release/examples/glass_spike.exe` |

### Measured Native Hardware Runs

Both release runs executed on `windows (x86_64)` with `NVIDIA GeForce RTX 3060 Ti` (`Backend: Vulkan`, `DeviceType: DiscreteGpu`, `Driver: NVIDIA (617.14)`), target format `Bgra8Unorm` (`is_srgb = false`), native OS DPI scale `1.00`.

#### Run A: `1920x1080` Native `1.00x` DPI (`--auto-bench --exit-after-bench --size 1920x1080`)

| Mode (Effective) | Cold Interval / CPU (`ui` + `prepare`) | Warm `frame_interval` p50 / p95 (`300` samples) | Warm `ui_cpu` p50 / p95 | Warm `prepare_cpu` p50 / p95 | GPU Pass | Warm Cache Deltas (`gpu_up`/`targets`/`blur`/`egui_alloc`/`egui_part`) | Memory (`Working Set` / `Private`) |
|---|---|---|---|---|---|---|---|
| `Solid` (`Solid`) | `53.692 ms` / `4.589 + 0.000 ms` | `17.340 ms` / `17.678 ms` | `0.292 ms` / `0.403 ms` | `0.000 ms` / `0.000 ms` | Unavailable | `0 / 0 / 0 / 0 / 0` | `237.6 MiB` / `436.5 MiB` |
| `Tint` (`Tint`) | `17.759 ms` / `0.405 + 0.000 ms` | `17.326 ms` / `17.669 ms` | `0.301 ms` / `0.381 ms` | `0.000 ms` / `0.000 ms` | Unavailable | `0 / 0 / 0 / 0 / 0` | `238.0 MiB` / `436.6 MiB` |
| `Glass` (`Glass (wgpu)`) | `17.589 ms` / `0.418 + 0.714 ms` (`+1` upload, `+1` target, `+1` blur) | `17.326 ms` / `17.667 ms` | `0.298 ms` / `0.395 ms` | `0.047 ms` / `0.070 ms` | Unavailable | `0 / 0 / 0 / 0 / 0` | `238.2 MiB` / `436.6 MiB` |

- **Stationary idle check (`5.00 s`)**: `Decorative repaint requests: 0`, `Completion wakeup timers: 1`, `Frames observed during idle: 0` (`Focused: false` in background launch).

#### Run B: `1280x720` Simulated `1.50x` Scale + Offset Panel (`--auto-bench --exit-after-bench --size 1280x720 --scale 1.5 --offset`)

| Mode (Effective) | Cold Interval / CPU (`ui` + `prepare`) | Warm `frame_interval` p50 / p95 (`300` samples) | Warm `ui_cpu` p50 / p95 | Warm `prepare_cpu` p50 / p95 | GPU Pass | Warm Cache Deltas | Memory (`Working Set` / `Private`) |
|---|---|---|---|---|---|---|---|
| `Solid` (`Solid`) | `49.073 ms` / `3.807 + 0.000 ms` | `17.323 ms` / `17.564 ms` | `0.280 ms` / `0.359 ms` | `0.000 ms` / `0.000 ms` | Unavailable | `0 / 0 / 0 / 0 / 0` | `201.5 MiB` / `417.2 MiB` |
| `Tint` (`Tint`) | `17.682 ms` / `0.406 + 0.000 ms` | `17.315 ms` / `17.647 ms` | `0.297 ms` / `0.356 ms` | `0.000 ms` / `0.000 ms` | Unavailable | `0 / 0 / 0 / 0 / 0` | `201.9 MiB` / `417.5 MiB` |
| `Glass` (`Glass (wgpu)`) | `17.288 ms` / `0.294 + 0.907 ms` (`+1` upload, `+1` target, `+1` blur) | `17.312 ms` / `17.640 ms` | `0.306 ms` / `0.390 ms` | `0.046 ms` / `0.063 ms` | Unavailable | `0 / 0 / 0 / 0 / 0` | `202.2 MiB` / `417.5 MiB` |

- **Stationary idle check (`5.00 s`)**: `Decorative repaint requests: 0`, `Completion wakeup timers: 1`, `Frames observed during idle: 0` (`Focused: false` in background launch).

### Measurement Methodology & Limitations

- **Timing definitions**:
  - `frame_interval`: Wall-clock delta between consecutive `eframe::App::ui` entries (includes scheduling and any time blocked by UI, GPU submission, or presentation between those entries. It does not timestamp actual display presentation).
  - `ui_cpu`: Elapsed CPU time inside `AthenaGlassSpikeApp::ui`.
  - `prepare_cpu`: Elapsed CPU time inside `GlassPanelCallback::prepare`.
- **Why `frame_interval` p50/p95 is ~`17.33 / 17.67 ms` even in `Solid`**: `request_repaint_after(16_666_667 ns)` arms a Windows wait timer after frame completion; Windows scheduler/timer granularity adds ~`0.65–1.0 ms` per cycle identically across `Solid` (`17.340 / 17.678 ms`), `Tint` (`17.326 / 17.669 ms`), and `Glass` (`17.326 / 17.667 ms`). Relative to the `Solid` baseline on this harness, the observed medians differ by less than 0.02 ms and the Glass callback prepare p95 is 0.070 ms. The measurement does not isolate a causal GPU cost.
- **GPU hardware timing**: `TIMESTAMP_QUERY` is not enabled on `eframe`'s default `wgpu::Device`, and `egui_wgpu::CallbackTrait::paint` does not expose `RenderPassTimestampWrites`, so GPU pass time is recorded as `Unavailable`.
- **Memory measurement**: Sampled via `powershell -NoProfile -Command Get-Process` (`WorkingSet64`, `PrivateMemorySize64`) once at the end of each mode outside the per-frame timing loop.
- **Pending manual & reference-device checks (not established by automated runs)**:
  1. Interactive visual inspection of text crispness, corner/edge anti-aliasing bleed, offset checker-boundary continuity, and Light/Dark theme readability at native Windows `100% / 125% / 150% / 200%` display scales.
  2. Interactive live window resize, `TextEdit` typing continuity across mode changes, and focused `5 s` idle verification (`Focused: true`).
  3. Reference integrated-GPU / low-power Windows laptop measurement (current measurements are from a discrete `RTX 3060 Ti` only).

## Recommendation

**Recommendation: `viable on the measured configuration` — keep `Glass` experimental and disabled for production by default.**

- On the measured discrete GPU (`RTX 3060 Ti`, Vulkan, `1920x1080`), cached quarter-resolution `Glass` matches `Solid` and `Tint` frame pacing (`p50 = 17.326 ms`, `p95 = 17.667 ms`), keeps steady-state CPU `prepare` at `p95 = 0.070 ms` with `0` warm uploads or re-blurs, shows a 0.6 MiB larger working-set sample than Solid (+0.1 MiB private), and emits zero decorative repaint requests during the unfocused idle check.
- However, visual quality (corner bleed, crisp text over high-contrast edges), focused interactive idle, native OS DPI scaling (`125%–200%`), and baseline integrated-GPU performance remain unverified in this automated run. Production appearance work should keep `Solid` / `Tint` as the default fallback path until those remaining gates pass on the target reference device.

The native example establishes that the shader runs on the tested discrete adapter. The 16.7 ms p95 gate was not met in this timed workload, and all modes share the same timer overhead. The result supports continuing the experiment. It does not pass the production glass gate. The forced-unavailable control proves mode selection, not recovery from actual device loss or a paint failure discovered after UI submission.
