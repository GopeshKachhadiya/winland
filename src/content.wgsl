// Instanced rounded-rect quads for island content (album art, progress bar,
// waveform bars, control glyph backings). Each instance is a rounded box with a
// premultiplied-ready color; the fragment shader evaluates a per-instance SDF
// for crisp anti-aliased corners and clips to the island's rounded rect so
// nothing spills outside during the morph.

struct Globals {
    resolution: vec2<f32>,
    premultiply: f32,
    _pad: f32,
    // Island clip rect (content must stay inside it).
    clip_center: vec2<f32>,
    clip_half: vec2<f32>,
    clip_radius: f32,
    _pad2: f32,
    _pad3: f32,
    _pad4: f32,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct Instance {
    @location(0) center: vec2<f32>,
    @location(1) half: vec2<f32>,
    @location(2) radius_pad: vec2<f32>, // x = corner radius, y = unused
    @location(3) color: vec4<f32>,      // straight color + alpha
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,   // position relative to instance center (px)
    @location(1) half: vec2<f32>,
    @location(2) radius: f32,
    @location(3) color: vec4<f32>,
    @location(4) screen: vec2<f32>,  // absolute pixel position (for clip)
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    // Unit quad (two triangles) with a 1.5px AA margin around the box.
    var unit = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let u = unit[vi];
    let margin = 2.0;
    let local = u * (inst.half + vec2<f32>(margin));
    let screen = inst.center + local;

    var out: VsOut;
    out.pos = vec4<f32>(
        (screen.x / g.resolution.x) * 2.0 - 1.0,
        1.0 - (screen.y / g.resolution.y) * 2.0,
        0.0,
        1.0,
    );
    out.local = local;
    out.half = inst.half;
    out.radius = inst.radius_pad.x;
    out.color = inst.color;
    out.screen = screen;
    return out;
}

fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = sd_round_box(in.local, in.half, min(in.radius, min(in.half.x, in.half.y)));
    let aa = max(fwidth(d), 0.75);
    var a = (1.0 - smoothstep(-aa, aa, d)) * in.color.a;

    // Clip to the island shape so content never pokes outside the morph.
    let cd = sd_round_box(in.screen - g.clip_center, g.clip_half, g.clip_radius);
    a = a * (1.0 - smoothstep(-1.0, 1.0, cd));

    let rgb = select(in.color.rgb, in.color.rgb * a, g.premultiply > 0.5);
    return vec4<f32>(rgb, a);
}
