//! Island content: instanced rounded-rect quads for album art, the progress
//! bar, the compact waveform, and (later) control glyphs. Text is layered on
//! top with glyphon.

use bytemuck::{Pod, Zeroable};

use crate::dashboard::{self, DashData};
use crate::state::{Layout, NowPlaying, Transient};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadInstance {
    center: [f32; 2],
    half: [f32; 2],
    radius_pad: [f32; 2],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    resolution: [f32; 2],
    premultiply: f32,
    _p0: f32,
    clip_center: [f32; 2],
    clip_half: [f32; 2],
    clip_radius: f32,
    _p1: f32,
    _p2: f32,
    _p3: f32,
}

const MAX_QUADS: usize = 64;

pub struct ContentRenderer {
    pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buf: wgpu::Buffer,
    instances: Vec<QuadInstance>,
    count: u32,
}

impl ContentRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("content-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("content.wgsl").into()),
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("content-globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("content-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("content-bg"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("content-pl"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<QuadInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x2, // center
                1 => Float32x2, // half
                2 => Float32x2, // radius_pad
                3 => Float32x4, // color
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("content-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(instance_layout)],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("content-instances"),
            size: (std::mem::size_of::<QuadInstance>() * MAX_QUADS) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            globals_buf,
            bind_group,
            instance_buf,
            instances: Vec::with_capacity(MAX_QUADS),
            count: 0,
        }
    }

    /// Rebuild the content quad list for this frame.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        resolution: [f32; 2],
        premultiply: bool,
        layout: &Layout,
        np: Option<&NowPlaying>,
        transient: Option<&Transient>,
        dash: Option<&DashData>,
        viz: &[f32],
        time: f32,
    ) {
        self.instances.clear();
        if let Some(t) = transient {
            build_transient(&mut self.instances, layout, t);
        } else if layout.dashboard > 0.5 {
            if let Some(d) = dash {
                build_dashboard(&mut self.instances, layout, np, d, viz);
            }
        } else {
            build_content(&mut self.instances, layout, np, viz, time);
        }
        if self.instances.len() > MAX_QUADS {
            self.instances.truncate(MAX_QUADS);
        }
        self.count = self.instances.len() as u32;

        let globals = Globals {
            resolution,
            premultiply: if premultiply { 1.0 } else { 0.0 },
            _p0: 0.0,
            clip_center: layout.center,
            clip_half: layout.half,
            clip_radius: layout.radius,
            _p1: 0.0,
            _p2: 0.0,
            _p3: 0.0,
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        if self.count > 0 {
            queue.write_buffer(
                &self.instance_buf,
                0,
                bytemuck::cast_slice(&self.instances),
            );
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buf.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Album-art rect (center, half-extent, corner radius) in physical pixels,
/// morphing from the compact leading square to the expanded top-left square.
/// Shared by the placeholder quad and the textured-art renderer.
pub fn album_art_rect(layout: &Layout) -> ([f32; 2], f32, f32) {
    // In the dashboard the art lives inside the media panel.
    if layout.dashboard > 0.5 {
        let ml = dashboard::media_layout(layout);
        return (ml.art_center, ml.art_half, ml.art_half * 0.14);
    }
    let s = layout.scale.max(0.5);
    let [cx, cy] = layout.center;
    let [hw, hh] = layout.half;
    let expand = layout.expand;
    let pad = 10.0 * s;

    let compact_half = hh - 5.0 * s;
    let compact_cx = cx - hw + compact_half + pad * 0.6;
    let exp_half = (hh - pad).max(14.0 * s);
    let exp_cx = cx - hw + exp_half + pad;
    let exp_cy = cy - hh + exp_half + pad;

    let art_half = lerp(compact_half, exp_half, expand);
    let art_cx = lerp(compact_cx, exp_cx, expand);
    let art_cy = lerp(cy, exp_cy, expand);
    ([art_cx, art_cy], art_half, art_half * 0.28)
}

/// Transient indicator quads (volume level bar / battery shape). The leading
/// icon glyph and any percent text are drawn by the text layer.
fn build_transient(out: &mut Vec<QuadInstance>, layout: &Layout, t: &Transient) {
    let s = layout.scale.max(0.5);
    let [cx, cy] = layout.center;
    let [hw, _hh] = layout.half;
    let pad = 12.0 * s;

    match t {
        Transient::Volume { level, muted } => {
            let (level, muted) = (*level, *muted);
            // Leave room on the left for the speaker glyph, bar fills the rest.
            let icon_w = 22.0 * s;
            let bar_left = cx - hw + pad + icon_w;
            let bar_right = cx + hw - pad;
            let bar_w = (bar_right - bar_left).max(2.0);
            let bar_h = 4.0 * s;

            out.push(QuadInstance {
                center: [(bar_left + bar_right) * 0.5, cy],
                half: [bar_w * 0.5, bar_h * 0.5],
                radius_pad: [bar_h * 0.5, 0.0],
                color: [1.0, 1.0, 1.0, 0.22],
            });
            let lvl = if muted { 0.0 } else { level.clamp(0.0, 1.0) };
            let fill_w = bar_w * lvl;
            if fill_w > 0.5 {
                out.push(QuadInstance {
                    center: [bar_left + fill_w * 0.5, cy],
                    half: [fill_w * 0.5, bar_h * 0.5],
                    radius_pad: [bar_h * 0.5, 0.0],
                    color: [1.0, 1.0, 1.0, 0.96],
                });
            }
        }
        Transient::Battery { charging, percent } => {
            let (charging, percent) = (*charging, *percent);
            // Draw a battery as a hollow white body (outline cut out with the
            // island color) + a fill sized to percent + a terminal nub.
            let batt_w = 26.0 * s;
            let batt_h = 13.0 * s;
            let batt_cx = cx - hw + pad + batt_w * 0.5;
            let island = [0.015, 0.015, 0.02, 1.0];

            out.push(QuadInstance {
                center: [batt_cx, cy],
                half: [batt_w * 0.5, batt_h * 0.5],
                radius_pad: [3.0 * s, 0.0],
                color: [1.0, 1.0, 1.0, 0.95],
            });
            out.push(QuadInstance {
                center: [batt_cx, cy],
                half: [batt_w * 0.5 - 1.6 * s, batt_h * 0.5 - 1.6 * s],
                radius_pad: [2.0 * s, 0.0],
                color: island,
            });
            // Nub.
            out.push(QuadInstance {
                center: [batt_cx + batt_w * 0.5 + 1.6 * s, cy],
                half: [1.6 * s, 3.0 * s],
                radius_pad: [1.0 * s, 0.0],
                color: [1.0, 1.0, 1.0, 0.95],
            });
            // Fill.
            let inner_w = batt_w - 6.0 * s;
            let p = (percent as f32 / 100.0).clamp(0.0, 1.0);
            let fw = inner_w * p;
            let color = if charging {
                [0.30, 0.86, 0.40, 1.0]
            } else if percent <= 20 {
                [0.95, 0.35, 0.30, 1.0]
            } else {
                [1.0, 1.0, 1.0, 0.95]
            };
            if fw > 0.5 {
                let fill_left = batt_cx - inner_w * 0.5;
                out.push(QuadInstance {
                    center: [fill_left + fw * 0.5, cy],
                    half: [fw * 0.5, batt_h * 0.5 - 3.0 * s],
                    radius_pad: [1.0 * s, 0.0],
                    color,
                });
            }
        }
        // The device-connect popup is drawn entirely by the text layer
        // (Bluetooth glyph + device name).
        Transient::Device { .. } => {}
    }
}

/// Dashboard quads: dividers, top-bar group backings, media progress + art
/// placeholder, calendar event dots, weather hi/lo markers.
fn build_dashboard(
    out: &mut Vec<QuadInstance>,
    layout: &Layout,
    np: Option<&NowPlaying>,
    data: &DashData,
    _viz: &[f32],
) {
    let a = layout.dashboard.clamp(0.0, 1.0);
    let f = dashboard::frame(layout);
    let s = f.s;

    // Vertical dividers between the three columns.
    for x in [f.div1, f.div2] {
        let y0 = f.content_top;
        let y1 = f.bottom - f.pad;
        out.push(QuadInstance {
            center: [x, (y0 + y1) * 0.5],
            half: [0.7 * s, (y1 - y0) * 0.5],
            radius_pad: [0.7 * s, 0.0],
            color: [1.0, 1.0, 1.0, 0.09 * a],
        });
    }

    // Top-bar group backings (subtle rounded pills behind the icon groups).
    let bar = dashboard::bar_layout(layout, 4, 4);
    for group in [&bar.left_icons, &bar.right_icons] {
        if let (Some(first), Some(last)) = (group.first(), group.last()) {
            let l = first[0] - 18.0 * s;
            let r = last[0] + 18.0 * s;
            out.push(QuadInstance {
                center: [(l + r) * 0.5, bar.cy],
                half: [(r - l) * 0.5, f.bar_h * 0.42],
                radius_pad: [f.bar_h * 0.42, 0.0],
                color: [1.0, 1.0, 1.0, 0.06 * a],
            });
        }
    }

    // Media art placeholder (real texture drawn on top by the art renderer).
    let ml = dashboard::media_layout(layout);
    out.push(QuadInstance {
        center: ml.art_center,
        half: [ml.art_half, ml.art_half],
        radius_pad: [ml.art_half * 0.14, 0.0],
        color: [0.10, 0.10, 0.13, a],
    });

    // Media progress bar.
    let track_h = 2.6 * s;
    out.push(QuadInstance {
        center: [(ml.prog_left + ml.prog_right) * 0.5, ml.prog_y],
        half: [(ml.prog_right - ml.prog_left) * 0.5, track_h],
        radius_pad: [track_h, 0.0],
        color: [1.0, 1.0, 1.0, 0.2 * a],
    });
    let p = np.map(|n| n.progress).unwrap_or(0.0).clamp(0.0, 1.0);
    let fill_w = (ml.prog_right - ml.prog_left) * p;
    if fill_w > 0.5 {
        out.push(QuadInstance {
            center: [ml.prog_left + fill_w * 0.5, ml.prog_y],
            half: [fill_w * 0.5, track_h],
            radius_pad: [track_h, 0.0],
            color: [1.0, 1.0, 1.0, 0.92 * a],
        });
    }

    // Calendar event dots.
    let cal = dashboard::cal_layout(layout, data.events.len());
    for (i, ev) in data.events.iter().enumerate() {
        let dot = cal.rows[i].dot;
        out.push(QuadInstance {
            center: dot,
            half: [3.6 * s, 3.6 * s],
            radius_pad: [3.6 * s, 0.0],
            color: [ev.color[0], ev.color[1], ev.color[2], ev.color[3] * a],
        });
    }

    // Weather high/low markers.
    if data.weather.is_some() {
        let wl = dashboard::weather_layout(layout);
        out.push(QuadInstance {
            center: wl.hi_dot,
            half: [4.0 * s, 4.0 * s],
            radius_pad: [4.0 * s, 0.0],
            color: [1.0, 0.6, 0.2, a],
        });
        out.push(QuadInstance {
            center: wl.lo_dot,
            half: [4.0 * s, 4.0 * s],
            radius_pad: [4.0 * s, 0.0],
            color: [0.35, 0.62, 1.0, a],
        });
    }

    // Top-bar battery: a clean, custom-drawn minimal battery (crisper than a
    // font glyph) — hollow body + a fill showing the level, green while charging.
    if let Some((charging, pct)) = data.battery {
        let p = bar.right_icons[1];
        let bw = 21.0 * s;
        let bh = 10.5 * s;
        let shell = [0.88, 0.90, 0.95, 0.92 * a];
        let island = [0.02, 0.02, 0.028, a];
        // Body shell.
        out.push(QuadInstance {
            center: p,
            half: [bw * 0.5, bh * 0.5],
            radius_pad: [2.8 * s, 0.0],
            color: shell,
        });
        // Hollow interior.
        out.push(QuadInstance {
            center: p,
            half: [bw * 0.5 - 1.4 * s, bh * 0.5 - 1.4 * s],
            radius_pad: [1.9 * s, 0.0],
            color: island,
        });
        // Terminal nub.
        out.push(QuadInstance {
            center: [p[0] + bw * 0.5 + 1.6 * s, p[1]],
            half: [1.3 * s, 2.7 * s],
            radius_pad: [1.0 * s, 0.0],
            color: shell,
        });
        // Fill bar.
        let inner_w = bw - 5.2 * s;
        let fw = inner_w * (pct as f32 / 100.0).clamp(0.0, 1.0);
        let col = if charging {
            [0.30, 0.86, 0.45]
        } else if pct <= 20 {
            [0.96, 0.36, 0.30]
        } else {
            [0.90, 0.92, 0.96]
        };
        if fw > 0.5 {
            let fill_left = p[0] - inner_w * 0.5;
            out.push(QuadInstance {
                center: [fill_left + fw * 0.5, p[1]],
                half: [fw * 0.5, bh * 0.5 - 2.3 * s],
                radius_pad: [1.2 * s, 0.0],
                color: [col[0], col[1], col[2], a],
            });
        }
    }
}

/// Build the quad list from the current island layout and now-playing data.
/// Average the `viz` bands down to `n` bars (0..1 each).
fn viz_bars(viz: &[f32], n: usize) -> Vec<f32> {
    if viz.is_empty() {
        return vec![0.0; n];
    }
    let mut out = vec![0.0f32; n];
    let group = (viz.len() as f32 / n as f32).max(1.0);
    for (i, o) in out.iter_mut().enumerate() {
        let lo = (i as f32 * group) as usize;
        let hi = (((i + 1) as f32 * group) as usize).min(viz.len()).max(lo + 1);
        let mut s = 0.0;
        for v in &viz[lo..hi.min(viz.len())] {
            s += v;
        }
        *o = s / (hi - lo) as f32;
    }
    out
}

fn build_content(
    out: &mut Vec<QuadInstance>,
    layout: &Layout,
    np: Option<&NowPlaying>,
    viz: &[f32],
    time: f32,
) {
    let _ = time;
    let Some(np) = np else { return };
    let content = layout.content;
    if content <= 0.001 {
        return;
    }
    // The dashboard draws its own media panel.
    if layout.dashboard > 0.5 {
        return;
    }

    let s = layout.scale.max(0.5);
    let [cx, cy] = layout.center;
    let [hw, hh] = layout.half;
    let expand = layout.expand;
    let pad = 10.0 * s;

    // ---- Album art placeholder (the textured art, when available, is drawn on
    // top of this by the art renderer). ----
    let ([art_cx, art_cy], art_half, art_radius) = album_art_rect(layout);
    out.push(QuadInstance {
        center: [art_cx, art_cy],
        half: [art_half, art_half],
        radius_pad: [art_radius, 0.0],
        color: [0.58, 0.36, 0.92, content],
    });

    // ---- Compact waveform: bars on the trailing edge. They dance only while
    // playing; when paused they settle to short, dim, static bars so the island
    // clearly reads as "paused" rather than "playing". ----
    let wave_a = content * (1.0 - expand);
    if wave_a > 0.01 {
        let bars = 5;
        let bar_w = 3.0 * s;
        let gap = 3.5 * s;
        let base_x = cx + hw - pad - (bars as f32 * (bar_w + gap));
        let levels = viz_bars(viz, bars);
        for i in 0..bars {
            // `viz` is already frozen by the app when paused, so the bars simply
            // hold their last height — the "paused" animation.
            let bh = (3.0 + levels[i] * 12.0) * s;
            let alpha = if np.playing { wave_a } else { wave_a * 0.55 };
            out.push(QuadInstance {
                center: [base_x + i as f32 * (bar_w + gap), cy],
                half: [bar_w * 0.5, bh],
                radius_pad: [bar_w * 0.5, 0.0],
                color: [0.95, 0.95, 0.97, alpha],
            });
        }
    }

    // ---- Expanded-only: progress bar near the bottom. ----
    if expand > 0.01 {
        let track_left = art_cx + art_half + pad;
        let track_right = cx + hw - pad;
        let track_w = (track_right - track_left).max(0.0);
        let track_y = cy + hh - pad - 3.0 * s;
        let track_half_h = 2.0 * s;
        // Track.
        out.push(QuadInstance {
            center: [(track_left + track_right) * 0.5, track_y],
            half: [track_w * 0.5, track_half_h],
            radius_pad: [track_half_h, 0.0],
            color: [1.0, 1.0, 1.0, 0.22 * expand],
        });
        // Fill.
        let p = np.progress.clamp(0.0, 1.0);
        let fill_w = track_w * p;
        out.push(QuadInstance {
            center: [track_left + fill_w * 0.5, track_y],
            half: [(fill_w * 0.5).max(0.0), track_half_h],
            radius_pad: [track_half_h, 0.0],
            color: [1.0, 1.0, 1.0, 0.95 * expand],
        });
    }
}
