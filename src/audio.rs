//! System-audio visualizer source. Captures the default render endpoint in
//! WASAPI loopback mode, runs an FFT on the mixed-mono signal, and exposes a
//! handful of log-spaced frequency-band magnitudes (0..1) for the UI to draw.
//! Falls back silently (all-zero bands) if loopback can't be opened.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

pub const BANDS: usize = 16;
const FFT_SIZE: usize = 1024;

#[derive(Clone)]
pub struct Audio {
    shared: Arc<Mutex<[f32; BANDS]>>,
}

impl Audio {
    pub fn start() -> Self {
        let shared = Arc::new(Mutex::new([0.0f32; BANDS]));
        let s2 = shared.clone();
        thread::Builder::new()
            .name("winland-audio".into())
            .spawn(move || unsafe {
                if let Err(e) = capture_loop(&s2) {
                    log::warn!("audio loopback unavailable: {e:?}");
                }
            })
            .ok();
        Self { shared }
    }

    pub fn bands(&self) -> [f32; BANDS] {
        *self.shared.lock().unwrap()
    }
}

unsafe fn capture_loop(shared: &Arc<Mutex<[f32; BANDS]>>) -> windows::core::Result<()> {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

    let mix = client.GetMixFormat()?;
    let channels = (*mix).nChannels as usize;

    client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK,
        2_000_000, // 200ms buffer in 100ns units
        0,
        mix,
        None,
    )?;
    let capture: IAudioCaptureClient = client.GetService()?;
    client.Start()?;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let mut acc: Vec<f32> = Vec::with_capacity(FFT_SIZE * 2);

    loop {
        let mut packet = capture.GetNextPacketSize()?;
        while packet != 0 {
            let mut data_ptr: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            capture.GetBuffer(&mut data_ptr, &mut frames, &mut flags, None, None)?;

            let n = frames as usize;
            if (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 && !data_ptr.is_null() {
                let samples = std::slice::from_raw_parts(data_ptr as *const f32, n * channels);
                for f in 0..n {
                    let mut s = 0.0f32;
                    for c in 0..channels {
                        s += samples[f * channels + c];
                    }
                    acc.push(s / channels as f32);
                }
            } else {
                for _ in 0..n {
                    acc.push(0.0);
                }
            }

            capture.ReleaseBuffer(frames)?;
            packet = capture.GetNextPacketSize()?;
        }

        if acc.len() >= FFT_SIZE {
            // Use the most recent window, drop the rest.
            let start = acc.len() - FFT_SIZE;
            let mut buf: Vec<Complex<f32>> = (0..FFT_SIZE)
                .map(|i| {
                    let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (FFT_SIZE as f32 - 1.0)).cos();
                    Complex::new(acc[start + i] * w, 0.0)
                })
                .collect();
            acc.clear();
            fft.process(&mut buf);

            let bands = to_bands(&buf);
            if let Ok(mut g) = shared.lock() {
                *g = bands;
            }
        }

        thread::sleep(Duration::from_millis(16));
    }
}

/// Bucket FFT magnitudes into log-spaced bands, normalized to ~0..1.
fn to_bands(spectrum: &[Complex<f32>]) -> [f32; BANDS] {
    let half = spectrum.len() / 2;
    let mut out = [0.0f32; BANDS];
    for b in 0..BANDS {
        let lo = band_edge(b, half);
        let hi = band_edge(b + 1, half).max(lo + 1);
        let mut sum = 0.0f32;
        let mut cnt = 0u32;
        for k in lo..hi.min(half) {
            sum += spectrum[k].norm();
            cnt += 1;
        }
        let mag = if cnt > 0 { sum / cnt as f32 } else { 0.0 };
        // Perceptual-ish compression + gain, then clamp.
        out[b] = ((mag * 0.09).sqrt() * 1.1).min(1.0);
    }
    out
}

fn band_edge(b: usize, half: usize) -> usize {
    // Log-spaced from bin 1 to `half`.
    let t = b as f32 / BANDS as f32;
    (1.0 * (half as f32).powf(t)).round() as usize
}
