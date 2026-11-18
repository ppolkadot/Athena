use eframe::wgpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMode {
    Solid,
    Tint,
    Glass,
}

impl SurfaceMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Tint => "Tint",
            Self::Glass => "Glass",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    MissingWgpuState,
    SoftwareAdapter,
    ResourceInitFailed,
    ForcedUnavailable,
    UnsupportedTargetFormat(wgpu::TextureFormat),
    ZeroAreaSurface,
    ValidationError,
}

impl FallbackReason {
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Self::MissingWgpuState => "missing wgpu render state".to_owned(),
            Self::SoftwareAdapter => "software adapter gated to solid".to_owned(),
            Self::ResourceInitFailed => "GPU shader/resource initialization failed".to_owned(),
            Self::ForcedUnavailable => "forced resource-unavailable toggle".to_owned(),
            Self::UnsupportedTargetFormat(fmt) => format!("unsupported target format {fmt:?}"),
            Self::ZeroAreaSurface => "zero-area viewport or clip".to_owned(),
            Self::ValidationError => "wgpu validation error captured".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveSurface {
    Solid { fallback: Option<FallbackReason> },
    Tint,
    Glass,
}

impl EffectiveSurface {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Solid { fallback: None } => "Solid",
            Self::Solid { fallback: Some(_) } => "Solid (fallback)",
            Self::Tint => "Tint",
            Self::Glass => "Glass (wgpu)",
        }
    }

    #[must_use]
    pub const fn fallback_reason(self) -> Option<FallbackReason> {
        match self {
            Self::Solid { fallback } => fallback,
            Self::Tint | Self::Glass => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvRect {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct GlassGateInput {
    pub requested: SurfaceMode,
    pub has_wgpu_state: bool,
    pub is_software_adapter: bool,
    pub resources_ready: bool,
    pub forced_unavailable: bool,
    pub target_format_supported: bool,
    pub gpu_validation_failed: bool,
    pub zero_area: bool,
}

/// Computes a centered aspect-fill (`cover`) UV crop of `source_px` inside `viewport_pt`.
///
/// Returns `None` if any dimension is zero or non-finite.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn fill_uv_rect(source_px: [u32; 2], viewport_pt: [f32; 2]) -> Option<UvRect> {
    if source_px[0] == 0
        || source_px[1] == 0
        || !viewport_pt[0].is_finite()
        || !viewport_pt[1].is_finite()
        || viewport_pt[0] <= 0.0
        || viewport_pt[1] <= 0.0
    {
        return None;
    }

    let src_aspect = f64::from(source_px[0]) / f64::from(source_px[1]);
    let vp_aspect = f64::from(viewport_pt[0]) / f64::from(viewport_pt[1]);

    if (vp_aspect - src_aspect).abs() <= 1e-6 {
        Some(UvRect {
            min: [0.0, 0.0],
            max: [1.0, 1.0],
        })
    } else if vp_aspect > src_aspect {
        let v_span = (src_aspect / vp_aspect) as f32;
        let v_min = 0.5 * (1.0 - v_span);
        Some(UvRect {
            min: [0.0, v_min],
            max: [1.0, 1.0 - v_min],
        })
    } else {
        let u_span = (vp_aspect / src_aspect) as f32;
        let u_min = 0.5 * (1.0 - u_span);
        Some(UvRect {
            min: [u_min, 0.0],
            max: [1.0 - u_min, 1.0],
        })
    }
}

#[cfg(test)]
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn global_screen_uv_at_point(point_px: [f32; 2], screen_size_px: [u32; 2]) -> Option<[f32; 2]> {
    if screen_size_px[0] == 0
        || screen_size_px[1] == 0
        || !point_px[0].is_finite()
        || !point_px[1].is_finite()
    {
        return None;
    }
    Some([
        point_px[0] / (screen_size_px[0] as f32),
        point_px[1] / (screen_size_px[1] as f32),
    ])
}

#[cfg(test)]
#[must_use]
pub fn panel_local_point_px(
    panel_origin_pt: [f32; 2],
    local_offset_pt: [f32; 2],
    pixels_per_point: f32,
) -> [f32; 2] {
    [
        (panel_origin_pt[0] + local_offset_pt[0]) * pixels_per_point,
        (panel_origin_pt[1] + local_offset_pt[1]) * pixels_per_point,
    ]
}

#[cfg(test)]
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn wallpaper_uv_at_panel_local_point(
    source_px: [u32; 2],
    screen_size_px: [u32; 2],
    pixels_per_point: f32,
    panel_origin_pt: [f32; 2],
    local_offset_pt: [f32; 2],
) -> Option<[f32; 2]> {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return None;
    }
    let viewport_pt = [
        screen_size_px[0] as f32 / pixels_per_point,
        screen_size_px[1] as f32 / pixels_per_point,
    ];
    let crop = fill_uv_rect(source_px, viewport_pt)?;
    let point_px = panel_local_point_px(panel_origin_pt, local_offset_pt, pixels_per_point);
    let global_uv = global_screen_uv_at_point(point_px, screen_size_px)?;
    Some([
        crop.min[0] + global_uv[0] * (crop.max[0] - crop.min[0]),
        crop.min[1] + global_uv[1] * (crop.max[1] - crop.min[1]),
    ])
}

/// Quarter-resolution blur target dimensions for `screen_size_px`.
///
/// Returns `None` when either backing dimension is zero.
#[must_use]
pub const fn quarter_res_extent(screen_size_px: [u32; 2]) -> Option<[u32; 2]> {
    if screen_size_px[0] == 0 || screen_size_px[1] == 0 {
        return None;
    }
    Some([screen_size_px[0].div_ceil(4), screen_size_px[1].div_ceil(4)])
}

#[must_use]
pub const fn is_supported_target_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8UnormSrgb
    )
}

#[must_use]
pub fn resolve_effective_surface(
    input: GlassGateInput,
    unsupported_format: Option<wgpu::TextureFormat>,
) -> EffectiveSurface {
    match input.requested {
        SurfaceMode::Solid => EffectiveSurface::Solid { fallback: None },
        SurfaceMode::Tint => EffectiveSurface::Tint,
        SurfaceMode::Glass => {
            if input.forced_unavailable {
                EffectiveSurface::Solid {
                    fallback: Some(FallbackReason::ForcedUnavailable),
                }
            } else if !input.has_wgpu_state {
                EffectiveSurface::Solid {
                    fallback: Some(FallbackReason::MissingWgpuState),
                }
            } else if input.is_software_adapter {
                EffectiveSurface::Solid {
                    fallback: Some(FallbackReason::SoftwareAdapter),
                }
            } else if !input.target_format_supported {
                let fmt = unsupported_format.unwrap_or(wgpu::TextureFormat::Rgba16Float);
                EffectiveSurface::Solid {
                    fallback: Some(FallbackReason::UnsupportedTargetFormat(fmt)),
                }
            } else if input.gpu_validation_failed {
                EffectiveSurface::Solid {
                    fallback: Some(FallbackReason::ValidationError),
                }
            } else if !input.resources_ready {
                EffectiveSurface::Solid {
                    fallback: Some(FallbackReason::ResourceInitFailed),
                }
            } else if input.zero_area {
                EffectiveSurface::Solid {
                    fallback: Some(FallbackReason::ZeroAreaSurface),
                }
            } else {
                EffectiveSurface::Glass
            }
        }
    }
}

#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn percentile_ms(samples_ms: &[f64], percentile: f64) -> Option<f64> {
    if samples_ms.is_empty() {
        return None;
    }
    let mut sorted = samples_ms.to_vec();
    sorted.sort_by(f64::total_cmp);
    let clamped = percentile.clamp(0.0, 1.0);
    let idx = ((sorted.len() - 1) as f64 * clamped).round() as usize;
    sorted.get(idx).copied()
}

#[cfg(test)]
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn composite_gamma_u8(fg: [u8; 3], alpha: u8, bg: [u8; 3]) -> [u8; 3] {
    let a = u16::from(alpha);
    let inv = 255 - a;
    let ch = |f: u8, b: u8| -> u8 { ((u16::from(f) * a + u16::from(b) * inv + 127) / 255) as u8 };
    [ch(fg[0], bg[0]), ch(fg[1], bg[1]), ch(fg[2], bg[2])]
}

#[cfg(test)]
fn srgb_to_linear_channel(channel: u8) -> f32 {
    let c = f32::from(channel) / 255.0;
    if c <= 0.039_28 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
fn relative_luminance(rgb: [u8; 3]) -> f32 {
    0.2126 * srgb_to_linear_channel(rgb[0])
        + 0.7152 * srgb_to_linear_channel(rgb[1])
        + 0.0722 * srgb_to_linear_channel(rgb[2])
}

#[cfg(test)]
#[must_use]
pub fn contrast_ratio_u8(fg: [u8; 3], bg: [u8; 3]) -> f32 {
    let a = relative_luminance(fg);
    let b = relative_luminance(bg);
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn fill_uv_crop_covers_matching_wide_and_tall_16_9_viewports_symmetrically() {
        let src = [1920, 1080];

        let exact = fill_uv_rect(src, [1280.0, 720.0]).expect("16:9 viewport");
        assert!(approx_eq(exact.min[0], 0.0));
        assert!(approx_eq(exact.min[1], 0.0));
        assert!(approx_eq(exact.max[0], 1.0));
        assert!(approx_eq(exact.max[1], 1.0));

        // Wide 21:9 viewport crops top and bottom symmetrically.
        let wide = fill_uv_rect(src, [2520.0, 1080.0]).expect("wide viewport");
        assert!(approx_eq(wide.min[0], 0.0));
        assert!(approx_eq(wide.max[0], 1.0));
        assert!(wide.min[1] > 0.0 && wide.max[1] < 1.0);
        assert!(approx_eq(wide.min[1] + wide.max[1], 1.0));

        // Tall 9:16 viewport crops left and right symmetrically.
        let tall = fill_uv_rect(src, [1080.0, 1920.0]).expect("tall viewport");
        assert!(approx_eq(tall.min[1], 0.0));
        assert!(approx_eq(tall.max[1], 1.0));
        assert!(tall.min[0] > 0.0 && tall.max[0] < 1.0);
        assert!(approx_eq(tall.min[0] + tall.max[0], 1.0));
    }

    #[test]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn glass_samples_same_global_wallpaper_point_across_panel_offsets_and_scales() {
        let src = [1920, 1080];
        let base_pt = [1280.0_f32, 820.0_f32];

        // Global point at (540, 410) logical points.
        let target_pt = [540.0_f32, 410.0_f32];
        let centered_origin = [390.0_f32, 260.0_f32];
        let centered_local = [
            target_pt[0] - centered_origin[0],
            target_pt[1] - centered_origin[1],
        ];
        let offset_origin = [480.0_f32, 310.0_f32];
        let offset_local = [
            target_pt[0] - offset_origin[0],
            target_pt[1] - offset_origin[1],
        ];

        for scale in [1.0_f32, 2.0_f32] {
            let screen_px = [(base_pt[0] * scale) as u32, (base_pt[1] * scale) as u32];
            let uv_centered = wallpaper_uv_at_panel_local_point(
                src,
                screen_px,
                scale,
                centered_origin,
                centered_local,
            )
            .expect("centered UV");
            let uv_offset = wallpaper_uv_at_panel_local_point(
                src,
                screen_px,
                scale,
                offset_origin,
                offset_local,
            )
            .expect("offset UV");

            assert!(approx_eq(uv_centered[0], uv_offset[0]));
            assert!(approx_eq(uv_centered[1], uv_offset[1]));
            assert!(uv_centered[0] >= 0.0 && uv_centered[0] <= 1.0);
            assert!(uv_centered[1] >= 0.0 && uv_centered[1] <= 1.0);
        }

        // 1x vs 2x scale samples the same normalized wallpaper coordinate.
        let uv_1x = wallpaper_uv_at_panel_local_point(
            src,
            [1280, 820],
            1.0,
            centered_origin,
            centered_local,
        )
        .expect("1x UV");
        let uv_2x =
            wallpaper_uv_at_panel_local_point(src, [2560, 1640], 2.0, offset_origin, offset_local)
                .expect("2x UV");
        assert!(approx_eq(uv_1x[0], uv_2x[0]));
        assert!(approx_eq(uv_1x[1], uv_2x[1]));
    }

    #[test]
    fn zero_size_inputs_are_rejected_and_skip_gpu_targets() {
        assert_eq!(fill_uv_rect([0, 1080], [1280.0, 820.0]), None);
        assert_eq!(fill_uv_rect([1920, 0], [1280.0, 820.0]), None);
        assert_eq!(fill_uv_rect([1920, 1080], [0.0, 820.0]), None);
        assert_eq!(fill_uv_rect([1920, 1080], [1280.0, 0.0]), None);
        assert_eq!(fill_uv_rect([1920, 1080], [-10.0, 820.0]), None);
        assert_eq!(quarter_res_extent([0, 820]), None);
        assert_eq!(quarter_res_extent([1280, 0]), None);
        assert_eq!(quarter_res_extent([1281, 821]), Some([321, 206]));
        assert_eq!(global_screen_uv_at_point([10.0, 10.0], [0, 100]), None);
    }

    #[test]
    fn missing_failed_and_software_states_select_solid_fallback() {
        let healthy_glass = GlassGateInput {
            requested: SurfaceMode::Glass,
            has_wgpu_state: true,
            is_software_adapter: false,
            resources_ready: true,
            forced_unavailable: false,
            target_format_supported: true,
            gpu_validation_failed: false,
            zero_area: false,
        };
        assert_eq!(
            resolve_effective_surface(healthy_glass, None),
            EffectiveSurface::Glass
        );

        let missing = GlassGateInput {
            has_wgpu_state: false,
            ..healthy_glass
        };
        assert_eq!(
            resolve_effective_surface(missing, None),
            EffectiveSurface::Solid {
                fallback: Some(FallbackReason::MissingWgpuState)
            }
        );

        let software = GlassGateInput {
            is_software_adapter: true,
            ..healthy_glass
        };
        assert_eq!(
            resolve_effective_surface(software, None),
            EffectiveSurface::Solid {
                fallback: Some(FallbackReason::SoftwareAdapter)
            }
        );

        let failed_res = GlassGateInput {
            resources_ready: false,
            ..healthy_glass
        };
        assert_eq!(
            resolve_effective_surface(failed_res, None),
            EffectiveSurface::Solid {
                fallback: Some(FallbackReason::ResourceInitFailed)
            }
        );

        let forced = GlassGateInput {
            forced_unavailable: true,
            ..healthy_glass
        };
        assert_eq!(
            resolve_effective_surface(forced, None),
            EffectiveSurface::Solid {
                fallback: Some(FallbackReason::ForcedUnavailable)
            }
        );

        let bad_fmt = GlassGateInput {
            target_format_supported: false,
            ..healthy_glass
        };
        assert_eq!(
            resolve_effective_surface(bad_fmt, Some(wgpu::TextureFormat::Rgba16Float)),
            EffectiveSurface::Solid {
                fallback: Some(FallbackReason::UnsupportedTargetFormat(
                    wgpu::TextureFormat::Rgba16Float
                ))
            }
        );

        let validation = GlassGateInput {
            gpu_validation_failed: true,
            ..healthy_glass
        };
        assert_eq!(
            resolve_effective_surface(validation, None),
            EffectiveSurface::Solid {
                fallback: Some(FallbackReason::ValidationError)
            }
        );

        let zero_area = GlassGateInput {
            zero_area: true,
            ..healthy_glass
        };
        assert_eq!(
            resolve_effective_surface(zero_area, None),
            EffectiveSurface::Solid {
                fallback: Some(FallbackReason::ZeroAreaSurface)
            }
        );

        assert!(is_supported_target_format(wgpu::TextureFormat::Rgba8Unorm));
        assert!(is_supported_target_format(wgpu::TextureFormat::Bgra8Unorm));
        assert!(is_supported_target_format(
            wgpu::TextureFormat::Rgba8UnormSrgb
        ));
        assert!(is_supported_target_format(
            wgpu::TextureFormat::Bgra8UnormSrgb
        ));
        assert!(!is_supported_target_format(
            wgpu::TextureFormat::Rgba16Float
        ));
    }

    #[test]
    fn percentile_computes_median_and_p95_deterministically() {
        assert_eq!(percentile_ms(&[], 0.5), None);
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        let p50 = percentile_ms(&values, 0.5).expect("p50");
        let p95 = percentile_ms(&values, 0.95).expect("p95");
        assert!((p50 - 51.0).abs() <= 1.0);
        assert!((p95 - 95.0).abs() <= 1.0);
    }

    #[test]
    fn spike_tints_keep_text_contrast_above_wcag_aa_over_white_and_black() {
        let dark_panel = [0x0A, 0x0C, 0x10];
        let dark_text = [0xEC, 0xEE, 0xF4];
        let dark_muted = [0x9E, 0xA8, 0xBA];
        for bg in [[255, 255, 255], [0, 0, 0]] {
            let comp = composite_gamma_u8(dark_panel, 218, bg);
            assert!(contrast_ratio_u8(dark_text, comp) >= 4.5);
            assert!(contrast_ratio_u8(dark_muted, comp) >= 4.5);
        }

        let light_panel = [0xFF, 0xFF, 0xFF];
        let light_text = [0x14, 0x18, 0x21];
        let light_muted = [0x4A, 0x52, 0x62];
        for bg in [[255, 255, 255], [0, 0, 0]] {
            let comp = composite_gamma_u8(light_panel, 218, bg);
            assert!(contrast_ratio_u8(light_text, comp) >= 4.5);
            assert!(contrast_ratio_u8(light_muted, comp) >= 4.5);
        }
    }
}
