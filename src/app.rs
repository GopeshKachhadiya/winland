//! Window lifecycle and the winit event loop.

use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_LWIN,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNORMAL,
    WS_BORDER, WS_CAPTION, WS_DLGFRAME, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};

use crate::audio::{Audio, BANDS};
use crate::dashboard::{DashData, DashEvent, Weather};
use crate::gfx::Renderer;
use crate::media::{self, ArtImage, MediaCommand, MediaUpdate};
use crate::state::{Island, Mode, NowPlaying, Transient};
use crate::system::{self, SysEvent};
use crate::tray::{Tray, TrayAction};
use crate::autostart;

/// Logical size of the transparent canvas. Wide and tall enough to hold the
/// fully expanded island plus spring overshoot and drop shadow without clipping.
pub const CANVAS_W: f64 = 1000.0;
pub const CANVAS_H: f64 = 440.0;

pub struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    island: Island,
    now_playing: Option<NowPlaying>,
    media_rx: Option<Receiver<MediaUpdate>>,
    media_tx: Option<std::sync::mpsc::Sender<MediaCommand>>,
    sys_rx: Option<Receiver<SysEvent>>,
    transient: Option<(Transient, Instant)>,
    tray: Option<Tray>,
    /// Pending album-art change to apply to the renderer: Some(Some)=set,
    /// Some(None)=clear, None=nothing.
    pending_art: Option<Option<ArtImage>>,
    hovered: bool,
    animating: bool,
    /// Last island rect in screen pixels (left, top, right, bottom) for hit-testing.
    last_rect: (f32, f32, f32, f32),
    /// Transport button centers in screen pixels + glyph size, for click routing.
    last_buttons: Option<([[f32; 2]; 3], f32)>,
    /// Whether the full dashboard is open (toggled by click while hovered).
    dashboard_open: bool,
    /// Top-bar icon screen centers + glyph size, for click routing.
    last_bar: Option<([[f32; 2]; 8], f32)>,
    /// Latest weather, updated by the weather thread.
    weather: Option<Weather>,
    weather_rx: Option<Receiver<Weather>>,
    /// Latest earbuds battery, updated by the bluetooth thread.
    buds: Option<(String, u8)>,
    buds_rx: Option<Receiver<crate::bluetooth::Buds>>,
    /// Real Google Calendar events (None until the fetcher reports; replaces samples).
    cal_events: Option<Vec<DashEvent>>,
    cal_tx: Option<std::sync::mpsc::Sender<Vec<DashEvent>>>,
    cal_rx: Option<Receiver<Vec<DashEvent>>>,
    /// System-audio spectrum source + smoothed visualizer bar heights.
    audio: Option<Audio>,
    viz: [f32; BANDS],
    /// Animated island-background state (phase frozen on pause; colours from art).
    bg_phase: f32,
    bg_intensity: f32,
    bg_color0: [f32; 4],
    bg_color1: [f32; 4],
    forced: bool,
    start: Option<Instant>,
    last_frame: Option<Instant>,
    // Temporary self-demo so the morph can be exercised without system input.
    demo: bool,
    demo_accum: f32,
    demo_idx: usize,
}

impl Default for App {
    fn default() -> Self {
        Self {
            window: None,
            renderer: None,
            island: Island::new(),
            now_playing: None,
            media_rx: None,
            media_tx: None,
            sys_rx: None,
            transient: None,
            tray: None,
            pending_art: None,
            hovered: false,
            animating: true,
            last_rect: (0.0, 0.0, 0.0, 0.0),
            last_buttons: None,
            dashboard_open: false,
            last_bar: None,
            weather: None,
            weather_rx: None,
            buds: None,
            buds_rx: None,
            cal_events: None,
            cal_tx: None,
            cal_rx: None,
            audio: None,
            viz: [0.0; BANDS],
            bg_phase: 0.0,
            bg_intensity: 0.35,
            bg_color0: [0.40, 0.20, 0.70, 1.0],
            bg_color1: [0.20, 0.55, 0.80, 1.0],
            forced: false,
            start: None,
            last_frame: None,
            demo: std::env::var("WINLAND_DEMO").is_ok(),
            demo_accum: 0.0,
            demo_idx: 0,
        }
    }
}

impl App {
    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        use winit::platform::windows::WindowAttributesExtWindows;

        let attrs = WindowAttributes::default()
            .with_title("WinLand")
            .with_inner_size(LogicalSize::new(CANVAS_W, CANVAS_H))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_active(false)
            .with_visible(false)
            .with_skip_taskbar(true)
            .with_undecorated_shadow(false);

        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        // Force the borderless overlay styles via Win32, in case the decorated
        // frame ever slips back in (title bar / resize border).
        enforce_overlay_styles(&window);

        // Click-through everywhere for now; interaction hit-testing comes later.
        let _ = window.set_cursor_hittest(false);

        // Pin to top-center of the primary monitor, flush to the top edge.
        if let Some(mon) = event_loop.primary_monitor() {
            let mpos = mon.position();
            let msize = mon.size();
            let outer = window.outer_size();
            let x = mpos.x + ((msize.width as i32 - outer.width as i32) / 2);
            let y = mpos.y;
            window.set_outer_position(PhysicalPosition::new(x, y));
        }

        let renderer = Renderer::new(window.clone());

        window.set_visible(true);
        self.window = Some(window);
        self.renderer = Some(renderer);
        let now = Instant::now();
        self.start = Some(now);
        self.last_frame = Some(now);

        // Optional fixed-mode override for visual verification.
        if let Ok(m) = std::env::var("WINLAND_MODE") {
            let mode = match m.as_str() {
                "compact" => Mode::Compact,
                "expanded" => Mode::Expanded,
                _ => Mode::Idle,
            };
            self.island.set_mode(mode);
            if mode != Mode::Idle {
                self.now_playing = Some(NowPlaying {
                    title: "Midnight City".into(),
                    artist: "M83".into(),
                    progress: 0.42,
                    playing: true,
                    position: 101.0,
                    duration: 240.0,
                });
            }
            self.demo = false;
            self.forced = true;
        }

        // Start the real now-playing reader and system-indicator reader unless
        // we're in a canned visual-test mode.
        if !self.demo && !self.forced {
            let (rx, tx) = media::start();
            self.media_rx = Some(rx);
            self.media_tx = Some(tx);
            self.sys_rx = Some(system::start());
            self.weather_rx = Some(crate::weather::start());
            self.buds_rx = Some(crate::bluetooth::start());

            // Google Calendar: ensure the config template exists, then start the
            // fetcher (it only runs if already connected).
            crate::gcal::ensure_config();
            let (ctx, crx) = std::sync::mpsc::channel();
            crate::gcal::start(ctx.clone());
            self.cal_tx = Some(ctx);
            self.cal_rx = Some(crx);

            self.audio = Some(Audio::start());

            self.tray = Tray::new();
        }

        // Preview a transient indicator for screenshots: WINLAND_TEST_INDICATOR=volume|battery.
        if let Ok(v) = std::env::var("WINLAND_TEST_INDICATOR") {
            let t = match v.as_str() {
                "battery" => Transient::Battery {
                    charging: true,
                    percent: 78,
                },
                "device" => Transient::Device {
                    name: "AirPods Pro".into(),
                },
                _ => Transient::Volume {
                    level: 0.6,
                    muted: false,
                },
            };
            self.transient = Some((t, Instant::now() + Duration::from_secs(600)));
        }
    }

    /// Drain system events into a transient indicator, and expire old ones.
    fn pump_system(&mut self) {
        let mut events = Vec::new();
        if let Some(rx) = self.sys_rx.as_ref() {
            while let Ok(e) = rx.try_recv() {
                events.push(e);
            }
        }
        for e in events {
            let (t, secs) = match e {
                SysEvent::Volume { level, muted } => (Transient::Volume { level, muted }, 1.3),
                SysEvent::Power { charging, percent } => {
                    (Transient::Battery { charging, percent }, 2.2)
                }
                SysEvent::Device { name } => (Transient::Device { name }, 2.6),
            };
            self.transient = Some((t, Instant::now() + Duration::from_secs_f32(secs)));
        }
        if let Some((_, exp)) = self.transient {
            if Instant::now() >= exp {
                self.transient = None;
            }
        }
    }

    /// Pick the island mode from current state: a transient indicator wins,
    /// then now-playing (expanded on hover), otherwise idle.
    fn update_mode(&mut self) {
        if self.demo || self.forced {
            return;
        }
        let mode = if self.transient.is_some() && !self.dashboard_open {
            Mode::Compact
        } else if self.dashboard_open && self.hovered {
            Mode::Dashboard
        } else if self.now_playing.is_some() {
            if self.hovered {
                Mode::Expanded
            } else {
                Mode::Compact
            }
        } else {
            Mode::Idle
        };
        self.island.set_mode(mode);
    }

    /// Assemble the dashboard data (real date + sample events + live weather).
    fn build_dash_data(&self) -> DashData {
        let t = unsafe { GetLocalTime() };
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MONTHS: [&str; 13] = [
            "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let day = DAYS.get(t.wDayOfWeek as usize).copied().unwrap_or("");
        let month = MONTHS.get(t.wMonth as usize).copied().unwrap_or("");
        let date_line = format!("{}, {} {}", day, t.wDay, month);

        // Real Google Calendar events once connected; otherwise a hint.
        let (events, summary) = match self.cal_events.as_ref() {
            Some(ev) if !ev.is_empty() => {
                let s = if ev.len() == 1 {
                    "1 event coming up".to_string()
                } else {
                    format!("{} events coming up", ev.len())
                };
                (ev.clone(), s)
            }
            Some(_) => (Vec::new(), "No upcoming events".to_string()),
            None => (Vec::new(), "Connect in tray \u{2192} Google Calendar".to_string()),
        };

        DashData {
            date_line,
            event_summary: summary,
            events,
            weather: self.weather.clone(),
            battery: read_power(),
            buds: self.buds.clone(),
        }
    }

    /// Hover detection: poll the global cursor against the island's screen rect.
    /// Inside → interactive + expanded; outside → click-through + collapsed.
    fn update_hover(&mut self) {
        if self.demo || self.forced {
            return;
        }
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        let (l, t, r, b) = self.last_rect;
        let inside = (pt.x as f32) >= l
            && (pt.x as f32) <= r
            && (pt.y as f32) >= t
            && (pt.y as f32) <= b;

        if inside != self.hovered {
            self.hovered = inside;
            if !inside {
                // Leaving the island closes the dashboard.
                self.dashboard_open = false;
            }
            if let Some(w) = self.window.as_ref() {
                let _ = w.set_cursor_hittest(inside);
            }
        }
    }

    /// Which transport command (if any) the cursor is over right now.
    fn button_at_cursor(&self) -> Option<MediaCommand> {
        let (centers, size) = self.last_buttons?;
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return None;
        }
        let r = size * 1.4;
        let hit = |c: [f32; 2]| (pt.x as f32 - c[0]).abs() <= r && (pt.y as f32 - c[1]).abs() <= r;
        if hit(centers[0]) {
            Some(MediaCommand::Previous)
        } else if hit(centers[1]) {
            Some(MediaCommand::TogglePlayPause)
        } else if hit(centers[2]) {
            Some(MediaCommand::Next)
        } else {
            None
        }
    }

    /// Which top-bar icon (0..8) the cursor is over, if any.
    fn bar_icon_at_cursor(&self) -> Option<usize> {
        let (centers, size) = self.last_bar?;
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return None;
        }
        let r = size * 1.6;
        centers.iter().position(|c| {
            (pt.x as f32 - c[0]).abs() <= r && (pt.y as f32 - c[1]).abs() <= r
        })
    }

    /// Run the action for a top-bar icon — all via direct Win32 APIs (no child
    /// process, no console flash): `ShellExecuteW` opens the protocol/app
    /// handler straight away, exactly like a native click.
    fn run_bar_action(&self, i: usize) {
        match i {
            0 => send_win_combo(0x44), // Home -> show desktop (Win+D)
            1 => open_uri("mailto:"),  // Mail -> default mail app
            2 => send_win_combo(0x56), // Copy -> clipboard history (Win+V)
            3 => open_uri("ms-settings:display"), // Brightness -> display settings
            4 => unsafe {
                let _ = LockWorkStation();
            },
            5 => open_uri("ms-settings:batterysaver"), // Battery -> power settings
            6 => send_win_combo(0x4E),                 // Bell -> notifications (Win+N)
            7 => open_uri("ms-settings:"),             // Settings
            _ => {}
        }
    }

    /// Drain pending media updates and reflect them in the island.
    fn pump_media(&mut self) {
        let mut updates = Vec::new();
        if let Some(rx) = self.media_rx.as_ref() {
            while let Ok(u) = rx.try_recv() {
                updates.push(u);
            }
        }
        for u in updates {
            if u.has_session {
                let progress = if u.duration > 0.5 {
                    (u.position / u.duration).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                if let Some(art) = u.art {
                    let (c0, c1) = dominant_colors(&art);
                    self.bg_color0 = c0;
                    self.bg_color1 = c1;
                    self.pending_art = Some(Some(art));
                }
                self.now_playing = Some(NowPlaying {
                    title: u.title,
                    artist: u.artist,
                    progress,
                    playing: u.playing,
                    position: u.position,
                    duration: u.duration,
                });
            } else {
                self.now_playing = None;
                self.pending_art = Some(None);
                // Back to the default idle palette when nothing is playing.
                self.bg_color0 = [0.40, 0.20, 0.70, 1.0];
                self.bg_color1 = [0.20, 0.55, 0.80, 1.0];
            }
        }
    }

    fn frame(&mut self) {
        if self.window.is_none() || self.renderer.is_none() {
            return;
        }

        // Delta time since the previous frame.
        let now = Instant::now();
        let dt = self
            .last_frame
            .map(|t| (now - t).as_secs_f32())
            .unwrap_or(1.0 / 60.0);
        self.last_frame = Some(now);

        if self.demo {
            self.demo_accum += dt;
            if self.demo_accum >= 2.5 {
                self.demo_accum = 0.0;
                self.demo_idx = (self.demo_idx + 1) % 3;
                let mode = [Mode::Idle, Mode::Compact, Mode::Expanded][self.demo_idx];
                self.island.set_mode(mode);
                // Placeholder track while there's no real media source yet.
                self.now_playing = if mode == Mode::Idle {
                    None
                } else {
                    Some(NowPlaying {
                        title: "Midnight City".into(),
                        artist: "M83".into(),
                        progress: 0.0,
                        playing: true,
                        position: 0.0,
                        duration: 240.0,
                    })
                };
            }
        }

        // Real now-playing data (no-op in demo/forced modes).
        self.pump_media();
        // Volume / battery transient indicators.
        self.pump_system();
        // Hover-to-expand against last frame's island rect.
        self.update_hover();
        // Resolve the island mode from all the inputs above.
        self.update_mode();

        let time = self
            .start
            .map(|t| (now - t).as_secs_f32())
            .unwrap_or(0.0);

        // Only animate placeholder progress in the canned demo.
        if self.demo {
            if let Some(np) = self.now_playing.as_mut() {
                np.progress = (time * 0.04) % 1.0;
            }
        }

        let moving = self.island.tick(dt);
        // Keep rendering at full rate while anything is in motion or live.
        let playing = self.now_playing.as_ref().map(|n| n.playing).unwrap_or(false);
        self.animating = moving || self.transient.is_some() || self.hovered || playing;

        // Now borrow the window + renderer (all self-mutation is done).
        let window = self.window.as_ref().unwrap();
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let layout = self
            .island
            .layout(size.width as f32, size.height as f32, scale);

        // Record the island's screen rect (with a margin) for next-frame hover.
        let wpos = window
            .outer_position()
            .unwrap_or(PhysicalPosition::new(0, 0));
        let [cx, cy] = layout.center;
        let [hw, hh] = layout.half;
        let m = 12.0 * scale;
        self.last_rect = (
            wpos.x as f32 + cx - hw - m,
            wpos.y as f32 + cy - hh - m,
            wpos.x as f32 + cx + hw + m,
            wpos.y as f32 + cy + hh + m,
        );
        // Transport buttons: dashboard media controls when open, else the
        // now-playing controls.
        let button_centers = if layout.dashboard > 0.5 {
            Some((crate::dashboard::media_layout(&layout).ctrl, 17.0 * scale))
        } else {
            crate::state::transport_buttons(&layout).map(|b| (b.centers, b.size))
        };
        self.last_buttons = button_centers.map(|(centers, size)| {
            let c = centers.map(|p| [wpos.x as f32 + p[0], wpos.y as f32 + p[1]]);
            (c, size)
        });

        // Top-bar icon screen centers (home,mail,copy,sun,lock,battery,bell,settings).
        self.last_bar = if layout.dashboard > 0.5 {
            let bar = crate::dashboard::bar_layout(&layout, 4, 4);
            let mut c = [[0.0f32; 2]; 8];
            for i in 0..4 {
                c[i] = [wpos.x as f32 + bar.left_icons[i][0], wpos.y as f32 + bar.left_icons[i][1]];
                c[i + 4] = [wpos.x as f32 + bar.right_icons[i][0], wpos.y as f32 + bar.right_icons[i][1]];
            }
            Some((c, bar.icon_size))
        } else {
            None
        };

        // Drain weather + earbuds-battery updates.
        if let Some(rx) = self.weather_rx.as_ref() {
            while let Ok(w) = rx.try_recv() {
                self.weather = Some(w);
            }
        }
        let mut buds_updates = Vec::new();
        if let Some(rx) = self.buds_rx.as_ref() {
            while let Ok(b) = rx.try_recv() {
                buds_updates.push(b);
            }
        }
        for b in buds_updates {
            let disconnected = self.buds.is_some() && b.is_none();
            self.buds = b;
            // When the earbuds disconnect (put in case), pause playback so the
            // music doesn't keep going on the speakers.
            if disconnected {
                let playing = self.now_playing.as_ref().map(|n| n.playing).unwrap_or(false);
                if playing {
                    if let Some(tx) = self.media_tx.as_ref() {
                        let _ = tx.send(MediaCommand::TogglePlayPause);
                    }
                }
            }
        }
        if let Some(rx) = self.cal_rx.as_ref() {
            while let Ok(ev) = rx.try_recv() {
                self.cal_events = Some(ev);
            }
        }

        // Build dashboard data only when the panel is (even partly) open.
        let dash = if layout.dashboard > 0.01 {
            Some(self.build_dash_data())
        } else {
            None
        };

        let renderer = self.renderer.as_mut().unwrap();

        // Apply any pending album-art change before rendering.
        match self.pending_art.take() {
            Some(Some(art)) => renderer.set_album_art(&art.rgba, art.w, art.h),
            Some(None) => renderer.clear_album_art(),
            None => {}
        }

        // Update the visualizer: react to audio while playing, freeze when
        // paused, gentle idle wave when nothing is playing.
        let playing = self.now_playing.as_ref().map(|n| n.playing).unwrap_or(false);
        let has_song = self.now_playing.is_some();
        let target: [f32; BANDS] = if playing {
            self.audio.as_ref().map(|a| a.bands()).unwrap_or([0.0; BANDS])
        } else if has_song {
            self.viz // paused -> hold current (frozen)
        } else {
            let mut idle = [0.0f32; BANDS];
            for (i, v) in idle.iter_mut().enumerate() {
                *v = 0.12 + 0.14 * (0.5 + 0.5 * (time * 2.2 + i as f32 * 0.55).sin());
            }
            idle
        };
        for i in 0..BANDS {
            let k = if target[i] > self.viz[i] { 0.55 } else { 0.14 };
            self.viz[i] += (target[i] - self.viz[i]) * k;
        }

        // Animated island background: advance the phase only while playing (so
        // it freezes on pause) or drift slowly with no song; intensity tracks
        // the overall audio energy.
        let level = self.viz.iter().sum::<f32>() / BANDS as f32;
        if playing {
            self.bg_phase += dt * (0.8 + level * 3.2);
            let target_i = (0.45 + level * 0.9).min(1.0);
            self.bg_intensity += (target_i - self.bg_intensity) * 0.15;
        } else if has_song {
            // paused: hold phase + intensity (frozen).
        } else {
            self.bg_phase += dt * 0.25;
            self.bg_intensity += (0.35 - self.bg_intensity) * 0.05;
        }

        renderer.uniforms.bg_phase = self.bg_phase;
        renderer.uniforms.bg_intensity = self.bg_intensity;
        renderer.uniforms.bg_color0 = self.bg_color0;
        renderer.uniforms.bg_color1 = self.bg_color1;

        let np = self.now_playing.as_ref();
        let transient = self.transient.as_ref().map(|(t, _)| t);
        renderer.render(&layout, np, transient, dash.as_ref(), &self.viz, time);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.create_window(event_loop);
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            // Ignore OS close requests (Alt+F4, window close) — the island is a
            // permanent overlay. Quit only via the tray menu.
            WindowEvent::CloseRequested => {}
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
            }
            WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                // A click only reaches us when the cursor is over the island.
                // Over a transport button -> send that command; otherwise toggle
                // the full dashboard open/closed.
                if let Some(cmd) = self.button_at_cursor() {
                    if let Some(tx) = self.media_tx.as_ref() {
                        let _ = tx.send(cmd);
                    }
                } else if let Some(i) = self.bar_icon_at_cursor() {
                    self.run_bar_action(i);
                } else {
                    self.dashboard_open = !self.dashboard_open;
                }
            }
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(tray) = self.tray.as_ref() {
            match tray.poll() {
                Some(TrayAction::Quit) => {
                    event_loop.exit();
                    return;
                }
                Some(TrayAction::SetAutostart(enabled)) => autostart::set(enabled),
                Some(TrayAction::ConnectCalendar) => {
                    if let Some(tx) = self.cal_tx.clone() {
                        crate::gcal::connect(tx);
                    }
                }
                None => {}
            }
        }
        // Throttle the loop: 60 fps while animating, ~15 fps when idle (still
        // responsive to media/volume, far gentler on CPU and battery).
        let interval = if self.animating {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(66)
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + interval));

        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }
}

/// Open a URI / protocol handler (mailto:, ms-settings:, …) directly via the
/// shell — the native equivalent of a double-click, with no child process.
fn open_uri(uri: &str) {
    let file = HSTRING::from(uri);
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            &file,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Strip any window-frame styles and mark the window as a no-activate tool
/// window, so no title bar / resize border can appear.
fn enforce_overlay_styles(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(w) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(w.hwnd.get() as *mut _);
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let remove = WS_CAPTION.0
            | WS_THICKFRAME.0
            | WS_MINIMIZEBOX.0
            | WS_MAXIMIZEBOX.0
            | WS_SYSMENU.0
            | WS_BORDER.0
            | WS_DLGFRAME.0;
        let new_style = (style & !remove) | WS_POPUP.0;
        SetWindowLongPtrW(hwnd, GWL_STYLE, new_style as isize);

        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let new_ex = ex | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex as isize);

        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED | SWP_NOACTIVATE,
        );
    }
}

/// Press Win+<vk> (e.g. Win+D, Win+V, Win+N).
fn send_win_combo(vk: u16) {
    unsafe {
        keybd_event(VK_LWIN.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(vk as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(vk as u8, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_LWIN.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// Extract two background colours from album art: the average tone and the most
/// vibrant sample. The animated island background flows between them.
fn dominant_colors(art: &ArtImage) -> ([f32; 4], [f32; 4]) {
    let px = &art.rgba;
    let (mut sr, mut sg, mut sb, mut cnt) = (0u64, 0u64, 0u64, 0u64);
    let mut best_sat = -1.0f32;
    let mut vib = [0.4f32, 0.3, 0.7];
    let step = 4 * 7; // sample every 7th pixel
    let mut i = 0usize;
    while i + 3 < px.len() {
        let (r, g, b) = (
            px[i] as f32 / 255.0,
            px[i + 1] as f32 / 255.0,
            px[i + 2] as f32 / 255.0,
        );
        sr += px[i] as u64;
        sg += px[i + 1] as u64;
        sb += px[i + 2] as u64;
        cnt += 1;
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let sat = mx - mn;
        if sat > best_sat && mx > 0.25 {
            best_sat = sat;
            vib = [r, g, b];
        }
        i += step;
    }
    let avg = if cnt > 0 {
        [
            sr as f32 / cnt as f32 / 255.0,
            sg as f32 / cnt as f32 / 255.0,
            sb as f32 / cnt as f32 / 255.0,
        ]
    } else {
        [0.3, 0.3, 0.4]
    };
    ([avg[0], avg[1], avg[2], 1.0], [vib[0], vib[1], vib[2], 1.0])
}

/// Laptop battery (charging, percent), or None if there's no system battery.
fn read_power() -> Option<(bool, u8)> {
    let mut st = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut st) }.is_err() {
        return None;
    }
    if st.BatteryFlag == 128 {
        return None; // no system battery (desktop)
    }
    let charging = st.ACLineStatus == 1;
    let percent = if st.BatteryLifePercent == 255 {
        100
    } else {
        st.BatteryLifePercent
    };
    Some((charging, percent))
}
