// WinLand — a faithful Apple Dynamic Island for Windows, in native Rust.
//
// Hide the console window in release builds; keep it in debug for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod app;
mod art;
mod audio;
mod autostart;
mod bluetooth;
mod content;
mod dashboard;
mod gcal;
mod gfx;
mod media;
mod state;
mod system;
mod text;
mod tray;
mod weather;

use winit::event_loop::{ControlFlow, EventLoop};

use windows::core::w;
use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::System::Threading::CreateMutexW;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Single instance: a named mutex. If it already exists, another WinLand is
    // running, so bail out. The handle lives for the whole process.
    let _mutex = unsafe { CreateMutexW(None, true, w!("Local\\WinLandSingletonMutex")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        log::info!("another WinLand instance is already running; exiting");
        return;
    }

    // In normal use, enable run-at-login on first launch (skipped in test modes).
    let test_mode =
        std::env::var("WINLAND_DEMO").is_ok() || std::env::var("WINLAND_MODE").is_ok();
    if !test_mode {
        autostart::first_run_enable();
    }

    let event_loop = EventLoop::new().expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = app::App::default();
    event_loop.run_app(&mut app).expect("run app");
}
