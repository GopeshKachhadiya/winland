//! Google Calendar via OAuth 2.0 (desktop/installed-app flow).
//!
//! The user creates a "Desktop app" OAuth client in Google Cloud Console and
//! pastes the client id/secret into `%APPDATA%\WinLand\google.json`. Clicking
//! "Connect Google Calendar" in the tray runs the consent flow once (browser +
//! loopback redirect), stores the refresh token, and from then on we fetch the
//! real upcoming events (recurring instances expanded by the API).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::SystemInformation::GetSystemTime;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::dashboard::DashEvent;

const SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Default, Serialize, Deserialize)]
struct Config {
    client_id: String,
    client_secret: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn config_path() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("WinLand").join("google.json")
}

/// Create the config folder + template if missing; return the path.
pub fn ensure_config() -> PathBuf {
    let path = config_path();
    if !path.exists() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmpl = Config::default();
        if let Ok(s) = serde_json::to_string_pretty(&tmpl) {
            let _ = std::fs::write(&path, s);
        }
    }
    path
}

fn load_config() -> Option<Config> {
    let s = std::fs::read_to_string(config_path()).ok()?;
    serde_json::from_str(&s).ok()
}

fn save_config(cfg: &Config) {
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(config_path(), s);
    }
}

/// On startup: if connected, fetch; if credentials are present but we're not
/// connected yet, run the consent flow automatically (so pasting creds + a
/// restart is all it takes).
pub fn start(tx: Sender<Vec<DashEvent>>) {
    thread::Builder::new()
        .name("winland-gcal".into())
        .spawn(move || {
            let Some(mut cfg) = load_config() else {
                log::warn!("gcal: no config at {}", config_path().display());
                return;
            };
            if cfg.client_id.is_empty() {
                log::info!(
                    "gcal: paste client_id/secret into {} then restart",
                    config_path().display()
                );
                return;
            }
            if cfg.refresh_token.is_none() {
                log::info!("gcal: credentials present, starting OAuth consent flow");
                match oauth_flow(&cfg) {
                    Some(refresh) => {
                        cfg.refresh_token = Some(refresh);
                        save_config(&cfg);
                        log::info!("gcal: connected");
                    }
                    None => {
                        log::warn!("gcal: OAuth flow failed");
                        return;
                    }
                }
            }
            fetch_loop(cfg, tx);
        })
        .ok();
}

/// Triggered from the tray: run the OAuth consent flow, then fetch events.
pub fn connect(tx: Sender<Vec<DashEvent>>) {
    thread::Builder::new()
        .name("winland-gcal-connect".into())
        .spawn(move || {
            let mut cfg = match load_config() {
                Some(c) if !c.client_id.is_empty() => c,
                _ => {
                    log::warn!(
                        "google.json missing client_id/secret — fill {} first",
                        config_path().display()
                    );
                    return;
                }
            };
            match oauth_flow(&cfg) {
                Some(refresh) => {
                    cfg.refresh_token = Some(refresh);
                    save_config(&cfg);
                    log::info!("Google Calendar connected");
                    fetch_loop(cfg, tx);
                }
                None => log::warn!("Google Calendar OAuth flow failed"),
            }
        })
        .ok();
}

fn fetch_loop(cfg: Config, tx: Sender<Vec<DashEvent>>) {
    let Some(refresh) = cfg.refresh_token.clone() else { return };
    loop {
        match refresh_access_token(&cfg, &refresh) {
            Some(access) => match fetch_events(&access) {
                Some(events) => {
                    log::info!("gcal: fetched {} upcoming events", events.len());
                    if tx.send(events).is_err() {
                        return;
                    }
                }
                None => log::warn!("gcal: fetch_events failed (check Calendar API enabled)"),
            },
            None => log::warn!("gcal: access-token refresh failed (check client id/secret)"),
        }
        thread::sleep(Duration::from_secs(600)); // refresh every 10 min
    }
}

/// Run the loopback OAuth consent flow; returns the refresh token.
fn oauth_flow(cfg: &Config) -> Option<String> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    let redirect = format!("http://127.0.0.1:{port}");

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        enc(&cfg.client_id),
        enc(&redirect),
        enc(SCOPE),
    );
    open_url(&auth_url);

    // Wait for Google to redirect back with ?code=...
    let (stream, _) = listener.accept().ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    // "GET /?code=4/0A...&scope=... HTTP/1.1"
    let code_enc = line.split("code=").nth(1)?.split(['&', ' ']).next()?;
    let code = dec(code_enc);

    // Respond so the browser tab shows a friendly message.
    let body = "<html><body style='font-family:Segoe UI,sans-serif;background:#111;color:#eee;text-align:center;padding-top:60px'><h2>WinLand is connected to Google Calendar.</h2><p>You can close this tab.</p></body></html>";
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let mut s = stream;
    let _ = s.write_all(resp.as_bytes());

    // Exchange the code for tokens.
    let resp = ureq::post(TOKEN_URL)
        .send_form(&[
            ("code", code.as_str()),
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .ok()?
        .into_string()
        .ok()?;
    let j: serde_json::Value = serde_json::from_str(&resp).ok()?;
    j["refresh_token"].as_str().map(|s| s.to_string())
}

fn refresh_access_token(cfg: &Config, refresh: &str) -> Option<String> {
    let resp = ureq::post(TOKEN_URL)
        .send_form(&[
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ])
        .ok()?
        .into_string()
        .ok()?;
    let j: serde_json::Value = serde_json::from_str(&resp).ok()?;
    j["access_token"].as_str().map(|s| s.to_string())
}

fn fetch_events(access: &str) -> Option<Vec<DashEvent>> {
    let now = utc_now_rfc3339();
    let url = format!(
        "https://www.googleapis.com/calendar/v3/calendars/primary/events?singleEvents=true&orderBy=startTime&maxResults=8&timeMin={}",
        enc(&now)
    );
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {access}"))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let j: serde_json::Value = serde_json::from_str(&resp).ok()?;
    let items = j["items"].as_array()?;

    let today = local_today();
    const PALETTE: [[f32; 4]; 4] = [
        [0.40, 0.55, 1.0, 1.0],
        [0.65, 0.40, 0.95, 1.0],
        [0.95, 0.55, 0.25, 1.0],
        [0.30, 0.80, 0.45, 1.0],
    ];

    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let title = it["summary"].as_str().unwrap_or("(no title)").to_string();
        let start = &it["start"];
        let end = &it["end"];
        let time = format_when(start, end, &today);
        out.push(DashEvent {
            title,
            time,
            color: PALETTE[i % PALETTE.len()],
        });
        if out.len() >= 3 {
            break;
        }
    }
    Some(out)
}

/// Human-friendly time string for an event, relative to today.
fn format_when(start: &serde_json::Value, end: &serde_json::Value, today: &str) -> String {
    const MONTHS: [&str; 13] = [
        "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date_label = |date: &str| -> String {
        // date = YYYY-MM-DD
        let m: usize = date.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(0);
        let d: u32 = date.get(8..10).and_then(|s| s.parse().ok()).unwrap_or(0);
        format!("{} {}", MONTHS.get(m).copied().unwrap_or(""), d)
    };

    if let Some(dt) = start["dateTime"].as_str() {
        let date = &dt.get(0..10).unwrap_or("");
        let t1 = to12(dt.get(11..16).unwrap_or(""));
        let t2 = end["dateTime"]
            .as_str()
            .and_then(|e| e.get(11..16).map(to12))
            .unwrap_or_default();
        if *date == today {
            if t2.is_empty() {
                t1
            } else {
                format!("{t1} - {t2}")
            }
        } else {
            format!("{} \u{00B7} {t1}", date_label(date))
        }
    } else if let Some(d) = start["date"].as_str() {
        if d == today {
            "Today \u{00B7} All day".into()
        } else {
            format!("{} \u{00B7} All day", date_label(d))
        }
    } else {
        String::new()
    }
}

fn to12(hhmm: &str) -> String {
    let h: u32 = hhmm.get(0..2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let m = hhmm.get(3..5).unwrap_or("00");
    let (h12, ap) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{m} {ap}")
}

fn utc_now_rfc3339() -> String {
    let t = unsafe { GetSystemTime() };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

fn local_today() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

fn open_url(url: &str) {
    let file = HSTRING::from(url);
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

/// Percent-encode a query value.
fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

/// Percent-decode.
fn dec(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
