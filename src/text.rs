//! Crisp GPU text for the expanded island (track title + artist), via glyphon.

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};

use crate::content::album_art_rect;
use crate::dashboard::{self, DashData};
use crate::state::{transport_buttons, Layout, NowPlaying, Transient};

pub struct TextLayer {
    font_system: FontSystem,
    swash: SwashCache,
    atlas: TextAtlas,
    viewport: Viewport,
    renderer: TextRenderer,
    title: Buffer,
    artist: Buffer,
    icon: Buffer,
    percent: Buffer,
    ctrl: [Buffer; 3],
    /// Grow-on-demand buffers for the dashboard's many labels/glyphs.
    pool: Vec<Buffer>,
    has_text: bool,
}

/// Font family selector for a dashboard text item.
#[derive(Clone, Copy)]
enum Font {
    Sans,
    Icon,
    Emoji,
}

struct TextSpec {
    text: String,
    size: f32,
    bold: bool,
    font: Font,
    color: Color,
    left: f32,
    top: f32,
    maxw: f32,
}

impl TextLayer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let mut font_system = FontSystem::new();
        let title = Buffer::new(&mut font_system, Metrics::new(18.0, 22.0));
        let artist = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
        let icon = Buffer::new(&mut font_system, Metrics::new(16.0, 18.0));
        let percent = Buffer::new(&mut font_system, Metrics::new(14.0, 16.0));
        let ctrl = [
            Buffer::new(&mut font_system, Metrics::new(16.0, 18.0)),
            Buffer::new(&mut font_system, Metrics::new(16.0, 18.0)),
            Buffer::new(&mut font_system, Metrics::new(16.0, 18.0)),
        ];

        Self {
            font_system,
            swash: SwashCache::new(),
            atlas,
            viewport,
            renderer,
            title,
            artist,
            icon,
            percent,
            ctrl,
            pool: Vec::new(),
            has_text: false,
        }
    }

    /// Lay out and upload the title/artist for this frame.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        res_w: u32,
        res_h: u32,
        layout: &Layout,
        np: Option<&NowPlaying>,
        transient: Option<&Transient>,
        dash: Option<&DashData>,
        time: f32,
    ) {
        self.viewport.update(
            queue,
            Resolution {
                width: res_w,
                height: res_h,
            },
        );

        if let Some(t) = transient {
            self.prepare_transient(device, queue, layout, t);
            return;
        }

        if layout.dashboard > 0.5 {
            if let Some(d) = dash {
                self.prepare_dashboard(device, queue, layout, np, d);
            }
            return;
        }

        let expand = layout.expand;
        let np = match np {
            Some(n) if expand > 0.02 => n,
            _ => {
                self.has_text = false;
                let _ = self.renderer.prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    std::iter::empty::<TextArea>(),
                    &mut self.swash,
                );
                return;
            }
        };
        self.has_text = true;

        let s = layout.scale.max(0.5);
        let [cx, cy] = layout.center;
        let [hw, hh] = layout.half;
        let pad = 10.0 * s;

        // Album art occupies the top-left; text sits strictly to its right.
        // Derive the gap from the *actual* art rect so they never overlap.
        let (art_center, art_half, _) = album_art_rect(layout);
        let text_left = art_center[0] + art_half + pad * 1.4;
        let text_right = cx + hw - pad;
        let text_w = (text_right - text_left).max(10.0);

        let title_attrs = Attrs::new().family(Family::SansSerif).weight(Weight::BOLD);
        let artist_attrs = Attrs::new().family(Family::SansSerif);

        // Shape the title. `borrow_with` couples the buffer to the font system
        // so the setters below reshape immediately.
        {
            let mut b = self.title.borrow_with(&mut self.font_system);
            b.set_wrap(Wrap::None);
            b.set_metrics(Metrics::new(18.0 * s, 22.0 * s));
            b.set_size(Some(text_w), Some(26.0 * s));
            b.set_text(&np.title, &title_attrs, Shaping::Advanced, None);
            b.shape_until_scroll(false);
        }
        {
            let mut b = self.artist.borrow_with(&mut self.font_system);
            b.set_wrap(Wrap::None);
            b.set_metrics(Metrics::new(14.0 * s, 18.0 * s));
            b.set_size(Some(text_w), Some(22.0 * s));
            b.set_text(&np.artist, &artist_attrs, Shaping::Advanced, None);
            b.shape_until_scroll(false);
        }

        let a = (expand.clamp(0.0, 1.0) * 255.0) as u8;
        let title_top = cy - hh + pad + art_half * 0.22;
        let artist_top = title_top + 24.0 * s;

        let clip = TextBounds {
            left: (cx - hw) as i32,
            top: (cy - hh) as i32,
            right: (cx + hw) as i32,
            bottom: (cy + hh) as i32,
        };

        // Transport control glyphs (prev / play-pause / next), Segoe Fluent Icons.
        let buttons = transport_buttons(layout);
        if let Some(b) = buttons.as_ref() {
            let icon_attrs = Attrs::new().family(Family::Name("Segoe Fluent Icons"));
            let glyphs = [
                "\u{E892}",                                  // Previous
                if np.playing { "\u{E769}" } else { "\u{E768}" }, // Pause / Play
                "\u{E893}",                                  // Next
            ];
            for i in 0..3 {
                let mut buf = self.ctrl[i].borrow_with(&mut self.font_system);
                buf.set_wrap(Wrap::None);
                buf.set_metrics(Metrics::new(b.size, b.size + 2.0 * s));
                buf.set_size(Some(b.size * 2.0), Some(b.size * 2.0));
                buf.set_text(glyphs[i], &icon_attrs, Shaping::Advanced, None);
                buf.shape_until_scroll(false);
            }
        }

        // Marquee: if a line is wider than its column, scroll it horizontally.
        let title_w = self.title.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        let artist_w = self.artist.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        let title_off = marquee_offset(time, (title_w - text_w).max(0.0), s);
        let artist_off = marquee_offset(time + 0.7, (artist_w - text_w).max(0.0), s);

        // Clip title/artist to their column so scrolled text can't spill onto
        // the album art.
        let text_clip = TextBounds {
            left: text_left as i32,
            top: (cy - hh) as i32,
            right: text_right as i32,
            bottom: (cy + hh) as i32,
        };

        let mut areas: Vec<TextArea> = vec![
            TextArea {
                buffer: &self.title,
                left: text_left + title_off,
                top: title_top,
                scale: 1.0,
                bounds: text_clip,
                default_color: Color::rgba(255, 255, 255, a),
                custom_glyphs: &[],
            },
            TextArea {
                buffer: &self.artist,
                left: text_left + artist_off,
                top: artist_top,
                scale: 1.0,
                bounds: text_clip,
                default_color: Color::rgba(200, 200, 205, a),
                custom_glyphs: &[],
            },
        ];

        if let Some(b) = buttons.as_ref() {
            for i in 0..3 {
                areas.push(TextArea {
                    buffer: &self.ctrl[i],
                    left: b.centers[i][0] - b.size * 0.6,
                    top: b.centers[i][1] - b.size * 0.75,
                    scale: 1.0,
                    bounds: clip,
                    default_color: Color::rgba(255, 255, 255, a),
                    custom_glyphs: &[],
                });
            }
        }

        if let Err(e) = self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        ) {
            log::warn!("text prepare failed: {e:?}");
        }
    }

    /// Render the transient indicator's glyph/text (speaker icon, battery %).
    fn prepare_transient(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &Layout,
        t: &Transient,
    ) {
        self.has_text = true;
        let s = layout.scale.max(0.5);
        let [cx, cy] = layout.center;
        let [hw, hh] = layout.half;
        let pad = 12.0 * s;

        let clip = TextBounds {
            left: (cx - hw) as i32,
            top: (cy - hh) as i32,
            right: (cx + hw) as i32,
            bottom: (cy + hh) as i32,
        };

        let mut areas: Vec<TextArea> = Vec::new();

        match t {
            Transient::Volume { muted, .. } => {
                // Segoe Fluent Icons: Volume (E767) / Mute (E74F).
                let glyph = if *muted { "\u{E74F}" } else { "\u{E767}" };
                let attrs = Attrs::new().family(Family::Name("Segoe Fluent Icons"));
                {
                    let mut b = self.icon.borrow_with(&mut self.font_system);
                    b.set_wrap(Wrap::None);
                    b.set_metrics(Metrics::new(16.0 * s, 18.0 * s));
                    b.set_size(Some(28.0 * s), Some(22.0 * s));
                    b.set_text(glyph, &attrs, Shaping::Advanced, None);
                    b.shape_until_scroll(false);
                }
                areas.push(TextArea {
                    buffer: &self.icon,
                    left: cx - hw + pad,
                    top: cy - 9.0 * s,
                    scale: 1.0,
                    bounds: clip,
                    default_color: Color::rgba(255, 255, 255, 235),
                    custom_glyphs: &[],
                });
            }
            Transient::Battery { percent, .. } => {
                let attrs = Attrs::new().family(Family::SansSerif).weight(Weight::BOLD);
                let text = format!("{percent}%");
                {
                    let mut b = self.percent.borrow_with(&mut self.font_system);
                    b.set_wrap(Wrap::None);
                    b.set_metrics(Metrics::new(14.0 * s, 16.0 * s));
                    b.set_size(Some(60.0 * s), Some(20.0 * s));
                    b.set_text(&text, &attrs, Shaping::Advanced, None);
                    b.shape_until_scroll(false);
                }
                areas.push(TextArea {
                    buffer: &self.percent,
                    left: cx - hw + pad + 36.0 * s,
                    top: cy - 8.0 * s,
                    scale: 1.0,
                    bounds: clip,
                    default_color: Color::rgba(255, 255, 255, 235),
                    custom_glyphs: &[],
                });
            }
            Transient::Device { name } => {
                // Bluetooth glyph (MDL2 E702) + device name.
                let icon_attrs = Attrs::new().family(Family::Name("Segoe Fluent Icons"));
                {
                    let mut b = self.icon.borrow_with(&mut self.font_system);
                    b.set_wrap(Wrap::None);
                    b.set_metrics(Metrics::new(15.0 * s, 18.0 * s));
                    b.set_size(Some(26.0 * s), Some(22.0 * s));
                    b.set_text("\u{E702}", &icon_attrs, Shaping::Advanced, None);
                    b.shape_until_scroll(false);
                }
                areas.push(TextArea {
                    buffer: &self.icon,
                    left: cx - hw + pad,
                    top: cy - 9.0 * s,
                    scale: 1.0,
                    bounds: clip,
                    default_color: Color::rgba(255, 255, 255, 235),
                    custom_glyphs: &[],
                });

                let name_attrs = Attrs::new().family(Family::SansSerif).weight(Weight::BOLD);
                {
                    let mut b = self.percent.borrow_with(&mut self.font_system);
                    b.set_wrap(Wrap::None);
                    b.set_metrics(Metrics::new(14.0 * s, 16.0 * s));
                    b.set_size(Some(2.0 * hw - 44.0 * s), Some(20.0 * s));
                    b.set_text(name, &name_attrs, Shaping::Advanced, None);
                    b.shape_until_scroll(false);
                }
                areas.push(TextArea {
                    buffer: &self.percent,
                    left: cx - hw + pad + 30.0 * s,
                    top: cy - 8.0 * s,
                    scale: 1.0,
                    bounds: clip,
                    default_color: Color::rgba(255, 255, 255, 235),
                    custom_glyphs: &[],
                });
            }
        }

        if let Err(e) = self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        ) {
            log::warn!("transient text prepare failed: {e:?}");
        }
    }

    /// Render every label + glyph of the full dashboard using the buffer pool.
    fn prepare_dashboard(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &Layout,
        np: Option<&NowPlaying>,
        data: &DashData,
    ) {
        self.has_text = true;
        let s = layout.scale.max(0.5);
        let [cx, cy] = layout.center;
        let [hw, hh] = layout.half;
        let da = (layout.dashboard.clamp(0.0, 1.0) * 255.0) as u8;
        let white = Color::rgba(255, 255, 255, da);
        let dim = Color::rgba(168, 168, 176, da);

        let mut specs: Vec<TextSpec> = Vec::new();
        let glyph = |specs: &mut Vec<TextSpec>, g: &str, p: [f32; 2], size: f32, font: Font, color: Color| {
            specs.push(TextSpec {
                text: g.to_string(),
                size,
                bold: false,
                font,
                color,
                left: p[0] - size * 0.62,
                top: p[1] - size * 0.82,
                maxw: size * 2.4,
            });
        };

        // ---- Top icon bar ----
        let bar = dashboard::bar_layout(layout, 4, 4);
        let left_glyphs = ["\u{E80F}", "\u{E715}", "\u{E8C8}", "\u{E706}"]; // home, mail, copy, sun
        for (i, g) in left_glyphs.iter().enumerate() {
            glyph(&mut specs, g, bar.left_icons[i], bar.icon_size, Font::Icon, white);
        }
        // Right group: lock, (dynamic battery), bell, settings.
        let right_glyphs = ["\u{E785}", "", "\u{E7ED}", "\u{E713}"];
        for (i, g) in right_glyphs.iter().enumerate() {
            let p = bar.right_icons[i];
            if i == 1 {
                // Battery is custom-drawn in the content layer; here we only add
                // the small % label beneath it.
                if let Some((_charging, pct)) = data.battery {
                    specs.push(TextSpec {
                        text: format!("{pct}%"),
                        size: 9.0 * s,
                        bold: false,
                        font: Font::Sans,
                        color: dim,
                        left: p[0] - 10.0 * s,
                        top: p[1] + bar.icon_size * 0.78,
                        maxw: 40.0 * s,
                    });
                }
                continue;
            }
            glyph(&mut specs, g, p, bar.icon_size, Font::Icon, white);
        }

        // Earbuds battery (centered in the top bar) when a device reports it.
        if let Some((_, pct)) = &data.buds {
            glyph(&mut specs, "\u{1F3A7}", [cx - 22.0 * s, bar.cy], bar.icon_size + 1.0 * s, Font::Emoji, white);
            specs.push(TextSpec {
                text: format!("{pct}%"),
                size: 11.0 * s,
                bold: true,
                font: Font::Sans,
                color: white,
                left: cx - 2.0 * s,
                top: bar.cy - 8.0 * s,
                maxw: 50.0 * s,
            });
        }

        // ---- Media panel ----
        let ml = dashboard::media_layout(layout);
        if let Some(n) = np {
            specs.push(TextSpec { text: n.title.clone(), size: 16.0 * s, bold: true, font: Font::Sans, color: white, left: ml.title_pos[0], top: ml.title_pos[1], maxw: ml.text_w });
            specs.push(TextSpec { text: n.artist.clone(), size: 12.5 * s, bold: false, font: Font::Sans, color: dim, left: ml.artist_pos[0], top: ml.artist_pos[1], maxw: ml.text_w });
            specs.push(TextSpec { text: fmt_time(n.position), size: 10.5 * s, bold: false, font: Font::Sans, color: dim, left: ml.prog_left, top: ml.time_y, maxw: 50.0 * s });
            specs.push(TextSpec { text: fmt_time(n.duration), size: 10.5 * s, bold: false, font: Font::Sans, color: dim, left: ml.prog_right - 34.0 * s, top: ml.time_y, maxw: 50.0 * s });
        } else {
            specs.push(TextSpec { text: "Nothing playing".into(), size: 13.0 * s, bold: false, font: Font::Sans, color: dim, left: ml.title_pos[0], top: ml.title_pos[1], maxw: ml.text_w });
        }
        let playing = np.map(|n| n.playing).unwrap_or(false);
        let ctrl_glyphs = ["\u{E892}", if playing { "\u{E769}" } else { "\u{E768}" }, "\u{E893}"];
        for i in 0..3 {
            glyph(&mut specs, ctrl_glyphs[i], ml.ctrl[i], ml.ctrl_size, Font::Icon, white);
        }

        // ---- Calendar panel ----
        let cal = dashboard::cal_layout(layout, data.events.len());
        specs.push(TextSpec { text: data.date_line.clone(), size: 16.0 * s, bold: true, font: Font::Sans, color: white, left: cal.date_pos[0], top: cal.date_pos[1], maxw: cal.col_w });
        specs.push(TextSpec { text: data.event_summary.clone(), size: 11.5 * s, bold: false, font: Font::Sans, color: dim, left: cal.summary_pos[0], top: cal.summary_pos[1], maxw: cal.col_w });
        for (i, ev) in data.events.iter().enumerate() {
            let r = &cal.rows[i];
            specs.push(TextSpec { text: ev.title.clone(), size: 13.0 * s, bold: true, font: Font::Sans, color: white, left: r.title[0], top: r.title[1], maxw: cal.col_w - 18.0 * s });
            specs.push(TextSpec { text: ev.time.clone(), size: 10.5 * s, bold: false, font: Font::Sans, color: dim, left: r.time[0], top: r.time[1], maxw: cal.col_w - 18.0 * s });
        }

        // ---- Weather panel ----
        let wl = dashboard::weather_layout(layout);
        if let Some(w) = &data.weather {
            glyph(&mut specs, dashboard::weather_emoji(w.code), wl.icon_pos, 24.0 * s, Font::Emoji, white);
            specs.push(TextSpec { text: format!("{}\u{00B0}", w.temp), size: 30.0 * s, bold: true, font: Font::Sans, color: white, left: wl.temp_pos[0], top: wl.temp_pos[1], maxw: 90.0 * s });
            specs.push(TextSpec { text: w.condition.clone(), size: 13.0 * s, bold: false, font: Font::Sans, color: dim, left: wl.cond_pos[0], top: wl.cond_pos[1], maxw: 150.0 * s });
            specs.push(TextSpec { text: w.city.clone(), size: 15.0 * s, bold: true, font: Font::Sans, color: white, left: wl.city_pos[0], top: wl.city_pos[1], maxw: 150.0 * s });
            specs.push(TextSpec { text: "High".into(), size: 10.0 * s, bold: false, font: Font::Sans, color: dim, left: wl.hi_label[0], top: wl.hi_label[1], maxw: 60.0 * s });
            specs.push(TextSpec { text: format!("{}\u{00B0}", w.high), size: 14.0 * s, bold: true, font: Font::Sans, color: white, left: wl.hi_val[0], top: wl.hi_val[1], maxw: 60.0 * s });
            specs.push(TextSpec { text: "Low".into(), size: 10.0 * s, bold: false, font: Font::Sans, color: dim, left: wl.lo_label[0], top: wl.lo_label[1], maxw: 60.0 * s });
            specs.push(TextSpec { text: format!("{}\u{00B0}", w.low), size: 14.0 * s, bold: true, font: Font::Sans, color: white, left: wl.lo_val[0], top: wl.lo_val[1], maxw: 60.0 * s });
        } else {
            specs.push(TextSpec { text: "Loading weather\u{2026}".into(), size: 12.0 * s, bold: false, font: Font::Sans, color: dim, left: wl.icon_pos[0], top: wl.cond_pos[1], maxw: 150.0 * s });
        }

        // Grow the pool and shape every spec.
        while self.pool.len() < specs.len() {
            self.pool
                .push(Buffer::new(&mut self.font_system, Metrics::new(14.0, 16.0)));
        }
        for (i, sp) in specs.iter().enumerate() {
            let fam = match sp.font {
                Font::Sans => Family::SansSerif,
                Font::Icon => Family::Name("Segoe Fluent Icons"),
                Font::Emoji => Family::Name("Segoe UI Emoji"),
            };
            let mut attrs = Attrs::new().family(fam);
            if sp.bold {
                attrs = attrs.weight(Weight::BOLD);
            }
            let mut b = self.pool[i].borrow_with(&mut self.font_system);
            b.set_wrap(Wrap::None);
            b.set_metrics(Metrics::new(sp.size, sp.size * 1.3));
            b.set_size(Some(sp.maxw), Some(sp.size * 2.2));
            b.set_text(&sp.text, &attrs, Shaping::Advanced, None);
            b.shape_until_scroll(false);
        }

        // Clip each column's text to its own column so long titles/artists are
        // cut off at the divider instead of bleeding into the next panel.
        let f = dashboard::frame(layout);
        let col_top = f.content_top as i32;
        let bottom = (cy + hh) as i32;
        let island_clip = TextBounds {
            left: (cx - hw) as i32,
            top: (cy - hh) as i32,
            right: (cx + hw) as i32,
            bottom,
        };
        let media_clip = TextBounds {
            left: (f.left + f.pad) as i32,
            top: col_top,
            right: (f.div1 - f.pad * 0.4) as i32,
            bottom,
        };
        let cal_clip = TextBounds {
            left: (f.div1 + f.pad * 0.5) as i32,
            top: col_top,
            right: (f.div2 - f.pad * 0.4) as i32,
            bottom,
        };
        let weather_clip = TextBounds {
            left: (f.div2 + f.pad * 0.5) as i32,
            top: col_top,
            right: (f.right - f.pad * 0.4) as i32,
            bottom,
        };
        let areas: Vec<TextArea> = specs
            .iter()
            .enumerate()
            .map(|(i, sp)| {
                // Top-bar items (above the columns) aren't column-clipped.
                let clip = if sp.top < f.content_top {
                    island_clip
                } else if sp.left < f.div1 {
                    media_clip
                } else if sp.left < f.div2 {
                    cal_clip
                } else {
                    weather_clip
                };
                TextArea {
                    buffer: &self.pool[i],
                    left: sp.left,
                    top: sp.top,
                    scale: 1.0,
                    bounds: clip,
                    default_color: sp.color,
                    custom_glyphs: &[],
                }
            })
            .collect();

        if let Err(e) = self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        ) {
            log::warn!("dashboard text prepare failed: {e:?}");
        }
    }

    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        let _ = self.renderer.render(&self.atlas, &self.viewport, pass);
    }
}

fn fmt_time(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "0:00".into();
    }
    let s = secs as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Ping-pong marquee: pause, scroll to reveal the end, pause, scroll back.
/// Returns a negative x-offset in pixels (0 when the text fits).
fn marquee_offset(time: f32, overflow: f32, scale: f32) -> f32 {
    if overflow <= 1.0 {
        return 0.0;
    }
    let speed = 34.0 * scale; // px/s
    let scroll_t = overflow / speed;
    let pause = 1.6;
    let period = 2.0 * (scroll_t + pause);
    let t = time.rem_euclid(period);
    if t < pause {
        0.0
    } else if t < pause + scroll_t {
        -(t - pause) * speed
    } else if t < 2.0 * pause + scroll_t {
        -overflow
    } else {
        -(overflow - (t - 2.0 * pause - scroll_t) * speed)
    }
}
