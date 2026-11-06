//! System-tray icon + menu (the only way to quit a taskbar-less overlay).

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub enum TrayAction {
    Quit,
    SetAutostart(bool),
    ConnectCalendar,
}

pub struct Tray {
    _tray: TrayIcon,
    quit_id: MenuId,
    connect_cal_id: MenuId,
    autostart: CheckMenuItem,
}

impl Tray {
    pub fn new() -> Option<Self> {
        let menu = Menu::new();
        let title = MenuItem::new("WinLand — Dynamic Island", false, None);
        let connect_cal = MenuItem::new("Connect Google Calendar", true, None);
        let autostart =
            CheckMenuItem::new("Start at login", true, crate::autostart::is_enabled(), None);
        let quit = MenuItem::new("Quit WinLand", true, None);

        menu.append(&title).ok()?;
        menu.append(&PredefinedMenuItem::separator()).ok()?;
        menu.append(&connect_cal).ok()?;
        menu.append(&autostart).ok()?;
        menu.append(&PredefinedMenuItem::separator()).ok()?;
        menu.append(&quit).ok()?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("WinLand — Dynamic Island")
            .with_icon(make_icon())
            .build()
            .ok()?;

        Some(Self {
            _tray: tray,
            quit_id: quit.id().clone(),
            connect_cal_id: connect_cal.id().clone(),
            autostart,
        })
    }

    /// Drain tray menu events; returns an action if one fired.
    pub fn poll(&self) -> Option<TrayAction> {
        if let Ok(ev) = MenuEvent::receiver().try_recv() {
            if ev.id == self.quit_id {
                return Some(TrayAction::Quit);
            }
            if ev.id == self.connect_cal_id {
                return Some(TrayAction::ConnectCalendar);
            }
            if ev.id == *self.autostart.id() {
                return Some(TrayAction::SetAutostart(self.autostart.is_checked()));
            }
        }
        None
    }
}

/// A 32×32 white rounded-pill icon (the island silhouette) on transparency.
fn make_icon() -> Icon {
    let (w, h) = (32u32, 32u32);
    let mut data = vec![0u8; (w * h * 4) as usize];
    let (cx, cy, hw, hh, r) = (16.0f32, 16.0f32, 13.0f32, 6.0f32, 6.0f32);
    for y in 0..h {
        for x in 0..w {
            let px = (x as f32 + 0.5 - cx).abs() - hw + r;
            let py = (y as f32 + 0.5 - cy).abs() - hh + r;
            let d = px.max(0.0).hypot(py.max(0.0)) + px.max(py).min(0.0) - r;
            let a = (0.5 - d).clamp(0.0, 1.0);
            let i = ((y * w + x) * 4) as usize;
            data[i] = 255;
            data[i + 1] = 255;
            data[i + 2] = 255;
            data[i + 3] = (a * 255.0) as u8;
        }
    }
    Icon::from_rgba(data, w, h).expect("tray icon")
}
