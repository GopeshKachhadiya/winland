//! Now-playing reader backed by the Windows System Media Transport Controls
//! (`GlobalSystemMediaTransportControls`) — the same source that powers the
//! volume-flyout media widget. Runs on its own thread and pushes updates
//! through a channel the UI drains each frame.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamWithContentType};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

/// Decoded album artwork (RGBA8).
#[derive(Clone, Debug)]
pub struct ArtImage {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, Default)]
pub struct MediaUpdate {
    pub has_session: bool,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    /// Set only when the track changes (artwork decode is expensive).
    pub art: Option<ArtImage>,
}

/// Transport commands sent from the UI to the current media session.
#[derive(Clone, Copy, Debug)]
pub enum MediaCommand {
    TogglePlayPause,
    Next,
    Previous,
}

/// Spawn the background reader; returns the update receiver and a command sender.
pub fn start() -> (Receiver<MediaUpdate>, Sender<MediaCommand>) {
    let (tx, rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    thread::Builder::new()
        .name("winland-media".into())
        .spawn(move || run(tx, cmd_rx))
        .expect("spawn media thread");
    (rx, cmd_tx)
}

fn run(tx: Sender<MediaUpdate>, cmd_rx: Receiver<MediaCommand>) {
    // WinRT async waits want an initialized apartment.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let manager = match Manager::RequestAsync().and_then(|op| op.join()) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("GSMTC manager unavailable: {e:?}");
            return;
        }
    };

    let mut last = MediaUpdate::default();
    let mut last_track = String::new();
    let mut first = true;
    loop {
        // Execute any pending transport commands on the current session.
        while let Ok(cmd) = cmd_rx.try_recv() {
            if let Ok(session) = manager.GetCurrentSession() {
                let _ = match cmd {
                    MediaCommand::TogglePlayPause => {
                        session.TryTogglePlayPauseAsync().and_then(|op| op.join())
                    }
                    MediaCommand::Next => session.TrySkipNextAsync().and_then(|op| op.join()),
                    MediaCommand::Previous => {
                        session.TrySkipPreviousAsync().and_then(|op| op.join())
                    }
                };
            }
        }

        let session = manager.GetCurrentSession().ok();
        let mut update = session
            .as_ref()
            .and_then(|s| read_session(s))
            .unwrap_or_default();

        // Decode artwork only when the track actually changes.
        let track = format!("{}\u{1}{}", update.title, update.artist);
        if update.has_session {
            if track != last_track {
                update.art = session.as_ref().and_then(|s| read_thumbnail(s));
                last_track = track;
            }
        } else {
            last_track.clear();
        }

        if std::env::var("WINLAND_MEDIA_DEBUG").is_ok() {
            log::info!(
                "media: session={} '{}' - '{}' playing={} {:.0}/{:.0}s",
                update.has_session,
                update.title,
                update.artist,
                update.playing,
                update.position,
                update.duration
            );
        }
        if first || changed(&last, &update) {
            if tx.send(update.clone()).is_err() {
                return; // UI gone
            }
            last = update;
            first = false;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// Only push when something the UI cares about moved.
fn changed(a: &MediaUpdate, b: &MediaUpdate) -> bool {
    a.has_session != b.has_session
        || a.title != b.title
        || a.artist != b.artist
        || a.playing != b.playing
        || (a.position - b.position).abs() > 0.4
        || (a.duration - b.duration).abs() > 0.4
}

/// Fetch and decode the current track's thumbnail into RGBA (expensive; called
/// only on track change).
fn read_thumbnail(session: &Session) -> Option<ArtImage> {
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let thumb = props.Thumbnail().ok()?;
    let stream: IRandomAccessStreamWithContentType = thumb.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > 12_000_000 {
        return None;
    }
    let input = stream.GetInputStreamAt(0).ok()?;
    let reader = DataReader::CreateDataReader(&input).ok()?;
    reader.LoadAsync(size).ok()?.join().ok()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf).ok()?;

    let img = image::load_from_memory(&buf).ok()?;
    let img = img.resize_to_fill(192, 192, image::imageops::FilterType::Triangle);
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some(ArtImage {
        rgba: rgba.into_raw(),
        w,
        h,
    })
}

fn read_session(session: &Session) -> Option<MediaUpdate> {
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = props.Title().map(|h| h.to_string_lossy()).unwrap_or_default();
    let artist = props
        .Artist()
        .map(|h| h.to_string_lossy())
        .unwrap_or_default();

    let playing = session
        .GetPlaybackInfo()
        .and_then(|pb| pb.PlaybackStatus())
        .map(|s| s == PlaybackStatus::Playing)
        .unwrap_or(false);

    let (position, duration) = session
        .GetTimelineProperties()
        .map(|tl| {
            let pos = tl.Position().map(|t| t.Duration as f64 / 1e7).unwrap_or(0.0);
            let end = tl.EndTime().map(|t| t.Duration as f64 / 1e7).unwrap_or(0.0);
            (pos, end)
        })
        .unwrap_or((0.0, 0.0));

    Some(MediaUpdate {
        has_session: !title.is_empty() || !artist.is_empty(),
        title,
        artist,
        playing,
        position,
        duration,
        art: None,
    })
}
