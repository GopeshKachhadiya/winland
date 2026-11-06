//! Island state machine and geometry layout.
//!
//! Each mode maps to a target rounded-rectangle (half-width, half-height,
//! corner radius). Springs chase those targets, and because the SDF shader is
//! driven entirely by those few numbers, the transition *is* the fluid morph.

use crate::anim::Spring;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Bare pill, nothing playing.
    Idle,
    /// Now-playing pill: album art on the leading edge, visualizer trailing.
    Compact,
    /// Full now-playing panel with art, text, progress and controls.
    Expanded,
    /// Full multi-panel dashboard (media + calendar + weather + icon bar).
    Dashboard,
}

/// Target shape for a mode, in *logical* pixels (DPI applied at read time).
struct Dims {
    hw: f32,
    hh: f32,
    radius: f32,
}

impl Mode {
    fn dims(self) -> Dims {
        match self {
            Mode::Idle => Dims { hw: 72.0, hh: 17.0, radius: 17.0 },
            Mode::Compact => Dims { hw: 122.0, hh: 18.0, radius: 18.0 },
            Mode::Expanded => Dims { hw: 180.0, hh: 78.0, radius: 38.0 },
            Mode::Dashboard => Dims { hw: 430.0, hh: 128.0, radius: 42.0 },
        }
    }

    /// How "opened up" the now-playing content is (0 = pill, 1 = panel).
    fn expand(self) -> f32 {
        match self {
            Mode::Expanded => 1.0,
            _ => 0.0,
        }
    }

    /// How "opened up" the full dashboard is (0 = not, 1 = full).
    fn dashboard(self) -> f32 {
        match self {
            Mode::Dashboard => 1.0,
            _ => 0.0,
        }
    }

    /// How much now-playing content is present (0 = none, 1 = full).
    fn content(self) -> f32 {
        match self {
            Mode::Idle => 0.0,
            _ => 1.0,
        }
    }
}

/// Vertical gap from the very top edge, logical px.
const TOP_MARGIN: f32 = 5.0;

/// A transient system indicator shown briefly in the island.
#[derive(Clone, Debug)]
pub enum Transient {
    Volume { level: f32, muted: bool },
    Battery { charging: bool, percent: u8 },
    Device { name: String },
}

/// Current now-playing data driving the content layer.
#[derive(Clone, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    /// Playback progress 0..1.
    pub progress: f32,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
}

/// Resolved, DPI-applied geometry handed to the renderer each frame.
pub struct Layout {
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub radius: f32,
    /// Now-playing panel-openness 0..1 (content layer fades/scales with this).
    pub expand: f32,
    /// Full-dashboard openness 0..1.
    pub dashboard: f32,
    /// Now-playing content presence 0..1.
    pub content: f32,
    /// DPI scale factor, so the content layer can size itself.
    pub scale: f32,
}

/// Screen-local centers of the three transport buttons (prev, play/pause, next)
/// plus their glyph size — shared by the text layer (rendering) and the app
/// (click hit-testing) so they always agree.
pub struct Buttons {
    pub centers: [[f32; 2]; 3],
    pub size: f32,
}

/// Transport button layout for the expanded panel, or None when too collapsed.
pub fn transport_buttons(layout: &Layout) -> Option<Buttons> {
    if layout.expand < 0.5 {
        return None;
    }
    let s = layout.scale.max(0.5);
    let [cx, cy] = layout.center;
    let [hw, hh] = layout.half;
    let pad = 10.0 * s;
    let art_half = (hh - pad).max(14.0 * s);
    let area_left = cx - hw + art_half * 2.0 + pad * 2.0;
    let area_right = cx + hw - pad;
    let area_cx = (area_left + area_right) * 0.5;
    let cy_btn = cy + hh * 0.24;
    let gap = 32.0 * s;
    Some(Buttons {
        centers: [
            [area_cx - gap, cy_btn],
            [area_cx, cy_btn],
            [area_cx + gap, cy_btn],
        ],
        size: 16.0 * s,
    })
}

pub struct Island {
    pub mode: Mode,
    hw: Spring,
    hh: Spring,
    radius: Spring,
    expand: Spring,
    dashboard: Spring,
    content: Spring,
}

impl Island {
    pub fn new() -> Self {
        let d = Mode::Idle.dims();
        // SwiftUI's fluid-island feel: response 0.5s, damping 0.825.
        let mk = |v: f32| Spring::new(v, 0.5, 0.825);
        Self {
            mode: Mode::Idle,
            hw: mk(d.hw),
            hh: mk(d.hh),
            radius: mk(d.radius),
            expand: Spring::new(0.0, 0.45, 0.85),
            dashboard: Spring::new(0.0, 0.45, 0.9),
            content: Spring::new(0.0, 0.4, 0.9),
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        let d = mode.dims();
        self.hw.set_target(d.hw);
        self.hh.set_target(d.hh);
        self.radius.set_target(d.radius);
        self.expand.set_target(mode.expand());
        self.dashboard.set_target(mode.dashboard());
        self.content.set_target(mode.content());
    }

    /// Advance the animation; returns true while any spring is still moving.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut moving = false;
        moving |= self.hw.update(dt);
        moving |= self.hh.update(dt);
        moving |= self.radius.update(dt);
        moving |= self.expand.update(dt);
        moving |= self.dashboard.update(dt);
        moving |= self.content.update(dt);
        moving
    }

    /// Resolve current geometry in physical pixels for a given surface size
    /// and DPI scale factor.
    pub fn layout(&self, res_w: f32, _res_h: f32, scale: f32) -> Layout {
        let hw = self.hw.value * scale;
        let hh = self.hh.value * scale;
        let radius = self.radius.value.min(self.hh.value) * scale;
        let cx = res_w * 0.5;
        let cy = (TOP_MARGIN + self.hh.value) * scale;
        Layout {
            center: [cx, cy],
            half: [hw, hh],
            radius,
            expand: self.expand.value,
            dashboard: self.dashboard.value,
            content: self.content.value,
            scale,
        }
    }
}
