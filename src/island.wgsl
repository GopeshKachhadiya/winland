// Island SDF shader.
//
// A single full-screen triangle is rasterized; the fragment shader evaluates a
// signed-distance field for the island's rounded-rectangle and derives an
// anti-aliased alpha plus a soft drop shadow. Because the whole shape is
// described by a handful of SDF parameters (center, half-size, corner radius),
// morphing between island states is just interpolating those numbers on the
// CPU — which is what gives the fluid "blob" feel.

struct Uniforms {
    resolution: vec2<f32>,   // canvas size in physical pixels
    center: vec2<f32>,       // island center in physical pixels
    half_size: vec2<f32>,    // half extent of the rounded box
    radius: f32,             // corner radius
    premultiply: f32,        // 1.0 when the surface expects premultiplied alpha
    shadow_radius: f32,      // blur radius of the drop shadow in pixels
    shadow_alpha: f32,       // peak opacity of the drop shadow
    bg_phase: f32,           // animation phase (frozen while paused)
    bg_intensity: f32,       // how strongly the colour field shows (0..1)
    bg_color0: vec4<f32>,    // background colour A (from album art)
    bg_color1: vec4<f32>,    // background colour B
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) px: vec2<f32>,   // pixel coordinate of this fragment
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Oversized triangle covering the whole clip space.
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vi];
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    // Clip space (-1..1, y up) -> pixel space (0..res, y down).
    out.px = vec2<f32>(
        (xy.x * 0.5 + 0.5) * u.resolution.x,
        (1.0 - (xy.y * 0.5 + 0.5)) * u.resolution.y,
    );
    return out;
}

// Signed distance to a rounded box centered at the origin.
fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = in.px - u.center;
    let d = sd_round_box(p, u.half_size, u.radius);

    // Edge anti-aliasing: ~1px feather based on the screen-space derivative.
    let aa = max(fwidth(d), 0.75);
    let fill = 1.0 - smoothstep(-aa, aa, d);

    // Soft drop shadow: a wider, blurred falloff outside the shape.
    let shadow = smoothstep(u.shadow_radius, 0.0, d) * u.shadow_alpha;

    // Animated, colourful background inside the island — a flowing field built
    // from a few sine waves, tinted by the album-art colours, and scaled by
    // `bg_intensity`. `bg_phase` is advanced by the app only while the song
    // plays (so it freezes on pause) and drifts slowly when nothing is on.
    let np = p / max(u.half_size, vec2<f32>(1.0, 1.0));
    let t = u.bg_phase;
    let w1 = sin(np.x * 2.3 + t) * cos(np.y * 2.6 - t * 0.6);
    let w2 = sin((np.x + np.y) * 1.7 - t * 0.9);
    let w3 = sin(np.x * 1.1 - np.y * 1.4 + t * 0.5);
    let m1 = 0.5 + 0.5 * w1;
    let m2 = 0.5 + 0.5 * w2;
    let m3 = 0.5 + 0.5 * w3;
    let flow = mix(u.bg_color0.rgb, u.bg_color1.rgb, m1) * (0.65 + 0.5 * m3);
    let base = vec3<f32>(0.02, 0.02, 0.028);
    let amt = clamp(u.bg_intensity, 0.0, 1.0) * (0.22 + 0.45 * m2);
    let island_color = mix(base, flow, amt);

    let alpha = max(fill, shadow);
    // Straight (un-premultiplied) color: island where filled, black in the
    // shadow-only region (island_color * 0 == black).
    let straight_rgb = island_color * fill;
    let out_rgb = select(straight_rgb, straight_rgb * alpha, u.premultiply > 0.5);

    return vec4<f32>(out_rgb, alpha);
}
