// Textured, rounded album-art quad, clipped to the island shape.

struct U {
    resolution: vec2<f32>,
    center: vec2<f32>,
    half: vec2<f32>,
    clip_center: vec2<f32>,
    clip_half: vec2<f32>,
    radius: f32,
    alpha: f32,
    premultiply: f32,
    clip_radius: f32,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) screen: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var unit = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let un = unit[vi];
    let local = un * u.half;
    let screen = u.center + local;

    var out: VsOut;
    out.pos = vec4<f32>(
        (screen.x / u.resolution.x) * 2.0 - 1.0,
        1.0 - (screen.y / u.resolution.y) * 2.0,
        0.0,
        1.0,
    );
    out.local = local;
    out.uv = un * 0.5 + vec2<f32>(0.5);
    out.screen = screen;
    return out;
}

fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = sd_round_box(in.local, u.half, min(u.radius, min(u.half.x, u.half.y)));
    let aa = max(fwidth(d), 0.75);
    var a = (1.0 - smoothstep(-aa, aa, d)) * u.alpha;

    // Clip to the island.
    let cd = sd_round_box(in.screen - u.clip_center, u.clip_half, u.clip_radius);
    a = a * (1.0 - smoothstep(-1.0, 1.0, cd));

    let rgb = textureSample(tex, samp, in.uv).rgb;
    let out_rgb = select(rgb, rgb * a, u.premultiply > 0.5);
    return vec4<f32>(out_rgb, a);
}
