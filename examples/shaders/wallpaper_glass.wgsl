// Separable quarter-resolution blur and rounded-rect gamma-space glass composite shaders
// for the Athena S1/A1 native glass spike.

struct BlurUniforms {
    crop_min_max: vec4<f32>,
    texel_step: vec2<f32>,
    blur_radius: f32,
    use_crop: u32,
};

@group(0) @binding(0) var<uniform> u_blur: BlurUniforms;
@group(0) @binding(1) var t_blur_src: texture_2d<f32>;
@group(0) @binding(2) var s_blur: sampler;

struct FullscreenOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vid: u32) -> FullscreenOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let pos = positions[vid];
    var out: FullscreenOut;
    out.clip_pos = vec4<f32>(pos, 0.0, 1.0);
    // Map clip space (-1..1, Y up) to texture UV (0..1, Y down).
    out.uv = vec2<f32>(pos.x * 0.5 + 0.5, 0.5 - pos.y * 0.5);
    return out;
}

fn blur_sample(uv: vec2<f32>) -> vec4<f32> {
    if u_blur.use_crop != 0u {
        let cropped = mix(u_blur.crop_min_max.xy, u_blur.crop_min_max.zw, uv);
        let clamped = clamp(cropped, u_blur.crop_min_max.xy, u_blur.crop_min_max.zw);
        return textureSampleLevel(t_blur_src, s_blur, clamped, 0.0);
    }
    return textureSampleLevel(t_blur_src, s_blur, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
}

fn separable_blur_9(uv: vec2<f32>) -> vec4<f32> {
    // 9-tap normalized Gaussian weights (sigma ~= 2.2 in quarter-res space).
    let w0 = 0.22702703;
    let w1 = 0.19459459;
    let w2 = 0.12162162;
    let w3 = 0.05405405;
    let w4 = 0.01621622;

    let step = u_blur.texel_step * u_blur.blur_radius;
    var acc = blur_sample(uv) * w0;
    acc += (blur_sample(uv + step * 1.0) + blur_sample(uv - step * 1.0)) * w1;
    acc += (blur_sample(uv + step * 2.0) + blur_sample(uv - step * 2.0)) * w2;
    acc += (blur_sample(uv + step * 3.0) + blur_sample(uv - step * 3.0)) * w3;
    acc += (blur_sample(uv + step * 4.0) + blur_sample(uv - step * 4.0)) * w4;
    return acc;
}

@fragment
fn fs_blur_h(in: FullscreenOut) -> @location(0) vec4<f32> {
    return separable_blur_9(in.uv);
}

@fragment
fn fs_blur_v(in: FullscreenOut) -> @location(0) vec4<f32> {
    return separable_blur_9(in.uv);
}

struct GlassUniforms {
    panel_rect_px: vec4<f32>,
    screen_size_px: vec2<f32>,
    corner_radius_px: f32,
    target_is_srgb: u32,
    tint_gamma: vec4<f32>,
    border_gamma: vec4<f32>,
    border_width_px: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u_glass: GlassUniforms;
@group(0) @binding(1) var t_blurred_wallpaper: texture_2d<f32>;
@group(0) @binding(2) var s_glass: sampler;

// Exact inverse of egui-wgpu's linear_from_gamma_rgb / gamma_from_linear_rgb.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

fn gamma_from_linear_rgb(rgb: vec3<f32>) -> vec3<f32> {
    let clamped = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let cutoff = clamped < vec3<f32>(0.0031308);
    let lower = clamped * vec3<f32>(12.92);
    let higher = vec3<f32>(1.055) * pow(clamped, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(higher, lower, cutoff);
}

fn rounded_box_sdf_px(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(half_size.x, half_size.y));
    let q = abs(p) - (half_size - vec2<f32>(r));
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@vertex
fn vs_glass(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[vid], 0.0, 1.0);
}

@fragment
fn fs_glass(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let center_px = 0.5 * (u_glass.panel_rect_px.xy + u_glass.panel_rect_px.zw);
    let half_size_px = 0.5 * (u_glass.panel_rect_px.zw - u_glass.panel_rect_px.xy);
    let sdf = rounded_box_sdf_px(frag_coord.xy - center_px, half_size_px, u_glass.corner_radius_px);

    // 1px anti-aliased rounded mask; pixels outside corners yield (0, 0, 0, 0).
    let mask_alpha = clamp(0.5 - sdf, 0.0, 1.0);
    if mask_alpha <= 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // Sample the screen-aligned blurred wallpaper at global framebuffer UV.
    let global_uv = clamp(frag_coord.xy / max(u_glass.screen_size_px, vec2<f32>(1.0)), vec2<f32>(0.0), vec2<f32>(1.0));
    let blurred_linear = textureSampleLevel(t_blurred_wallpaper, s_glass, global_uv, 0.0);
    let blurred_gamma = gamma_from_linear_rgb(blurred_linear.rgb);

    // Composite tint in gamma space to match egui's non-sRGB framebuffer blending.
    var panel_gamma = mix(blurred_gamma, u_glass.tint_gamma.rgb, u_glass.tint_gamma.a);

    // Subtle 1px inner border along the rounded perimeter.
    let bw = max(u_glass.border_width_px, 1.0);
    let border_band = clamp(1.0 - abs(sdf + 0.5 * bw) / (0.65 * bw), 0.0, 1.0);
    panel_gamma = mix(panel_gamma, u_glass.border_gamma.rgb, border_band * u_glass.border_gamma.a);

    var out_rgb = panel_gamma;
    if u_glass.target_is_srgb != 0u {
        out_rgb = linear_from_gamma_rgb(panel_gamma);
    }

    // Output premultiplied alpha matching egui-wgpu's (One, OneMinusSrcAlpha) pipeline.
    return vec4<f32>(out_rgb * mask_alpha, mask_alpha);
}
