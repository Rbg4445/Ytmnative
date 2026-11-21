use std::ffi::c_void;
use std::sync::mpsc::Sender;
use std::time::Duration;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};
use tracing::warn;

use crate::model::Track;

#[derive(Debug, Clone, Copy)]
pub enum MediaAction {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
}

/// Wrapper around `souvlaki::MediaControls` for Windows SMTC & hardware media keys.
pub struct SystemMedia {
    controls: Option<MediaControls>,
}

impl SystemMedia {
    pub fn new(hwnd: Option<*mut c_void>, tx: Sender<MediaAction>, wakeup: Box<dyn Fn() + Send + Sync>) -> Self {
        let config = PlatformConfig {
            dbus_name: "ytm_native",
            display_name: "YTM Native",
            hwnd,
        };

        let controls = match MediaControls::new(config) {
            Ok(mut c) => {
                let attach_res = c.attach(move |event: MediaControlEvent| {
                    let action = match event {
                        MediaControlEvent::Play => Some(MediaAction::Play),
                        MediaControlEvent::Pause => Some(MediaAction::Pause),
                        MediaControlEvent::Toggle => Some(MediaAction::Toggle),
                        MediaControlEvent::Next => Some(MediaAction::Next),
                        MediaControlEvent::Previous => Some(MediaAction::Previous),
                        MediaControlEvent::Stop => Some(MediaAction::Stop),
                        _ => None,
                    };
                    if let Some(a) = action {
                        let _ = tx.send(a);
                        wakeup();
                    }
                });
                if let Err(e) = attach_res {
                    warn!("Failed to attach media key listener: {e:?}");
                    None
                } else {
                    Some(c)
                }
            }
            Err(e) => {
                warn!("System media controls unavailable: {e:?}");
                None
            }
        };

        Self { controls }
    }

    pub fn set_track(&mut self, track: &Track) {
        if let Some(c) = self.controls.as_mut() {
            let _ = c.set_metadata(MediaMetadata {
                title: Some(&track.title),
                artist: Some(&track.artist),
                album: track.album.as_deref(),
                cover_url: track.thumbnail_url.as_deref(),
                duration: track.duration_secs.map(|s| Duration::from_secs(s as u64)),
            });
        }
    }

    pub fn set_playing(&mut self, playing: bool, stopped: bool) {
        if let Some(c) = self.controls.as_mut() {
            let state = if stopped {
                MediaPlayback::Stopped
            } else if playing {
                MediaPlayback::Playing { progress: None }
            } else {
                MediaPlayback::Paused { progress: None }
            };
            let _ = c.set_playback(state);
        }
    }
}
