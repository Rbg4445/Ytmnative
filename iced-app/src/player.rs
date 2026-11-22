//! Playback controller for the iced UI: the same logic as the Tauri `AppController`
//! (backend.rs) without any Tauri dependency. Owns the audio engine, queue and library.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use tracing::warn;

use crate::audio::AudioEngine;
use crate::media::{MediaAction, SystemMedia};
use crate::model::Track;
use crate::queue::{PlayQueue, RepeatMode};
use crate::storage::LibraryData;
use crate::ytm::YtmClient;

/// What the UI needs to draw the player each tick.
#[derive(Debug, Clone, Default)]
pub struct PlaybackView {
    pub current: Option<Track>,
    pub queue: Vec<Track>,
    pub current_index: Option<usize>,
    pub is_playing: bool,
    pub is_paused: bool,
    pub is_buffering: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub error: Option<String>,
}

pub struct Player {
    pub ytm: YtmClient,
    pub audio: AudioEngine,
    pub queue: Mutex<PlayQueue>,
    pub library: Mutex<LibraryData>,
    pub http: reqwest::Client,
    rt: tokio::runtime::Runtime,
    generation: AtomicU64,
    resolving: AtomicBool,
    last_end_seq: AtomicU64,
    last_error: Mutex<Option<String>>,
    media: Mutex<Option<SystemMedia>>,
    media_tx: Sender<MediaAction>,
    media_rx: Mutex<Receiver<MediaAction>>,
    last_media_track: Mutex<String>,
    last_media_state: Mutex<(bool, bool)>,
}

impl Player {
    pub fn new() -> anyhow::Result<Arc<Self>> {
        let library = LibraryData::load();
        let ytm = YtmClient::new(library.auth_cookie.clone())?;
        let audio = AudioEngine::new()?;
        audio.set_volume(library.volume.clamp(0.0, 1.0));

        // Restore the last played track (paused, ready to resume)
        let mut queue = PlayQueue::new();
        if let Some(last) = library.last_track.clone() {
            queue.start_with(last);
        }

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ytm-player")
            .enable_all()
            .build()?;

        let (media_tx, media_rx) = mpsc::channel();
        let player = Arc::new(Self {
            ytm,
            audio,
            queue: Mutex::new(queue),
            library: Mutex::new(library),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()?,
            rt,
            generation: AtomicU64::new(0),
            resolving: AtomicBool::new(false),
            last_end_seq: AtomicU64::new(0),
            last_error: Mutex::new(None),
            media: Mutex::new(None),
            media_tx,
            media_rx: Mutex::new(media_rx),
            last_media_track: Mutex::new(String::new()),
            last_media_state: Mutex::new((false, true)),
        });

        let ytm = player.ytm.clone();
        player.rt.spawn(async move {
            ytm.warmup().await;
            // Confirms the saved YouTube session so radios/mixes are personalized.
            let _ = ytm.is_youtube_connected().await;
        });
        crate::services::spawn_integrations(Arc::clone(&player));
        Ok(player)
    }

    pub fn rt_handle(&self) -> &tokio::runtime::Handle {
        self.rt.handle()
    }

    pub fn attach_media(&self, hwnd: Option<*mut c_void>) {
        if let Ok(mut guard) = self.media.lock() {
            if guard.is_some() {
                return;
            }
            let sm = SystemMedia::new(hwnd, self.media_tx.clone(), Box::new(|| {}));
            *guard = Some(sm);
        }
    }

    // ---------- Playback ----------

    pub fn play_track(self: &Arc<Self>, track: Track) {
        if let Ok(mut q) = self.queue.lock() {
            q.start_with(track.clone());
        }
        self.radio_refill(track.id.clone());
        self.trigger(track);
    }

    pub fn play_list(self: &Arc<Self>, tracks: Vec<Track>, start: usize) {
        let first = self.queue.lock().ok().and_then(|mut q| {
            q.start_with_list(tracks, start);
            q.current().cloned()
        });
        if let Some(t) = first {
            self.trigger(t);
        }
    }

    pub fn select_index(self: &Arc<Self>, idx: usize) {
        let picked = self.queue.lock().ok().and_then(|mut q| {
            let t = q.select_index(idx).cloned();
            let needs = q.needs_refill();
            t.map(|t| (t, needs))
        });
        if let Some((t, needs)) = picked {
            if needs {
                self.radio_refill(t.id.clone());
            }
            self.trigger(t);
        }
    }

    /// Resolve the stream in the background and start audio (UI never blocks).
    fn trigger(self: &Arc<Self>, track: Track) {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.resolving.store(true, Ordering::Relaxed);
        self.audio.stop();
        if let Ok(mut e) = self.last_error.lock() {
            *e = None;
        }
        if let Ok(mut lib) = self.library.lock() {
            lib.record_played(&track);
        }
        if let Ok(mut q) = self.queue.lock() {
            q.note_played(&track);
        }

        let me = Arc::clone(self);
        self.rt.spawn(async move {
            match me.ytm.resolve_stream(&track.id).await {
                Ok(stream) => {
                    if me.generation.load(Ordering::SeqCst) != gen {
                        return; // a newer track was clicked meanwhile
                    }
                    me.resolving.store(false, Ordering::Relaxed);
                    me.audio.play(stream);
                    me.prefetch_upcoming();
                }
                Err(e) => {
                    if me.generation.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    me.resolving.store(false, Ordering::Relaxed);
                    if let Ok(mut err) = me.last_error.lock() {
                        *err = Some(e.to_string());
                    }
                }
            }
        });
    }

    fn prefetch_upcoming(self: &Arc<Self>) {
        let ids = self.queue.lock().ok().map(|q| q.peek_next_ids(2)).unwrap_or_default();
        for id in ids {
            let ytm = self.ytm.clone();
            self.rt.spawn(async move { ytm.prefetch_stream(&id).await });
        }
    }

    /// Hover prefetch from the UI.
    pub fn prefetch(&self, id: String) {
        let ytm = self.ytm.clone();
        self.rt.spawn(async move { ytm.prefetch_stream(&id).await });
    }

    /// Top up autoplay; same strategy as the Tauri app (current song, then the session seed).
    fn radio_refill(self: &Arc<Self>, seed: String) {
        const MIN_NEW: usize = 8;
        let me = Arc::clone(self);
        self.rt.spawn(async move {
            let origin = me.queue.lock().ok().and_then(|q| q.origin_seed());
            let mut seeds = vec![seed];
            if let Some(o) = origin {
                if o != seeds[0] {
                    seeds.push(o);
                }
            }
            let mut added = 0;
            for s in seeds {
                match me.ytm.fetch_radio(&s).await {
                    Ok(tracks) => {
                        if let Ok(mut q) = me.queue.lock() {
                            added += q.append_radio(tracks);
                        }
                    }
                    Err(e) => warn!("Radio refill from {s} failed: {e}"),
                }
                if added >= MIN_NEW {
                    break;
                }
            }
            if added > 0 {
                me.prefetch_upcoming();
            }
        });
    }

    pub fn next(self: &Arc<Self>, auto_end: bool) {
        let next = self.queue.lock().ok().and_then(|mut q| {
            let t = q.advance_next(auto_end).cloned();
            let needs = q.needs_refill();
            t.map(|t| (t, needs))
        });
        if let Some((t, needs)) = next {
            if needs {
                self.radio_refill(t.id.clone());
            }
            self.trigger(t);
        }
    }

    pub fn prev(self: &Arc<Self>) {
        if self.audio.snapshot().position_secs > 3.0 {
            self.audio.seek(0.0);
            return;
        }
        let prev = self.queue.lock().ok().and_then(|mut q| q.step_prev().cloned());
        if let Some(t) = prev {
            self.trigger(t);
        }
    }

    pub fn toggle_pause(self: &Arc<Self>) {
        let snap = self.audio.snapshot();
        // A restored last track is queued but not loaded yet: start it.
        if !snap.is_playing && !snap.is_paused && !snap.is_buffering && !self.resolving.load(Ordering::Relaxed) {
            if let Some(cur) = self.queue.lock().ok().and_then(|q| q.current().cloned()) {
                self.trigger(cur);
                return;
            }
        }
        self.audio.toggle_pause();
    }

    pub fn seek(&self, secs: f64) {
        self.audio.seek(secs);
    }

    pub fn set_volume(&self, v: f32) {
        self.audio.set_volume(v);
        if let Ok(mut lib) = self.library.lock() {
            lib.volume = v;
        }
    }

    /// Persist volume etc. (called rarely, e.g. on slider release).
    pub fn save_library(&self) {
        if let Ok(lib) = self.library.lock() {
            lib.save();
        }
    }

    pub fn toggle_shuffle(&self) {
        if let Ok(mut q) = self.queue.lock() {
            q.toggle_shuffle();
        }
    }

    pub fn cycle_repeat(&self) {
        if let Ok(mut q) = self.queue.lock() {
            q.cycle_repeat();
        }
    }

    pub fn toggle_like(&self, track: &Track) -> bool {
        self.library.lock().map(|mut l| l.toggle_like(track)).unwrap_or(false)
    }

    pub fn is_liked(&self, id: &str) -> bool {
        self.library.lock().map(|l| l.is_liked(id)).unwrap_or(false)
    }

    pub fn pause(&self) {
        let snap = self.audio.snapshot();
        if snap.is_playing && !snap.is_paused {
            self.audio.toggle_pause();
        }
    }

    pub fn play_or_resume(self: &Arc<Self>) {
        let snap = self.audio.snapshot();
        if snap.is_paused {
            self.audio.toggle_pause();
        } else if !snap.is_playing && !snap.is_buffering && !self.resolving.load(Ordering::Relaxed) {
            if let Some(cur) = self.queue.lock().ok().and_then(|q| q.current().cloned()) {
                self.trigger(cur);
            }
        }
    }

    pub fn create_playlist(&self, name: String) -> String {
        self.library
            .lock()
            .map(|mut l| l.create_playlist(name))
            .unwrap_or_default()
    }

    pub fn delete_playlist(&self, playlist_id: &str) {
        if let Ok(mut l) = self.library.lock() {
            l.delete_playlist(playlist_id);
        }
    }

    pub fn add_track_to_playlist(&self, playlist_id: &str, track: &Track) -> bool {
        self.library
            .lock()
            .map(|mut l| l.add_track_to_playlist(playlist_id, track))
            .unwrap_or(false)
    }

    pub fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) {
        if let Ok(mut l) = self.library.lock() {
            l.remove_track_from_playlist(playlist_id, track_id);
        }
    }

    pub fn snapshot_for_api(&self) -> PlaybackView {
        let snap = self.audio.snapshot();
        let (current, queue, current_index, shuffle, repeat) = match self.queue.lock() {
            Ok(q) => (q.current().cloned(), q.tracks().to_vec(), q.current_index(), q.shuffle, q.repeat),
            Err(_) => (None, Vec::new(), None, false, RepeatMode::Off),
        };
        let error = snap.last_error.or_else(|| self.last_error.lock().ok().and_then(|e| e.clone()));
        PlaybackView {
            duration: if snap.duration_secs > 0.0 {
                snap.duration_secs
            } else {
                current.as_ref().and_then(|t| t.duration_secs).unwrap_or(0) as f64
            },
            current,
            queue,
            current_index,
            is_playing: snap.is_playing,
            is_paused: snap.is_paused,
            is_buffering: snap.is_buffering || self.resolving.load(Ordering::Relaxed),
            position: snap.position_secs,
            volume: snap.volume,
            shuffle,
            repeat,
            error,
        }
    }

    /// Called by the UI ~4x per second: auto-advances at track end, drains hardware media keys,
    /// updates Windows SMTC metadata, and returns the state to draw.
    pub fn poll(self: &Arc<Self>) -> PlaybackView {
        // Drain hardware media key events (souvlaki / SMTC)
        let actions: Vec<MediaAction> = self
            .media_rx
            .lock()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for action in actions {
            match action {
                MediaAction::Play => self.play_or_resume(),
                MediaAction::Pause => self.pause(),
                MediaAction::Toggle => self.toggle_pause(),
                MediaAction::Next => self.next(false),
                MediaAction::Previous => self.prev(),
                MediaAction::Stop => self.audio.stop(),
            }
        }

        let snap = self.audio.snapshot();
        if snap.track_end_seq != self.last_end_seq.load(Ordering::Relaxed) {
            self.last_end_seq.store(snap.track_end_seq, Ordering::Relaxed);
            self.next(true);
        }
        let view = self.snapshot_for_api();

        // Sync SMTC metadata & playback state when changed
        if let Ok(mut guard) = self.media.lock() {
            if let Some(sm) = guard.as_mut() {
                if let Some(ref track) = view.current {
                    let changed = self
                        .last_media_track
                        .lock()
                        .map(|mut last| {
                            if *last != track.id {
                                *last = track.id.clone();
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if changed {
                        sm.set_track(track);
                    }
                }
                let playing = view.is_playing && !view.is_paused;
                let stopped = view.current.is_none() || (!view.is_playing && !view.is_paused && !view.is_buffering);
                let state_changed = self
                    .last_media_state
                    .lock()
                    .map(|mut last| {
                        if *last != (playing, stopped) {
                            *last = (playing, stopped);
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if state_changed {
                    sm.set_playing(playing, stopped);
                }
            }
        }

        view
    }
}
