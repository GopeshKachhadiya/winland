//! Run-at-login via the HKCU `...\CurrentVersion\Run` registry key.

use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
use winreg::RegKey;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const NAME: &str = "WinLand";

fn exe_command() -> Option<String> {
    let p = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", p.to_string_lossy()))
}

pub fn is_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN)
        .and_then(|k| k.get_value::<String, _>(NAME))
        .is_ok()
}

pub fn enable() {
    if let Some(cmd) = exe_command() {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok((key, _)) = hkcu.create_subkey(RUN) {
            if let Err(e) = key.set_value(NAME, &cmd) {
                log::warn!("autostart enable failed: {e}");
            }
        }
    }
}

pub fn disable() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(RUN, KEY_WRITE) {
        let _ = key.delete_value(NAME);
    }
}

pub fn set(enabled: bool) {
    if enabled {
        enable();
    } else {
        disable();
    }
}

const APP_KEY: &str = r"Software\WinLand";

/// On the very first run, turn autostart on (the user asked for it to load at
/// login), then remember we've done so — later runs respect the tray toggle.
pub fn first_run_enable() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey(APP_KEY) {
        if key.get_value::<u32, _>("Configured").is_err() {
            enable();
            let _ = key.set_value("Configured", &1u32);
        }
    }
}
