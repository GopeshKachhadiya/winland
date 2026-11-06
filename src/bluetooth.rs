//! Best-effort Bluetooth device battery via the `DEVPKEY_Bluetooth_Battery`
//! device property (the same value Windows Settings shows). Not all earbuds
//! report it; when none do, we simply show nothing.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDevicePropertyW, DIGCF_ALLCLASSES, DIGCF_PRESENT, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::DEVPROPTYPE;
use windows::Win32::Foundation::DEVPROPKEY;

// {104EA319-6EE2-4701-BD47-8DDBF425BBE5}, PID 2 = DEVPKEY_Bluetooth_Battery.
const FMTID_BLUETOOTH: GUID = GUID::from_u128(0x104EA319_6EE2_4701_BD47_8DDBF425BBE5);
// {b725f130-47ef-101a-a5f1-02608c9eebac}, PID 10 = DEVPKEY_NAME.
const FMTID_NAME: GUID = GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac);
// {83DA6326-97A6-4088-9453-A1923F573B29}, PID 15 = DEVPKEY_Device_IsConnected.
const FMTID_IS_CONNECTED: GUID = GUID::from_u128(0x83DA6326_97A6_4088_9453_A1923F573B29);
const DEVPROP_TYPE_BYTE: u32 = 0x03;
const DEVPROP_TYPE_BOOLEAN: u32 = 0x11;

/// (device name, battery percent).
pub type Buds = Option<(String, u8)>;

pub fn start() -> Receiver<Buds> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("winland-bt".into())
        .spawn(move || run(tx))
        .expect("spawn bluetooth thread");
    rx
}

fn run(tx: Sender<Buds>) {
    let mut last: Buds = None;
    let mut first = true;
    loop {
        let cur = unsafe { read_buds() };
        if std::env::var("WINLAND_BT_DEBUG").is_ok() {
            log::info!("bluetooth buds: {:?}", cur);
        }
        if first || cur != last {
            if tx.send(cur.clone()).is_err() {
                return;
            }
            last = cur;
            first = false;
        }
        thread::sleep(Duration::from_millis(2500));
    }
}

unsafe fn read_buds() -> Buds {
    let devinfo = SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_ALLCLASSES)
        .ok()?;

    let mut result: Buds = None;
    let mut i = 0u32;
    loop {
        let mut data = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        if SetupDiEnumDeviceInfo(devinfo, i, &mut data).is_err() {
            break;
        }
        i += 1;

        // Only report a device that is *currently connected* — the battery
        // property lingers (cached) on paired-but-disconnected devices.
        if !is_connected(devinfo, &data) {
            continue;
        }

        let bat_key = DEVPROPKEY {
            fmtid: FMTID_BLUETOOTH,
            pid: 2,
        };
        let mut ptype = DEVPROPTYPE::default();
        let mut buf = [0u8; 4];
        let mut required = 0u32;
        let ok = SetupDiGetDevicePropertyW(
            devinfo,
            &data,
            &bat_key,
            &mut ptype,
            Some(&mut buf),
            Some(&mut required),
            0,
        )
        .is_ok();
        if ok && ptype.0 == DEVPROP_TYPE_BYTE {
            let battery = buf[0];
            if battery <= 100 {
                let name = device_name(devinfo, &data).unwrap_or_else(|| "Earbuds".into());
                result = Some((name, battery));
                break;
            }
        }
    }

    let _ = SetupDiDestroyDeviceInfoList(devinfo);
    result
}

/// Whether the device node reports itself as currently connected.
unsafe fn is_connected(
    devinfo: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    data: &SP_DEVINFO_DATA,
) -> bool {
    let key = DEVPROPKEY {
        fmtid: FMTID_IS_CONNECTED,
        pid: 15,
    };
    let mut ptype = DEVPROPTYPE::default();
    let mut buf = [0u8; 4];
    let mut required = 0u32;
    if SetupDiGetDevicePropertyW(
        devinfo,
        data,
        &key,
        &mut ptype,
        Some(&mut buf),
        Some(&mut required),
        0,
    )
    .is_ok()
        && ptype.0 == DEVPROP_TYPE_BOOLEAN
    {
        // DEVPROP_TRUE is 0xFF; DEVPROP_FALSE is 0x00.
        return buf[0] != 0;
    }
    false
}

unsafe fn device_name(
    devinfo: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    data: &SP_DEVINFO_DATA,
) -> Option<String> {
    let name_key = DEVPROPKEY {
        fmtid: FMTID_NAME,
        pid: 10,
    };
    let mut ptype = DEVPROPTYPE::default();
    let mut u16buf = [0u16; 128];
    let bytes = std::slice::from_raw_parts_mut(u16buf.as_mut_ptr() as *mut u8, u16buf.len() * 2);
    let mut required = 0u32;
    if SetupDiGetDevicePropertyW(
        devinfo,
        data,
        &name_key,
        &mut ptype,
        Some(bytes),
        Some(&mut required),
        0,
    )
    .is_ok()
    {
        let len = u16buf.iter().position(|&c| c == 0).unwrap_or(u16buf.len());
        Some(String::from_utf16_lossy(&u16buf[..len]))
    } else {
        None
    }
}
