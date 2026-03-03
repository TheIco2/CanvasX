// sentinel-runtime/src/gpu/shaders/ui_box.wgsl
//
// SDF-based UI box renderer.
// Renders rounded rectangles with borders, backgrounds, and optional textures.
// Analytic anti-aliasing via SDF — no CPU tessellation needed.

// ─── Global uniforms ───
struct Globals {
    viewport: vec4<f32>,  // width, height, 1/width, 1/height
    time: f32,
    scale: f32,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

// ─── Texture ───
@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse: sampler;

// ─── Vertex input ───
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

// ─── Per-instance input ───
struct InstanceInput {
    @location(2)  rect: vec4<f32>,          // x, y, w, h
    @location(3)  bg_color: vec4<f32>,
    @location(4)  border_color: vec4<f32>,
    @location(5)  border_width: vec4<f32>,  // top, right, bottom, left
    @location(6)  border_radius: vec4<f32>, // TL, TR, BR, BL
    @location(7)  clip_rect: vec4<f32>,
    @location(8)  texture_index: i32,
    @location(9)  opacity: f32,
    @location(10) flags: u32,
};

// ─── Vertex → Fragment ───
struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) pixel_pos: vec2<f32>,  // position in pixel-space
    @location(2) rect: vec4<f32>,
    @location(3) bg_color: vec4<f32>,
    @location(4) border_color: vec4<f32>,
    @location(5) border_width: vec4<f32>,
    @location(6) border_radius: vec4<f32>,
    @location(7) clip_rect: vec4<f32>,
    @location(8) @interpolate(flat) texture_index: i32,
    @location(9) opacity: f32,
    @location(10) @interpolate(flat) flags: u32,
};

// ─── Vertex shader ───
@vertex
fn vs_main(vert: VertexInput, inst: InstanceInput) -> VertexOutput {
    var out: VertexOutput;

    // Transform unit quad [0,1]² to the element's pixel rect.
    let pixel_x = inst.rect.x + vert.position.x * inst.rect.z;
    let pixel_y = inst.rect.y + vert.position.y * inst.rect.w;

    // Convert pixel coords → NDC (clip space).
    // viewport.x = width, viewport.y = height
    let ndc_x = (pixel_x / globals.viewport.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (pixel_y / globals.viewport.y) * 2.0;

    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.uv = vert.uv;
    out.pixel_pos = vec2<f32>(pixel_x, pixel_y);
    out.rect = inst.rect;
    out.bg_color = inst.bg_color;
    out.border_color = inst.border_color;
    out.border_width = inst.border_width;
    out.border_radius = inst.border_radius;
    out.clip_rect = inst.clip_rect;
    out.texture_index = inst.texture_index;
    out.opacity = inst.opacity;
    out.flags = inst.flags;

    return out;
}

// ─── SDF: Rounded rectangle ───
// Returns the signed distance from point `p` to a rounded rectangle
// centered at origin with half-extents `b` and per-corner radii `r`.
fn sdf_rounded_rect(p: vec2<f32>, b: vec2<f32>, r: vec4<f32>) -> f32 {
    // Select radius based on quadrant
    var radius: f32;
    if p.x > 0.0 {
        if p.y > 0.0 {
            radius = r.z; // bottom-right
        } else {
            radius = r.y; // top-right
        }
    } else {
        if p.y > 0.0 {
            radius = r.w; // bottom-left
        } else {
            radius = r.x; // top-left
        }
    }

    let q = abs(p) - b + vec2<f32>(radius, radius);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - radius;
}

// ─── Fragment shader ───
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let FLAG_HAS_BG: u32      = 1u;
    let FLAG_HAS_BORDER: u32  = 2u;
    let FLAG_HAS_TEXTURE: u32 = 4u;
    let FLAG_HAS_CLIP: u32    = 8u;

    // Clip test (overflow: hidden)
    if (in.flags & FLAG_HAS_CLIP) != 0u {
        let cx = in.clip_rect.x;
        let cy = in.clip_rect.y;
        let cw = in.clip_rect.z;
        let ch = in.clip_rect.w;
        if in.pixel_pos.x < cx || in.pixel_pos.x > cx + cw
            || in.pixel_pos.y < cy || in.pixel_pos.y > cy + ch {
            discard;
        }
    }

    // Element center and half-size
    let center = vec2<f32>(in.rect.x + in.rect.z * 0.5, in.rect.y + in.rect.w * 0.5);
    let half = vec2<f32>(in.rect.z * 0.5, in.rect.w * 0.5);
    let p = in.pixel_pos - center;

    // Outer SDF (shape boundary)
    let d_outer = sdf_rounded_rect(p, half, in.border_radius);

    // Average border width for inner SDF
    let bw = max(in.border_width.x, max(in.border_width.y, max(in.border_width.z, in.border_width.w)));
    let inner_half = half - vec2<f32>(bw, bw);
    let inner_radius = max(in.border_radius - vec4<f32>(bw, bw, bw, bw), vec4<f32>(0.0));
    let d_inner = sdf_rounded_rect(p, inner_half, inner_radius);

    // Anti-aliasing: 1px smooth edge
    let aa = fwidth(d_outer);
    let outer_alpha = 1.0 - smoothstep(-aa, aa, d_outer);
    let inner_alpha = 1.0 - smoothstep(-aa, aa, d_inner);

    var color = vec4<f32>(0.0, 0.0, 0.0, 0.0);

    // Background — fills the inner rect (inside the border)
    if (in.flags & FLAG_HAS_BG) != 0u {
        var bg = in.bg_color;

        // Sample texture if available
        if (in.flags & FLAG_HAS_TEXTURE) != 0u {
            let tex_sample = textureSample(t_diffuse, s_diffuse, in.uv);
            // tiny-skia canvas data is already premultiplied alpha,
            // so use it directly without re-premultiplying.
            color = tex_sample * inner_alpha;
        } else {
            // Non-texture background: convert straight-alpha bg to premultiplied,
            // then apply SDF mask.
            let premul_bg = vec4<f32>(bg.rgb * bg.a, bg.a);
            color = premul_bg * inner_alpha;
        }
    }

    // Border — fills the ring between outer and inner rects
    if (in.flags & FLAG_HAS_BORDER) != 0u && bw > 0.0 {
        let border_alpha = max(outer_alpha - inner_alpha, 0.0);
        let premul_border = vec4<f32>(
            in.border_color.rgb * in.border_color.a,
            in.border_color.a
        );
        color = color + premul_border * border_alpha;
    }

    // Apply opacity (premultiplied: scale all channels uniformly)
    color = color * in.opacity;

    return color;
}
