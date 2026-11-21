use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{
    self, Align, Color32, ColorImage, CornerRadius, Frame, Key, Layout, Margin, Pos2, Rect,
    RichText, Sense, Stroke, TextureHandle, Vec2,
};
use tokio::runtime::Runtime;
use tracing::{debug, warn};

use crate::audio::AudioEngine;
use crate::media::{MediaAction, SystemMedia};
use crate::model::{format_seconds, ResolvedStream, Track};
use crate::queue::PlayQueue;
use crate::storage::LibraryData;
use crate::updater::{self, UpdateInfo, CURRENT_VERSION, DEFAULT_GITHUB_REPO};
use crate::ytm::YtmClient;

/// 96 cached 160x160 RGBA thumbnails (~100 KB max each -> ~9 MB cap, zero flicker/thrashing).
const MAX_THUMBNAILS: usize = 96;
const MAX_CONCURRENT_THUMB_FETCH: usize = 8;

// Official YouTube Music Pitch-Black Palette
const BG_BLACK: Color32 = Color32::from_rgb(3, 3, 3);
const BG_ELEVATED: Color32 = Color32::from_rgb(29, 29, 29);
const BG_CHIP: Color32 = Color32::from_rgb(26, 26, 26);
const BG_CHIP_HOVER: Color32 = Color32::from_rgb(44, 44, 44);
const BG_PLAYER: Color32 = Color32::from_rgb(33, 33, 33);
const BORDER_SUBTLE: Color32 = Color32::from_rgb(38, 38, 38);
const BORDER_CHIP: Color32 = Color32::from_rgb(54, 54, 54);
const YT_RED: Color32 = Color32::from_rgb(255, 0, 51);
const TEXT_WHITE: Color32 = Color32::from_rgb(255, 255, 255);
const TEXT_MUTED: Color32 = Color32::from_rgb(170, 170, 170);
const TEXT_DIM: Color32 = Color32::from_rgb(122, 122, 122);

const MOOD_CHIPS: &[(&str, &str)] = &[
    ("Enerjik", "enerjik müzik mix"),
    ("Keyifli", "keyifli pop şarkılar"),
    ("Rahatlama", "rahatlatıcı chill müzik"),
    ("Antrenman", "workout gym music"),
    ("İşe gidip gelme", "yol şarkıları mix"),
    ("Hüzünlü", "hüzünlü akustik şarkılar"),
    ("Parti", "parti dans hitleri"),
    ("Romantik", "romantik slow şarkılar"),
    ("Odaklanma", "odaklanma lofi instrumental"),
    ("Uyku", "uyku ambient müzik"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActivePage {
    Home,
    Explore,
    Library,
    Search,
    LikedSongs,
    Playlist(String),
    Account,
    Updates,
}

#[derive(Debug, Clone)]
enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Available(UpdateInfo),
    Downloading(String),
    Error(String),
}

enum AsyncEvent {
    SearchDone {
        query: String,
        result: Result<Vec<Track>, String>,
    },
    MoodLoaded {
        mood: String,
        tracks: Vec<Track>,
    },
    HomeFeedLoaded {
        yt_history: Vec<Track>,
        recommendations: Vec<Track>,
    },
    YtAuthStatus {
        connected: bool,
        message: Option<String>,
        error: Option<String>,
    },
    YtLikedLoaded(Vec<Track>),
    StreamResolved {
        track: Track,
        result: Result<ResolvedStream, String>,
    },
    RadioLoaded {
        seed_id: String,
        tracks: Vec<Track>,
    },
    ThumbnailLoaded {
        url: String,
        image: Option<ColorImage>,
    },
    UpdateChecked(Result<Option<UpdateInfo>, String>),
    UpdateInstallResult(Result<(), String>),
}

pub struct YtmApp {
    rt: Arc<Runtime>,
    ytm: YtmClient,
    audio: AudioEngine,
    queue: PlayQueue,
    media: SystemMedia,
    library: LibraryData,

    // Channels
    event_tx: Sender<AsyncEvent>,
    event_rx: Receiver<AsyncEvent>,
    media_rx: Receiver<MediaAction>,

    // Navigation & UI state
    page: ActivePage,
    sidebar_expanded: bool,
    show_queue_panel: bool,
    search_input: String,
    focus_search_next: bool,
    is_searching: bool,
    search_results: Vec<Track>,

    // Mood chips & carousel scroll offsets
    active_mood: Option<String>,
    mood_tracks: Vec<Track>,
    loading_mood: bool,
    recent_scroll_idx: usize,
    rec_scroll_idx: usize,

    // Home feed & YouTube Account state
    yt_connected: bool,
    yt_auth_busy: bool,
    yt_auth_msg: Option<String>,
    yt_auth_err: Option<String>,
    cookie_input: String,
    yt_history: Vec<Track>,
    recommendations: Vec<Track>,
    loading_home: bool,

    // Library playlist creation
    creating_playlist: bool,
    new_playlist_name: String,

    // Updater state (VDS-free GitHub Releases)
    repo_input: String,
    update_status: UpdateStatus,

    // Status banners
    status_message: Option<String>,
    error_message: Option<String>,
    active_stream_meta: Option<String>,

    // Playback tracking
    last_track_end_seq: u64,
    last_media_playing: Option<bool>,
    volume: f32,
    seeking_preview: Option<f64>,
    prefetched_id: Option<String>,
    radio_in_flight: bool,

    // Flicker-free LRU thumbnail cache
    frame_tick: u64,
    thumbnails: HashMap<String, (TextureHandle, u64)>,
    thumb_in_flight: HashSet<String>,
    thumb_failed: HashSet<String>,
    http_client: reqwest::Client,
}

impl YtmApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        configure_ytm_theme(&cc.egui_ctx);

        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("ytm-async")
                .enable_all()
                .build()?,
        );

        let library = LibraryData::load();
        let ytm = YtmClient::new()?;
        let audio = AudioEngine::new()?;
        let volume = library.volume.clamp(0.0, 1.0);
        audio.set_volume(volume);

        let (event_tx, event_rx) = mpsc::channel();
        let (media_tx, media_rx) = mpsc::channel();

        let hwnd = extract_hwnd(cc);
        let wakeup_ctx = cc.egui_ctx.clone();
        let media = SystemMedia::new(
            hwnd,
            media_tx,
            Box::new(move || wakeup_ctx.request_repaint()),
        );

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .pool_max_idle_per_host(4)
            .build()?;

        let repo_input = library
            .update_repo
            .clone()
            .unwrap_or_else(|| DEFAULT_GITHUB_REPO.to_string());

        let mut app = Self {
            rt,
            ytm,
            audio,
            queue: PlayQueue::new(),
            media,
            library,
            event_tx,
            event_rx,
            media_rx,
            page: ActivePage::Home,
            sidebar_expanded: true,
            show_queue_panel: false,
            search_input: String::new(),
            focus_search_next: false,
            is_searching: false,
            search_results: Vec::new(),
            active_mood: None,
            mood_tracks: Vec::new(),
            loading_mood: false,
            recent_scroll_idx: 0,
            rec_scroll_idx: 0,
            yt_connected: false,
            yt_auth_busy: false,
            yt_auth_msg: None,
            yt_auth_err: None,
            cookie_input: String::new(),
            yt_history: Vec::new(),
            recommendations: Vec::new(),
            loading_home: false,
            creating_playlist: false,
            new_playlist_name: String::new(),
            repo_input: repo_input.clone(),
            update_status: UpdateStatus::Idle,
            status_message: None,
            error_message: None,
            active_stream_meta: None,
            last_track_end_seq: 0,
            last_media_playing: None,
            volume,
            seeking_preview: None,
            prefetched_id: None,
            radio_in_flight: false,
            frame_tick: 0,
            thumbnails: HashMap::with_capacity(MAX_THUMBNAILS),
            thumb_in_flight: HashSet::new(),
            thumb_failed: HashSet::new(),
            http_client,
        };

        app.check_yt_session(&cc.egui_ctx);
        app.refresh_home_feed(&cc.egui_ctx);

        if !repo_input.trim().is_empty() {
            app.check_updates(&cc.egui_ctx);
        }

        Ok(app)
    }

    fn select_mood(&mut self, mood: &str, query: &str, ctx: &egui::Context) {
        if self.active_mood.as_deref() == Some(mood) {
            self.active_mood = None;
            self.mood_tracks.clear();
            return;
        }
        self.active_mood = Some(mood.to_string());
        self.loading_mood = true;
        self.rec_scroll_idx = 0;

        let mood_owned = mood.to_string();
        let query_owned = query.to_string();
        let ytm = self.ytm.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            let tracks = ytm.search_songs(&query_owned).await.unwrap_or_default();
            let _ = tx.send(AsyncEvent::MoodLoaded {
                mood: mood_owned,
                tracks,
            });
            repaint.request_repaint();
        });
    }

    fn check_yt_session(&mut self, ctx: &egui::Context) {
        let ytm = self.ytm.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();
        self.rt.spawn(async move {
            let connected = ytm.is_youtube_connected().await;
            let _ = tx.send(AsyncEvent::YtAuthStatus {
                connected,
                message: None,
                error: None,
            });
            if connected {
                if let Ok(liked) = ytm.fetch_yt_liked().await {
                    let _ = tx.send(AsyncEvent::YtLikedLoaded(liked));
                }
            }
            repaint.request_repaint();
        });
    }

    fn connect_yt_browser(&mut self, browser: &str, ctx: &egui::Context) {
        if self.yt_auth_busy {
            return;
        }
        self.yt_auth_busy = true;
        self.yt_auth_err = None;
        self.yt_auth_msg = Some(format!("{browser} tarayıcısından oturum alınıyor..."));

        let browser_owned = browser.to_string();
        let ytm = self.ytm.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            match ytm.connect_from_browser(&browser_owned).await {
                Ok(()) => {
                    let _ = tx.send(AsyncEvent::YtAuthStatus {
                        connected: true,
                        message: Some(format!("✓ {browser_owned} ile YouTube Music bağlandı!")),
                        error: None,
                    });
                    if let Ok(liked) = ytm.fetch_yt_liked().await {
                        let _ = tx.send(AsyncEvent::YtLikedLoaded(liked));
                    }
                }
                Err(e) => {
                    let _ = tx.send(AsyncEvent::YtAuthStatus {
                        connected: false,
                        message: None,
                        error: Some(e.to_string()),
                    });
                }
            }
            repaint.request_repaint();
        });
    }

    fn connect_yt_cookies(&mut self, ctx: &egui::Context) {
        if self.yt_auth_busy {
            return;
        }
        let raw = self.cookie_input.trim().to_string();
        if raw.is_empty() {
            self.yt_auth_err = Some("Lütfen çerez (cookie) metnini yapıştır.".into());
            return;
        }
        self.yt_auth_busy = true;
        self.yt_auth_err = None;
        self.yt_auth_msg = Some("YouTube çerezi doğrulanıyor...".into());

        let ytm = self.ytm.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            match ytm.connect_with_cookies(&raw).await {
                Ok(()) => {
                    let _ = tx.send(AsyncEvent::YtAuthStatus {
                        connected: true,
                        message: Some("✓ YouTube Music hesabın bağlandı!".into()),
                        error: None,
                    });
                    if let Ok(liked) = ytm.fetch_yt_liked().await {
                        let _ = tx.send(AsyncEvent::YtLikedLoaded(liked));
                    }
                }
                Err(e) => {
                    let _ = tx.send(AsyncEvent::YtAuthStatus {
                        connected: false,
                        message: None,
                        error: Some(e.to_string()),
                    });
                }
            }
            repaint.request_repaint();
        });
    }

    fn disconnect_yt(&mut self, ctx: &egui::Context) {
        let ytm = self.ytm.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();
        self.yt_history.clear();
        self.rt.spawn(async move {
            let _ = ytm.disconnect_youtube().await;
            let _ = tx.send(AsyncEvent::YtAuthStatus {
                connected: false,
                message: Some("YouTube hesap bağlantısı kaldırıldı.".into()),
                error: None,
            });
            repaint.request_repaint();
        });
    }

    fn refresh_home_feed(&mut self, ctx: &egui::Context) {
        if self.loading_home {
            return;
        }
        self.loading_home = true;
        let local_recent: Vec<String> = self
            .library
            .recently_played
            .iter()
            .take(3)
            .map(|t| t.id.clone())
            .collect();
        let ytm = self.ytm.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            let (yt_history, recommendations) = ytm.fetch_home_feed(&local_recent).await;
            let _ = tx.send(AsyncEvent::HomeFeedLoaded {
                yt_history,
                recommendations,
            });
            repaint.request_repaint();
        });
    }

    fn check_updates(&mut self, ctx: &egui::Context) {
        let repo = self.repo_input.trim().to_string();
        if repo.is_empty() {
            self.update_status =
                UpdateStatus::Error("Önce GitHub deposunu gir (örn: kullanici/ytm-native)".into());
            return;
        }
        self.update_status = UpdateStatus::Checking;
        let client = self.http_client.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            let res = updater::check_for_update(&client, &repo)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AsyncEvent::UpdateChecked(res));
            repaint.request_repaint();
        });
    }

    fn install_update(&mut self, info: UpdateInfo, ctx: &egui::Context) {
        self.update_status = UpdateStatus::Downloading(info.version.clone());
        let client = self.http_client.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            let res = updater::download_and_install(&client, &info)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AsyncEvent::UpdateInstallResult(res));
            repaint.request_repaint();
        });
    }

    fn trigger_search(&mut self, ctx: &egui::Context) {
        let query = self.search_input.trim().to_string();
        if query.is_empty() {
            return;
        }
        self.page = ActivePage::Search;
        self.is_searching = true;
        self.error_message = None;
        self.status_message = Some(format!("Aranıyor: \"{query}\"..."));

        let ytm = self.ytm.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            let result = ytm.search_songs(&query).await.map_err(|e| e.to_string());
            let _ = tx.send(AsyncEvent::SearchDone { query, result });
            repaint.request_repaint();
        });
    }

    fn play_track_with_radio(&mut self, track: Track, ctx: &egui::Context) {
        self.queue.start_with(track.clone());
        self.prefetched_id = None;
        self.start_playing_track(track.clone(), ctx);
        self.request_radio_for(&track.id, ctx);
    }

    fn play_collection(&mut self, tracks: Vec<Track>, start_idx: usize, ctx: &egui::Context) {
        if tracks.is_empty() {
            return;
        }
        self.queue.start_with_list(tracks, start_idx);
        self.prefetched_id = None;
        if let Some(current) = self.queue.current().cloned() {
            self.start_playing_track(current, ctx);
            self.maybe_prefetch_next();
        }
    }

    fn start_playing_track(&mut self, track: Track, ctx: &egui::Context) {
        self.error_message = None;
        self.status_message = Some(format!("Hazırlanıyor: {} — {}", track.artist, track.title));
        self.library.record_played(&track);
        self.media.set_track(&track);

        let ytm = self.ytm.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            let result = ytm.resolve_stream(&track.id).await.map_err(|e| e.to_string());
            let _ = tx.send(AsyncEvent::StreamResolved { track, result });
            repaint.request_repaint();
        });
    }

    fn request_radio_for(&mut self, seed_id: &str, ctx: &egui::Context) {
        if self.radio_in_flight {
            return;
        }
        self.radio_in_flight = true;
        let seed = seed_id.to_string();
        let ytm = self.ytm.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            match ytm.fetch_radio(&seed).await {
                Ok(tracks) => {
                    let _ = tx.send(AsyncEvent::RadioLoaded {
                        seed_id: seed,
                        tracks,
                    });
                }
                Err(e) => {
                    warn!("Radio fetch failed for {seed}: {e}");
                    let _ = tx.send(AsyncEvent::RadioLoaded {
                        seed_id: seed,
                        tracks: Vec::new(),
                    });
                }
            }
            repaint.request_repaint();
        });
    }

    fn maybe_prefetch_next(&mut self) {
        let Some(next) = self.queue.peek_next() else {
            return;
        };
        if self.prefetched_id.as_deref() == Some(next.id.as_str()) {
            return;
        }
        let next_id = next.id.clone();
        self.prefetched_id = Some(next_id.clone());
        let ytm = self.ytm.clone();
        self.rt.spawn(async move {
            debug!("Prefetching next track stream: {next_id}");
            ytm.prefetch_stream(&next_id).await;
        });
    }

    fn play_next(&mut self, ctx: &egui::Context) {
        if let Some(next) = self.queue.advance_next().cloned() {
            self.start_playing_track(next.clone(), ctx);
            self.maybe_prefetch_next();
            if self.queue.needs_refill() {
                self.request_radio_for(&next.id, ctx);
            }
        }
    }

    fn play_prev(&mut self, ctx: &egui::Context) {
        let snap = self.audio.snapshot();
        if snap.position_secs > 3.0 {
            self.audio.seek(0.0);
            return;
        }
        if let Some(prev) = self.queue.step_prev().cloned() {
            self.start_playing_track(prev, ctx);
            self.maybe_prefetch_next();
        }
    }

    /// Flicker-free thumbnail lookup + bounded async fetch.
    fn get_or_fetch_thumb(&mut self, url: &str, ctx: &egui::Context) -> Option<egui::TextureId> {
        if let Some((handle, last_used)) = self.thumbnails.get_mut(url) {
            *last_used = self.frame_tick;
            return Some(handle.id());
        }
        if self.thumb_failed.contains(url)
            || self.thumb_in_flight.contains(url)
            || self.thumb_in_flight.len() >= MAX_CONCURRENT_THUMB_FETCH
        {
            return None;
        }

        self.thumb_in_flight.insert(url.to_string());
        let url_owned = url.to_string();
        let client = self.http_client.clone();
        let tx = self.event_tx.clone();
        let repaint = ctx.clone();

        self.rt.spawn(async move {
            let mut decoded: Option<ColorImage> = None;
            if let Ok(resp) = client.get(&url_owned).send().await {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        if let Ok(img) = image::load_from_memory(&bytes) {
                            let small = img.thumbnail(160, 160).to_rgba8();
                            let size = [small.width() as usize, small.height() as usize];
                            decoded = Some(ColorImage::from_rgba_unmultiplied(size, small.as_raw()));
                        }
                    }
                }
            }
            let _ = tx.send(AsyncEvent::ThumbnailLoaded {
                url: url_owned,
                image: decoded,
            });
            repaint.request_repaint();
        });
        None
    }

    /// Render a square cover art with optional centered white play triangle overlay (like official YTM).
    fn draw_ytm_cover(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        url: Option<&str>,
        size: f32,
        rounding: u8,
        show_play_overlay: bool,
        is_active: bool,
    ) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
        let tex_id = url.and_then(|u| self.get_or_fetch_thumb(u, ctx));
        let painter = ui.painter();

        if let Some(id) = tex_id {
            painter.image(
                id,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
            // Subtle rounded frame mask border
            painter.rect_stroke(
                rect,
                CornerRadius::same(rounding),
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 14)),
                egui::StrokeKind::Inside,
            );
        } else {
            painter.rect_filled(rect, CornerRadius::same(rounding), Color32::from_rgb(18, 18, 18));
            painter.rect_stroke(
                rect,
                CornerRadius::same(rounding),
                Stroke::new(1.0_f32, Color32::from_rgb(32, 32, 32)),
                egui::StrokeKind::Inside,
            );
        }

        // Centered white play triangle overlay (exact official YouTube Music style!)
        if show_play_overlay && (resp.hovered() || is_active || tex_id.is_none()) {
            if resp.hovered() || is_active {
                painter.rect_filled(
                    rect,
                    CornerRadius::same(rounding),
                    Color32::from_rgba_unmultiplied(0, 0, 0, 105),
                );
            }
            let c = rect.center();
            let r = (size * 0.11).clamp(8.0, 18.0);
            let tri = vec![
                Pos2::new(c.x - r * 0.55, c.y - r * 0.85),
                Pos2::new(c.x - r * 0.55, c.y + r * 0.85),
                Pos2::new(c.x + r * 0.95, c.y),
            ];
            painter.add(egui::Shape::convex_polygon(
                tri,
                if is_active { YT_RED } else { Color32::WHITE },
                Stroke::NONE,
            ));
        }

        resp
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                AsyncEvent::SearchDone { query, result } => {
                    self.is_searching = false;
                    match result {
                        Ok(tracks) => {
                            self.status_message = None;
                            self.search_results = tracks;
                            if self.search_results.is_empty() {
                                self.status_message = Some(format!("\"{query}\" için sonuç bulunamadı."));
                            }
                        }
                        Err(e) => {
                            self.error_message = Some(format!("Arama hatası: {e}"));
                            self.status_message = None;
                        }
                    }
                }
                AsyncEvent::MoodLoaded { mood, tracks } => {
                    if self.active_mood.as_deref() == Some(mood.as_str()) {
                        self.loading_mood = false;
                        self.mood_tracks = tracks;
                    }
                }
                AsyncEvent::HomeFeedLoaded {
                    yt_history,
                    recommendations,
                } => {
                    self.loading_home = false;
                    if !yt_history.is_empty() {
                        self.yt_history = yt_history;
                    }
                    if !recommendations.is_empty() {
                        self.recommendations = recommendations;
                    }
                }
                AsyncEvent::YtAuthStatus {
                    connected,
                    message,
                    error,
                } => {
                    self.yt_auth_busy = false;
                    let newly_connected = connected && !self.yt_connected;
                    self.yt_connected = connected;
                    if message.is_some() {
                        self.yt_auth_msg = message;
                    }
                    self.yt_auth_err = error;
                    if newly_connected {
                        self.cookie_input.clear();
                        self.refresh_home_feed(ctx);
                    }
                }
                AsyncEvent::YtLikedLoaded(yt_liked) => {
                    let mut changed = false;
                    for t in yt_liked.into_iter().rev() {
                        if !self.library.is_liked(&t.id) {
                            self.library.liked_tracks.insert(0, t);
                            changed = true;
                        }
                    }
                    if changed {
                        self.library.save();
                    }
                }
                AsyncEvent::StreamResolved { track, result } => {
                    if self.queue.current().map(|t| t.id.as_str()) != Some(track.id.as_str()) {
                        continue;
                    }
                    match result {
                        Ok(stream) => {
                            let kbps = stream.bitrate / 1000;
                            self.active_stream_meta = Some(format!("AAC {kbps}k"));
                            self.status_message = None;
                            self.error_message = None;
                            self.audio.play(stream);
                            self.maybe_prefetch_next();
                        }
                        Err(e) => {
                            self.error_message = Some(e);
                            self.status_message = None;
                        }
                    }
                }
                AsyncEvent::RadioLoaded { seed_id, tracks } => {
                    self.radio_in_flight = false;
                    if !tracks.is_empty() {
                        debug!("Loaded {} radio tracks for {seed_id}", tracks.len());
                        self.queue.append_radio(tracks);
                        self.maybe_prefetch_next();
                    }
                }
                AsyncEvent::ThumbnailLoaded { url, image } => {
                    self.thumb_in_flight.remove(&url);
                    if let Some(color_img) = image {
                        if self.thumbnails.len() >= MAX_THUMBNAILS {
                            if let Some(oldest_key) = self
                                .thumbnails
                                .iter()
                                .min_by_key(|(_, (_, tick))| *tick)
                                .map(|(k, _)| k.clone())
                            {
                                self.thumbnails.remove(&oldest_key);
                            }
                        }
                        let handle =
                            ctx.load_texture(&url, color_img, egui::TextureOptions::LINEAR);
                        self.thumbnails.insert(url, (handle, self.frame_tick));
                    } else {
                        self.thumb_failed.insert(url);
                    }
                }
                AsyncEvent::UpdateChecked(res) => match res {
                    Ok(Some(info)) => self.update_status = UpdateStatus::Available(info),
                    Ok(None) => self.update_status = UpdateStatus::UpToDate,
                    Err(e) => self.update_status = UpdateStatus::Error(e),
                },
                AsyncEvent::UpdateInstallResult(res) => match res {
                    Ok(()) => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Err(e) => {
                        self.update_status = UpdateStatus::Error(format!("Kurulum hatası: {e}"));
                    }
                },
            }
        }

        while let Ok(action) = self.media_rx.try_recv() {
            match action {
                MediaAction::Play => self.audio.resume(),
                MediaAction::Pause => self.audio.pause(),
                MediaAction::Toggle => self.audio.toggle_pause(),
                MediaAction::Next => self.play_next(ctx),
                MediaAction::Previous => self.play_prev(ctx),
                MediaAction::Stop => self.audio.stop(),
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context, snap: &crate::audio::AudioSnapshot) {
        let typing = ctx.wants_keyboard_input();
        ctx.input(|i| {
            if i.modifiers.ctrl && i.key_pressed(Key::K) {
                self.focus_search_next = true;
                return;
            }
            if typing {
                return;
            }
            if i.key_pressed(Key::Slash) {
                self.focus_search_next = true;
            } else if i.key_pressed(Key::Space) {
                self.audio.toggle_pause();
            } else if i.modifiers.ctrl && i.key_pressed(Key::ArrowRight) {
                self.play_next(ctx);
            } else if i.modifiers.ctrl && i.key_pressed(Key::ArrowLeft) {
                self.play_prev(ctx);
            } else if i.key_pressed(Key::ArrowRight) {
                let target = (snap.position_secs + 5.0).min(snap.duration_secs.max(0.0));
                self.audio.seek(target);
            } else if i.key_pressed(Key::ArrowLeft) {
                let target = (snap.position_secs - 5.0).max(0.0);
                self.audio.seek(target);
            } else if i.key_pressed(Key::ArrowUp) {
                self.volume = (self.volume + 0.05).min(1.0);
                self.audio.set_volume(self.volume);
                self.library.volume = self.volume;
            } else if i.key_pressed(Key::ArrowDown) {
                self.volume = (self.volume - 0.05).max(0.0);
                self.audio.set_volume(self.volume);
                self.library.volume = self.volume;
            }
        });
    }

    /// Official YouTube Music Left Sidebar (`#030303` + `Music` logo + Nav + `+ Yeni oynatma listesi` + Playlists)
    fn render_sidebar(&mut self, ctx: &egui::Context) {
        let width = if self.sidebar_expanded { 240.0 } else { 72.0 };
        egui::SidePanel::left("ytm_sidebar")
            .resizable(false)
            .exact_width(width)
            .frame(
                Frame::NONE
                    .fill(BG_BLACK)
                    .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
                    .inner_margin(Margin::symmetric(12, 14)),
            )
            .show(ctx, |ui| {
                // Top Brand Row: Hamburger + Official Red Circle Music Logo + "Music"
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("☰").size(18.0).color(TEXT_WHITE))
                                .frame(false),
                        )
                        .clicked()
                    {
                        self.sidebar_expanded = !self.sidebar_expanded;
                    }
                    if self.sidebar_expanded {
                        ui.add_space(10.0);
                        draw_ytm_logo(ui);
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Music")
                                .strong()
                                .size(20.0)
                                .color(TEXT_WHITE),
                        );
                    }
                });

                ui.add_space(20.0);

                // Primary 3 Navigation Items (Ana Sayfa, Keşfet, Kitaplık)
                if ytm_nav_row(
                    ui,
                    "🏠",
                    "Ana Sayfa",
                    self.page == ActivePage::Home,
                    self.sidebar_expanded,
                ) {
                    self.page = ActivePage::Home;
                }
                ui.add_space(2.0);
                if ytm_nav_row(
                    ui,
                    "🧭",
                    "Keşfet",
                    self.page == ActivePage::Explore,
                    self.sidebar_expanded,
                ) {
                    self.page = ActivePage::Explore;
                }
                ui.add_space(2.0);
                if ytm_nav_row(
                    ui,
                    "📚",
                    "Kitaplık",
                    self.page == ActivePage::Library,
                    self.sidebar_expanded,
                ) {
                    self.page = ActivePage::Library;
                }

                if !self.sidebar_expanded {
                    return;
                }

                ui.add_space(14.0);
                // Hairline divider
                let (div_rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(div_rect, 0.0, BORDER_SUBTLE);
                ui.add_space(14.0);

                // "+ Yeni oynatma listesi" pill button (exact YTM style)
                let new_pl_btn = egui::Button::new(
                    RichText::new("+   Yeni oynatma listesi")
                        .size(13.5)
                        .strong()
                        .color(TEXT_WHITE),
                )
                .fill(BG_ELEVATED)
                .corner_radius(CornerRadius::same(18))
                .min_size(Vec2::new(ui.available_width(), 36.0));
                if ui.add(new_pl_btn).clicked() {
                    self.creating_playlist = !self.creating_playlist;
                    self.new_playlist_name.clear();
                }

                if self.creating_playlist {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.new_playlist_name)
                                .hint_text("Liste adı...")
                                .desired_width(142.0),
                        );
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Oluştur").small().color(TEXT_WHITE))
                                    .fill(YT_RED)
                                    .corner_radius(CornerRadius::same(8)),
                            )
                            .clicked()
                            || enter
                        {
                            let name = std::mem::take(&mut self.new_playlist_name);
                            let id = self.library.create_playlist(name);
                            self.creating_playlist = false;
                            self.page = ActivePage::Playlist(id);
                        }
                    });
                }

                ui.add_space(14.0);

                // Scrollable playlists list (Beğendiğim müzikler + Custom playlists)
                let avail_h = (ui.available_height() - 40.0).max(80.0);
                let playlists_meta: Vec<(String, String, usize)> = self
                    .library
                    .playlists
                    .iter()
                    .map(|p| (p.id.clone(), p.name.clone(), p.tracks.len()))
                    .collect();

                egui::ScrollArea::vertical()
                    .id_salt("ytm_sidebar_playlists")
                    .max_height(avail_h)
                    .show(ui, |ui| {
                        // Beğendiğim müzikler
                        if ytm_playlist_sidebar_item(
                            ui,
                            "Beğendiğim müzikler",
                            &format!("📌 Otomatik oynatma listesi • {}", self.library.liked_tracks.len()),
                            self.page == ActivePage::LikedSongs,
                        ) {
                            self.page = ActivePage::LikedSongs;
                        }
                        ui.add_space(6.0);

                        for (id, name, count) in playlists_meta {
                            let active = self.page == ActivePage::Playlist(id.clone());
                            if ytm_playlist_sidebar_item(
                                ui,
                                &name,
                                &format!("Oynatma listesi • {count} parça"),
                                active,
                            ) {
                                self.page = ActivePage::Playlist(id);
                            }
                            ui.add_space(6.0);
                        }
                    });

                // Minimal bottom updater link
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    let (upd_text, upd_col) = match &self.update_status {
                        UpdateStatus::Available(info) => (format!("⬆ {} Güncelleme Hazır", info.version), YT_RED),
                        UpdateStatus::Checking => ("⟳ Güncelleme kontrolü...".into(), TEXT_DIM),
                        UpdateStatus::Downloading(v) => (format!("⬇ İndiriliyor {v}..."), YT_RED),
                        _ => (format!("v{CURRENT_VERSION} • Güncelleme"), TEXT_DIM),
                    };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(upd_text).small().color(upd_col))
                                .frame(false),
                        )
                        .clicked()
                    {
                        self.page = ActivePage::Updates;
                    }
                });
            });
    }

    /// Official YouTube Music Top Header Bar (Search Input + Queue + Account Avatar)
    fn render_top_header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("ytm_top_header")
            .exact_height(64.0)
            .frame(
                Frame::NONE
                    .fill(BG_BLACK)
                    .inner_margin(Margin::symmetric(28, 12)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // Search bar pill (exact YTM dark charcoal box)
                    let search_w = (ui.available_width() - 180.0).clamp(240.0, 480.0);
                    Frame::NONE
                        .fill(Color32::from_rgb(33, 33, 33))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(56, 56, 56)))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.set_width(search_w);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("🔍").size(15.0).color(TEXT_MUTED));
                                let edit = egui::TextEdit::singleline(&mut self.search_input)
                                    .frame(false)
                                    .hint_text(
                                        RichText::new("Şarkı, albüm, sanatçı veya podcast arayın")
                                            .color(TEXT_MUTED),
                                    )
                                    .desired_width(search_w - 34.0);
                                let resp = ui.add(edit);
                                if self.focus_search_next {
                                    resp.request_focus();
                                    self.focus_search_next = false;
                                }
                                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                    self.trigger_search(ctx);
                                }
                            });
                        });

                    // Right side: Account Avatar + Queue toggle
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Circular Account Avatar (exact YTM top-right circle)
                        let (av_rect, av_resp) =
                            ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                        let av_fill = if self.yt_connected {
                            YT_RED
                        } else {
                            Color32::from_rgb(120, 120, 120)
                        };
                        ui.painter().circle_filled(av_rect.center(), 14.0, av_fill);
                        if self.yt_connected {
                            ui.painter().text(
                                av_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "✓",
                                egui::FontId::proportional(13.0),
                                Color32::WHITE,
                            );
                        }
                        let av_resp = av_resp.on_hover_text(if self.yt_connected {
                            "YouTube Hesabı Bağlı (Ayarlar)"
                        } else {
                            "YouTube Hesabı Bağla (Geçmiş & Öneriler)"
                        });
                        if av_resp.clicked() {
                            self.page = ActivePage::Account;
                        }

                        ui.add_space(12.0);

                        // Subtle Queue toggle button
                        let q_col = if self.show_queue_panel {
                            YT_RED
                        } else {
                            TEXT_MUTED
                        };
                        if ui
                            .add(
                                egui::Button::new(RichText::new("☰ Sıra").small().color(q_col))
                                    .fill(BG_CHIP)
                                    .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
                                    .corner_radius(CornerRadius::same(14)),
                            )
                            .clicked()
                        {
                            self.show_queue_panel = !self.show_queue_panel;
                        }
                    });
                });
            });
    }

    fn render_queue_panel(&mut self, ctx: &egui::Context) {
        if !self.show_queue_panel {
            return;
        }
        egui::SidePanel::right("ytm_queue")
            .resizable(false)
            .exact_width(290.0)
            .frame(
                Frame::NONE
                    .fill(BG_BLACK)
                    .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
                    .inner_margin(Margin::symmetric(14, 14)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("SIRADAKİLER")
                            .strong()
                            .size(13.0)
                            .color(TEXT_WHITE),
                    );
                    if self.radio_in_flight {
                        ui.label(RichText::new("• radyo").small().color(YT_RED));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(RichText::new("✕").color(TEXT_MUTED)).frame(false))
                            .clicked()
                        {
                            self.show_queue_panel = false;
                        }
                    });
                });
                ui.add_space(10.0);

                let queue_tracks = self.queue.tracks().to_vec();
                let current_idx = self.queue.current_index();
                let mut select_idx: Option<usize> = None;

                egui::ScrollArea::vertical()
                    .id_salt("ytm_queue_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if queue_tracks.is_empty() {
                            ui.label(
                                RichText::new("Bir şarkı başlattığında benzer parçalar burada sıralanır.")
                                    .small()
                                    .color(TEXT_MUTED),
                            );
                        }
                        for (idx, track) in queue_tracks.iter().enumerate() {
                            let is_cur = current_idx == Some(idx);
                            let bg = if is_cur { BG_ELEVATED } else { Color32::TRANSPARENT };
                            let resp = Frame::NONE
                                .fill(bg)
                                .corner_radius(CornerRadius::same(6))
                                .inner_margin(Margin::symmetric(6, 5))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        self.draw_ytm_cover(
                                            ui,
                                            ctx,
                                            track.thumbnail_url.as_deref(),
                                            38.0,
                                            4,
                                            false,
                                            is_cur,
                                        );
                                        ui.vertical(|ui| {
                                            ui.label(
                                                RichText::new(truncate_str(&track.title, 24))
                                                    .strong()
                                                    .size(13.0)
                                                    .color(if is_cur { YT_RED } else { TEXT_WHITE }),
                                            );
                                            ui.label(
                                                RichText::new(truncate_str(&track.artist, 26))
                                                    .small()
                                                    .color(TEXT_MUTED),
                                            );
                                        });
                                    });
                                })
                                .response
                                .interact(Sense::click());

                            if resp.clicked() {
                                select_idx = Some(idx);
                            }
                            ui.add_space(2.0);
                        }
                    });

                if let Some(idx) = select_idx {
                    if let Some(track) = self.queue.select_index(idx).cloned() {
                        self.start_playing_track(track.clone(), ctx);
                        self.maybe_prefetch_next();
                        if self.queue.needs_refill() {
                            self.request_radio_for(&track.id, ctx);
                        }
                    }
                }
            });
    }

    /// Official YouTube Music Bottom Bar (Slim Red Seek Bar at top edge + Left Transport + Center Track + Right Volume)
    fn render_player_bar(&mut self, ctx: &egui::Context, snap: &crate::audio::AudioSnapshot) {
        let has_track = self.queue.current().is_some();
        if !has_track && !snap.is_playing && !snap.is_buffering {
            return;
        }

        egui::TopBottomPanel::bottom("ytm_player_bar")
            .exact_height(74.0)
            .frame(Frame::NONE.fill(BG_PLAYER).inner_margin(Margin::ZERO))
            .show(ctx, |ui| {
                // Full-width thin YouTube Red seek bar along the very top edge
                let dur = snap
                    .duration_secs
                    .max(
                        self.queue
                            .current()
                            .and_then(|t| t.duration_secs)
                            .unwrap_or(0) as f64,
                    )
                    .max(1.0);
                let pos = self.seeking_preview.unwrap_or(snap.position_secs).min(dur);
                let frac = (pos / dur).clamp(0.0, 1.0) as f32;

                let (bar_rect, bar_resp) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 6.0),
                    Sense::click_and_drag(),
                );
                let track_rect = Rect::from_min_max(
                    Pos2::new(bar_rect.left(), bar_rect.top()),
                    Pos2::new(bar_rect.right(), bar_rect.top() + 3.0),
                );
                let fill_rect = Rect::from_min_max(
                    track_rect.min,
                    Pos2::new(
                        track_rect.left() + track_rect.width() * frac,
                        track_rect.bottom(),
                    ),
                );
                let painter = ui.painter();
                painter.rect_filled(track_rect, 0.0, Color32::from_rgb(65, 65, 65));
                painter.rect_filled(fill_rect, 0.0, YT_RED);
                if bar_resp.hovered() || bar_resp.dragged() {
                    painter.circle_filled(
                        Pos2::new(fill_rect.right(), track_rect.center().y),
                        6.0,
                        YT_RED,
                    );
                }
                if let Some(pointer) = bar_resp.interact_pointer_pos() {
                    let new_frac =
                        ((pointer.x - bar_rect.left()) / bar_rect.width().max(1.0)).clamp(0.0, 1.0);
                    let target = new_frac as f64 * dur;
                    if bar_resp.dragged() {
                        self.seeking_preview = Some(target);
                    }
                    if bar_resp.drag_stopped() || bar_resp.clicked() {
                        self.seeking_preview = None;
                        self.audio.seek(target);
                    }
                }

                // Controls row
                Frame::NONE
                    .inner_margin(Margin::symmetric(18, 6))
                    .show(ui, |ui| {
                        let total_w = ui.available_width();
                        let side_w = (total_w * 0.28).max(170.0);
                        let center_w = (total_w - side_w * 2.0).max(220.0);

                        ui.horizontal_centered(|ui| {
                            // LEFT: Transport + Time (Official YTM layout!)
                            ui.allocate_ui_with_layout(
                                Vec2::new(side_w, 54.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("⏮").size(18.0).color(TEXT_WHITE),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        self.play_prev(ctx);
                                    }
                                    ui.add_space(10.0);
                                    let icon = if snap.is_buffering {
                                        "⏳"
                                    } else if snap.is_playing {
                                        "⏸"
                                    } else {
                                        "▶"
                                    };
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(icon).size(22.0).color(TEXT_WHITE),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        self.audio.toggle_pause();
                                    }
                                    ui.add_space(10.0);
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("⏭").size(18.0).color(TEXT_WHITE),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        self.play_next(ctx);
                                    }
                                    ui.add_space(14.0);
                                    ui.label(
                                        RichText::new(format!(
                                            "{} / {}",
                                            format_seconds(pos),
                                            format_seconds(dur)
                                        ))
                                        .small()
                                        .color(TEXT_MUTED),
                                    );
                                },
                            );

                            // CENTER: Cover + Title/Artist + Like
                            ui.allocate_ui_with_layout(
                                Vec2::new(center_w, 54.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    if let Some(current) = self.queue.current().cloned() {
                                        self.draw_ytm_cover(
                                            ui,
                                            ctx,
                                            current.thumbnail_url.as_deref(),
                                            44.0,
                                            4,
                                            false,
                                            false,
                                        );
                                        ui.add_space(8.0);
                                        ui.vertical(|ui| {
                                            ui.add_space(6.0);
                                            ui.label(
                                                RichText::new(truncate_str(&current.title, 34))
                                                    .strong()
                                                    .size(14.0)
                                                    .color(TEXT_WHITE),
                                            );
                                            let mut sub = truncate_str(&current.artist, 30);
                                            if let Some(meta) = self.active_stream_meta.as_deref() {
                                                sub.push_str(" • ");
                                                sub.push_str(meta);
                                            }
                                            ui.label(RichText::new(sub).small().color(TEXT_MUTED));
                                        });
                                        ui.add_space(10.0);
                                        let liked = self.library.is_liked(&current.id);
                                        let heart = if liked { "♥" } else { "♡" };
                                        let h_col = if liked { YT_RED } else { TEXT_MUTED };
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new(heart).size(18.0).color(h_col),
                                                )
                                                .frame(false),
                                            )
                                            .clicked()
                                        {
                                            self.library.toggle_like(&current);
                                        }
                                    }
                                },
                            );

                            // RIGHT: Volume + Queue
                            ui.allocate_ui_with_layout(
                                Vec2::new(side_w, 54.0),
                                Layout::right_to_left(Align::Center),
                                |ui| {
                                    ui.style_mut().spacing.slider_width = 88.0;
                                    let vol_resp = ui.add(
                                        egui::Slider::new(&mut self.volume, 0.0..=1.0)
                                            .show_value(false)
                                            .trailing_fill(true),
                                    );
                                    if vol_resp.changed() {
                                        self.audio.set_volume(self.volume);
                                    }
                                    if vol_resp.drag_stopped() {
                                        self.library.volume = self.volume;
                                        self.library.save();
                                    }
                                    ui.label(RichText::new("🔊").color(TEXT_MUTED));
                                },
                            );
                        });
                    });
            });
    }

    /// Official YouTube Music Home Page:
    /// Mood Chips Row + "Yeniden dinleyin" horizontal carousel + "Sana özel öneriler" horizontal carousel!
    fn render_home_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // 1. Mood Filter Chips Row (Enerjik, Keyifli, Rahatlama, Antrenman, ...)
        let mut mood_clicked: Option<(&'static str, &'static str)> = None;
        egui::ScrollArea::horizontal()
            .id_salt("mood_chips_scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for &(label, query) in MOOD_CHIPS {
                        let is_selected = self.active_mood.as_deref() == Some(label);
                        let fill = if is_selected { TEXT_WHITE } else { BG_CHIP };
                        let text_col = if is_selected { BG_BLACK } else { TEXT_WHITE };
                        let btn = egui::Button::new(
                            RichText::new(label).size(13.0).color(text_col),
                        )
                        .fill(fill)
                        .stroke(Stroke::new(1.0_f32, BORDER_CHIP))
                        .corner_radius(CornerRadius::same(8))
                        .min_size(Vec2::new(0.0, 32.0));
                        if ui.add(btn).clicked() {
                            mood_clicked = Some((label, query));
                        }
                        ui.add_space(4.0);
                    }
                });
            });

        if let Some((label, query)) = mood_clicked {
            self.select_mood(label, query, ctx);
        }

        ui.add_space(24.0);

        // If a mood chip is active, show its tracks cleanly at the top!
        if let Some(mood) = self.active_mood.clone() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{mood} müzikleri"))
                        .strong()
                        .size(26.0)
                        .color(TEXT_WHITE),
                );
                if self.loading_mood {
                    ui.label(RichText::new("• yükleniyor...").color(YT_RED));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Filtreyi temizle").small().color(TEXT_WHITE))
                                .fill(BG_CHIP)
                                .stroke(Stroke::new(1.0_f32, BORDER_CHIP))
                                .corner_radius(CornerRadius::same(14)),
                        )
                        .clicked()
                    {
                        self.active_mood = None;
                        self.mood_tracks.clear();
                    }
                });
            });
            ui.add_space(14.0);
            let m_tracks = self.mood_tracks.clone();
            self.render_card_carousel(ui, ctx, &m_tracks, &mut self.rec_scroll_idx.clone());
            ui.add_space(28.0);
        }

        // 2. "Yeniden dinleyin" Section (YouTube History if connected, else Local History)
        let recent_tracks: Vec<Track> = if !self.yt_history.is_empty() {
            self.yt_history.clone()
        } else {
            self.library.recently_played.clone()
        };

        if !recent_tracks.is_empty() {
            let eyebrow = if !self.yt_history.is_empty() {
                "YOUTUBE GEÇMİŞİN"
            } else {
                "SON DİNLEDİKLERİN"
            };
            let max_idx = recent_tracks.len().saturating_sub(1);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(eyebrow).small().color(TEXT_MUTED));
                    ui.label(
                        RichText::new("Yeniden dinleyin")
                            .strong()
                            .size(28.0)
                            .color(TEXT_WHITE),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if carousel_arrow_btn(ui, "›", self.recent_scroll_idx + 1 < recent_tracks.len())
                    {
                        self.recent_scroll_idx = (self.recent_scroll_idx + 2).min(max_idx);
                    }
                    ui.add_space(6.0);
                    if carousel_arrow_btn(ui, "‹", self.recent_scroll_idx > 0) {
                        self.recent_scroll_idx = self.recent_scroll_idx.saturating_sub(2);
                    }
                });
            });
            ui.add_space(14.0);

            let mut idx = self.recent_scroll_idx;
            self.render_card_carousel(ui, ctx, &recent_tracks, &mut idx);
            self.recent_scroll_idx = idx;
            ui.add_space(34.0);
        }

        // 3. "Sana özel öneriler" / YouTube Recommendations Carousel
        let recs = self.recommendations.clone();
        let rec_title = if !self.yt_history.is_empty() {
            "YouTube geçmişine göre öneriler"
        } else if !self.library.recently_played.is_empty() {
            "Sana özel öneriler"
        } else {
            "Hızlı seçimler"
        };

        let rec_max = recs.len().saturating_sub(1);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                if self.loading_home {
                    ui.label(RichText::new("YÜKLENİYOR...").small().color(YT_RED));
                } else {
                    ui.label(RichText::new("SENİN İÇİN SEÇTİKLERİMİZ").small().color(TEXT_MUTED));
                }
                ui.label(
                    RichText::new(rec_title)
                        .strong()
                        .size(28.0)
                        .color(TEXT_WHITE),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if carousel_arrow_btn(ui, "›", self.rec_scroll_idx + 1 < recs.len()) {
                    self.rec_scroll_idx = (self.rec_scroll_idx + 2).min(rec_max);
                }
                ui.add_space(6.0);
                if carousel_arrow_btn(ui, "‹", self.rec_scroll_idx > 0) {
                    self.rec_scroll_idx = self.rec_scroll_idx.saturating_sub(2);
                }
                ui.add_space(10.0);
                if ui
                    .add(
                        egui::Button::new(RichText::new("Diğer").small().color(TEXT_WHITE))
                            .fill(BG_BLACK)
                            .stroke(Stroke::new(1.0_f32, BORDER_CHIP))
                            .corner_radius(CornerRadius::same(16))
                            .min_size(Vec2::new(62.0, 30.0)),
                    )
                    .clicked()
                {
                    self.refresh_home_feed(ctx);
                }
            });
        });
        ui.add_space(14.0);

        let mut r_idx = self.rec_scroll_idx;
        self.render_card_carousel(ui, ctx, &recs, &mut r_idx);
        self.rec_scroll_idx = r_idx;
    }

    /// Official YouTube Music horizontal borderless album-art card carousel!
    fn render_card_carousel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tracks: &[Track],
        scroll_idx: &mut usize,
    ) {
        if tracks.is_empty() {
            return;
        }
        let card_size = 180.0_f32;
        let gap = 18.0_f32;
        let visible_count = ((ui.available_width() + gap) / (card_size + gap))
            .floor()
            .max(2.0) as usize;

        if *scroll_idx >= tracks.len() {
            *scroll_idx = 0;
        }
        let slice_end = (*scroll_idx + visible_count).min(tracks.len());
        let current_id = self.queue.current().map(|t| t.id.clone());
        let mut clicked_track: Option<Track> = None;

        ui.horizontal_top(|ui| {
            for track in &tracks[*scroll_idx..slice_end] {
                let is_cur = current_id.as_deref() == Some(track.id.as_str());
                ui.allocate_ui_with_layout(
                    Vec2::new(card_size, card_size + 58.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        let cover_resp = self.draw_ytm_cover(
                            ui,
                            ctx,
                            track.thumbnail_url.as_deref(),
                            card_size,
                            6,
                            true,
                            is_cur,
                        );
                        ui.add_space(8.0);
                        let title_resp = ui
                            .add(
                                egui::Label::new(
                                    RichText::new(truncate_str(&track.title, 22))
                                        .strong()
                                        .size(14.0)
                                        .color(if is_cur { YT_RED } else { TEXT_WHITE }),
                                )
                                .sense(Sense::click()),
                            );
                        ui.label(
                            RichText::new(truncate_str(&track.artist, 26))
                                .size(13.0)
                                .color(TEXT_MUTED),
                        );

                        if cover_resp.clicked() || title_resp.clicked() {
                            clicked_track = Some(track.clone());
                        }
                    },
                );
                ui.add_space(gap - 6.0);
            }
        });

        if let Some(t) = clicked_track {
            self.play_track_with_radio(t, ctx);
        }
    }

    fn render_explore_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            RichText::new("Keşfet")
                .strong()
                .size(28.0)
                .color(TEXT_WHITE),
        );
        ui.add_space(16.0);
        ui.label(
            RichText::new("Ruh halleri ve türler")
                .strong()
                .size(18.0)
                .color(TEXT_WHITE),
        );
        ui.add_space(10.0);

        let mut picked: Option<(&'static str, &'static str)> = None;
        for chunk in MOOD_CHIPS.chunks(4) {
            ui.horizontal(|ui| {
                for &(label, query) in chunk {
                    let btn = egui::Button::new(
                        RichText::new(label).strong().size(14.0).color(TEXT_WHITE),
                    )
                    .fill(BG_CHIP)
                    .stroke(Stroke::new(1.0_f32, BORDER_CHIP))
                    .corner_radius(CornerRadius::same(8))
                    .min_size(Vec2::new(165.0, 44.0));
                    if ui.add(btn).clicked() {
                        picked = Some((label, query));
                    }
                    ui.add_space(8.0);
                }
            });
            ui.add_space(8.0);
        }

        if let Some((label, query)) = picked {
            self.page = ActivePage::Home;
            self.select_mood(label, query, ctx);
        }
    }

    fn render_library_overview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            RichText::new("Kitaplık")
                .strong()
                .size(28.0)
                .color(TEXT_WHITE),
        );
        ui.add_space(16.0);

        let mut open_page: Option<ActivePage> = None;
        ui.horizontal_wrapped(|ui| {
            // Liked songs card
            let liked_resp = Frame::NONE
                .fill(BG_CHIP)
                .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(160.0);
                    ui.set_height(190.0);
                    ui.vertical(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(132.0), Sense::hover());
                        ui.painter()
                            .rect_filled(r, CornerRadius::same(6), Color32::from_rgb(56, 16, 26));
                        ui.painter().text(
                            r.center(),
                            egui::Align2::CENTER_CENTER,
                            "♥",
                            egui::FontId::proportional(44.0),
                            YT_RED,
                        );
                        ui.add_space(8.0);
                        ui.label(RichText::new("Beğendiğim müzikler").strong().color(TEXT_WHITE));
                        ui.label(
                            RichText::new(format!("{} parça", self.library.liked_tracks.len()))
                                .small()
                                .color(TEXT_MUTED),
                        );
                    });
                })
                .response
                .interact(Sense::click());
            if liked_resp.clicked() {
                open_page = Some(ActivePage::LikedSongs);
            }

            let playlists = self.library.playlists.clone();
            for pl in playlists {
                ui.add_space(10.0);
                let first_thumb = pl.tracks.first().and_then(|t| t.thumbnail_url.clone());
                let resp = Frame::NONE
                    .fill(BG_CHIP)
                    .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(160.0);
                        ui.set_height(190.0);
                        ui.vertical(|ui| {
                            self.draw_ytm_cover(ui, ctx, first_thumb.as_deref(), 132.0, 6, false, false);
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(truncate_str(&pl.name, 18))
                                    .strong()
                                    .color(TEXT_WHITE),
                            );
                            ui.label(
                                RichText::new(format!("{} parça", pl.tracks.len()))
                                    .small()
                                    .color(TEXT_MUTED),
                            );
                        });
                    })
                    .response
                    .interact(Sense::click());
                if resp.clicked() {
                    open_page = Some(ActivePage::Playlist(pl.id));
                }
            }
        });

        if let Some(p) = open_page {
            self.page = p;
        }
    }

    fn render_search_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            RichText::new("Arama sonuçları")
                .strong()
                .size(24.0)
                .color(TEXT_WHITE),
        );
        ui.add_space(14.0);
        let tracks = self.search_results.clone();
        if tracks.is_empty() && !self.is_searching {
            ui.label(
                RichText::new("Üstteki arama çubuğuna bir şarkı veya sanatçı yazıp Enter'a bas.")
                    .color(TEXT_MUTED),
            );
            return;
        }
        self.render_track_list(ui, ctx, &tracks, None, false);
    }

    fn render_liked_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let tracks = self.library.liked_tracks.clone();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("OTOMATİK OYNATMA LİSTESİ").small().color(TEXT_MUTED));
                ui.label(
                    RichText::new("Beğendiğim müzikler")
                        .strong()
                        .size(28.0)
                        .color(TEXT_WHITE),
                );
                ui.label(
                    RichText::new(format!("{} parça", tracks.len()))
                        .small()
                        .color(TEXT_MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !tracks.is_empty() {
                    let btn = egui::Button::new(
                        RichText::new("▶  Oynat").strong().color(BG_BLACK),
                    )
                    .fill(TEXT_WHITE)
                    .corner_radius(CornerRadius::same(18))
                    .min_size(Vec2::new(104.0, 36.0));
                    if ui.add(btn).clicked() {
                        self.play_collection(tracks.clone(), 0, ctx);
                    }
                }
            });
        });
        ui.add_space(18.0);
        if tracks.is_empty() {
            ui.label(
                RichText::new("Henüz beğendiğin şarkı yok. ♡ simgesiyle beğen veya YouTube hesabını bağla.")
                    .color(TEXT_MUTED),
            );
            return;
        }
        self.render_track_list(ui, ctx, &tracks, None, true);
    }

    fn render_playlist_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, playlist_id: &str) {
        let Some(pl) = self.library.playlists.iter().find(|p| p.id == playlist_id).cloned() else {
            self.page = ActivePage::Home;
            return;
        };
        let mut delete_pl = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("OYNATMA LİSTESİ").small().color(TEXT_MUTED));
                ui.label(RichText::new(&pl.name).strong().size(28.0).color(TEXT_WHITE));
                ui.label(
                    RichText::new(format!("{} parça", pl.tracks.len()))
                        .small()
                        .color(TEXT_MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("Sil").small().color(YT_RED))
                            .fill(BG_CHIP)
                            .stroke(Stroke::new(1.0_f32, BORDER_CHIP))
                            .corner_radius(CornerRadius::same(16)),
                    )
                    .clicked()
                {
                    delete_pl = true;
                }
                ui.add_space(8.0);
                if !pl.tracks.is_empty() {
                    let btn = egui::Button::new(
                        RichText::new("▶  Oynat").strong().color(BG_BLACK),
                    )
                    .fill(TEXT_WHITE)
                    .corner_radius(CornerRadius::same(18))
                    .min_size(Vec2::new(104.0, 36.0));
                    if ui.add(btn).clicked() {
                        self.play_collection(pl.tracks.clone(), 0, ctx);
                    }
                }
            });
        });

        if delete_pl {
            self.library.delete_playlist(playlist_id);
            self.page = ActivePage::Home;
            return;
        }

        ui.add_space(18.0);
        if pl.tracks.is_empty() {
            ui.label(
                RichText::new("Bu liste boş. Arama ekranından '+' ile parça ekleyebilirsin.")
                    .color(TEXT_MUTED),
            );
            return;
        }
        self.render_track_list(ui, ctx, &pl.tracks, Some(playlist_id), true);
    }

    fn render_account_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            RichText::new("YouTube Music Hesabı")
                .strong()
                .size(26.0)
                .color(TEXT_WHITE),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new("Hesabını bağladığında 'Yeniden dinleyin' ve öneriler doğrudan YouTube geçmişinden çekilir.")
                .color(TEXT_MUTED),
        );
        ui.add_space(16.0);

        Frame::NONE
            .fill(BG_CHIP)
            .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.yt_connected {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("● YouTube Music Bağlı").strong().color(YT_RED));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Bağlantıyı Kes").clicked() {
                                self.disconnect_yt(ctx);
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Geçmişi Şimdi Eşitle").color(BG_BLACK),
                                    )
                                    .fill(TEXT_WHITE)
                                    .corner_radius(CornerRadius::same(14)),
                                )
                                .clicked()
                            {
                                self.refresh_home_feed(ctx);
                            }
                        });
                    });
                } else {
                    ui.label(RichText::new("Tarayıcıdan Tek Tıkla Bağla").strong().color(TEXT_WHITE));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for browser in ["chrome", "edge", "firefox", "brave", "opera"] {
                            let label = match browser {
                                "chrome" => "Chrome",
                                "edge" => "Edge",
                                "firefox" => "Firefox",
                                "brave" => "Brave",
                                _ => "Opera",
                            };
                            if ui
                                .add_enabled(
                                    !self.yt_auth_busy,
                                    egui::Button::new(RichText::new(label).strong().color(BG_BLACK))
                                        .fill(TEXT_WHITE)
                                        .corner_radius(CornerRadius::same(16))
                                        .min_size(Vec2::new(78.0, 32.0)),
                                )
                                .clicked()
                            {
                                self.connect_yt_browser(browser, ctx);
                            }
                        }
                    });

                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(12.0);
                    ui.label(RichText::new("Veya Çerez (Cookie) Yapıştır").strong().color(TEXT_WHITE));
                    ui.add_space(6.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.cookie_input)
                            .desired_rows(3)
                            .desired_width(ui.available_width())
                            .hint_text("SAPISID=...; veya Netscape cookies.txt"),
                    );
                    ui.add_space(8.0);
                    if ui
                        .add_enabled(
                            !self.yt_auth_busy,
                            egui::Button::new(RichText::new("Bağlan").strong().color(TEXT_WHITE))
                                .fill(YT_RED)
                                .corner_radius(CornerRadius::same(16))
                                .min_size(Vec2::new(110.0, 32.0)),
                        )
                        .clicked()
                    {
                        self.connect_yt_cookies(ctx);
                    }
                }

                if let Some(msg) = self.yt_auth_msg.as_ref() {
                    ui.add_space(10.0);
                    ui.label(RichText::new(msg).color(TEXT_WHITE));
                }
                if let Some(err) = self.yt_auth_err.as_ref() {
                    ui.add_space(10.0);
                    ui.label(RichText::new(format!("⚠ {err}")).color(YT_RED));
                }
            });
    }

    fn render_updates_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            RichText::new("Güncelleme Sistemi (GitHub Releases)")
                .strong()
                .size(24.0)
                .color(TEXT_WHITE),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("Mevcut sürüm: v{CURRENT_VERSION} • VDS gerektirmez."))
                .color(TEXT_MUTED),
        );
        ui.add_space(16.0);

        Frame::NONE
            .fill(BG_CHIP)
            .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.repo_input)
                            .hint_text("kullanici/ytm-native")
                            .desired_width(280.0),
                    );
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Kaydet ve Kontrol Et").strong().color(BG_BLACK),
                            )
                            .fill(TEXT_WHITE)
                            .corner_radius(CornerRadius::same(16)),
                        )
                        .clicked()
                    {
                        let trimmed = self.repo_input.trim().to_string();
                        self.library.update_repo = if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed)
                        };
                        self.library.save();
                        self.check_updates(ctx);
                    }
                });
                ui.add_space(12.0);
                match self.update_status.clone() {
                    UpdateStatus::Idle => {
                        ui.label(RichText::new("Yeni sürüm için repoya v0.2.0 etiketi atman yeterli.").small().color(TEXT_MUTED));
                    }
                    UpdateStatus::Checking => {
                        ui.label(RichText::new("⟳ Kontrol ediliyor...").color(TEXT_WHITE));
                    }
                    UpdateStatus::UpToDate => {
                        ui.label(RichText::new("✓ Uygulama güncel!").color(TEXT_WHITE));
                    }
                    UpdateStatus::Downloading(v) => {
                        ui.label(RichText::new(format!("⬇ {v} indiriliyor...")).color(YT_RED));
                    }
                    UpdateStatus::Error(e) => {
                        ui.label(RichText::new(format!("⚠ {e}")).color(YT_RED));
                    }
                    UpdateStatus::Available(info) => {
                        ui.label(
                            RichText::new(format!("Yeni sürüm hazır: {}", info.version))
                                .strong()
                                .color(TEXT_WHITE),
                        );
                        ui.add_space(8.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("⬆ İndir, Güncelle ve Yeniden Başlat")
                                        .strong()
                                        .color(TEXT_WHITE),
                                )
                                .fill(YT_RED)
                                .corner_radius(CornerRadius::same(18)),
                            )
                            .clicked()
                        {
                            self.install_update(info, ctx);
                        }
                    }
                }
            });
    }

    fn render_track_list(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tracks: &[Track],
        playlist_ctx: Option<&str>,
        play_as_collection: bool,
    ) {
        let current_id = self.queue.current().map(|t| t.id.clone());
        let playlists_meta: Vec<(String, String)> = self
            .library
            .playlists
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();

        let mut play_idx: Option<usize> = None;
        let mut like_toggle: Option<Track> = None;
        let mut add_to_pl: Option<(String, Track)> = None;
        let mut remove_from_pl: Option<(String, String)> = None;

        for (idx, track) in tracks.iter().enumerate() {
            let is_cur = current_id.as_deref() == Some(track.id.as_str());
            let is_liked = self.library.is_liked(&track.id);

            Frame::NONE
                .fill(if is_cur { BG_ELEVATED } else { Color32::TRANSPARENT })
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let cover = self.draw_ytm_cover(
                            ui,
                            ctx,
                            track.thumbnail_url.as_deref(),
                            44.0,
                            4,
                            true,
                            is_cur,
                        );
                        let info_resp = ui
                            .vertical(|ui| {
                                ui.add_space(3.0);
                                ui.label(
                                    RichText::new(&track.title)
                                        .strong()
                                        .size(14.0)
                                        .color(if is_cur { YT_RED } else { TEXT_WHITE }),
                                );
                                ui.label(
                                    RichText::new(&track.artist).size(13.0).color(TEXT_MUTED),
                                );
                            })
                            .response
                            .interact(Sense::click());

                        if cover.clicked() || info_resp.clicked() {
                            play_idx = Some(idx);
                        }

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(track.duration_string())
                                    .small()
                                    .color(TEXT_MUTED),
                            );
                            ui.add_space(10.0);

                            if let Some(pl_id) = playlist_ctx {
                                if ui
                                    .add(
                                        egui::Button::new(RichText::new("✕").color(TEXT_MUTED))
                                            .frame(false),
                                    )
                                    .clicked()
                                {
                                    remove_from_pl = Some((pl_id.to_string(), track.id.clone()));
                                }
                            } else if !playlists_meta.is_empty() {
                                ui.menu_button(RichText::new("+").color(TEXT_MUTED), |ui| {
                                    for (pl_id, pl_name) in &playlists_meta {
                                        if ui.button(pl_name).clicked() {
                                            add_to_pl = Some((pl_id.clone(), track.clone()));
                                            ui.close_menu();
                                        }
                                    }
                                });
                            }

                            let heart = if is_liked { "♥" } else { "♡" };
                            let h_col = if is_liked { YT_RED } else { TEXT_MUTED };
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(heart).color(h_col))
                                        .frame(false),
                                )
                                .clicked()
                            {
                                like_toggle = Some(track.clone());
                            }
                        });
                    });
                });
            // Subtle hairline between rows (exact YTM list look)
            let (line_r, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
            ui.painter()
                .rect_filled(line_r, 0.0, Color32::from_rgb(24, 24, 24));
        }

        if let Some(t) = like_toggle {
            self.library.toggle_like(&t);
        }
        if let Some((pl_id, t)) = add_to_pl {
            if self.library.add_track_to_playlist(&pl_id, &t) {
                self.status_message = Some(format!("\"{}\" listeye eklendi.", t.title));
            }
        }
        if let Some((pl_id, tid)) = remove_from_pl {
            self.library.remove_track_from_playlist(&pl_id, &tid);
        }
        if let Some(idx) = play_idx {
            if play_as_collection {
                self.play_collection(tracks.to_vec(), idx, ctx);
            } else {
                self.play_track_with_radio(tracks[idx].clone(), ctx);
            }
        }
    }
}

impl eframe::App for YtmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_tick = self.frame_tick.wrapping_add(1);
        self.drain_events(ctx);

        let snap = self.audio.snapshot();

        if let Some(err) = snap.last_error.as_ref() {
            self.error_message = Some(err.clone());
        }

        if snap.track_end_seq != self.last_track_end_seq {
            self.last_track_end_seq = snap.track_end_seq;
            self.play_next(ctx);
        }

        let stopped = !snap.is_playing && !snap.is_paused && !snap.is_buffering;
        if self.last_media_playing != Some(snap.is_playing) {
            self.media.set_playing(snap.is_playing, stopped);
            self.last_media_playing = Some(snap.is_playing);
        }

        self.handle_shortcuts(ctx, &snap);

        // ZERO CPU when paused/idle
        if snap.is_playing || snap.is_buffering || self.is_searching {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        self.render_player_bar(ctx, &snap);
        self.render_sidebar(ctx);
        self.render_top_header(ctx);
        self.render_queue_panel(ctx);

        egui::CentralPanel::default()
            .frame(
                Frame::NONE
                    .fill(BG_BLACK)
                    .inner_margin(Margin::symmetric(36, 16)),
            )
            .show(ctx, |ui| {
                if let Some(err) = self.error_message.as_ref() {
                    Frame::NONE
                        .fill(Color32::from_rgb(48, 12, 18))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.label(RichText::new(format!("⚠ {err}")).small().color(TEXT_WHITE));
                        });
                    ui.add_space(10.0);
                } else if let Some(msg) = self.status_message.as_ref() {
                    ui.label(RichText::new(msg).small().color(TEXT_MUTED));
                    ui.add_space(8.0);
                }

                egui::ScrollArea::vertical()
                    .id_salt("ytm_main_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let page = self.page.clone();
                        match page {
                            ActivePage::Home => self.render_home_page(ui, ctx),
                            ActivePage::Explore => self.render_explore_page(ui, ctx),
                            ActivePage::Library => self.render_library_overview(ui, ctx),
                            ActivePage::Search => self.render_search_page(ui, ctx),
                            ActivePage::LikedSongs => self.render_liked_page(ui, ctx),
                            ActivePage::Playlist(id) => self.render_playlist_page(ui, ctx, &id),
                            ActivePage::Account => self.render_account_page(ui, ctx),
                            ActivePage::Updates => self.render_updates_page(ui, ctx),
                        }
                    });
            });
    }
}

/// Custom-painted official YouTube Music circular red play icon (red disc + thin white inner ring + play triangle).
fn draw_ytm_logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    p.circle_filled(c, 12.5, YT_RED);
    p.circle_stroke(c, 8.2, Stroke::new(1.2_f32, Color32::WHITE));
    let tri = vec![
        Pos2::new(c.x - 2.6, c.y - 4.2),
        Pos2::new(c.x - 2.6, c.y + 4.2),
        Pos2::new(c.x + 4.4, c.y),
    ];
    p.add(egui::Shape::convex_polygon(tri, Color32::WHITE, Stroke::NONE));
}

fn ytm_nav_row(ui: &mut egui::Ui, icon: &str, label: &str, active: bool, expanded: bool) -> bool {
    let fill = if active { BG_ELEVATED } else { Color32::TRANSPARENT };
    let text = if expanded {
        format!("{icon}    {label}")
    } else {
        icon.to_string()
    };
    let btn = egui::Button::new(
        RichText::new(text)
            .size(14.5)
            .strong()
            .color(if active { TEXT_WHITE } else { TEXT_MUTED }),
    )
    .fill(fill)
    .corner_radius(CornerRadius::same(8))
    .min_size(Vec2::new(ui.available_width(), 42.0));
    ui.add(btn).clicked()
}

fn ytm_playlist_sidebar_item(ui: &mut egui::Ui, title: &str, subtitle: &str, active: bool) -> bool {
    let resp = Frame::NONE
        .fill(if active { BG_ELEVATED } else { Color32::TRANSPARENT })
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(truncate_str(title, 24))
                        .strong()
                        .size(13.5)
                        .color(TEXT_WHITE),
                );
                ui.label(
                    RichText::new(truncate_str(subtitle, 28))
                        .small()
                        .color(TEXT_DIM),
                );
            });
        })
        .response
        .interact(Sense::click());
    resp.clicked()
}

fn carousel_arrow_btn(ui: &mut egui::Ui, symbol: &str, enabled: bool) -> bool {
    let col = if enabled { TEXT_WHITE } else { Color32::from_rgb(64, 64, 64) };
    let btn = egui::Button::new(RichText::new(symbol).size(18.0).color(col))
        .fill(BG_BLACK)
        .stroke(Stroke::new(
            1.0_f32,
            if enabled { BORDER_CHIP } else { BORDER_SUBTLE },
        ))
        .corner_radius(CornerRadius::same(18))
        .min_size(Vec2::splat(34.0));
    ui.add_enabled(enabled, btn).clicked()
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{collected}…")
    } else {
        collected
    }
}

fn configure_ytm_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = BG_BLACK;
    visuals.panel_fill = BG_BLACK;
    visuals.selection.bg_fill = YT_RED;
    visuals.selection.stroke = Stroke::new(1.0_f32, YT_RED);
    visuals.widgets.inactive.bg_fill = BG_CHIP;
    visuals.widgets.hovered.bg_fill = BG_CHIP_HOVER;
    visuals.widgets.active.bg_fill = YT_RED;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER_SUBTLE);
    ctx.set_visuals(visuals);
}

#[cfg(windows)]
fn extract_hwnd(cc: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = cc.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(windows))]
fn extract_hwnd(_cc: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    None
}
