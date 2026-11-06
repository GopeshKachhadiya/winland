//! System indicators on a background thread:
//! - master volume (Core Audio `IAudioEndpointVolume`)
//! - battery / charging (`GetSystemPowerStatus`)
//! - audio output device changes (AirPods / headphones connect)
//!
//! Emits an event only when something changes, so the island can flash a
//! transient indicator.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

#[derive(Clone, Debug)]
pub enum SysEvent {
    Volume { level: f32, muted: bool },
    Power { charging: bool, percent: u8 },
    Device { name: String },
}

pub fn start() -> Receiver<SysEvent> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("winland-sys".into())
        .spawn(move || run(tx))
        .expect("spawn sys thread");
    rx
}

fn run(tx: Sender<SysEvent>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let enumerator: Option<IMMDeviceEnumerator> =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok();
    if enumerator.is_none() {
        log::warn!("audio enumerator unavailable; volume/device indicators disabled");
    }

    let mut endpoint: Option<IAudioEndpointVolume> = None;
    let mut last_vol = -1.0f32;
    let mut last_muted = false;
    let mut last_ac: i16 = -1;
    let mut last_dev = String::new();
    let mut first = true;

    loop {
        // --- Default audio device: detect connect/switch (AirPods etc.) ---
        if let Some(enm) = enumerator.as_ref() {
            if let Ok(dev) = unsafe { enm.GetDefaultAudioEndpoint(eRender, eConsole) } {
                let name = unsafe { device_name(&dev) }.unwrap_or_default();
                if name != last_dev {
                    if !first && !name.is_empty() {
                        let _ = tx.send(SysEvent::Device { name: name.clone() });
                    }
                    // Re-acquire the volume endpoint for the new device.
                    endpoint = unsafe { dev.Activate(CLSCTX_ALL, None) }.ok();
                    last_dev = name;
                }
            }
        }

        // --- Volume ---
        if let Some(ep) = endpoint.as_ref() {
            unsafe {
                if let (Ok(level), Ok(mute)) = (ep.GetMasterVolumeLevelScalar(), ep.GetMute()) {
                    let muted = mute.as_bool();
                    if !first && ((level - last_vol).abs() > 0.004 || muted != last_muted) {
                        let _ = tx.send(SysEvent::Volume { level, muted });
                    }
                    last_vol = level;
                    last_muted = muted;
                }
            }
        }

        // --- Battery / AC ---
        let mut st = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut st) }.is_ok() {
            let ac = st.ACLineStatus as i16;
            if !first && ac != last_ac && st.ACLineStatus != 255 {
                let percent = if st.BatteryLifePercent == 255 {
                    100
                } else {
                    st.BatteryLifePercent
                };
                let _ = tx.send(SysEvent::Power {
                    charging: st.ACLineStatus == 1,
                    percent,
                });
            }
            last_ac = ac;
        }

        first = false;
        thread::sleep(Duration::from_millis(150));
    }
}

/// Read a device's friendly name (e.g. "AirPods Pro") from its property store.
unsafe fn device_name(dev: &IMMDevice) -> Option<String> {
    let store = dev.OpenPropertyStore(STGM_READ).ok()?;
    let pv = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
    let pwstr = PropVariantToStringAlloc(&pv).ok()?;
    let s = pwstr.to_string().ok();
    CoTaskMemFree(Some(pwstr.0 as *const _));
    s
}
