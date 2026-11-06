//! Shared data + geometry for the full dashboard (media · calendar · weather).

use crate::state::Layout;

#[derive(Clone, Debug)]
pub struct DashEvent {
    pub title: String,
    pub time: String,
    pub color: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct Weather {
    pub temp: i32,
    pub condition: String,
    pub city: String,
    pub high: i32,
    pub low: i32,
    /// WMO weather code, mapped to an emoji by `weather_emoji`.
    pub code: u8,
}

#[derive(Clone, Debug, Default)]
pub struct DashData {
    pub date_line: String,     // "Fri, 3 Apr"
    pub event_summary: String, // "1 event coming up"
    pub events: Vec<DashEvent>,
    pub weather: Option<Weather>,
    pub battery: Option<(bool, u8)>, // laptop (charging, percent)
    pub buds: Option<(String, u8)>,  // bluetooth earbuds (name, percent)
}

/// Emoji for a WMO weather code (rendered via the color-emoji font).
pub fn weather_emoji(code: u8) -> &'static str {
    match code {
        0 => "\u{2600}\u{FE0F}",       // ☀️ clear
        1 | 2 => "\u{26C5}",           // ⛅ partly cloudy
        3 => "\u{2601}\u{FE0F}",       // ☁️ overcast
        45 | 48 => "\u{1F32B}\u{FE0F}", // 🌫️ fog
        51..=57 | 61..=67 | 80..=82 => "\u{1F327}\u{FE0F}", // 🌧️ rain
        71..=77 | 85 | 86 => "\u{2744}\u{FE0F}",            // ❄️ snow
        95..=99 => "\u{26C8}\u{FE0F}", // ⛈️ thunder
        _ => "\u{2601}\u{FE0F}",
    }
}

/// Key boundaries of the dashboard panel in physical pixels.
pub struct DashFrame {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub content_top: f32,
    pub div1: f32,
    pub div2: f32,
    pub pad: f32,
    pub s: f32,
    pub bar_h: f32,
}

/// Media-panel sub-layout, shared by the quad, text, and click-hit-test code.
pub struct MediaLayout {
    pub art_center: [f32; 2],
    pub art_half: f32,
    pub title_pos: [f32; 2],
    pub artist_pos: [f32; 2],
    pub text_w: f32,
    pub ctrl: [[f32; 2]; 3],
    pub ctrl_size: f32,
    pub prog_left: f32,
    pub prog_right: f32,
    pub prog_y: f32,
    pub time_y: f32,
}

pub fn media_layout(layout: &Layout) -> MediaLayout {
    let f = frame(layout);
    let s = f.s;
    let m_left = f.left + f.pad;
    let m_right = f.div1 - f.pad * 0.7;
    let m_top = f.content_top + 2.0 * s;
    let m_bottom = f.bottom - f.pad;

    let title_pos = [m_left, m_top];
    let artist_pos = [m_left, m_top + 22.0 * s];
    let text_w = m_right - m_left;

    let prog_y = m_bottom - 10.0 * s;
    let time_y = prog_y + 6.0 * s;

    let art_top = m_top + 44.0 * s;
    let art_bottom = prog_y - 14.0 * s;
    let art_half = ((art_bottom - art_top) * 0.5).max(14.0 * s);
    let art_center = [m_left + art_half, (art_top + art_bottom) * 0.5];
    let h = art_bottom - art_top; // height of the right-of-art column

    // Controls centered in the right-of-art column.
    let rx_left = art_center[0] + art_half + 14.0 * s;
    let rx_right = m_right;
    let rx_cx = (rx_left + rx_right) * 0.5;
    let ctrl_cy = art_top + h * 0.5;
    let gap = 34.0 * s;

    MediaLayout {
        art_center,
        art_half,
        title_pos,
        artist_pos,
        text_w,
        ctrl: [
            [rx_cx - gap, ctrl_cy],
            [rx_cx, ctrl_cy],
            [rx_cx + gap, ctrl_cy],
        ],
        ctrl_size: 17.0 * s,
        prog_left: m_left,
        prog_right: m_right,
        prog_y,
        time_y,
    }
}

pub struct EventRow {
    pub dot: [f32; 2],
    pub title: [f32; 2],
    pub time: [f32; 2],
}

pub struct CalLayout {
    pub date_pos: [f32; 2],
    pub summary_pos: [f32; 2],
    pub rows: Vec<EventRow>,
    pub col_w: f32,
}

pub fn cal_layout(layout: &Layout, n_events: usize) -> CalLayout {
    let f = frame(layout);
    let s = f.s;
    let left = f.div1 + f.pad * 0.9;
    let top = f.content_top + 2.0 * s;
    let date_pos = [left, top];
    let summary_pos = [left, top + 24.0 * s];
    let mut rows = Vec::new();
    for i in 0..n_events {
        let y = top + 54.0 * s + i as f32 * 36.0 * s;
        rows.push(EventRow {
            dot: [left + 5.0 * s, y + 9.0 * s],
            title: [left + 18.0 * s, y],
            time: [left + 18.0 * s, y + 16.0 * s],
        });
    }
    CalLayout {
        date_pos,
        summary_pos,
        rows,
        col_w: f.div2 - f.pad * 0.6 - left,
    }
}

pub struct WeatherLayout {
    pub icon_pos: [f32; 2],
    pub temp_pos: [f32; 2],
    pub cond_pos: [f32; 2],
    pub city_pos: [f32; 2],
    pub hi_dot: [f32; 2],
    pub hi_label: [f32; 2],
    pub hi_val: [f32; 2],
    pub lo_dot: [f32; 2],
    pub lo_label: [f32; 2],
    pub lo_val: [f32; 2],
}

pub fn weather_layout(layout: &Layout) -> WeatherLayout {
    let f = frame(layout);
    let s = f.s;
    let left = f.div2 + f.pad * 0.9;
    let right = f.right - f.pad;
    let top = f.content_top + 4.0 * s;
    let mid = left + (right - left) * 0.5;
    WeatherLayout {
        icon_pos: [left, top],
        temp_pos: [left + 42.0 * s, top - 4.0 * s],
        cond_pos: [left, top + 40.0 * s],
        city_pos: [left, top + 66.0 * s],
        hi_dot: [left + 6.0 * s, top + 104.0 * s],
        hi_label: [left + 20.0 * s, top + 96.0 * s],
        hi_val: [left + 20.0 * s, top + 112.0 * s],
        lo_dot: [mid + 6.0 * s, top + 104.0 * s],
        lo_label: [mid + 20.0 * s, top + 96.0 * s],
        lo_val: [mid + 20.0 * s, top + 112.0 * s],
    }
}

/// Top icon-bar glyph centers (left group, right group).
pub struct BarLayout {
    pub left_icons: Vec<[f32; 2]>,
    pub right_icons: Vec<[f32; 2]>,
    pub icon_size: f32,
    pub cy: f32,
}

pub fn bar_layout(layout: &Layout, n_left: usize, n_right: usize) -> BarLayout {
    let f = frame(layout);
    let s = f.s;
    let cy = f.top + f.bar_h * 0.5 + 2.0 * s;
    let gap = 40.0 * s;
    let mut left_icons = Vec::new();
    for i in 0..n_left {
        left_icons.push([f.left + f.pad + 14.0 * s + i as f32 * gap, cy]);
    }
    let mut right_icons = Vec::new();
    for i in 0..n_right {
        let x = f.right - f.pad - 14.0 * s - (n_right - 1 - i) as f32 * gap;
        right_icons.push([x, cy]);
    }
    BarLayout {
        left_icons,
        right_icons,
        icon_size: 15.0 * s,
        cy,
    }
}

pub fn frame(layout: &Layout) -> DashFrame {
    let s = layout.scale.max(0.5);
    let [cx, cy] = layout.center;
    let [hw, hh] = layout.half;
    let pad = 20.0 * s;
    let left = cx - hw;
    let right = cx + hw;
    let top = cy - hh;
    let bottom = cy + hh;
    let bar_h = 42.0 * s;
    let content_top = top + bar_h + 8.0 * s;

    let inner_left = left + pad;
    let inner_right = right - pad;
    let total_w = inner_right - inner_left;
    let div1 = inner_left + total_w * 0.40;
    let div2 = div1 + total_w * 0.31;

    DashFrame {
        left,
        right,
        top,
        bottom,
        content_top,
        div1,
        div2,
        pad,
        s,
        bar_h,
    }
}
