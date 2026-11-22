use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tracing::{debug, warn};

use crate::audio::AudioEngine;
use crate::integrations::{
    extract_video_id_from_url, fetch_lrclib_lyrics, lastfm_scrobble, lastfm_update_now_playing,
    spawn_obs_api_server, DiscordRpcClient, LyricLineDto,
};
use crate::media::{MediaAction, SystemMedia};
use crate::model::{ArtistDetailDto, ArtistSummary, SearchBundleDto, Track};
use crate::queue::{PlayQueue, RepeatMode};
use crate::storage::LibraryData;
use crate::updater::{self, UpdateInfo};
use crate::ytm::YtmClient;

#[derive(Debug, Clone, Serialize)]
pub struct PlaybackStateDto {
    pub current: Option<Track>,
    pub queue: Vec<Track>,
    pub current_index: Option<usize>,
    pub is_playing: bool,
    pub is_paused: bool,
    pub is_buffering: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub volume: f32,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub sleep_remaining_secs: Option<u64>,
    pub sleep_after_track: bool,
    pub stream_meta: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HomeFeedDto {
    pub yt_connected: bool,
    pub yt_history: Vec<Track>,
    pub recommendations: Vec<Track>,
}

/// Result of a YouTube account sync, shown to the user.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AccountSyncReport {
    pub connected: bool,
    pub liked: usize,
    pub playlists: usize,
    pub artists: usize,
    /// Parts that could not be fetched (e.g. "playlists"); empty when everything worked.
    pub failed: Vec<String>,
}

/// Pull liked songs, library playlists and library artists from the signed-in account
/// and merge them into the local library.
async fn run_account_sync(ctrl: &AppController) -> Result<AccountSyncReport, String> {
    if !ctrl.ytm.is_youtube_connected().await {
        return Ok(AccountSyncReport::default());
    }
    let snap = ctrl.ytm.fetch_account_snapshot().await;
    let mut failed = Vec::new();
    if snap.liked.is_none() {
        failed.push("liked".to_string());
    }
    if snap.playlists.is_none() {
        failed.push("playlists".to_string());
    }
    if snap.artists.is_none() {
        failed.push("artists".to_string());
    }
    if failed.len() == 3 {
        return Err(
            "YouTube hesabından veri alınamadı. Oturum süresi dolmuş olabilir; çıkış yapıp tekrar Google ile giriş yap."
                .to_string(),
        );
    }
    let (liked, playlists, artists) = ctrl
        .library
        .lock()
        .map(|mut lib| lib.apply_account_sync(snap.liked, snap.playlists, snap.artists))
        .map_err(|_| "Kitaplık kilitlenemedi".to_string())?;
    Ok(AccountSyncReport {
        connected: true,
        liked,
        playlists,
        artists,
        failed,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct LyricsDto {
    pub body: String,
    pub footer: String,
    pub synced: Option<Vec<LyricLineDto>>,
}

#[derive(Debug, Default)]
struct SleepTimer {
    deadline: Option<Instant>,
    stop_after_track: bool,
}

pub struct AppController {
    pub ytm: YtmClient,
    pub audio: AudioEngine,
    pub queue: Mutex<PlayQueue>,
    pub library: Mutex<LibraryData>,
    pub media: Mutex<SystemMedia>,
    pub stream_meta: Mutex<Option<String>>,
    pub last_error: Mutex<Option<String>>,
    pub http: reqwest::Client,
    sleep_timer: Mutex<SleepTimer>,
    /// Incremented on every new play request so stale in-flight stream resolutions abort instantly.
    play_generation: AtomicU64,
    is_resolving: AtomicBool,
    last_track_end_seq: AtomicU64,
}

impl AppController {
    pub fn new(
        hwnd: Option<*mut std::ffi::c_void>,
    ) -> anyhow::Result<(Arc<Self>, mpsc::Receiver<MediaAction>)> {
        let library = LibraryData::load();
        let ytm = YtmClient::new(library.auth_cookie.clone())?;
        let audio = AudioEngine::new()?;
        audio.set_volume(library.volume.clamp(0.0, 1.0));

        // Restore last played track into queue (paused, ready to resume like ytmdesktop!)
        let mut queue = PlayQueue::new();
        if let Some(last) = library.last_track.clone() {
            queue.start_with(last);
        }

        let (media_tx, media_rx) = mpsc::channel();
        let media = SystemMedia::new(hwnd, media_tx, Box::new(|| {}));

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(2)
            .build()?;

        let ctrl = Arc::new(Self {
            ytm,
            audio,
            queue: Mutex::new(queue),
            library: Mutex::new(library),
            media: Mutex::new(media),
            stream_meta: Mutex::new(None),
            last_error: Mutex::new(None),
            http,
            sleep_timer: Mutex::new(SleepTimer::default()),
            play_generation: AtomicU64::new(0),
            is_resolving: AtomicBool::new(false),
            last_track_end_seq: AtomicU64::new(0),
        });

        Ok((ctrl, media_rx))
    }

    /// Non-blocking instant track launch (< 2ms): updates UI state immediately and resolves stream in background.
    pub fn trigger_play_track(self: &Arc<Self>, track: Track) {
        let gen = self.play_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.is_resolving.store(true, Ordering::Relaxed);
        self.audio.stop();

        if let Ok(mut err) = self.last_error.lock() {
            *err = None;
        }
        if let Ok(mut lib) = self.library.lock() {
            lib.record_played(&track);
        }
        if let Ok(mut q) = self.queue.lock() {
            q.note_played(&track);
        }
        if let Ok(mut m) = self.media.lock() {
            m.set_track(&track);
        }

        let ctrl = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            match ctrl.ytm.resolve_stream(&track.id).await {
                Ok(stream) => {
                    // Abort if user already clicked a newer track!
                    if ctrl.play_generation.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    let kbps = stream.bitrate / 1000;
                    let norm = match stream.loudness_db {
                        Some(db) if db > 0.0 => format!(" • -{db:.1} dB"),
                        _ => String::new(),
                    };
                    if let Ok(mut meta) = ctrl.stream_meta.lock() {
                        *meta = Some(format!("AAC {kbps}k HQ{norm}"));
                    }
                    ctrl.is_resolving.store(false, Ordering::Relaxed);
                    ctrl.audio.play(stream);
                    if let Ok(mut m) = ctrl.media.lock() {
                        m.set_playing(true, false);
                    }
                    ctrl.spawn_prefetch_upcoming();
                }
                Err(e) => {
                    if ctrl.play_generation.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    ctrl.is_resolving.store(false, Ordering::Relaxed);
                    let msg = e.to_string();
                    if let Ok(mut err) = ctrl.last_error.lock() {
                        *err = Some(msg);
                    }
                }
            }
        });
    }

    /// Prefetch the next 2 tracks in queue so track transitions are instant!
    pub fn spawn_prefetch_upcoming(self: &Arc<Self>) {
        let next_ids = self
            .queue
            .lock()
            .ok()
            .map(|q| q.peek_next_ids(2))
            .unwrap_or_default();
        for id in next_ids {
            let ytm = self.ytm.clone();
            tauri::async_runtime::spawn(async move {
                debug!("Prefetching upcoming stream: {id}");
                ytm.prefetch_stream(&id).await;
            });
        }
    }

    /// Top up autoplay. Radio of the current song first; if that brings too few NEW songs
    /// (radios of songs deep in a chain keep circling the same tracks), also use the song
    /// the session started from.
    pub fn spawn_radio_refill(self: &Arc<Self>, seed_id: String) {
        const MIN_NEW: usize = 8;
        let ctrl = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let origin = ctrl.queue.lock().ok().and_then(|q| q.origin_seed());
            let mut seeds = vec![seed_id];
            if let Some(o) = origin {
                if o != seeds[0] {
                    seeds.push(o);
                }
            }
            let mut added = 0;
            for seed in seeds {
                match ctrl.ytm.fetch_radio(&seed).await {
                    Ok(tracks) => {
                        if let Ok(mut q) = ctrl.queue.lock() {
                            added += q.append_radio(tracks);
                        }
                    }
                    Err(e) => warn!("Radio refill from {seed} failed: {e}"),
                }
                if added >= MIN_NEW {
                    break;
                }
            }
            if added > 0 {
                ctrl.spawn_prefetch_upcoming();
            }
        });
    }

    pub fn advance_next(self: &Arc<Self>, from_auto_end: bool) {
        // Check sleep timer "stop after current track"
        if from_auto_end {
            if let Ok(mut st) = self.sleep_timer.lock() {
                if st.stop_after_track {
                    st.stop_after_track = false;
                    st.deadline = None;
                    self.audio.pause();
                    return;
                }
            }
        }

        let next_opt = self.queue.lock().ok().and_then(|mut q| {
            let next = q.advance_next(from_auto_end).cloned();
            let needs = q.needs_refill();
            next.map(|t| (t, needs))
        });
        if let Some((track, needs_refill)) = next_opt {
            if needs_refill {
                self.spawn_radio_refill(track.id.clone());
            }
            self.trigger_play_track(track);
        }
    }

    pub fn step_prev(self: &Arc<Self>) {
        if self.audio.snapshot().position_secs > 3.0 {
            self.audio.seek(0.0);
            return;
        }
        let prev_opt = self
            .queue
            .lock()
            .ok()
            .and_then(|mut q| q.step_prev().cloned());
        if let Some(track) = prev_opt {
            self.trigger_play_track(track);
        }
    }

    pub fn toggle_pause(self: &Arc<Self>) {
        let snap = self.audio.snapshot();
        // If a restored last_track is sitting in the queue but not loaded into audio yet, play it!
        if !snap.is_playing && !snap.is_paused && !snap.is_buffering && !self.is_resolving.load(Ordering::Relaxed) {
            if let Some(cur) = self.queue.lock().ok().and_then(|q| q.current().cloned()) {
                self.trigger_play_track(cur);
                return;
            }
        }
        self.audio.toggle_pause();
        let after = self.audio.snapshot();
        if let Ok(mut m) = self.media.lock() {
            m.set_playing(!after.is_paused && after.is_playing, false);
        }
    }

    pub fn playback_dto(&self) -> PlaybackStateDto {
        let snap = self.audio.snapshot();
        let (current, queue, current_index, repeat, shuffle) = match self.queue.lock() {
            Ok(q) => (
                q.current().cloned(),
                q.tracks().to_vec(),
                q.current_index(),
                q.repeat,
                q.shuffle,
            ),
            Err(_) => (None, Vec::new(), None, RepeatMode::Off, false),
        };
        let (sleep_remaining_secs, sleep_after_track) = match self.sleep_timer.lock() {
            Ok(st) => {
                let rem = st
                    .deadline
                    .map(|d| d.saturating_duration_since(Instant::now()).as_secs());
                (rem, st.stop_after_track)
            }
            Err(_) => (None, false),
        };
        let stream_meta = self.stream_meta.lock().ok().and_then(|m| m.clone());
        let last_error = snap
            .last_error
            .or_else(|| self.last_error.lock().ok().and_then(|e| e.clone()));

        PlaybackStateDto {
            current,
            queue,
            current_index,
            is_playing: snap.is_playing,
            is_paused: snap.is_paused,
            is_buffering: snap.is_buffering || self.is_resolving.load(Ordering::Relaxed),
            position_secs: snap.position_secs,
            duration_secs: snap.duration_secs,
            volume: snap.volume,
            repeat,
            shuffle,
            sleep_remaining_secs,
            sleep_after_track,
            stream_meta,
            last_error,
        }
    }
}

/// Lightweight background loop for auto-advance, Windows SMTC media keys, and Sleep Timer
/// — keeps working even when the WebView2 window is destroyed in System Tray!
pub fn spawn_background_supervisor(
    ctrl: Arc<AppController>,
    media_rx: mpsc::Receiver<MediaAction>,
) {
    let warm_ytm = ctrl.ytm.clone();
    tauri::async_runtime::spawn(async move {
        warm_ytm.warmup().await;
    });

    // Launch OBS Browser Source & Stream Deck Companion API server on http://127.0.0.1:13091
    spawn_obs_api_server(Arc::clone(&ctrl));

    tauri::async_runtime::spawn(async move {
        let mut last_playing = false;
        let mut discord = DiscordRpcClient::new();
        let mut lastfm_now_id = String::new();
        let mut lastfm_scrobbled_id = String::new();

        loop {
            while let Ok(action) = media_rx.try_recv() {
                match action {
                    MediaAction::Play => ctrl.audio.resume(),
                    MediaAction::Pause => ctrl.audio.pause(),
                    MediaAction::Toggle => ctrl.toggle_pause(),
                    MediaAction::Stop => ctrl.audio.stop(),
                    MediaAction::Next => ctrl.advance_next(false),
                    MediaAction::Previous => ctrl.step_prev(),
                }
            }

            // Sleep timer check
            let sleep_expired = ctrl
                .sleep_timer
                .lock()
                .ok()
                .map(|mut st| {
                    if let Some(dl) = st.deadline {
                        if Instant::now() >= dl {
                            st.deadline = None;
                            st.stop_after_track = false;
                            return true;
                        }
                    }
                    false
                })
                .unwrap_or(false);
            if sleep_expired {
                ctrl.audio.pause();
            }

            let snap = ctrl.audio.snapshot();
            if snap.is_playing != last_playing {
                last_playing = snap.is_playing;
                let stopped = !snap.is_playing && !snap.is_paused && !snap.is_buffering;
                if let Ok(mut m) = ctrl.media.lock() {
                    m.set_playing(snap.is_playing, stopped);
                }
            }

            // Discord Rich Presence & Last.fm Scrobbler (ytmdesktop2 integrations)
            let cur_track = ctrl.queue.lock().ok().and_then(|q| q.current().cloned());
            let (discord_on, discord_cid, lfm_on, lfm_key, lfm_sec, lfm_sk) = ctrl
                .library
                .lock()
                .map(|l| {
                    (
                        l.discord_rpc_enabled,
                        l.discord_client_id.clone(),
                        l.lastfm_enabled,
                        l.lastfm_api_key.clone(),
                        l.lastfm_secret.clone(),
                        l.lastfm_session_key.clone(),
                    )
                })
                .unwrap_or((true, String::new(), false, String::new(), String::new(), String::new()));

            let track_ref = cur_track.as_ref().map(|t| {
                (
                    t.id.as_str(),
                    t.title.as_str(),
                    t.artist.as_str(),
                    t.thumbnail_url.as_deref(),
                )
            });
            discord.update_presence(
                discord_on,
                if discord_cid.trim().is_empty() { None } else { Some(discord_cid.trim()) },
                track_ref,
                snap.is_playing && !snap.is_paused,
                snap.position_secs,
                snap.duration_secs,
            );

            if lfm_on && snap.is_playing && !snap.is_paused {
                if let Some(ref t) = cur_track {
                    if lastfm_now_id != t.id {
                        lastfm_now_id = t.id.clone();
                        let http = ctrl.http.clone();
                        let (k, s, sk, art, tit) = (
                            lfm_key.clone(),
                            lfm_sec.clone(),
                            lfm_sk.clone(),
                            t.artist.clone(),
                            t.title.clone(),
                        );
                        tauri::async_runtime::spawn(async move {
                            lastfm_update_now_playing(&http, &k, &s, &sk, &art, &tit).await;
                        });
                    }
                    let threshold = if snap.duration_secs > 20.0 {
                        (snap.duration_secs * 0.5).min(120.0)
                    } else {
                        30.0
                    };
                    if snap.position_secs >= threshold && lastfm_scrobbled_id != t.id {
                        lastfm_scrobbled_id = t.id.clone();
                        let http = ctrl.http.clone();
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let (k, s, sk, art, tit) =
                            (lfm_key, lfm_sec, lfm_sk, t.artist.clone(), t.title.clone());
                        tauri::async_runtime::spawn(async move {
                            lastfm_scrobble(&http, &k, &s, &sk, &art, &tit, ts).await;
                        });
                    }
                }
            }

            let prev_seq = ctrl.last_track_end_seq.load(Ordering::Relaxed);
            if snap.track_end_seq != prev_seq {
                ctrl.last_track_end_seq
                    .store(snap.track_end_seq, Ordering::Relaxed);
                ctrl.advance_next(true);
            }

            let interval = if snap.is_playing { 300 } else { 500 };
            tokio::time::sleep(Duration::from_millis(interval)).await;
        }
    });
}

pub fn open_or_focus_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        return;
    }

    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("YTM Native")
        .inner_size(1240.0, 780.0)
        .min_inner_size(860.0, 540.0);

    #[cfg(windows)]
    let builder = builder
        .transparent(true)
        .effects(
            tauri::window::EffectsBuilder::new()
                .effect(tauri::window::Effect::MicaDark)
                .build(),
        );

    #[cfg(not(windows))]
    let builder = builder.background_color(tauri::window::Color(3, 3, 3, 255));

    if let Ok(win) = builder.build() {
        attach_close_to_tray(&win);
        let _ = win.set_focus();
    }
}

pub fn attach_close_to_tray(win: &tauri::WebviewWindow) {
    let win_clone = win.clone();
    win.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = win_clone.destroy();
        }
    });
}

pub fn setup_system_tray(app: &AppHandle, ctrl: Arc<AppController>) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", "YTM Native'i Aç", true, None::<&str>)?;
    let toggle_i = MenuItem::with_id(app, "toggle", "Oynat / Duraklat", true, None::<&str>)?;
    let next_i = MenuItem::with_id(app, "next", "Sonraki Parça", true, None::<&str>)?;
    let prev_i = MenuItem::with_id(app, "prev", "Önceki Parça", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show_i, &toggle_i, &next_i, &prev_i, &quit_i])?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;

    let ctrl_menu = Arc::clone(&ctrl);
    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("YouTube Music — Native")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => open_or_focus_main_window(app),
            "toggle" => ctrl_menu.toggle_pause(),
            "next" => ctrl_menu.advance_next(false),
            "prev" => ctrl_menu.step_prev(),
            "quit" => {
                ctrl_menu.audio.stop();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                open_or_focus_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

// --- TAURI COMMANDS ---

#[tauri::command]
pub fn get_playback_state(ctrl: State<'_, Arc<AppController>>) -> PlaybackStateDto {
    ctrl.playback_dto()
}

#[tauri::command]
pub fn get_library(ctrl: State<'_, Arc<AppController>>) -> LibraryData {
    ctrl.library
        .lock()
        .map(|l| l.clone())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn search_songs(
    ctrl: State<'_, Arc<AppController>>,
    query: String,
) -> Result<Vec<Track>, String> {
    // Support shared links (ytmd://, music.youtube.com/watch?v=..., youtu.be/...) directly!
    let effective_query = extract_video_id_from_url(&query).unwrap_or(query);
    ctrl.ytm
        .search_songs(&effective_query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_all(
    ctrl: State<'_, Arc<AppController>>,
    query: String,
) -> Result<SearchBundleDto, String> {
    let effective_query = extract_video_id_from_url(&query).unwrap_or(query);
    ctrl.ytm
        .search_all(&effective_query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_artist(
    ctrl: State<'_, Arc<AppController>>,
    artist_id: String,
    artist_name: Option<String>,
) -> Result<ArtistDetailDto, String> {
    ctrl.ytm
        .fetch_artist(&artist_id, artist_name.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_follow_artist(
    ctrl: State<'_, Arc<AppController>>,
    artist: ArtistSummary,
) -> bool {
    ctrl.library
        .lock()
        .map(|mut l| l.toggle_follow_artist(&artist))
        .unwrap_or(false)
}

#[tauri::command]
pub async fn search_suggestions(
    ctrl: State<'_, Arc<AppController>>,
    query: String,
) -> Result<Vec<String>, String> {
    ctrl.ytm
        .search_suggestions(&query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn fetch_home_feed(
    ctrl: State<'_, Arc<AppController>>,
    exclude_ids: Option<Vec<String>>,
    refresh_nonce: Option<u64>,
) -> Result<HomeFeedDto, String> {
    let local_seeds: Vec<String> = ctrl
        .library
        .lock()
        .ok()
        .map(|l| {
            l.recently_played
                .iter()
                .take(10)
                .chain(l.liked_tracks.iter().take(10))
                .map(|t| t.id.clone())
                .collect()
        })
        .unwrap_or_default();

    let yt_connected = ctrl.ytm.is_youtube_connected().await;
    let exclude = exclude_ids.unwrap_or_default();
    let nonce = refresh_nonce.unwrap_or(0);
    let (yt_history, recommendations) = ctrl.ytm.fetch_home_feed(&local_seeds, &exclude, nonce).await;
    Ok(HomeFeedDto {
        yt_connected,
        yt_history,
        recommendations,
    })
}

/// Instant non-blocking play command (< 2ms return!)
#[tauri::command]
pub fn play_track(ctrl: State<'_, Arc<AppController>>, track: Track) {
    if let Ok(mut q) = ctrl.queue.lock() {
        q.start_with(track.clone());
    }
    ctrl.spawn_radio_refill(track.id.clone());
    ctrl.trigger_play_track(track);
}

#[tauri::command]
pub fn play_collection(
    ctrl: State<'_, Arc<AppController>>,
    tracks: Vec<Track>,
    start_idx: usize,
) {
    let first = ctrl.queue.lock().ok().and_then(|mut q| {
        q.start_with_list(tracks, start_idx);
        q.current().cloned()
    });
    if let Some(track) = first {
        ctrl.trigger_play_track(track);
    }
}

#[tauri::command]
pub fn select_queue_index(ctrl: State<'_, Arc<AppController>>, idx: usize) {
    let selected = ctrl.queue.lock().ok().and_then(|mut q| {
        let t = q.select_index(idx).cloned();
        let needs = q.needs_refill();
        t.map(|track| (track, needs))
    });
    if let Some((track, needs)) = selected {
        if needs {
            ctrl.spawn_radio_refill(track.id.clone());
        }
        ctrl.trigger_play_track(track);
    }
}

/// Hover prefetch: pre-resolves stream URL in the background when user hovers a card!
#[tauri::command]
pub fn prefetch_track(ctrl: State<'_, Arc<AppController>>, track_id: String) {
    let ytm = ctrl.ytm.clone();
    tauri::async_runtime::spawn(async move {
        ytm.prefetch_stream(&track_id).await;
    });
}

#[tauri::command]
pub fn toggle_pause(ctrl: State<'_, Arc<AppController>>) {
    ctrl.toggle_pause();
}

#[tauri::command]
pub fn play_next(ctrl: State<'_, Arc<AppController>>) {
    ctrl.advance_next(false);
}

#[tauri::command]
pub fn play_prev(ctrl: State<'_, Arc<AppController>>) {
    ctrl.step_prev();
}

#[tauri::command]
pub fn cycle_repeat(ctrl: State<'_, Arc<AppController>>) -> RepeatMode {
    ctrl.queue
        .lock()
        .map(|mut q| q.cycle_repeat())
        .unwrap_or(RepeatMode::Off)
}

#[tauri::command]
pub fn toggle_shuffle(ctrl: State<'_, Arc<AppController>>) -> bool {
    let res = ctrl
        .queue
        .lock()
        .map(|mut q| q.toggle_shuffle())
        .unwrap_or(false);
    ctrl.spawn_prefetch_upcoming();
    res
}

#[tauri::command]
pub fn set_sleep_timer(
    ctrl: State<'_, Arc<AppController>>,
    minutes: Option<u32>,
    stop_after_track: bool,
) {
    if let Ok(mut st) = ctrl.sleep_timer.lock() {
        st.stop_after_track = stop_after_track;
        st.deadline = minutes.map(|m| Instant::now() + Duration::from_secs(m as u64 * 60));
    }
}

#[tauri::command]
pub async fn get_lyrics(
    ctrl: State<'_, Arc<AppController>>,
    video_id: String,
    title: Option<String>,
    artist: Option<String>,
) -> Result<Option<LyricsDto>, String> {
    let t = title.unwrap_or_default();
    let a = artist.unwrap_or_default();

    // Try LRCLIB time-synced karaoke lyrics first (like ytmdesktop2!)
    if !t.is_empty() {
        if let Some((synced, body, footer)) = fetch_lrclib_lyrics(&ctrl.http, &t, &a).await {
            return Ok(Some(LyricsDto {
                body,
                footer,
                synced,
            }));
        }
    }

    ctrl.ytm
        .fetch_lyrics(&video_id)
        .await
        .map(|opt| {
            opt.map(|(body, footer)| LyricsDto {
                body,
                footer,
                synced: None,
            })
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_integrations(
    ctrl: State<'_, Arc<AppController>>,
    discord_rpc_enabled: bool,
    discord_client_id: Option<String>,
    obs_api_enabled: bool,
    lastfm_enabled: bool,
    lastfm_api_key: String,
    lastfm_secret: String,
    lastfm_session_key: String,
    theme_preset: String,
) {
    if let Ok(mut lib) = ctrl.library.lock() {
        lib.discord_rpc_enabled = discord_rpc_enabled;
        if let Some(cid) = discord_client_id {
            lib.discord_client_id = cid.trim().to_string();
        }
        lib.obs_api_enabled = obs_api_enabled;
        lib.lastfm_enabled = lastfm_enabled;
        lib.lastfm_api_key = lastfm_api_key.trim().to_string();
        lib.lastfm_secret = lastfm_secret.trim().to_string();
        lib.lastfm_session_key = lastfm_session_key.trim().to_string();
        lib.theme_preset = theme_preset.trim().to_string();
        lib.save();
    }
}

#[tauri::command]
pub fn set_mini_player(app: AppHandle, enabled: bool) {
    if let Some(win) = app.get_webview_window("main") {
        if enabled {
            let _ = win.set_min_size(Some(tauri::Size::Logical(tauri::LogicalSize::new(340.0, 440.0))));
            let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(380.0, 500.0)));
            let _ = win.set_always_on_top(true);
        } else {
            let _ = win.set_always_on_top(false);
            let _ = win.set_min_size(Some(tauri::Size::Logical(tauri::LogicalSize::new(860.0, 540.0))));
            let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(1240.0, 780.0)));
        }
    }
}

#[tauri::command]
pub fn set_always_on_top(app: AppHandle, enabled: bool) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(enabled);
    }
}

#[tauri::command]
pub fn save_custom_css(ctrl: State<'_, Arc<AppController>>, css: String) {
    if let Ok(mut lib) = ctrl.library.lock() {
        lib.custom_css = css;
        lib.save();
    }
}

#[tauri::command]
pub fn mark_intro_seen(ctrl: State<'_, Arc<AppController>>) {
    if let Ok(mut lib) = ctrl.library.lock() {
        if !lib.intro_seen {
            lib.intro_seen = true;
            lib.save();
        }
    }
}

#[tauri::command]
pub fn set_language(ctrl: State<'_, Arc<AppController>>, language: String) {
    if let Ok(mut lib) = ctrl.library.lock() {
        lib.language = language.trim().to_string();
        lib.save();
    }
}

#[tauri::command]
pub fn seek(ctrl: State<'_, Arc<AppController>>, seconds: f64) {
    ctrl.audio.seek(seconds);
}

#[tauri::command]
pub fn set_volume(ctrl: State<'_, Arc<AppController>>, volume: f32) {
    ctrl.audio.set_volume(volume);
    if let Ok(mut lib) = ctrl.library.lock() {
        lib.volume = volume.clamp(0.0, 1.0);
        lib.save();
    }
}

#[tauri::command]
pub fn toggle_like(ctrl: State<'_, Arc<AppController>>, track: Track) -> bool {
    ctrl.library
        .lock()
        .map(|mut l| l.toggle_like(&track))
        .unwrap_or(false)
}

#[tauri::command]
pub fn create_playlist(ctrl: State<'_, Arc<AppController>>, name: String) -> String {
    ctrl.library
        .lock()
        .map(|mut l| l.create_playlist(name))
        .unwrap_or_default()
}

#[tauri::command]
pub fn delete_playlist(ctrl: State<'_, Arc<AppController>>, playlist_id: String) {
    if let Ok(mut l) = ctrl.library.lock() {
        l.delete_playlist(&playlist_id);
    }
}

#[tauri::command]
pub fn add_to_playlist(
    ctrl: State<'_, Arc<AppController>>,
    playlist_id: String,
    track: Track,
) -> bool {
    ctrl.library
        .lock()
        .map(|mut l| l.add_track_to_playlist(&playlist_id, &track))
        .unwrap_or(false)
}

#[tauri::command]
pub fn remove_from_playlist(
    ctrl: State<'_, Arc<AppController>>,
    playlist_id: String,
    track_id: String,
) {
    if let Ok(mut l) = ctrl.library.lock() {
        l.remove_track_from_playlist(&playlist_id, &track_id);
    }
}

/// Re-sync liked songs, playlists and artists from the YouTube account (startup + "Eşitle" button).
#[tauri::command]
pub async fn sync_youtube_account(
    ctrl: State<'_, Arc<AppController>>,
) -> Result<AccountSyncReport, String> {
    run_account_sync(&ctrl).await
}

fn play_list_now(ctrl: &Arc<AppController>, tracks: Vec<Track>) -> usize {
    let n = tracks.len();
    let first = ctrl.queue.lock().ok().and_then(|mut q| {
        q.start_with_list(tracks, 0);
        q.current().cloned()
    });
    if let Some(track) = first {
        ctrl.trigger_play_track(track);
    }
    n
}

/// "Mix başlat" for a song: the song followed by its (personalized) radio.
#[tauri::command]
pub async fn start_mix(ctrl: State<'_, Arc<AppController>>, track: Track) -> Result<usize, String> {
    let radio = ctrl.ytm.fetch_radio(&track.id).await.unwrap_or_default();
    let mut list = vec![track];
    list.extend(radio);
    Ok(play_list_now(&ctrl, list))
}

/// "Mix başlat" for an artist: YouTube's artist radio.
#[tauri::command]
pub async fn start_artist_mix(
    ctrl: State<'_, Arc<AppController>>,
    artist_id: String,
    artist_name: Option<String>,
) -> Result<usize, String> {
    let list = ctrl
        .ytm
        .fetch_artist_mix(&artist_id, artist_name.as_deref())
        .await
        .map_err(|e| format!("Mix başlatılamadı: {e}"))?;
    if list.is_empty() {
        return Err("Bu sanatçı için mix bulunamadı".to_string());
    }
    Ok(play_list_now(&ctrl, list))
}

#[tauri::command]
pub async fn connect_yt_browser(
    ctrl: State<'_, Arc<AppController>>,
    browser: String,
) -> Result<(), String> {
    let cookie = ctrl
        .ytm
        .connect_from_browser(&browser)
        .await
        .map_err(|e| e.to_string())?;
    if let Ok(mut lib) = ctrl.library.lock() {
        lib.auth_cookie = Some(cookie);
        lib.save();
    }
    if let Err(e) = run_account_sync(&ctrl).await {
        warn!("Account sync after connect failed: {e}");
    }
    Ok(())
}

#[tauri::command]
pub async fn connect_yt_cookies(
    ctrl: State<'_, Arc<AppController>>,
    raw: String,
) -> Result<(), String> {
    let cookie = ctrl
        .ytm
        .connect_with_cookies(&raw)
        .await
        .map_err(|e| e.to_string())?;
    if let Ok(mut lib) = ctrl.library.lock() {
        lib.auth_cookie = Some(cookie);
        lib.save();
    }
    if let Err(e) = run_account_sync(&ctrl).await {
        warn!("Account sync after connect failed: {e}");
    }
    Ok(())
}

/// Opens a native Google Sign-In window pointing straight to YouTube Music.
/// As soon as Google finishes login and redirects to music.youtube.com with SAPISID cookies,
/// extracts the session cookies directly from WebView2, destroys the popup, and syncs YouTube Music!
#[tauri::command]
pub async fn open_google_login(
    app: AppHandle,
    ctrl: State<'_, Arc<AppController>>,
) -> Result<bool, String> {
    if let Some(existing) = app.get_webview_window("google-login") {
        let _ = existing.set_focus();
        return Ok(false);
    }

    let login_url: tauri::Url = "https://accounts.google.com/ServiceLogin?service=youtube&uilel=3&passive=true&continue=https%3A%2F%2Fmusic.youtube.com%2F"
        .parse()
        .map_err(|e| format!("{e}"))?;

    let login_win = WebviewWindowBuilder::new(&app, "google-login", WebviewUrl::External(login_url))
        .title("Google ile Oturum Aç — YouTube Music")
        .inner_size(500.0, 680.0)
        .center()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("Oturum açma penceresi açılamadı: {e}"))?;

    let ytm_url: tauri::Url = "https://music.youtube.com/"
        .parse()
        .map_err(|e| format!("{e}"))?;

    // Poll until login completes (redirected to music.youtube.com with auth cookies) or window closed.
    loop {
        tokio::time::sleep(Duration::from_millis(600)).await;

        let Some(win) = app.get_webview_window("google-login") else {
            return Ok(false);
        };

        // Only the final music.youtube.com page counts. Intermediate hops such as
        // accounts.youtube.com also contain "youtube.com" but do not have the full session yet.
        let on_ytm = win
            .url()
            .ok()
            .and_then(|u| u.host_str().map(|h| h == "music.youtube.com"))
            .unwrap_or(false);

        if on_ytm {
            if let Ok(cookies) = win.cookies_for_url(ytm_url.clone()) {
                let has_auth = cookies.iter().any(|c| {
                    let n = c.name();
                    n == "SAPISID" || n == "__Secure-3PAPISID" || n == "__Secure-1PAPISID"
                });
                if has_auth {
                    let cookie_header = cookies
                        .iter()
                        .map(|c| format!("{}={}", c.name(), c.value()))
                        .collect::<Vec<_>>()
                        .join("; ");

                    let _ = login_win.destroy();

                    let saved_cookie = ctrl
                        .ytm
                        .connect_with_cookies(&cookie_header)
                        .await
                        .map_err(|e| format!("YouTube oturumu doğrulanamadı: {e}"))?;

                    if let Ok(mut lib) = ctrl.library.lock() {
                        lib.auth_cookie = Some(saved_cookie);
                        lib.save();
                    }

                    if let Err(e) = run_account_sync(&ctrl).await {
                        warn!("Account sync after Google login failed: {e}");
                    }
                    return Ok(true);
                }
            }
        }
    }
}

#[tauri::command]
pub async fn disconnect_yt(ctrl: State<'_, Arc<AppController>>) -> Result<(), String> {
    if let Ok(mut lib) = ctrl.library.lock() {
        lib.auth_cookie = None;
        lib.save();
    }
    ctrl.ytm.disconnect_youtube().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_update(
    ctrl: State<'_, Arc<AppController>>,
    repo: String,
) -> Result<Option<UpdateInfo>, String> {
    let trimmed = repo.trim().to_string();
    if let Ok(mut lib) = ctrl.library.lock() {
        lib.update_repo = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.clone())
        };
        lib.save();
    }
    updater::check_for_update(&ctrl.http, &trimmed)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    ctrl: State<'_, Arc<AppController>>,
    info: UpdateInfo,
) -> Result<(), String> {
    updater::download_and_install(&ctrl.http, &info)
        .await
        .map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}
