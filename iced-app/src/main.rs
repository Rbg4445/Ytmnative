//! YTM Native 8 — native iced UI (SimpMusic-style layout) on the shared pure-Rust core.
//!
//! The core (YouTube client, audio engine, queue, library) is compiled straight from ../src,
//! the same files the 7.x Tauri app uses; library.json (likes, playlists, YouTube session)
//! is shared, so switching versions keeps everything.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

#[path = "../../src/audio/mod.rs"]
#[allow(unused_imports)]
mod audio;
#[path = "../../src/lyrics.rs"]
mod lyrics;
#[path = "../../src/media.rs"]
mod media;
#[path = "../../src/model.rs"]
mod model;
#[path = "../../src/queue.rs"]
mod queue;
#[path = "../../src/rpc.rs"]
mod rpc;
#[path = "../../src/storage.rs"]
mod storage;
#[path = "../../src/updater.rs"]
mod updater;
#[path = "../../src/ytm.rs"]
mod ytm;

mod images;
mod player;
mod services;
mod theme;

use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::alignment::{Horizontal, Vertical};
use iced::gradient::Linear;
use iced::widget::{
    button, column, container, horizontal_space, image, mouse_area, opaque, row, scrollable, slider, stack, svg,
    text, text_input, toggler, vertical_space, Space,
};
use iced::{
    keyboard, window, Background, Border, Color, Element, Gradient, Length, Padding, Radians, Shadow, Size,
    Subscription, Task, Theme, Vector,
};

use images::Kind;
use model::{ArtistDetailDto, ArtistSummary, Playlist, SearchBundleDto, Track};
use player::{PlaybackView, Player};
use queue::RepeatMode;
use theme::*;

// ---------- Startup intro ("Powered by iced UI", Mojang Studios style) ----------
const INTRO_FADE_IN: f32 = 0.6;
const INTRO_MIN_HOLD: f32 = 2.0;
const INTRO_FADE_OUT: f32 = 0.9;
const INTRO_MAX_HOLD: f32 = 6.0; // never block the app longer than this, even offline

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

const MOODS: [(&str, &str); 10] = [
    ("Enerjik", "enerjik müzik mix"),
    ("Keyifli", "keyifli pop şarkılar"),
    ("Rahatlama", "rahatlatıcı chill müzik"),
    ("Antrenman", "workout gym music"),
    ("Yol şarkıları", "yol şarkıları mix"),
    ("Hüzünlü", "hüzünlü akustik şarkılar"),
    ("Parti", "parti dans hitleri"),
    ("Romantik", "romantik slow şarkılar"),
    ("Odaklanma", "odaklanma lofi instrumental"),
    ("Uyku", "uyku ambient müzik"),
];

#[derive(Debug, Clone, PartialEq)]
enum Page {
    Home,
    Mix,
    Explore,
    Library,
    Liked,
    Playlist(String),
    Artist(String, String),
    Search,
    Settings,
}

#[derive(Debug, Clone)]
struct LyricsView {
    track_id: String,
    synced: Option<Vec<lyrics::LyricLineDto>>,
    plain: String,
    source: String,
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    IntroFrame(Instant),
    // navigation
    Nav(Page),
    Back,
    OpenArtist(String, String),
    // data
    HomeLoaded(Vec<Track>, Vec<Track>),
    RefreshHome,
    SearchInput(String),
    SearchDebounced(u64),
    SearchSubmit,
    SearchDone(u64, Result<SearchBundleDto, String>),
    ArtistLoaded(String, Result<ArtistDetailDto, String>),
    ImageLoaded(String, Kind, Option<image::Handle>),
    LyricsLoaded(String, Option<LyricsView>),
    // playback
    PlayList(Arc<Vec<Track>>, usize),
    QueueSelect(usize),
    TogglePlay,
    Next,
    Prev,
    SeekDrag(f64),
    SeekRelease,
    Volume(f32),
    VolumeRelease,
    Shuffle,
    Repeat,
    Like(Track),
    LikeCurrent,
    StartMix(Track),
    StartArtistMix(String, String),
    MixReady(String, Result<Vec<Track>, String>),
    Prefetch(String),
    // UI
    Chip(usize),
    LibChip(usize),
    TogglePanel,
    PanelTab(usize),
    ClearBanner,
    // custom (Discord-style) title bar
    TitlePress,
    WinMinimize,
    WinToggleMaximize,
    WinClose,
    WinResized,
    WinMaximized(bool),
    ResizeFrom(Edge),
    Noop,
    // screenshot mode (YTM_SHOT=path, YTM_SHOT_DELAY=ms)
    TakeShot,
    Shot(window::Screenshot),
    // media keys, keyboard & tray
    MediaAttached,
    KeyPressed(keyboard::Key, keyboard::Modifiers),
    // mini player
    ToggleMiniPlayer,
    // playlists
    OpenAddToPlaylist(Track),
    OpenCreatePlaylist,
    ClosePlaylistModal,
    AddTrackToPlaylist(String),
    NewPlaylistInput(String),
    CreatePlaylistConfirm,
    DeletePlaylist(String),
    RemoveFromPlaylist(String, String),
    // settings & integrations
    ToggleDiscordRpc(bool),
    SetDiscordClientId(String),
    ToggleObsApi(bool),
    ToggleLastfm(bool),
    SetLastfmApiKey(String),
    SetLastfmSecret(String),
    SetLastfmSessionKey(String),
    ToggleCloseToTray(bool),
    SetUpdateRepo(String),
    SaveSettings,
    // sleep timer
    SetSleepTimer(Option<u64>),
    // auto updater
    CheckUpdate,
    UpdateChecked(Result<Option<updater::UpdateInfo>, String>),
    InstallUpdate,
    UpdateInstalled(Result<(), String>),
}

struct App {
    player: Arc<Player>,
    pb: PlaybackView,
    last_track_id: Option<String>,

    page: Page,
    history: Vec<Page>,

    home_history: Vec<Track>,
    home_recs: Vec<Track>,
    home_loading: bool,
    home_nonce: u64,

    query: String,
    search_seq: u64,
    search: Option<SearchBundleDto>,
    searching: bool,

    artist: Option<ArtistDetailDto>,
    artist_loading: bool,

    lyrics: Option<LyricsView>,
    images: HashMap<(String, Kind), image::Handle>,
    pending: HashSet<(String, Kind)>,
    art: Option<(String, image::Handle)>,

    seeking: Option<f64>,
    volume: f32,

    // library snapshot (refreshed from the player every ~2 s or after changes)
    liked: Vec<Track>,
    liked_ids: HashSet<String>,
    playlists: Vec<Playlist>,
    artists: Vec<ArtistSummary>,
    recent: Vec<Track>,
    lib_tick: u32,

    chip: usize,
    lib_chip: usize,
    panel_open: bool,
    panel_tab: usize,
    banner: Option<(String, bool)>,

    intro_start: Instant,
    intro_t: f32,
    intro_fade_from: Option<f32>,
    intro_done: bool,
    shot_path: Option<String>,

    maximized: bool,
    last_title_press: Option<Instant>,
    app_icon: image::Handle,

    // tray, mini player, sleep timer
    tray: Option<services::AppTray>,
    close_to_tray: bool,
    mini_mode: bool,
    sleep_deadline: Option<Instant>,

    // playlist modal
    playlist_modal_track: Option<Track>,
    creating_playlist: bool,
    new_playlist_name: String,

    // settings fields (backed by library.json)
    cfg_discord_enabled: bool,
    cfg_discord_client_id: String,
    cfg_obs_enabled: bool,
    cfg_lastfm_enabled: bool,
    cfg_lastfm_key: String,
    cfg_lastfm_secret: String,
    cfg_lastfm_sk: String,
    cfg_update_repo: String,

    // auto updater
    update_checking: bool,
    update_installing: bool,
    update_info: Option<updater::UpdateInfo>,
    update_status: Option<String>,
}

impl App {
    fn new(player: Arc<Player>) -> (Self, Task<Message>) {
        let (
            volume,
            cfg_discord_enabled,
            cfg_discord_client_id,
            cfg_obs_enabled,
            cfg_lastfm_enabled,
            cfg_lastfm_key,
            cfg_lastfm_secret,
            cfg_lastfm_sk,
            cfg_update_repo,
        ) = player
            .library
            .lock()
            .map(|l| {
                (
                    l.volume,
                    l.discord_rpc_enabled,
                    l.discord_client_id.clone(),
                    l.obs_api_enabled,
                    l.lastfm_enabled,
                    l.lastfm_api_key.clone(),
                    l.lastfm_secret.clone(),
                    l.lastfm_session_key.clone(),
                    l.update_repo
                        .clone()
                        .unwrap_or_else(|| updater::DEFAULT_GITHUB_REPO.to_string()),
                )
            })
            .unwrap_or((
                0.85,
                true,
                String::new(),
                true,
                false,
                String::new(),
                String::new(),
                String::new(),
                updater::DEFAULT_GITHUB_REPO.to_string(),
            ));

        let tray = if std::env::var("YTM_TEST_HIDDEN").is_ok() {
            None
        } else {
            services::AppTray::new(APP_ICON_PNG)
        };

        let mut app = Self {
            player,
            pb: PlaybackView::default(),
            last_track_id: None,
            page: Page::Home,
            history: Vec::new(),
            home_history: Vec::new(),
            home_recs: Vec::new(),
            home_loading: true,
            home_nonce: 0,
            query: String::new(),
            search_seq: 0,
            search: None,
            searching: false,
            artist: None,
            artist_loading: false,
            lyrics: None,
            images: HashMap::new(),
            pending: HashSet::new(),
            art: None,
            seeking: None,
            volume,
            liked: Vec::new(),
            liked_ids: HashSet::new(),
            playlists: Vec::new(),
            artists: Vec::new(),
            recent: Vec::new(),
            lib_tick: 0,
            chip: 0,
            lib_chip: 0,
            panel_open: true,
            panel_tab: 1,
            banner: None,
            intro_start: Instant::now(),
            intro_t: 0.0,
            intro_fade_from: None,
            intro_done: false,
            shot_path: std::env::var("YTM_SHOT").ok(),
            maximized: false,
            last_title_press: None,
            app_icon: image::Handle::from_bytes(APP_ICON_PNG),
            tray,
            close_to_tray: true,
            mini_mode: false,
            sleep_deadline: None,
            playlist_modal_track: None,
            creating_playlist: false,
            new_playlist_name: String::new(),
            cfg_discord_enabled,
            cfg_discord_client_id,
            cfg_obs_enabled,
            cfg_lastfm_enabled,
            cfg_lastfm_key,
            cfg_lastfm_secret,
            cfg_lastfm_sk,
            cfg_update_repo,
            update_checking: false,
            update_installing: false,
            update_info: None,
            update_status: None,
        };
        app.refresh_library();
        app.pb = app.player.poll();

        // Attach Windows SMTC / hardware media keys using the real window HWND
        let p_media = Arc::clone(&app.player);
        let attach_media = window::get_latest().and_then(move |id| {
            let p_inner = Arc::clone(&p_media);
            window::run_with_handle(id, move |handle| {
                #[cfg(windows)]
                let hwnd = match handle.as_raw() {
                    raw_window_handle::RawWindowHandle::Win32(h) => {
                        Some(h.hwnd.get() as *mut std::ffi::c_void)
                    }
                    _ => None,
                };
                #[cfg(not(windows))]
                let hwnd = {
                    let _ = handle;
                    None
                };
                p_inner.attach_media(hwnd);
                Message::MediaAttached
            })
        });

        let mut tasks = vec![
            app.load_home(),
            app.on_track_change(),
            app.request_library_images(),
            attach_media,
        ];
        // Test hook: YTM_TEST_PLAY=1 presses play (quietly) 3 s after start to verify real playback.
        if std::env::var("YTM_TEST_PLAY").is_ok() {
            app.player.audio.set_volume(0.05); // saved volume is untouched
            tasks.push(Task::perform(tokio::time::sleep(Duration::from_secs(3)), |_| Message::TogglePlay));
        }
        if let Some(ms) = std::env::var("YTM_SHOT_DELAY").ok().and_then(|v| v.parse::<u64>().ok()) {
            if app.shot_path.is_some() {
                tasks.push(Task::perform(tokio::time::sleep(Duration::from_millis(ms)), |_| Message::TakeShot));
            }
        }
        (app, Task::batch(tasks))
    }

    // ---------- data helpers ----------
    fn refresh_library(&mut self) {
        if let Ok(lib) = self.player.library.lock() {
            self.liked = lib.liked_tracks.clone();
            self.playlists = lib.playlists.clone();
            self.recent = lib.recently_played.clone();
            // followed artists + artists you listen to (like the 7.x sidebar)
            let mut seen = HashSet::new();
            let mut arts = Vec::new();
            for a in &lib.followed_artists {
                if seen.insert(a.name.to_lowercase()) {
                    arts.push(a.clone());
                }
            }
            for t in lib.recently_played.iter().chain(lib.liked_tracks.iter()) {
                let name = t.artist.split(',').next().unwrap_or("").trim().to_string();
                if name.is_empty() || !seen.insert(name.to_lowercase()) {
                    continue;
                }
                arts.push(ArtistSummary {
                    id: t.artist_id.clone().unwrap_or_default(),
                    name,
                    avatar_url: t.thumbnail_url.clone(),
                    subscriber_count: None,
                });
                if arts.len() >= 24 {
                    break;
                }
            }
            self.artists = arts;
        }
        self.liked_ids = self.liked.iter().map(|t| t.id.clone()).collect();
    }

    fn load_home(&mut self) -> Task<Message> {
        self.home_loading = true;
        let ytm = self.player.ytm.clone();
        let seeds: Vec<String> = self
            .recent
            .iter()
            .take(10)
            .chain(self.liked.iter().take(10))
            .map(|t| t.id.clone())
            .collect();
        let exclude: Vec<String> = if self.home_nonce > 0 { self.home_recs.iter().map(|t| t.id.clone()).collect() } else { Vec::new() };
        let nonce = self.home_nonce;
        Task::perform(
            async move { ytm.fetch_home_feed(&seeds, &exclude, nonce).await },
            |(h, r)| Message::HomeLoaded(h, r),
        )
    }

    fn want(&mut self, url: Option<&String>, kind: Kind) -> Option<Task<Message>> {
        let url = url?.clone();
        let key = (url.clone(), kind);
        if self.images.contains_key(&key) || self.pending.contains(&key) {
            return None;
        }
        self.pending.insert(key);
        let http = self.player.http.clone();
        Some(Task::perform(images::load(http, url.clone(), kind), move |h| Message::ImageLoaded(url.clone(), kind, h)))
    }

    fn want_tracks<'a>(&mut self, tracks: impl IntoIterator<Item = &'a Track>, kind: Kind) -> Task<Message> {
        let urls: Vec<Option<String>> = tracks.into_iter().map(|t| t.thumbnail_url.clone()).collect();
        Task::batch(urls.iter().filter_map(|u| self.want(u.as_ref(), kind)))
    }

    fn want_artists<'a>(&mut self, artists: impl IntoIterator<Item = &'a ArtistSummary>) -> Task<Message> {
        let urls: Vec<Option<String>> = artists.into_iter().map(|a| a.avatar_url.clone()).collect();
        Task::batch(urls.iter().filter_map(|u| self.want(u.as_ref(), Kind::Circle)))
    }

    fn request_library_images(&mut self) -> Task<Message> {
        let pls: Vec<Track> = self.playlists.iter().filter_map(|p| p.tracks.first().cloned()).collect();
        let arts = self.artists.clone();
        Task::batch([self.want_tracks(pls.iter(), Kind::Thumb), self.want_artists(arts.iter())])
    }

    /// New current song: load its large artwork, lyrics and the queue thumbnails.
    fn on_track_change(&mut self) -> Task<Message> {
        let Some(cur) = self.pb.current.clone() else { return Task::none() };
        let mut tasks = Vec::new();
        if let Some(url) = cur.thumbnail_url.clone() {
            let id = cur.id.clone();
            let http = self.player.http.clone();
            tasks.push(Task::perform(images::load(http, url, Kind::Art), move |h| {
                Message::ImageLoaded(format!("art:{id}"), Kind::Art, h)
            }));
        }
        let (http, ytm) = (self.player.http.clone(), self.player.ytm.clone());
        let id = cur.id.clone();
        tasks.push(Task::perform(load_lyrics(http, ytm, cur.clone()), move |l| Message::LyricsLoaded(id.clone(), l)));
        tasks.push(self.want(cur.thumbnail_url.as_ref(), Kind::Thumb).unwrap_or_else(Task::none));
        let q = self.pb.queue.clone();
        tasks.push(self.want_tracks(q.iter().take(40), Kind::Thumb));
        Task::batch(tasks)
    }

    fn navigate(&mut self, page: Page) {
        if self.page != page {
            self.history.push(self.page.clone());
            if self.history.len() > 30 {
                self.history.remove(0);
            }
            self.page = page;
        }
    }

    fn banner(&mut self, text: impl Into<String>, error: bool) -> Task<Message> {
        self.banner = Some((text.into(), error));
        Task::perform(tokio::time::sleep(Duration::from_secs(if error { 6 } else { 3 })), |_| Message::ClearBanner)
    }

    // ---------- update ----------
    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Tick => {
                self.pb = self.player.poll();
                self.lib_tick += 1;

                // Poll system tray clicks & menu items
                if let Some(ref tray) = self.tray {
                    for action in tray.poll() {
                        match action {
                            services::TrayAction::Show => {
                                return window::get_latest().and_then(|id| {
                                    Task::batch([
                                        window::change_mode(id, window::Mode::Windowed),
                                        window::minimize(id, false),
                                        window::gain_focus(id),
                                    ])
                                });
                            }
                            services::TrayAction::TogglePlay => self.player.toggle_pause(),
                            services::TrayAction::Prev => self.player.prev(),
                            services::TrayAction::Next => self.player.next(false),
                            services::TrayAction::Quit => return iced::exit(),
                        }
                    }
                }

                // Check sleep timer deadline
                if let Some(deadline) = self.sleep_deadline {
                    if Instant::now() >= deadline {
                        self.sleep_deadline = None;
                        self.player.pause();
                        return self.banner("Uyku zamanlayıcısı: Oynatma durduruldu", false);
                    }
                }
                // Test hooks (headless verification): YTM_TEST_LOG=path, YTM_TEST_EXIT=seconds
                if self.lib_tick % 4 == 0 {
                    if let Ok(path) = std::env::var("YTM_TEST_LOG") {
                        use std::io::Write;
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                            let _ = writeln!(
                                f,
                                "t={:>3}s playing={} paused={} buffering={} pos={:.1}/{:.1} title={:?} home_loading={} recs={} err={:?}",
                                self.intro_start.elapsed().as_secs(),
                                self.pb.is_playing,
                                self.pb.is_paused,
                                self.pb.is_buffering,
                                self.pb.position,
                                self.pb.duration,
                                self.pb.current.as_ref().map(|t| t.title.as_str()).unwrap_or("-"),
                                self.home_loading,
                                self.home_recs.len(),
                                self.pb.error
                            );
                        }
                    }
                }
                if let Some(secs) = std::env::var("YTM_TEST_EXIT").ok().and_then(|v| v.parse::<u64>().ok()) {
                    if self.intro_start.elapsed().as_secs() >= secs {
                        return iced::exit();
                    }
                }
                if self.lib_tick % 8 == 0 {
                    self.refresh_library();
                }
                let id = self.pb.current.as_ref().map(|t| t.id.clone());
                if id != self.last_track_id {
                    self.last_track_id = id;
                    return self.on_track_change();
                }
                if self.lib_tick % 8 == 0 {
                    let q = self.pb.queue.clone();
                    return self.want_tracks(q.iter().take(40), Kind::Thumb);
                }
            }
            Message::IntroFrame(now) => {
                self.intro_t = now.duration_since(self.intro_start).as_secs_f32();
                let ready = !self.home_loading || self.intro_t >= INTRO_MAX_HOLD;
                if self.intro_fade_from.is_none() && self.intro_t >= INTRO_MIN_HOLD && ready {
                    self.intro_fade_from = Some(self.intro_t);
                }
                if let Some(from) = self.intro_fade_from {
                    if self.intro_t - from >= INTRO_FADE_OUT {
                        self.intro_done = true;
                        if self.shot_path.is_some() && std::env::var("YTM_SHOT_DELAY").is_err() {
                            return Task::perform(tokio::time::sleep(Duration::from_millis(2500)), |_| Message::TakeShot);
                        }
                    }
                }
            }
            Message::Nav(p) => {
                if matches!(p, Page::Library) {
                    let t = self.request_library_images();
                    self.navigate(p);
                    return t;
                }
                let tasks = match &p {
                    Page::Liked => {
                        let l = self.liked.clone();
                        self.want_tracks(l.iter().take(150), Kind::Thumb)
                    }
                    Page::Playlist(id) => {
                        let tracks = self.playlists.iter().find(|pl| &pl.id == id).map(|pl| pl.tracks.clone()).unwrap_or_default();
                        self.want_tracks(tracks.iter().take(150), Kind::Thumb)
                    }
                    Page::Mix => {
                        let seeds = self.mix_seeds();
                        let arts = self.artists.clone();
                        Task::batch([self.want_tracks(seeds.iter(), Kind::Card), self.want_artists(arts.iter().take(10))])
                    }
                    _ => Task::none(),
                };
                self.navigate(p);
                return tasks;
            }
            Message::Back => {
                if let Some(p) = self.history.pop() {
                    self.page = p;
                }
            }
            Message::OpenArtist(id, name) => {
                self.navigate(Page::Artist(id.clone(), name.clone()));
                self.artist = None;
                self.artist_loading = true;
                let ytm = self.player.ytm.clone();
                let key = id.clone();
                return Task::perform(
                    async move { ytm.fetch_artist(&id, Some(&name)).await.map_err(|e| e.to_string()) },
                    move |r| Message::ArtistLoaded(key.clone(), r),
                );
            }
            Message::ArtistLoaded(_, res) => {
                self.artist_loading = false;
                match res {
                    Ok(a) => {
                        let tracks = a.tracks.clone();
                        let sims = a.similar_artists.clone();
                        let avatar = a.avatar_url.clone();
                        self.artist = Some(a);
                        return Task::batch([
                            self.want(avatar.as_ref(), Kind::Circle).unwrap_or_else(Task::none),
                            self.want_tracks(tracks.iter().take(40), Kind::Thumb),
                            self.want_artists(sims.iter()),
                        ]);
                    }
                    Err(e) => return self.banner(format!("Sanatçı yüklenemedi: {e}"), true),
                }
            }
            Message::HomeLoaded(h, r) => {
                self.home_loading = false;
                self.home_history = h;
                self.home_recs = r;
                let qp: Vec<Track> = self.quick_picks();
                let recs = self.home_recs.clone();
                let again = self.listen_again();
                let arts = self.artists.clone();
                return Task::batch([
                    self.want_tracks(qp.iter(), Kind::Thumb),
                    self.want_tracks(recs.iter(), Kind::Card),
                    self.want_tracks(again.iter(), Kind::Card),
                    self.want_artists(arts.iter().take(14)),
                ]);
            }
            Message::RefreshHome => {
                self.home_nonce += 1;
                return self.load_home();
            }
            Message::SearchInput(q) => {
                self.query = q;
                self.search_seq += 1;
                let seq = self.search_seq;
                if self.query.trim().chars().count() < 2 {
                    return Task::none();
                }
                // live search after a short pause in typing (no Enter needed)
                return Task::perform(tokio::time::sleep(Duration::from_millis(450)), move |_| Message::SearchDebounced(seq));
            }
            Message::SearchDebounced(seq) | Message::SearchDone(seq, _) if seq != self.search_seq => {}
            Message::SearchDebounced(_) | Message::SearchSubmit => {
                let q = self.query.trim().to_string();
                if q.is_empty() {
                    return Task::none();
                }
                self.searching = true;
                self.navigate(Page::Search);
                let seq = self.search_seq;
                let ytm = self.player.ytm.clone();
                return Task::perform(
                    async move { ytm.search_all(&q).await.map_err(|e| e.to_string()) },
                    move |r| Message::SearchDone(seq, r),
                );
            }
            Message::SearchDone(_, res) => {
                self.searching = false;
                match res {
                    Ok(b) => {
                        let tracks = b.tracks.clone();
                        let arts = b.artists.clone();
                        self.search = Some(b);
                        return Task::batch([self.want_tracks(tracks.iter(), Kind::Thumb), self.want_artists(arts.iter())]);
                    }
                    Err(e) => return self.banner(format!("Arama hatası: {e}"), true),
                }
            }
            Message::ImageLoaded(key, kind, handle) => {
                if kind == Kind::Art {
                    if let (Some(h), Some(id)) = (handle, key.strip_prefix("art:")) {
                        self.art = Some((id.to_string(), h));
                    }
                } else {
                    self.pending.remove(&(key.clone(), kind));
                    if let Some(h) = handle {
                        self.images.insert((key, kind), h);
                    }
                }
            }
            Message::LyricsLoaded(id, l) => {
                if self.pb.current.as_ref().map(|t| t.id == id).unwrap_or(false) {
                    self.lyrics = l.or(Some(LyricsView { track_id: id, synced: None, plain: String::new(), source: String::new() }));
                }
            }
            Message::PlayList(list, idx) => {
                self.player.play_list(list.as_ref().clone(), idx);
                self.pb = self.player.poll();
            }
            Message::QueueSelect(i) => self.player.select_index(i),
            Message::TogglePlay => self.player.toggle_pause(),
            Message::Next => self.player.next(false),
            Message::Prev => self.player.prev(),
            Message::SeekDrag(v) => self.seeking = Some(v),
            Message::SeekRelease => {
                if let Some(v) = self.seeking.take() {
                    self.player.seek(v);
                }
            }
            Message::Volume(v) => {
                self.volume = v;
                self.player.set_volume(v);
            }
            Message::VolumeRelease => self.player.save_library(),
            Message::Shuffle => self.player.toggle_shuffle(),
            Message::Repeat => self.player.cycle_repeat(),
            Message::Like(t) => {
                self.player.toggle_like(&t);
                self.refresh_library();
            }
            Message::LikeCurrent => {
                if let Some(t) = self.pb.current.clone() {
                    self.player.toggle_like(&t);
                    self.refresh_library();
                }
            }
            Message::StartMix(t) => {
                let ytm = self.player.ytm.clone();
                let title = t.title.clone();
                let b = self.banner(format!("“{title}” mixi hazırlanıyor…"), false);
                return Task::batch([
                    b,
                    Task::perform(
                        async move {
                            let radio = ytm.fetch_radio(&t.id).await.unwrap_or_default();
                            let mut list = vec![t];
                            list.extend(radio);
                            Ok::<_, String>(list)
                        },
                        move |r| Message::MixReady(title.clone(), r),
                    ),
                ]);
            }
            Message::StartArtistMix(id, name) => {
                let ytm = self.player.ytm.clone();
                let b = self.banner(format!("{name} mixi hazırlanıyor…"), false);
                let label = name.clone();
                return Task::batch([
                    b,
                    Task::perform(
                        async move { ytm.fetch_artist_mix(&id, Some(&name)).await.map_err(|e| e.to_string()) },
                        move |r| Message::MixReady(label.clone(), r),
                    ),
                ]);
            }
            Message::MixReady(label, res) => match res {
                Ok(list) if !list.is_empty() => {
                    let n = list.len();
                    self.player.play_list(list, 0);
                    self.pb = self.player.poll();
                    return self.banner(format!("{label} mixi başladı · {n} şarkı"), false);
                }
                Ok(_) => return self.banner("Bu mix için şarkı bulunamadı", true),
                Err(e) => return self.banner(format!("Mix başlatılamadı: {e}"), true),
            },
            Message::Prefetch(id) => self.player.prefetch(id),
            Message::Chip(c) => {
                self.chip = c;
                if c == 1 {
                    self.navigate(Page::Explore);
                }
            }
            Message::LibChip(c) => self.lib_chip = c,
            Message::TogglePanel => self.panel_open = !self.panel_open,
            Message::PanelTab(t) => {
                self.panel_tab = t;
                self.panel_open = true;
            }
            Message::ClearBanner => self.banner = None,
            Message::TitlePress => {
                // Two presses within 400 ms = double-click: maximize/restore (like Windows).
                let now = Instant::now();
                let double = self.last_title_press.map(|t| now.duration_since(t) < Duration::from_millis(400)).unwrap_or(false);
                self.last_title_press = if double { None } else { Some(now) };
                return if double {
                    window::get_latest().and_then(window::toggle_maximize)
                } else {
                    window::get_latest().and_then(window::drag)
                };
            }
            Message::WinMinimize => return window::get_latest().and_then(|id| window::minimize(id, true)),
            Message::WinToggleMaximize => return window::get_latest().and_then(window::toggle_maximize),
            Message::WinClose => {
                if self.close_to_tray && self.tray.is_some() {
                    return window::get_latest()
                        .and_then(|id| window::change_mode(id, window::Mode::Hidden));
                }
                return window::get_latest().and_then(window::close);
            }
            Message::WinResized => {
                return window::get_latest().and_then(window::get_maximized).map(Message::WinMaximized);
            }
            Message::WinMaximized(m) => self.maximized = m,
            Message::ResizeFrom(edge) => {
                return window::get_latest()
                    .and_then(move |id| window::run_with_handle(id, move |h| begin_resize(h, edge)))
                    .map(|_| Message::Noop);
            }
            Message::Noop => {}
            Message::TakeShot => {
                return window::get_latest().and_then(window::screenshot).map(Message::Shot);
            }
            Message::Shot(shot) => {
                if let Some(path) = &self.shot_path {
                    if let Some(img) = image_crate::RgbaImage::from_raw(shot.size.width, shot.size.height, shot.bytes.to_vec()) {
                        let _ = img.save(path);
                    }
                }
                return iced::exit();
            }
            Message::MediaAttached => {}
            Message::KeyPressed(key, mods) => match key {
                keyboard::Key::Named(keyboard::key::Named::MediaPlayPause) => self.player.toggle_pause(),
                keyboard::Key::Named(keyboard::key::Named::MediaTrackNext) => self.player.next(false),
                keyboard::Key::Named(keyboard::key::Named::MediaTrackPrevious) => self.player.prev(),
                keyboard::Key::Named(keyboard::key::Named::MediaStop) => self.player.audio.stop(),
                keyboard::Key::Named(keyboard::key::Named::Escape) => {
                    if self.playlist_modal_track.is_some() || self.creating_playlist {
                        self.playlist_modal_track = None;
                        self.creating_playlist = false;
                        self.new_playlist_name.clear();
                    } else if self.mini_mode {
                        return self.update(Message::ToggleMiniPlayer);
                    }
                }
                keyboard::Key::Character(ref c) if mods.control() && c.eq_ignore_ascii_case("f") => {
                    self.navigate(Page::Search);
                }
                _ => {}
            },
            Message::ToggleMiniPlayer => {
                self.mini_mode = !self.mini_mode;
                let mini = self.mini_mode;
                return window::get_latest().and_then(move |id| {
                    if mini {
                        Task::batch([
                            window::change_level(id, window::Level::AlwaysOnTop),
                            window::resize(id, Size::new(400.0, 168.0)),
                        ])
                    } else {
                        Task::batch([
                            window::change_level(id, window::Level::Normal),
                            window::resize(id, Size::new(1240.0, 780.0)),
                        ])
                    }
                });
            }
            Message::OpenAddToPlaylist(track) => {
                self.playlist_modal_track = Some(track);
                self.creating_playlist = false;
                self.new_playlist_name.clear();
            }
            Message::OpenCreatePlaylist => {
                self.playlist_modal_track = None;
                self.creating_playlist = true;
                self.new_playlist_name.clear();
            }
            Message::ClosePlaylistModal => {
                self.playlist_modal_track = None;
                self.creating_playlist = false;
                self.new_playlist_name.clear();
            }
            Message::NewPlaylistInput(v) => self.new_playlist_name = v,
            Message::AddTrackToPlaylist(pl_id) => {
                if let Some(track) = self.playlist_modal_track.take() {
                    let added = self.player.add_track_to_playlist(&pl_id, &track);
                    self.refresh_library();
                    let pl_name = self
                        .playlists
                        .iter()
                        .find(|p| p.id == pl_id)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "Liste".to_string());
                    let msg = if added {
                        format!("“{}” → {} listesine eklendi", track.title, pl_name)
                    } else {
                        format!("“{}” zaten {} listesinde", track.title, pl_name)
                    };
                    return self.banner(msg, false);
                }
            }
            Message::CreatePlaylistConfirm => {
                let name = self.new_playlist_name.trim().to_string();
                let id = self.player.create_playlist(name);
                let mut msg = "Yeni çalma listesi oluşturuldu".to_string();
                if let Some(track) = self.playlist_modal_track.take() {
                    self.player.add_track_to_playlist(&id, &track);
                    msg = format!("Liste oluşturuldu ve “{}” eklendi", track.title);
                }
                self.creating_playlist = false;
                self.new_playlist_name.clear();
                self.refresh_library();
                return self.banner(msg, false);
            }
            Message::DeletePlaylist(pl_id) => {
                self.player.delete_playlist(&pl_id);
                self.refresh_library();
                if self.page == Page::Playlist(pl_id) {
                    self.page = Page::Library;
                }
                return self.banner("Çalma listesi silindi", false);
            }
            Message::RemoveFromPlaylist(pl_id, track_id) => {
                self.player.remove_track_from_playlist(&pl_id, &track_id);
                self.refresh_library();
                return self.banner("Şarkı listeden çıkarıldı", false);
            }
            Message::ToggleDiscordRpc(v) => {
                self.cfg_discord_enabled = v;
                self.persist_settings();
            }
            Message::SetDiscordClientId(v) => self.cfg_discord_client_id = v,
            Message::ToggleObsApi(v) => {
                self.cfg_obs_enabled = v;
                self.persist_settings();
            }
            Message::ToggleLastfm(v) => {
                self.cfg_lastfm_enabled = v;
                self.persist_settings();
            }
            Message::SetLastfmApiKey(v) => self.cfg_lastfm_key = v,
            Message::SetLastfmSecret(v) => self.cfg_lastfm_secret = v,
            Message::SetLastfmSessionKey(v) => self.cfg_lastfm_sk = v,
            Message::ToggleCloseToTray(v) => self.close_to_tray = v,
            Message::SetUpdateRepo(v) => self.cfg_update_repo = v,
            Message::SaveSettings => {
                self.persist_settings();
                return self.banner("Ayarlar kaydedildi", false);
            }
            Message::SetSleepTimer(mins) => {
                self.sleep_deadline = mins.map(|m| Instant::now() + Duration::from_secs(m * 60));
                let msg = match mins {
                    Some(m) => format!("Uyku zamanlayıcısı {m} dakikaya ayarlandı"),
                    None => "Uyku zamanlayıcısı kapatıldı".to_string(),
                };
                return self.banner(msg, false);
            }
            Message::CheckUpdate => {
                if self.update_checking {
                    return Task::none();
                }
                self.update_checking = true;
                self.update_status = Some("GitHub Releases kontrol ediliyor…".to_string());
                let http = self.player.http.clone();
                let repo = self.cfg_update_repo.clone();
                return Task::perform(
                    async move {
                        updater::check_for_update(&http, &repo)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    Message::UpdateChecked,
                );
            }
            Message::UpdateChecked(res) => {
                self.update_checking = false;
                match res {
                    Ok(Some(info)) => {
                        self.update_status = Some(format!("Yeni sürüm bulundu: {}", info.version));
                        self.update_info = Some(info);
                    }
                    Ok(None) => {
                        self.update_info = None;
                        self.update_status = Some(format!(
                            "En güncel sürümü kullanıyorsun (v{})",
                            updater::CURRENT_VERSION
                        ));
                    }
                    Err(e) => {
                        self.update_status = Some(format!("Kontrol hatası: {e}"));
                    }
                }
            }
            Message::InstallUpdate => {
                if self.update_installing {
                    return Task::none();
                }
                let Some(info) = self.update_info.clone() else {
                    return Task::none();
                };
                self.update_installing = true;
                self.update_status = Some(format!("{} indiriliyor ve kuruluyor…", info.version));
                let http = self.player.http.clone();
                return Task::perform(
                    async move {
                        updater::download_and_install(&http, &info)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    Message::UpdateInstalled,
                );
            }
            Message::UpdateInstalled(res) => {
                self.update_installing = false;
                match res {
                    Ok(()) => return iced::exit(),
                    Err(e) => {
                        self.update_status = Some(format!("Güncelleme başarısız: {e}"));
                        return self.banner(format!("Güncelleme hatası: {e}"), true);
                    }
                }
            }
        }
        Task::none()
    }

    fn persist_settings(&self) {
        if let Ok(mut lib) = self.player.library.lock() {
            lib.discord_rpc_enabled = self.cfg_discord_enabled;
            lib.discord_client_id = self.cfg_discord_client_id.trim().to_string();
            lib.obs_api_enabled = self.cfg_obs_enabled;
            lib.lastfm_enabled = self.cfg_lastfm_enabled;
            lib.lastfm_api_key = self.cfg_lastfm_key.trim().to_string();
            lib.lastfm_secret = self.cfg_lastfm_secret.trim().to_string();
            lib.lastfm_session_key = self.cfg_lastfm_sk.trim().to_string();
            let repo = self.cfg_update_repo.trim().to_string();
            lib.update_repo = if repo.is_empty() { None } else { Some(repo) };
            lib.save();
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick = iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick);
        let resized = window::resize_events().map(|_| Message::WinResized);
        let keys = keyboard::on_key_press(|key, mods| Some(Message::KeyPressed(key, mods)));
        if self.intro_done {
            Subscription::batch([tick, resized, keys])
        } else {
            Subscription::batch([tick, resized, keys, window::frames().map(Message::IntroFrame)])
        }
    }

    // ---------- derived lists ----------
    fn quick_picks(&self) -> Vec<Track> {
        let mut seen = HashSet::new();
        self.home_recs
            .iter()
            .chain(self.home_history.iter())
            .chain(self.recent.iter())
            .filter(|t| seen.insert(t.id.clone()))
            .take(16)
            .cloned()
            .collect()
    }

    fn listen_again(&self) -> Vec<Track> {
        if !self.home_history.is_empty() { self.home_history.clone() } else { self.recent.clone() }
    }

    fn mix_seeds(&self) -> Vec<Track> {
        let mut seen = HashSet::new();
        self.home_history
            .iter()
            .chain(self.liked.iter())
            .chain(self.recent.iter())
            .filter(|t| seen.insert(t.artist.split(',').next().unwrap_or("").trim().to_lowercase()))
            .take(12)
            .cloned()
            .collect()
    }

    fn img(&self, url: Option<&String>, kind: Kind, size: f32) -> Element<'_, Message> {
        let radius = match kind {
            Kind::Circle => size / 2.0,
            Kind::Thumb => size * 0.2,
            _ => 18.0,
        };
        match url.and_then(|u| self.images.get(&(u.clone(), kind))) {
            Some(h) => image(h.clone()).width(size).height(size).into(),
            None => container(Space::new(size, size)).style(move |_| rounded_bg(white(0.06), radius)).into(),
        }
    }

    // ================= VIEW =================
    fn view(&self) -> Element<'_, Message> {
        if self.mini_mode {
            return self.mini_player_view();
        }
        let mut center_col = column![self.top_bar()];
        if let Some((msg, err)) = &self.banner {
            let (bg, fg) = if *err { (ERROR_BG, rgb(0xfe, 0xca, 0xca)) } else { (white(0.08), TEXT) };
            center_col = center_col.push(
                container(text(msg.clone()).size(12.5).color(fg))
                    .padding([8, 14])
                    .width(Length::Fill)
                    .style(move |_| rounded_bg(bg, 12.0)),
            );
        }
        let center = container(
            center_col
                .push(scrollable(self.page_view()).height(Length::Fill).direction(thin_scroll(false)))
                .spacing(6),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(
                Linear::new(Radians(PI)).add_stop(0.0, mix(CARD, ACCENT, 0.12)).add_stop(0.35, CARD).add_stop(1.0, CARD),
            ))),
            border: Border { radius: 18.0.into(), width: 1.0, color: LINE },
            ..Default::default()
        });

        let left = stack![
            row![self.sidebar(), center].spacing(10),
            container(self.capsule())
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Bottom)
                .padding(Padding { top: 0.0, right: 16.0, bottom: 12.0, left: 16.0 }),
        ];

        let mut body = row![left].spacing(10);
        if self.panel_open {
            body = body.push(self.panel());
        }
        let content: Element<'_, Message> = container(body)
            .padding(Padding { top: 0.0, right: 10.0, bottom: 10.0, left: 10.0 })
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        let content: Element<'_, Message> = if self.playlist_modal_track.is_some() || self.creating_playlist {
            stack![content, self.playlist_modal()].into()
        } else {
            content
        };
        let content: Element<'_, Message> = if self.intro_done { content } else { stack![content, self.intro()].into() };

        let root: Element<'_, Message> = container(column![self.title_bar(), content])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style { background: Some(BG.into()), ..Default::default() })
            .into();
        // Frameless window: invisible strips on the edges resize it (not when maximized).
        if self.maximized { root } else { stack![root, resize_edges()].into() }
    }

    /// Discord-style title bar: app icon + name on the left, window buttons on the right,
    /// same color as the app. Drag to move, double-click to maximize.
    fn title_bar(&self) -> Element<'_, Message> {
        const H: f32 = 34.0;
        let drag_area = mouse_area(
            container(
                row![
                    image(self.app_icon.clone()).width(16).height(16),
                    text("YTM Native").size(12.5).font(SEMIBOLD).color(white(0.78)),
                ]
                .spacing(8)
                .align_y(Vertical::Center),
            )
            .width(Length::Fill)
            .height(H)
            .padding(Padding { top: 0.0, right: 0.0, bottom: 0.0, left: 14.0 })
            .center_y(H),
        )
        .on_press(Message::TitlePress);

        let win_btn = |ic: &'static str, msg: Message, danger: bool| {
            button(container(icon(ic, 12.0, white(0.85))).center_x(46).center_y(H))
                .width(46)
                .height(H)
                .padding(0)
                .on_press(msg)
                .style(move |_, status| button::Style {
                    background: Some(match status {
                        button::Status::Hovered if danger => rgb(0xe8, 0x11, 0x23).into(),
                        button::Status::Pressed if danger => rgb(0xf1, 0x70, 0x7a).into(),
                        button::Status::Hovered => white(0.08).into(),
                        button::Status::Pressed => white(0.05).into(),
                        _ => Color::TRANSPARENT.into(),
                    }),
                    text_color: Color::WHITE,
                    border: Border::default(),
                    shadow: Shadow::default(),
                })
        };

        container(
            row![
                drag_area,
                win_btn(I_WIN_MIN, Message::WinMinimize, false),
                win_btn(if self.maximized { I_WIN_RESTORE } else { I_WIN_MAX }, Message::WinToggleMaximize, false),
                win_btn(I_WIN_CLOSE, Message::WinClose, true),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .height(H)
        .into()
    }

    fn intro(&self) -> Element<'_, Message> {
        let t = self.intro_t;
        let screen_alpha = match self.intro_fade_from {
            Some(from) => 1.0 - smoothstep((t - from) / INTRO_FADE_OUT),
            None => 1.0,
        };
        let a = smoothstep(t / INTRO_FADE_IN) * screen_alpha;
        let progress = if self.home_loading { (t / INTRO_MAX_HOLD).min(0.9) } else { 1.0 };
        let bar = stack![
            container(Space::new(220, 3)).style(move |_| rounded_bg(white(0.12 * a), 2.0)),
            container(Space::new(220.0 * progress, 3)).style(move |_| rounded_bg(Color { a, ..ACCENT }, 2.0)),
        ];
        let content = column![
            svg(svg::Handle::from_memory(ICED_LOGO)).width(128).height(144).opacity(a),
            Space::with_height(18),
            text("Powered by").size(18).color(Color { a: 0.75 * a, ..MUTED }),
            text("iced UI").size(56).font(BOLD).color(Color { a, ..Color::WHITE }),
            Space::with_height(26),
            bar,
        ]
        .align_x(Horizontal::Center)
        .spacing(2);
        opaque(
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(move |_| container::Style { background: Some(Color { a: screen_alpha, ..BG }.into()), ..Default::default() }),
        )
    }

    // ---------- Sidebar ----------
    fn sidebar(&self) -> Element<'_, Message> {
        let nav = |ic: &'static str, label: &'static str, page: Page| {
            let active = self.page == page;
            let fg = if active { ACCENT } else { TEXT };
            button(
                row![icon(ic, 20.0, if active { ACCENT } else { MUTED }), text(label).size(14).font(if active { SEMIBOLD } else { BASE_FONT }).color(fg)]
                    .spacing(14)
                    .align_y(Vertical::Center),
            )
            .width(Length::Fill)
            .padding([10, 16])
            .on_press(Message::Nav(page))
            .style(move |_, status| button::Style {
                background: Some(match (active, status) {
                    (true, _) => white(0.04).into(),
                    (_, button::Status::Hovered) => white(0.05).into(),
                    _ => Color::TRANSPARENT.into(),
                }),
                text_color: fg,
                border: Border { radius: 999.0.into(), width: 1.0, color: if active { LINE_STRONG } else { Color::TRANSPARENT } },
                shadow: Shadow::default(),
            })
        };

        let mut list = column![].spacing(2);
        if self.lib_chip != 2 {
            list = list.push(self.lib_row(
                container(icon(I_HEART_FULL, 22.0, HEART)).width(48).height(48).center_x(48).center_y(48).style(|_| rounded_bg(white(0.07), 10.0)).into(),
                "Beğendiğim müzikler".into(),
                format!("Oynatma listesi · {} şarkı", self.liked.len()),
                Message::Nav(Page::Liked),
                self.page == Page::Liked,
            ));
            for pl in &self.playlists {
                let sub = format!(
                    "{} · {} şarkı",
                    if pl.source.as_deref() == Some("youtube") { "YouTube" } else { "Oynatma listesi" },
                    pl.tracks.len()
                );
                list = list.push(self.lib_row(
                    self.img(pl.tracks.first().and_then(|t| t.thumbnail_url.as_ref()), Kind::Thumb, 48.0),
                    pl.name.clone(),
                    sub,
                    Message::Nav(Page::Playlist(pl.id.clone())),
                    self.page == Page::Playlist(pl.id.clone()),
                ));
            }
        }
        if self.lib_chip != 1 {
            for a in &self.artists {
                list = list.push(self.lib_row(
                    self.img(a.avatar_url.as_ref(), Kind::Circle, 48.0),
                    a.name.clone(),
                    "Sanatçı".into(),
                    Message::OpenArtist(a.id.clone(), a.name.clone()),
                    matches!(&self.page, Page::Artist(_, n) if n == &a.name),
                ));
            }
        }

        container(
            column![
                column![
                    nav(I_HOME, "Ana Sayfa", Page::Home),
                    nav(I_MIX, "Mix", Page::Mix),
                    nav(I_EXPLORE, "Keşfet", Page::Explore),
                    nav(I_LIBRARY, "Kitaplık", Page::Library),
                    nav(I_SETTINGS, "Ayarlar", Page::Settings),
                ]
                .spacing(4),
                container(Space::new(Length::Fill, 1)).style(|_| rounded_bg(white(0.08), 0.0)),
                row![
                    icon(I_LIBRARY, 20.0, MUTED),
                    text("Kitaplığın").size(15).font(SEMIBOLD).color(TEXT),
                    horizontal_space(),
                    round_icon_button(I_PLUS, Message::OpenCreatePlaylist),
                    round_icon_button(I_PANEL, Message::TogglePanel),
                ]
                .spacing(6)
                .align_y(Vertical::Center)
                .padding([0, 6]),
                row![
                    chip("Tümü", self.lib_chip == 0, Message::LibChip(0)),
                    chip("Listeler", self.lib_chip == 1, Message::LibChip(1)),
                    chip("Sanatçılar", self.lib_chip == 2, Message::LibChip(2)),
                ]
                .spacing(6),
                scrollable(column![list, Space::with_height(84)]).height(Length::Fill).direction(thin_scroll(false)),
            ]
            .spacing(12),
        )
        .width(280)
        .height(Length::Fill)
        .padding(12)
        .style(|_| card_style())
        .into()
    }

    fn lib_row<'a>(&'a self, thumb: Element<'a, Message>, title: String, sub: String, msg: Message, active: bool) -> Element<'a, Message> {
        button(
            row![
                thumb,
                column![
                    text(ellipsize(&title, 24)).size(14).font(SEMIBOLD).color(if active { ACCENT } else { TEXT }),
                    text(ellipsize(&sub, 30)).size(12).color(MUTED),
                ]
                .spacing(2),
            ]
            .spacing(12)
            .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .padding([6, 8])
        .on_press(msg)
        .style(move |_, s| row_btn_style(active, s, 12.0))
        .into()
    }

    // ---------- Top bar ----------
    fn top_bar(&self) -> Element<'_, Message> {
        let search = text_input("Şarkı, albüm, sanatçı ara", &self.query)
            .on_input(Message::SearchInput)
            .on_submit(Message::SearchSubmit)
            .padding([10, 16])
            .size(13.5)
            .width(Length::Fill)
            .style(|_, status| {
                let focused = matches!(status, text_input::Status::Focused);
                text_input::Style {
                    background: white(if focused { 0.09 } else { 0.06 }).into(),
                    border: Border { radius: 999.0.into(), width: 1.0, color: if focused { white(0.3) } else { white(0.1) } },
                    icon: MUTED,
                    placeholder: MUTED,
                    value: TEXT,
                    selection: Color { a: 0.4, ..ACCENT },
                }
            });
        let connected = !self.player.ytm_cookie_missing();
        let mut right_bar = row![].spacing(4).align_y(Vertical::Center);
        if let Some(deadline) = self.sleep_deadline {
            let rem = deadline.saturating_duration_since(Instant::now()).as_secs();
            let lbl = format!("{}:{:02}", rem / 60, rem % 60);
            right_bar = right_bar.push(
                button(
                    row![icon(I_MOON, 14.0, ACCENT), text(lbl).size(12).font(SEMIBOLD).color(ACCENT)]
                        .spacing(5)
                        .align_y(Vertical::Center),
                )
                .padding([6, 12])
                .on_press(Message::Nav(Page::Settings))
                .style(|_, s| button::Style {
                    background: Some(white(if matches!(s, button::Status::Hovered) { 0.1 } else { 0.05 }).into()),
                    text_color: ACCENT,
                    border: Border { radius: 999.0.into(), width: 1.0, color: ACCENT },
                    shadow: Shadow::default(),
                }),
            );
        }
        right_bar = right_bar.push(
            container(
                row![
                    round_icon_button(I_MINI, Message::ToggleMiniPlayer),
                    round_icon_button(I_REFRESH, Message::RefreshHome),
                    round_icon_button(I_SETTINGS, Message::Nav(Page::Settings)),
                    round_icon_button(I_PANEL, Message::TogglePanel),
                ]
                .spacing(2),
            )
            .padding(3)
            .style(|_| container::Style {
                background: Some(white(0.03).into()),
                border: Border { radius: 999.0.into(), width: 1.0, color: LINE_STRONG },
                ..Default::default()
            }),
        );
        row![
            container(icon(if connected { I_CHECK } else { I_USER }, 16.0, if connected { ON_ACCENT } else { TEXT }))
                .width(36)
                .height(36)
                .center_x(36)
                .center_y(36)
                .style(move |_| rounded_bg(if connected { ACCENT } else { white(0.1) }, 999.0)),
            button(icon(I_BACK, 16.0, if self.history.is_empty() { MUTED } else { TEXT }))
                .padding(8)
                .on_press_maybe((!self.history.is_empty()).then_some(Message::Back))
                .style(|_, s| round_btn_style(s)),
            container(search).width(Length::Fill).max_width(460),
            horizontal_space(),
            right_bar,
        ]
        .spacing(8)
        .align_y(Vertical::Center)
        .padding(Padding { top: 14.0, right: 16.0, bottom: 8.0, left: 16.0 })
        .into()
    }

    // ---------- Pages ----------
    fn page_view(&self) -> Element<'_, Message> {
        let body: Element<'_, Message> = match &self.page {
            Page::Home => self.home_page(),
            Page::Search => self.search_page(),
            Page::Artist(_, name) => self.artist_page(name),
            Page::Liked => self.track_page("Beğendiğim müzikler", "Oynatma listesi", &self.liked, None),
            Page::Playlist(id) => match self.playlists.iter().find(|p| &p.id == id) {
                Some(pl) => {
                    let local_id = if pl.source.as_deref() == Some("youtube") {
                        None
                    } else {
                        Some(pl.id.clone())
                    };
                    self.track_page(
                        &pl.name,
                        if pl.source.as_deref() == Some("youtube") { "YouTube çalma listesi" } else { "Çalma listesi" },
                        &pl.tracks,
                        local_id,
                    )
                }
                None => text("Liste bulunamadı").color(MUTED).into(),
            },
            Page::Library => self.library_page(),
            Page::Mix => self.mix_page(),
            Page::Explore => self.explore_page(),
            Page::Settings => self.settings_page(),
        };
        column![body, Space::with_height(100)]
            .padding(Padding { top: 4.0, right: 24.0, bottom: 0.0, left: 24.0 })
            .into()
    }

    fn home_page(&self) -> Element<'_, Message> {
        let chips = row![
            chip("Tümü", self.chip == 0, Message::Chip(0)),
            chip("Ruh hali", self.chip == 1, Message::Chip(1)),
            chip("Listeler", false, Message::Nav(Page::Library)),
            chip("Sanatçılar", false, Message::LibChip(2)),
        ]
        .spacing(8);

        if self.home_loading && self.home_recs.is_empty() {
            return column![chips, text("Öneriler yükleniyor…").size(14).color(MUTED)].spacing(30).into();
        }

        let qp = Arc::new(self.quick_picks());
        let mut cols = row![].spacing(18);
        for start in (0..qp.len()).step_by(4) {
            let mut col = column![].spacing(4).width(300);
            for i in start..(start + 4).min(qp.len()) {
                col = col.push(self.compact_row(&qp, i));
            }
            cols = cols.push(col);
        }
        let quick = column![
            row![
                column![
                    text("BİR RADYO İLE BAŞLA").size(11).font(SEMIBOLD).color(MUTED),
                    text("Hızlı seçimler").size(21).font(SEMIBOLD).color(TEXT),
                ]
                .spacing(2),
                horizontal_space(),
                link_btn("Tümünü çal", Message::PlayList(qp.clone(), 0)),
            ]
            .align_y(Vertical::Bottom),
            scrollable(container(cols).padding(Padding { top: 0.0, right: 0.0, bottom: 10.0, left: 0.0 })).direction(thin_scroll(true)),
        ]
        .spacing(12);

        let mut page = column![chips, quick].spacing(30);
        if !self.home_recs.is_empty() {
            let title = if self.home_history.is_empty() { "Senin için" } else { "YouTube geçmişine göre öneriler" };
            page = page.push(section(title, self.card_row(Arc::new(self.home_recs.clone()))));
        }
        let again = self.listen_again();
        if !again.is_empty() {
            page = page.push(section("Yeniden dinle", self.card_row(Arc::new(again))));
        }
        if !self.artists.is_empty() {
            page = page.push(section("Favori sanatçıların", self.artist_row(&self.artists)));
        }
        page.into()
    }

    fn search_page(&self) -> Element<'_, Message> {
        let header = column![
            text("ARAMA SONUÇLARI").size(11).font(SEMIBOLD).color(MUTED),
            row![
                text(format!("“{}”", self.query.trim())).size(26).font(BOLD).color(TEXT),
                if self.searching { text("  aranıyor…").size(13).color(ACCENT) } else { text("") },
            ]
            .align_y(Vertical::Center),
        ]
        .spacing(2);
        let Some(res) = &self.search else {
            return header.into();
        };
        let mut page = column![header].spacing(26);
        if !res.artists.is_empty() {
            page = page.push(section("Sanatçılar", self.artist_row(&res.artists)));
        }
        if !res.tracks.is_empty() {
            let list = Arc::new(res.tracks.clone());
            page = page.push(column![
                row![
                    text("Şarkılar").size(21).font(SEMIBOLD).color(TEXT),
                    horizontal_space(),
                    accent_btn("▶  Tümünü çal", Message::PlayList(list.clone(), 0)),
                ]
                .align_y(Vertical::Center),
                self.track_list(&list),
            ]
            .spacing(12));
        }
        if res.artists.is_empty() && res.tracks.is_empty() && !self.searching {
            page = page.push(text("Sonuç bulunamadı.").color(MUTED));
        }
        page.into()
    }

    fn artist_page(&self, hint_name: &str) -> Element<'_, Message> {
        let Some(a) = &self.artist else {
            return column![
                text(hint_name.to_string()).size(30).font(BOLD).color(TEXT),
                text(if self.artist_loading { "Yükleniyor…" } else { "Sanatçı yüklenemedi." }).color(MUTED),
            ]
            .spacing(8)
            .into();
        };
        let tracks = Arc::new(a.tracks.clone());
        let hero = container(
            row![
                self.img(a.avatar_url.as_ref(), Kind::Circle, 150.0),
                column![
                    text(ellipsize(&a.name, 28)).size(40).font(BOLD).color(TEXT),
                    text(fmt_count(a.subscriber_count)).size(14).color(MUTED),
                    text(ellipsize(a.description.as_deref().unwrap_or(""), 160)).size(12).color(MUTED),
                    row![
                        accent_btn("▶  Oynat", Message::PlayList(tracks.clone(), 0)),
                        outline_btn("Mix başlat", Message::StartArtistMix(a.id.clone(), a.name.clone())),
                    ]
                    .spacing(10),
                ]
                .spacing(8)
                .width(Length::Fill),
            ]
            .spacing(24)
            .align_y(Vertical::Center),
        )
        .padding(22)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(
                Linear::new(Radians(PI * 0.75)).add_stop(0.0, rgb(0x11, 0x22, 0x2e)).add_stop(1.0, rgb(0x0f, 0x0f, 0x12)),
            ))),
            border: Border { radius: 20.0.into(), width: 1.0, color: LINE },
            ..Default::default()
        });
        let mut page = column![hero, section("Popüler şarkılar", self.track_list(&tracks))].spacing(28);
        if !a.similar_artists.is_empty() {
            page = page.push(section("Benzer sanatçılar", self.artist_row(&a.similar_artists)));
        }
        page.into()
    }

    fn track_page<'a>(&'a self, title: &str, kind: &str, tracks: &[Track], local_playlist_id: Option<String>) -> Element<'a, Message> {
        let list = Arc::new(tracks.to_vec());
        let mut actions = row![].spacing(10).align_y(Vertical::Center);
        if !tracks.is_empty() {
            actions = actions
                .push(accent_btn("▶  Oynat", Message::PlayList(list.clone(), 0)))
                .push(outline_btn("Mix başlat", Message::StartMix(tracks[0].clone())));
        }
        if let Some(ref pl_id) = local_playlist_id {
            actions = actions.push(
                button(
                    row![icon(I_TRASH, 15.0, HEART), text("Listeyi Sil").size(13).font(SEMIBOLD).color(HEART)]
                        .spacing(6)
                        .align_y(Vertical::Center),
                )
                .padding([8, 16])
                .on_press(Message::DeletePlaylist(pl_id.clone()))
                .style(|_, s| button::Style {
                    background: Some(if matches!(s, button::Status::Hovered) { ERROR_BG } else { Color::TRANSPARENT }.into()),
                    text_color: HEART,
                    border: Border { radius: 999.0.into(), width: 1.0, color: Color { a: 0.45, ..HEART } },
                    shadow: Shadow::default(),
                }),
            );
        }
        column![
            column![
                text(kind.to_uppercase()).size(11).font(SEMIBOLD).color(MUTED),
                text(title.to_string()).size(30).font(BOLD).color(TEXT),
                text(format!("{} şarkı", tracks.len())).size(13).color(MUTED),
            ]
            .spacing(4),
            actions,
            if tracks.is_empty() {
                Element::from(text("Henüz şarkı yok. Şarkıların yanındaki '+' ile buraya ekleyebilirsin.").color(MUTED))
            } else {
                self.track_list_with_remove(&list, local_playlist_id.as_deref())
            },
        ]
        .spacing(18)
        .into()
    }

    fn library_page(&self) -> Element<'_, Message> {
        let mut grid = row![].spacing(14);
        grid = grid.push(self.tile(
            container(icon(I_HEART_FULL, 54.0, HEART)).width(170).height(170).center_x(170).center_y(170).style(|_| rounded_bg(white(0.07), 18.0)).into(),
            "Beğendiğim müzikler".into(),
            format!("{} şarkı", self.liked.len()),
            Message::Nav(Page::Liked),
        ));
        let mut rows = column![].spacing(14);
        let mut count = 1;
        for pl in &self.playlists {
            if count % 5 == 0 {
                rows = rows.push(grid);
                grid = row![].spacing(14);
            }
            grid = grid.push(self.tile(
                self.img(pl.tracks.first().and_then(|t| t.thumbnail_url.as_ref()), Kind::Thumb, 170.0),
                pl.name.clone(),
                format!("{} şarkı", pl.tracks.len()),
                Message::Nav(Page::Playlist(pl.id.clone())),
            ));
            count += 1;
        }
        rows = rows.push(grid);
        let mut page = column![
            row![
                text("Kitaplık").size(30).font(BOLD).color(TEXT),
                horizontal_space(),
                accent_btn("+ Yeni Liste", Message::OpenCreatePlaylist),
            ]
            .align_y(Vertical::Center),
            rows,
        ]
        .spacing(20);
        if !self.artists.is_empty() {
            page = page.push(section("Sanatçılar", self.artist_row(&self.artists)));
        }
        page.into()
    }

    fn tile<'a>(&'a self, art: Element<'a, Message>, title: String, sub: String, msg: Message) -> Element<'a, Message> {
        button(column![art, text(ellipsize(&title, 22)).size(14).font(SEMIBOLD).color(TEXT), text(sub).size(12).color(MUTED)].spacing(6).width(170))
            .padding(8)
            .on_press(msg)
            .style(|_, s| row_btn_style(false, s, 16.0))
            .into()
    }

    fn mix_page(&self) -> Element<'_, Message> {
        let seeds = self.mix_seeds();
        let mut cards = row![].spacing(14);
        for t in seeds.iter() {
            let name = t.artist.split(',').next().unwrap_or("").trim().to_string();
            cards = cards.push(
                column![
                    button(stack![
                        self.img(t.thumbnail_url.as_ref(), Kind::Card, 200.0),
                        card_shade(200.0),
                        container(
                            column![
                                container(row![icon(I_MIX, 13.0, Color::WHITE), text("Mix").size(11).font(SEMIBOLD).color(Color::WHITE)].spacing(4))
                                    .padding([2, 8])
                                    .style(|_| rounded_bg(black(0.5), 999.0)),
                                vertical_space(),
                                text(ellipsize(&t.title, 20)).size(16).font(BOLD).color(Color::WHITE),
                            ]
                        )
                        .width(200)
                        .height(200)
                        .padding(12),
                    ])
                    .padding(0)
                    .on_press(Message::StartMix(t.clone()))
                    .style(|_, _| button::Style { background: None, ..Default::default() }),
                    text(format!("{} ve benzerleri", ellipsize(&name, 18))).size(12).color(MUTED),
                ]
                .spacing(8)
                .width(200),
            );
        }
        let mut arts = row![].spacing(12);
        for a in self.artists.iter().take(12) {
            arts = arts.push(
                button(
                    column![
                        self.img(a.avatar_url.as_ref(), Kind::Circle, 128.0),
                        text(format!("{} Mix", ellipsize(&a.name, 14))).size(14).font(SEMIBOLD).color(TEXT),
                        text("Sanatçı radyosu").size(12).color(MUTED),
                    ]
                    .spacing(6)
                    .align_x(Horizontal::Center)
                    .width(140),
                )
                .padding(6)
                .on_press(Message::StartArtistMix(a.id.clone(), a.name.clone()))
                .style(|_, s| row_btn_style(false, s, 16.0)),
            );
        }
        column![
            column![
                text("SENİN İÇİN").size(11).font(SEMIBOLD).color(MUTED),
                text("Mix'ler").size(30).font(BOLD).color(TEXT),
                text("Bir karta tıkla; o şarkıdan ya da sanatçıdan sonsuz bir mix başlasın.").size(13).color(MUTED),
            ]
            .spacing(4),
            if seeds.is_empty() {
                Element::from(text("Henüz mix oluşturacak dinleme geçmişin yok. Birkaç şarkı dinle.").color(MUTED))
            } else {
                section("Dinlediklerinden", scrollable(cards).direction(thin_scroll(true)).into())
            },
            section("Sanatçı mixleri", scrollable(arts).direction(thin_scroll(true)).into()),
        ]
        .spacing(28)
        .into()
    }

    fn explore_page(&self) -> Element<'_, Message> {
        let mut grid = column![].spacing(10);
        for chunk in MOODS.chunks(2) {
            let mut r = row![].spacing(10);
            for (label, query) in chunk {
                r = r.push(
                    button(text(*label).size(14).font(SEMIBOLD).color(TEXT))
                        .width(Length::Fill)
                        .padding([14, 18])
                        .on_press(Message::SearchInput(query.to_string()))
                        .style(|_, s| button::Style {
                            background: Some(white(if matches!(s, button::Status::Hovered) { 0.09 } else { 0.05 }).into()),
                            text_color: TEXT,
                            border: Border { radius: 14.0.into(), width: 1.0, color: LINE },
                            shadow: Shadow::default(),
                        }),
                );
            }
            grid = grid.push(r);
        }
        column![text("Keşfet").size(30).font(BOLD).color(TEXT), text("Ruh halleri ve türler").size(15).color(MUTED), grid]
            .spacing(14)
            .into()
    }

    // ---------- Reusable rows / rows of cards ----------
    fn compact_row(&self, list: &Arc<Vec<Track>>, i: usize) -> Element<'_, Message> {
        let t = &list[i];
        let active = self.pb.current.as_ref().map(|c| c.id == t.id).unwrap_or(false);
        mouse_area(
            button(
                row![
                    self.img(t.thumbnail_url.as_ref(), Kind::Thumb, 48.0),
                    column![
                        text(ellipsize(&t.title, 30)).size(14).font(SEMIBOLD).color(if active { ACCENT } else { TEXT }),
                        text(ellipsize(&t.artist, 36)).size(12).color(MUTED),
                    ]
                    .spacing(2),
                ]
                .spacing(12)
                .align_y(Vertical::Center),
            )
            .width(Length::Fill)
            .padding(6)
            .on_press(Message::PlayList(list.clone(), i))
            .style(move |_, s| row_btn_style(active, s, 12.0)),
        )
        .on_enter(Message::Prefetch(t.id.clone()))
        .into()
    }

    fn track_list(&self, list: &Arc<Vec<Track>>) -> Element<'_, Message> {
        self.track_list_with_remove(list, None)
    }

    fn track_list_with_remove(&self, list: &Arc<Vec<Track>>, remove_from_pl: Option<&str>) -> Element<'_, Message> {
        let mut col = column![].spacing(2);
        for (i, t) in list.iter().enumerate().take(200) {
            let active = self.pb.current.as_ref().map(|c| c.id == t.id).unwrap_or(false);
            let liked = self.liked_ids.contains(&t.id);
            let primary = t.artist.split(',').next().unwrap_or("").trim().to_string();
            let mut r = row![
                text(format!("{}", i + 1)).size(12).color(MUTED).width(24),
                self.img(t.thumbnail_url.as_ref(), Kind::Thumb, 40.0),
                column![
                    text(ellipsize(&t.title, 48)).size(14).font(SEMIBOLD).color(if active { ACCENT } else { TEXT }),
                    button(text(ellipsize(&t.artist, 56)).size(12).color(MUTED))
                        .padding(0)
                        .on_press(Message::OpenArtist(t.artist_id.clone().unwrap_or_default(), primary))
                        .style(|_, _| button::Style { background: None, text_color: MUTED, ..Default::default() }),
                ]
                .spacing(1)
                .width(Length::Fill),
                button(icon(I_PLUS, 16.0, MUTED)).padding(7).on_press(Message::OpenAddToPlaylist(t.clone())).style(|_, s| round_btn_style(s)),
                button(icon(I_MIX, 17.0, MUTED)).padding(7).on_press(Message::StartMix(t.clone())).style(|_, s| round_btn_style(s)),
                button(icon(if liked { I_HEART_FULL } else { I_HEART }, 17.0, if liked { HEART } else { MUTED }))
                    .padding(7)
                    .on_press(Message::Like(t.clone()))
                    .style(|_, s| round_btn_style(s)),
            ]
            .spacing(10)
            .align_y(Vertical::Center);
            if let Some(pl_id) = remove_from_pl {
                r = r.push(
                    button(icon(I_TRASH, 15.0, MUTED))
                        .padding(7)
                        .on_press(Message::RemoveFromPlaylist(pl_id.to_string(), t.id.clone()))
                        .style(|_, s| round_btn_style(s)),
                );
            }
            r = r.push(text(t.duration_secs.map(|d| fmt_time(d as f64)).unwrap_or_else(|| "--:--".into())).size(12).color(MUTED).width(44));
            col = col.push(
                mouse_area(
                    button(r)
                        .width(Length::Fill)
                        .padding([6, 10])
                        .on_press(Message::PlayList(list.clone(), i))
                        .style(move |_, s| row_btn_style(active, s, 12.0)),
                )
                .on_enter(Message::Prefetch(t.id.clone())),
            );
        }
        col.into()
    }

    fn card_row(&self, list: Arc<Vec<Track>>) -> Element<'_, Message> {
        let mut cards = row![].spacing(14);
        for (i, t) in list.iter().enumerate().take(20) {
            let active = self.pb.current.as_ref().map(|c| c.id == t.id).unwrap_or(false);
            let primary = t.artist.split(',').next().unwrap_or("").trim().to_string();
            cards = cards.push(
                column![
                    button(stack![
                        self.img(t.thumbnail_url.as_ref(), Kind::Card, 220.0),
                        card_shade(220.0),
                        container(text(ellipsize(&t.title, 22)).size(17).font(BOLD).color(Color::WHITE))
                            .width(220)
                            .height(220)
                            .align_bottom(220)
                            .padding(14),
                    ])
                    .padding(0)
                    .on_press(Message::PlayList(list.clone(), i))
                    .style(move |_, _| button::Style {
                        background: None,
                        text_color: TEXT,
                        border: Border { radius: 18.0.into(), width: if active { 2.0 } else { 0.0 }, color: ACCENT },
                        shadow: Shadow::default(),
                    }),
                    button(text(ellipsize(&t.artist, 28)).size(12.5).color(MUTED))
                        .padding(0)
                        .on_press(Message::OpenArtist(t.artist_id.clone().unwrap_or_default(), primary))
                        .style(|_, _| button::Style { background: None, text_color: MUTED, ..Default::default() }),
                ]
                .spacing(8)
                .width(220),
            );
        }
        scrollable(cards).direction(thin_scroll(true)).into()
    }

    fn artist_row<'a>(&'a self, artists: &'a [ArtistSummary]) -> Element<'a, Message> {
        let mut r = row![].spacing(12);
        for a in artists.iter().take(20) {
            r = r.push(
                button(
                    column![
                        self.img(a.avatar_url.as_ref(), Kind::Circle, 128.0),
                        text(ellipsize(&a.name, 16)).size(14).font(SEMIBOLD).color(TEXT),
                        text(fmt_count(a.subscriber_count)).size(12).color(MUTED),
                    ]
                    .spacing(6)
                    .align_x(Horizontal::Center)
                    .width(140),
                )
                .padding(6)
                .on_press(Message::OpenArtist(a.id.clone(), a.name.clone()))
                .style(|_, s| row_btn_style(false, s, 16.0)),
            );
        }
        scrollable(r).direction(thin_scroll(true)).into()
    }

    // ---------- Floating player capsule ----------
    fn capsule(&self) -> Element<'_, Message> {
        let pb = &self.pb;
        let cur = pb.current.as_ref();
        let dur = pb.duration.max(1.0);
        let pos = self.seeking.unwrap_or(pb.position).min(dur);
        let liked = cur.map(|t| self.liked_ids.contains(&t.id)).unwrap_or(false);

        let seek = slider(0.0..=dur, pos, Message::SeekDrag)
            .on_release(Message::SeekRelease)
            .step(0.5)
            .height(12)
            .style(|_, status| slider::Style {
                rail: slider::Rail {
                    backgrounds: (Color::WHITE.into(), white(0.16).into()),
                    width: if matches!(status, slider::Status::Hovered | slider::Status::Dragged) { 5.0 } else { 3.0 },
                    border: Border { radius: 2.0.into(), ..Default::default() },
                },
                handle: slider::Handle {
                    shape: slider::HandleShape::Circle {
                        radius: if matches!(status, slider::Status::Hovered | slider::Status::Dragged) { 6.0 } else { 0.0 },
                    },
                    background: Color::WHITE.into(),
                    border_width: 0.0,
                    border_color: Color::TRANSPARENT,
                },
            });

        let info = row![
            self.img(cur.and_then(|t| t.thumbnail_url.as_ref()), Kind::Thumb, 40.0),
            column![
                row![
                    text(ellipsize(cur.map(|t| t.title.as_str()).unwrap_or("Şarkı seçilmedi"), 34)).size(13.5).font(SEMIBOLD).color(TEXT),
                    horizontal_space(),
                    text(if pb.is_buffering { "yükleniyor…".to_string() } else { format!("{} / {}", fmt_time(pos), fmt_time(pb.duration)) })
                        .size(10.5)
                        .color(if pb.is_buffering { ACCENT } else { MUTED }),
                ],
                text(ellipsize(cur.map(|t| t.artist.as_str()).unwrap_or(""), 40)).size(12).color(MUTED),
                seek,
            ]
            .spacing(1)
            .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Vertical::Center)
        .width(Length::Fill);

        let playing = pb.is_playing && !pb.is_paused;
        let volume = slider(0.0..=1.0, self.volume, Message::Volume)
            .on_release(Message::VolumeRelease)
            .step(0.01_f32)
            .width(80)
            .style(|_, _| slider::Style {
                rail: slider::Rail {
                    backgrounds: (white(0.85).into(), white(0.2).into()),
                    width: 3.0,
                    border: Border { radius: 2.0.into(), ..Default::default() },
                },
                handle: slider::Handle {
                    shape: slider::HandleShape::Circle { radius: 6.0 },
                    background: Color::WHITE.into(),
                    border_width: 0.0,
                    border_color: Color::TRANSPARENT,
                },
            });

        let content = row![
            pbtn(I_SHUFFLE, 20.0, pb.shuffle, Message::Shuffle),
            pbtn(I_PREV, 20.0, false, Message::Prev),
            pbtn(if playing { I_PAUSE } else { I_PLAY }, 28.0, false, Message::TogglePlay),
            pbtn(I_NEXT, 20.0, false, Message::Next),
            stack![
                pbtn(I_REPEAT, 20.0, pb.repeat != RepeatMode::Off, Message::Repeat),
                if pb.repeat == RepeatMode::One { Element::from(text("1").size(9).font(BOLD).color(ACCENT)) } else { Space::new(0, 0).into() },
            ],
            divider(),
            info,
            divider(),
            button(icon(if liked { I_HEART_FULL } else { I_HEART }, 20.0, if liked { HEART } else { TEXT }))
                .padding(7)
                .on_press(Message::LikeCurrent)
                .style(|_, s| round_btn_style(s)),
            pbtn(I_QUEUE, 20.0, self.panel_open && self.panel_tab == 2, Message::PanelTab(2)),
            pbtn(I_LYRICS, 20.0, self.panel_open && self.panel_tab == 0, Message::PanelTab(0)),
            pbtn(I_MINI, 19.0, self.mini_mode, Message::ToggleMiniPlayer),
            pbtn(I_PANEL, 20.0, self.panel_open, Message::TogglePanel),
            icon(I_VOLUME, 18.0, TEXT),
            volume,
        ]
        .spacing(4)
        .align_y(Vertical::Center);

        container(content)
            .width(Length::Fill)
            .max_width(960)
            .height(64)
            .padding(Padding { top: 0.0, right: 18.0, bottom: 0.0, left: 12.0 })
            .center_y(64)
            .style(|_| container::Style {
                background: Some(Color { a: 0.95, ..rgb(0x1c, 0x1c, 0x20) }.into()),
                border: Border { radius: 999.0.into(), width: 1.0, color: LINE_STRONG },
                shadow: Shadow { color: black(0.55), offset: Vector::new(0.0, 16.0), blur_radius: 40.0 },
                ..Default::default()
            })
            .into()
    }

    // ---------- Right now-playing panel ----------
    fn panel(&self) -> Element<'_, Message> {
        let Some(cur) = self.pb.current.as_ref() else {
            return container(column![icon(I_NOTE, 36.0, MUTED), text("Henüz bir parça çalmıyor").color(MUTED)].spacing(12).align_x(Horizontal::Center))
                .width(380)
                .height(Length::Fill)
                .center_x(380)
                .center_y(Length::Fill)
                .style(|_| card_style())
                .into();
        };
        let liked = self.liked_ids.contains(&cur.id);
        let art_ok = self.art.as_ref().map(|(id, _)| id == &cur.id).unwrap_or(false);

        let tab = |ic: &'static str, idx: usize| {
            let active = self.panel_tab == idx;
            button(container(icon(ic, 19.0, Color::WHITE)).center_x(58).center_y(36))
                .width(58)
                .height(36)
                .padding(0)
                .on_press(Message::PanelTab(idx))
                .style(move |_, _| button::Style {
                    background: Some(if active { white(0.18) } else { Color::TRANSPARENT }.into()),
                    text_color: Color::WHITE,
                    border: Border { radius: 999.0.into(), ..Default::default() },
                    shadow: Shadow::default(),
                })
        };
        let tabs = container(container(row![tab(I_LYRICS, 0), tab(I_HEADPHONES, 1), tab(I_QUEUE, 2)]).padding(3).style(|_| rounded_bg(white(0.12), 999.0)))
            .width(Length::Fill)
            .center_x(Length::Fill);

        let primary = cur.artist.split(',').next().unwrap_or("").trim().to_string();
        let middle: Element<'_, Message> = match self.panel_tab {
            0 => self.lyrics_view(),
            2 => self.queue_view(),
            _ => column![
                vertical_space(),
                row![
                    column![
                        text(ellipsize(&cur.title, 26)).size(21).font(BOLD).color(Color::WHITE),
                        button(text(ellipsize(&cur.artist, 34)).size(14).color(white(0.75)))
                            .padding(0)
                            .on_press(Message::OpenArtist(cur.artist_id.clone().unwrap_or_default(), primary))
                            .style(|_, _| button::Style { background: None, text_color: Color::WHITE, ..Default::default() }),
                    ]
                    .spacing(4)
                    .width(Length::Fill),
                    button(icon(I_PLUS, 19.0, Color::WHITE))
                        .padding(8)
                        .on_press(Message::OpenAddToPlaylist(cur.clone()))
                        .style(|_, s| round_btn_style(s)),
                    button(icon(I_MIX, 20.0, Color::WHITE)).padding(8).on_press(Message::StartMix(cur.clone())).style(|_, s| round_btn_style(s)),
                    button(icon(if liked { I_HEART_FULL } else { I_HEART }, 20.0, if liked { HEART } else { Color::WHITE }))
                        .padding(8)
                        .on_press(Message::LikeCurrent)
                        .style(|_, s| round_btn_style(s)),
                ]
                .align_y(Vertical::Center),
                button(text(self.current_lyric_line().unwrap_or_default()).size(15).color(white(0.9)))
                    .padding(0)
                    .on_press(Message::PanelTab(0))
                    .style(|_, _| button::Style { background: None, text_color: Color::WHITE, ..Default::default() }),
            ]
            .spacing(12)
            .into(),
        };

        let dim = self.panel_tab != 1;
        let shade = container(Space::new(Length::Fill, Length::Fill)).style(move |_| container::Style {
            background: Some(if dim {
                black(0.72).into()
            } else {
                Background::Gradient(Gradient::Linear(
                    Linear::new(Radians(PI))
                        .add_stop(0.0, black(0.25))
                        .add_stop(0.3, Color::TRANSPARENT)
                        .add_stop(0.62, black(0.55))
                        .add_stop(1.0, black(0.9)),
                ))
            }),
            border: Border { radius: 18.0.into(), ..Default::default() },
            ..Default::default()
        });

        let header = row![
            button(icon(I_CHEVRON, 20.0, Color::WHITE)).padding(10).on_press(Message::TogglePanel).style(|_, s| button::Style {
                background: Some(black(if matches!(s, button::Status::Hovered) { 0.55 } else { 0.35 }).into()),
                text_color: Color::WHITE,
                border: Border { radius: 999.0.into(), ..Default::default() },
                shadow: Shadow::default(),
            }),
            horizontal_space(),
            text(match self.panel_tab {
                0 => "ŞARKI SÖZLERİ",
                2 => "SIRADAKİLER",
                _ => "",
            })
            .size(11)
            .font(SEMIBOLD)
            .color(white(0.7)),
        ]
        .align_y(Vertical::Center);

        container(stack![
            if art_ok {
                Element::from(image(self.art.as_ref().unwrap().1.clone()).width(Length::Fill).height(Length::Fill).content_fit(iced::ContentFit::Fill))
            } else {
                Space::new(Length::Fill, Length::Fill).into()
            },
            shade,
            container(column![header, middle, tabs].spacing(12)).padding(16).width(Length::Fill).height(Length::Fill),
        ])
        .width(380)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(CARD.into()),
            border: Border { radius: 18.0.into(), width: 1.0, color: LINE },
            ..Default::default()
        })
        .into()
    }

    fn current_lyric_index(&self) -> Option<usize> {
        let l = self.lyrics.as_ref()?;
        let synced = l.synced.as_ref()?;
        let pos = self.pb.position + 0.15;
        let mut idx = None;
        for (i, line) in synced.iter().enumerate() {
            if pos >= line.time_secs {
                idx = Some(i);
            } else {
                break;
            }
        }
        idx
    }

    fn current_lyric_line(&self) -> Option<String> {
        let l = self.lyrics.as_ref()?;
        if let Some(i) = self.current_lyric_index() {
            return l.synced.as_ref()?.get(i).map(|x| x.text.clone());
        }
        l.plain.lines().find(|x| !x.trim().is_empty()).map(|s| s.to_string())
    }

    /// Synced lyrics: a window of lines around the current one (no scroll needed).
    fn lyrics_view(&self) -> Element<'_, Message> {
        let Some(l) = &self.lyrics else {
            return container(text("Sözler yükleniyor…").color(white(0.6))).height(Length::Fill).into();
        };
        if let Some(synced) = &l.synced {
            let cur = self.current_lyric_index();
            let start = cur.unwrap_or(0).saturating_sub(3);
            let mut col = column![].spacing(10);
            for (i, line) in synced.iter().enumerate().skip(start).take(12) {
                let active = Some(i) == cur;
                col = col.push(
                    text(if line.text.is_empty() { "♪".to_string() } else { line.text.clone() })
                        .size(if active { 21 } else { 18 })
                        .font(if active { BOLD } else { SEMIBOLD })
                        .color(if active { Color::WHITE } else { white(0.38) }),
                );
            }
            return column![vertical_space().height(10), col, vertical_space(), text(l.source.clone()).size(10).color(white(0.35))]
                .height(Length::Fill)
                .into();
        }
        if l.plain.trim().is_empty() {
            return container(text("Bu şarkı için söz bulunamadı.").color(white(0.6))).height(Length::Fill).into();
        }
        scrollable(text(l.plain.clone()).size(15).color(white(0.85)).line_height(1.6)).height(Length::Fill).direction(thin_scroll(false)).into()
    }

    fn queue_view(&self) -> Element<'_, Message> {
        let mut col = column![text(format!("{} şarkı", self.pb.queue.len())).size(11).color(white(0.6))].spacing(2);
        for (i, t) in self.pb.queue.iter().enumerate() {
            let active = Some(i) == self.pb.current_index;
            col = col.push(
                button(
                    row![
                        self.img(t.thumbnail_url.as_ref(), Kind::Thumb, 44.0),
                        column![
                            text(ellipsize(&t.title, 30)).size(13.5).font(SEMIBOLD).color(if active { ACCENT } else { Color::WHITE }),
                            text(ellipsize(&t.artist, 34)).size(12).color(white(0.6)),
                        ]
                        .spacing(1),
                    ]
                    .spacing(10)
                    .align_y(Vertical::Center),
                )
                .width(Length::Fill)
                .padding(6)
                .on_press(Message::QueueSelect(i))
                .style(move |_, s| button::Style {
                    background: Some(if active { white(0.14) } else if matches!(s, button::Status::Hovered) { white(0.08) } else { Color::TRANSPARENT }.into()),
                    text_color: Color::WHITE,
                    border: Border { radius: 12.0.into(), ..Default::default() },
                    shadow: Shadow::default(),
                }),
            );
        }
        scrollable(col).height(Length::Fill).direction(thin_scroll(false)).into()
    }

    // ---------- Settings page (Discord RPC, OBS API, Last.fm, Sleep Timer, Tray, Updater) ----------
    fn settings_page(&self) -> Element<'_, Message> {
        let field_style = |_: &Theme, status: text_input::Status| {
            let focused = matches!(status, text_input::Status::Focused);
            text_input::Style {
                background: white(if focused { 0.09 } else { 0.05 }).into(),
                border: Border {
                    radius: 10.0.into(),
                    width: 1.0,
                    color: if focused { ACCENT } else { LINE_STRONG },
                },
                icon: MUTED,
                placeholder: MUTED,
                value: TEXT,
                selection: Color { a: 0.4, ..ACCENT },
            }
        };

        fn settings_card<'a>(
            title: &'static str,
            subtitle: &'static str,
            body: Element<'a, Message>,
        ) -> Element<'a, Message> {
            container(
                column![
                    column![
                        text(title).size(17).font(SEMIBOLD).color(TEXT),
                        text(subtitle).size(12.5).color(MUTED),
                    ]
                    .spacing(2),
                    container(Space::new(Length::Fill, 1)).style(|_| rounded_bg(LINE, 0.0)),
                    body,
                ]
                .spacing(12),
            )
            .width(Length::Fill)
            .padding(18)
            .style(|_| container::Style {
                background: Some(white(0.03).into()),
                border: Border { radius: 16.0.into(), width: 1.0, color: LINE },
                ..Default::default()
            })
            .into()
        }

        // 1. Discord Rich Presence
        let discord_body = column![
            row![
                text("Discord Rich Presence etkin").size(14).color(TEXT),
                horizontal_space(),
                toggler(self.cfg_discord_enabled).on_toggle(Message::ToggleDiscordRpc),
            ]
            .align_y(Vertical::Center),
            column![
                text("Özel Discord Client ID (opsiyonel — boş bırakılırsa varsayılan YouTube Music ID kullanılır)")
                    .size(12)
                    .color(MUTED),
                text_input(rpc::DEFAULT_DISCORD_CLIENT_ID, &self.cfg_discord_client_id)
                    .on_input(Message::SetDiscordClientId)
                    .on_submit(Message::SaveSettings)
                    .padding([9, 12])
                    .size(13)
                    .style(field_style),
            ]
            .spacing(6),
        ]
        .spacing(12)
        .into();

        // 2. OBS & Stream Deck API
        let obs_body = column![
            row![
                text("OBS Overlay & Companion API (127.0.0.1:13091)").size(14).color(TEXT),
                horizontal_space(),
                toggler(self.cfg_obs_enabled).on_toggle(Message::ToggleObsApi),
            ]
            .align_y(Vertical::Center),
            text(format!(
                "OBS Tarayıcı Kaynağı: http://127.0.0.1:{}/embed/now-playing",
                rpc::OBS_API_PORT
            ))
            .size(12.5)
            .color(ACCENT),
            text("Varyantlar: ?layout=compact | ?layout=stack | ?layout=text | ?layout=badge")
                .size(11.5)
                .color(MUTED),
        ]
        .spacing(8)
        .into();

        // 3. Last.fm Scrobbling
        let lastfm_body = column![
            row![
                text("Last.fm Scrobbling etkin").size(14).color(TEXT),
                horizontal_space(),
                toggler(self.cfg_lastfm_enabled).on_toggle(Message::ToggleLastfm),
            ]
            .align_y(Vertical::Center),
            row![
                text_input("API Key", &self.cfg_lastfm_key)
                    .on_input(Message::SetLastfmApiKey)
                    .padding([9, 12])
                    .size(13)
                    .style(field_style),
                text_input("API Secret", &self.cfg_lastfm_secret)
                    .on_input(Message::SetLastfmSecret)
                    .padding([9, 12])
                    .size(13)
                    .secure(true)
                    .style(field_style),
            ]
            .spacing(10),
            text_input("Session Key", &self.cfg_lastfm_sk)
                .on_input(Message::SetLastfmSessionKey)
                .on_submit(Message::SaveSettings)
                .padding([9, 12])
                .size(13)
                .secure(true)
                .style(field_style),
        ]
        .spacing(10)
        .into();

        // 4. Sleep Timer
        let sleep_status = match self.sleep_deadline {
            Some(d) => {
                let rem = d.saturating_duration_since(Instant::now()).as_secs();
                format!("Aktif — Kalan süre: {} dk {:02} sn", rem / 60, rem % 60)
            }
            None => "Kapalı".to_string(),
        };
        let sleep_body = column![
            text(sleep_status).size(13.5).font(SEMIBOLD).color(if self.sleep_deadline.is_some() { ACCENT } else { MUTED }),
            row![
                chip("Kapalı", self.sleep_deadline.is_none(), Message::SetSleepTimer(None)),
                chip("15 dk", false, Message::SetSleepTimer(Some(15))),
                chip("30 dk", false, Message::SetSleepTimer(Some(30))),
                chip("45 dk", false, Message::SetSleepTimer(Some(45))),
                chip("60 dk", false, Message::SetSleepTimer(Some(60))),
                chip("90 dk", false, Message::SetSleepTimer(Some(90))),
            ]
            .spacing(8),
        ]
        .spacing(12)
        .into();

        // 5. System Tray & Window
        let tray_body = column![
            row![
                text("Pencereyi kapatınca sistem tepsisine küçült (arka planda çalmaya devam et)")
                    .size(14)
                    .color(TEXT),
                horizontal_space(),
                toggler(self.close_to_tray).on_toggle(Message::ToggleCloseToTray),
            ]
            .align_y(Vertical::Center),
            text("Sistem tepsisi menüsünden Oynat/Duraklat, Önceki/Sonraki ve tam Çıkış yapabilirsin. Klavye medya tuşları (SMTC) da etkin.")
                .size(12)
                .color(MUTED),
        ]
        .spacing(8)
        .into();

        // 6. Auto Updater
        let mut upd_actions = row![
            accent_btn(
                if self.update_checking { "Kontrol ediliyor…" } else { "Güncellemeleri Kontrol Et" },
                Message::CheckUpdate
            ),
        ]
        .spacing(10)
        .align_y(Vertical::Center);
        if let Some(ref info) = self.update_info {
            upd_actions = upd_actions.push(accent_btn(
                if self.update_installing { "İndiriliyor…" } else { "İndir ve Güncelle" },
                Message::InstallUpdate,
            ));
            upd_actions = upd_actions.push(text(format!("Sürüm: {}", info.version)).size(12.5).color(ACCENT));
        }
        let mut upd_col = column![
            row![
                text(format!("Mevcut sürüm: v{}", updater::CURRENT_VERSION))
                    .size(13.5)
                    .font(SEMIBOLD)
                    .color(TEXT),
                horizontal_space(),
                text(format!(
                    "Çizim motoru: {}",
                    std::env::var("ICED_BACKEND").unwrap_or_else(|_| "wgpu".into())
                ))
                .size(12)
                .color(MUTED),
            ],
            text_input("GitHub Repo (örn: Rbg4445/Ytmnative)", &self.cfg_update_repo)
                .on_input(Message::SetUpdateRepo)
                .on_submit(Message::SaveSettings)
                .padding([9, 12])
                .size(13)
                .style(field_style),
            upd_actions,
        ]
        .spacing(10);
        if let Some(ref status) = self.update_status {
            upd_col = upd_col.push(text(status.clone()).size(12.5).color(MUTED));
        }

        column![
            row![
                column![
                    text("UYGULAMA VE ENTEGRASYONLAR").size(11).font(SEMIBOLD).color(MUTED),
                    text("Ayarlar").size(30).font(BOLD).color(TEXT),
                ]
                .spacing(4),
                horizontal_space(),
                accent_btn("Değişiklikleri Kaydet", Message::SaveSettings),
            ]
            .align_y(Vertical::Center),
            settings_card(
                "Discord Rich Presence",
                "Dinlediğin şarkıyı Discord profilinde 'Dinliyor' olarak göster",
                discord_body
            ),
            settings_card(
                "Uyku Zamanlayıcısı",
                "Belirlenen süre sonunda müziği otomatik duraklat",
                sleep_body
            ),
            settings_card(
                "Sistem Tepsisi & Medya Tuşları",
                "Arka plan oynatma ve Windows medya tuşu entegrasyonu",
                tray_body
            ),
            settings_card(
                "OBS & Stream Deck API",
                "Yayınlar için yerel HTTP durum ve kontrol sunucusu",
                obs_body
            ),
            settings_card(
                "Last.fm Scrobbling",
                "Dinlediğin şarkıları Last.fm hesabına kaydet",
                lastfm_body
            ),
            settings_card(
                "Otomatik Güncelleme",
                "GitHub Releases üzerinden yeni sürümleri kontrol et ve kur",
                upd_col.into()
            ),
        ]
        .spacing(16)
        .max_width(820)
        .into()
    }

    // ---------- Mini Player (always-on-top compact mode) ----------
    fn mini_player_view(&self) -> Element<'_, Message> {
        let pb = &self.pb;
        let cur = pb.current.as_ref();
        let dur = pb.duration.max(1.0);
        let pos = self.seeking.unwrap_or(pb.position).min(dur);
        let playing = pb.is_playing && !pb.is_paused;
        let liked = cur.map(|t| self.liked_ids.contains(&t.id)).unwrap_or(false);

        let top = row![
            mouse_area(
                container(
                    row![
                        image(self.app_icon.clone()).width(14).height(14),
                        text("YTM Native · Mini").size(11.5).font(SEMIBOLD).color(white(0.75)),
                    ]
                    .spacing(6)
                    .align_y(Vertical::Center),
                )
                .width(Length::Fill)
                .height(28)
                .center_y(28)
                .padding([0, 10]),
            )
            .on_press(Message::TitlePress),
            button(icon(I_MINI, 14.0, ACCENT))
                .padding([4, 8])
                .on_press(Message::ToggleMiniPlayer)
                .style(|_, s| round_btn_style(s)),
            button(icon(I_WIN_CLOSE, 11.0, white(0.85)))
                .padding([4, 10])
                .on_press(Message::WinClose)
                .style(|_, s| round_btn_style(s)),
        ]
        .align_y(Vertical::Center);

        let seek = slider(0.0..=dur, pos, Message::SeekDrag)
            .on_release(Message::SeekRelease)
            .step(0.5)
            .height(10)
            .style(|_, _| slider::Style {
                rail: slider::Rail {
                    backgrounds: (ACCENT.into(), white(0.16).into()),
                    width: 3.0,
                    border: Border { radius: 2.0.into(), ..Default::default() },
                },
                handle: slider::Handle {
                    shape: slider::HandleShape::Circle { radius: 5.0 },
                    background: Color::WHITE.into(),
                    border_width: 0.0,
                    border_color: Color::TRANSPARENT,
                },
            });

        let controls = row![
            pbtn(I_PREV, 18.0, false, Message::Prev),
            pbtn(if playing { I_PAUSE } else { I_PLAY }, 24.0, false, Message::TogglePlay),
            pbtn(I_NEXT, 18.0, false, Message::Next),
            horizontal_space(),
            button(icon(if liked { I_HEART_FULL } else { I_HEART }, 17.0, if liked { HEART } else { TEXT }))
                .padding(6)
                .on_press(Message::LikeCurrent)
                .style(|_, s| round_btn_style(s)),
            text(format!("{} / {}", fmt_time(pos), fmt_time(pb.duration)))
                .size(11)
                .color(MUTED),
        ]
        .spacing(4)
        .align_y(Vertical::Center);

        let body = row![
            self.img(cur.and_then(|t| t.thumbnail_url.as_ref()), Kind::Thumb, 74.0),
            column![
                text(ellipsize(cur.map(|t| t.title.as_str()).unwrap_or("Şarkı seçilmedi"), 28))
                    .size(14.5)
                    .font(BOLD)
                    .color(TEXT),
                text(ellipsize(cur.map(|t| t.artist.as_str()).unwrap_or(""), 34))
                    .size(12)
                    .color(MUTED),
                seek,
                controls,
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Vertical::Center)
        .padding([4, 12]);

        container(column![top, body].spacing(4))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style {
                background: Some(CARD.into()),
                border: Border { radius: 14.0.into(), width: 1.0, color: LINE_STRONG },
                ..Default::default()
            })
            .into()
    }

    // ---------- Playlist Modal (Add track to playlist / Create playlist) ----------
    fn playlist_modal(&self) -> Element<'_, Message> {
        let title_str = if let Some(ref t) = self.playlist_modal_track {
            format!("Listeye Ekle: {}", ellipsize(&t.title, 32))
        } else {
            "Yeni Çalma Listesi Oluştur".to_string()
        };

        let mut modal_col = column![
            row![
                text(title_str).size(17).font(BOLD).color(TEXT),
                horizontal_space(),
                button(icon(I_CLOSE, 16.0, MUTED))
                    .padding(6)
                    .on_press(Message::ClosePlaylistModal)
                    .style(|_, s| round_btn_style(s)),
            ]
            .align_y(Vertical::Center),
        ]
        .spacing(14);

        if self.playlist_modal_track.is_some() {
            let local_pls: Vec<&Playlist> = self
                .playlists
                .iter()
                .filter(|p| p.source.as_deref() != Some("youtube"))
                .collect();
            if !local_pls.is_empty() {
                let mut list = column![].spacing(4);
                for pl in local_pls {
                    list = list.push(
                        button(
                            row![
                                text(ellipsize(&pl.name, 30)).size(14).font(SEMIBOLD).color(TEXT),
                                horizontal_space(),
                                text(format!("{} şarkı", pl.tracks.len())).size(12).color(MUTED),
                            ]
                            .align_y(Vertical::Center),
                        )
                        .width(Length::Fill)
                        .padding([10, 14])
                        .on_press(Message::AddTrackToPlaylist(pl.id.clone()))
                        .style(|_, s| row_btn_style(false, s, 10.0)),
                    );
                }
                modal_col = modal_col.push(
                    scrollable(list)
                        .height(Length::Shrink)
                        .direction(thin_scroll(false)),
                );
            } else {
                modal_col = modal_col.push(
                    text("Henüz yerel çalma listen yok. Aşağıdan hemen oluştur:")
                        .size(12.5)
                        .color(MUTED),
                );
            }
        }

        let input = text_input("Yeni liste adı…", &self.new_playlist_name)
            .on_input(Message::NewPlaylistInput)
            .on_submit(Message::CreatePlaylistConfirm)
            .padding([10, 14])
            .size(13.5)
            .width(Length::Fill)
            .style(|_, status| {
                let focused = matches!(status, text_input::Status::Focused);
                text_input::Style {
                    background: white(if focused { 0.09 } else { 0.06 }).into(),
                    border: Border {
                        radius: 10.0.into(),
                        width: 1.0,
                        color: if focused { ACCENT } else { LINE_STRONG },
                    },
                    icon: MUTED,
                    placeholder: MUTED,
                    value: TEXT,
                    selection: Color { a: 0.4, ..ACCENT },
                }
            });

        let btn_lbl = if self.playlist_modal_track.is_some() {
            "Oluştur ve Ekle"
        } else {
            "Oluştur"
        };

        modal_col = modal_col.push(
            row![input, accent_btn(btn_lbl, Message::CreatePlaylistConfirm)]
                .spacing(8)
                .align_y(Vertical::Center),
        );

        let card = container(modal_col)
            .width(420)
            .padding(20)
            .style(|_| container::Style {
                background: Some(rgb(0x1a, 0x1a, 0x1e).into()),
                border: Border { radius: 18.0.into(), width: 1.0, color: LINE_STRONG },
                shadow: Shadow { color: black(0.7), offset: Vector::new(0.0, 18.0), blur_radius: 44.0 },
                ..Default::default()
            });

        opaque(
            container(card)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(|_| container::Style {
                    background: Some(black(0.65).into()),
                    ..Default::default()
                }),
        )
    }
}

// ---------- frameless window resizing ----------
#[derive(Debug, Clone, Copy)]
enum Edge {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

/// Thin invisible strips along the window edges; pressing one starts a native resize.
fn resize_edges() -> Element<'static, Message> {
    use iced::mouse::Interaction as I;
    const B: f32 = 5.0; // edge thickness
    const C: f32 = 12.0; // corner size
    let zone = |w: Length, h: Length, e: Edge, cursor: I| -> Element<'static, Message> {
        mouse_area(Space::new(w, h)).on_press(Message::ResizeFrom(e)).interaction(cursor).into()
    };
    column![
        row![
            zone(Length::Fixed(C), Length::Fixed(B), Edge::NW, I::ResizingDiagonallyDown),
            zone(Length::Fill, Length::Fixed(B), Edge::N, I::ResizingVertically),
            zone(Length::Fixed(C), Length::Fixed(B), Edge::NE, I::ResizingDiagonallyUp),
        ],
        row![
            zone(Length::Fixed(B), Length::Fill, Edge::W, I::ResizingHorizontally),
            Space::new(Length::Fill, Length::Fill),
            zone(Length::Fixed(B), Length::Fill, Edge::E, I::ResizingHorizontally),
        ]
        .height(Length::Fill),
        row![
            zone(Length::Fixed(C), Length::Fixed(B), Edge::SW, I::ResizingDiagonallyUp),
            zone(Length::Fill, Length::Fixed(B), Edge::S, I::ResizingVertically),
            zone(Length::Fixed(C), Length::Fixed(B), Edge::SE, I::ResizingDiagonallyDown),
        ],
    ]
    .into()
}

/// Hand the drag to Windows as if the user grabbed a real window border
/// (same technique winit uses for `drag_resize_window`).
#[cfg(windows)]
fn begin_resize(handle: raw_window_handle::WindowHandle<'_>, edge: Edge) {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, PostMessageW, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT,
        HTTOPRIGHT, WM_NCLBUTTONDOWN,
    };
    let RawWindowHandle::Win32(w) = handle.as_raw() else { return };
    let hwnd = w.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    let hit = match edge {
        Edge::N => HTTOP,
        Edge::S => HTBOTTOM,
        Edge::E => HTRIGHT,
        Edge::W => HTLEFT,
        Edge::NE => HTTOPRIGHT,
        Edge::NW => HTTOPLEFT,
        Edge::SE => HTBOTTOMRIGHT,
        Edge::SW => HTBOTTOMLEFT,
    };
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let lparam = ((pt.x as u32 & 0xFFFF) | ((pt.y as u32 & 0xFFFF) << 16)) as isize;
        ReleaseCapture();
        PostMessageW(hwnd, WM_NCLBUTTONDOWN, hit as usize, lparam);
    }
}

#[cfg(not(windows))]
fn begin_resize(_handle: raw_window_handle::WindowHandle<'_>, _edge: Edge) {}

// ---------- small widgets ----------
fn chip(label: &str, active: bool, msg: Message) -> Element<'_, Message> {
    button(text(label).size(12.5).font(if active { SEMIBOLD } else { BASE_FONT }))
        .padding([5, 16])
        .on_press(msg)
        .style(move |_, status| button::Style {
            background: Some(if active { ACCENT } else if matches!(status, button::Status::Hovered) { white(0.06) } else { Color::TRANSPARENT }.into()),
            text_color: if active { ON_ACCENT } else { TEXT },
            border: Border { radius: 999.0.into(), width: 1.0, color: if active { ACCENT } else { LINE_STRONG } },
            shadow: Shadow::default(),
        })
        .into()
}

fn accent_btn(label: &str, msg: Message) -> Element<'_, Message> {
    button(text(label).size(13).font(BOLD).color(ON_ACCENT))
        .padding([9, 20])
        .on_press(msg)
        .style(|_, s| button::Style {
            background: Some(if matches!(s, button::Status::Hovered) { mix(ACCENT, Color::WHITE, 0.15) } else { ACCENT }.into()),
            text_color: ON_ACCENT,
            border: Border { radius: 999.0.into(), ..Default::default() },
            shadow: Shadow::default(),
        })
        .into()
}

fn outline_btn(label: &str, msg: Message) -> Element<'_, Message> {
    button(row![icon(I_MIX, 15.0, TEXT), text(label).size(13).font(SEMIBOLD).color(TEXT)].spacing(6).align_y(Vertical::Center))
        .padding([8, 18])
        .on_press(msg)
        .style(|_, s| button::Style {
            background: Some(if matches!(s, button::Status::Hovered) { white(0.06) } else { Color::TRANSPARENT }.into()),
            text_color: TEXT,
            border: Border { radius: 999.0.into(), width: 1.0, color: white(0.25) },
            shadow: Shadow::default(),
        })
        .into()
}

fn link_btn(label: &str, msg: Message) -> Element<'_, Message> {
    button(text(label).size(12.5).color(MUTED))
        .on_press(msg)
        .style(|_, _| button::Style { background: None, text_color: MUTED, ..Default::default() })
        .into()
}

fn round_icon_button(ic: &'static str, msg: Message) -> Element<'static, Message> {
    button(icon(ic, 17.0, TEXT))
        .padding(8)
        .on_press(msg)
        .style(|_, status| button::Style {
            background: Some(if matches!(status, button::Status::Hovered) { white(0.08) } else { Color::TRANSPARENT }.into()),
            text_color: TEXT,
            border: Border { radius: 999.0.into(), width: 1.0, color: LINE_STRONG },
            shadow: Shadow::default(),
        })
        .into()
}

fn pbtn(ic: &'static str, size: f32, on: bool, msg: Message) -> Element<'static, Message> {
    button(icon(ic, size, if on { ACCENT } else { TEXT })).padding(7).on_press(msg).style(|_, s| round_btn_style(s)).into()
}

fn divider() -> Element<'static, Message> {
    container(container(Space::new(1, 30)).style(|_| rounded_bg(LINE_STRONG, 0.0))).padding([0, 6]).into()
}

fn card_shade(size: f32) -> Element<'static, Message> {
    container(Space::new(size, size))
        .style(|_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(
                Linear::new(Radians(PI)).add_stop(0.45, Color::TRANSPARENT).add_stop(1.0, black(0.8)),
            ))),
            border: Border { radius: 18.0.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

fn section<'a>(title: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    column![text(title).size(21).font(SEMIBOLD).color(TEXT), body].spacing(14).into()
}

/// LRCLIB synced lyrics first, then YouTube Music's own lyrics.
async fn load_lyrics(http: reqwest::Client, ytm: ytm::YtmClient, t: Track) -> Option<LyricsView> {
    if let Some((synced, plain, source)) = lyrics::fetch_lrclib_lyrics(&http, &t.title, &t.artist).await {
        return Some(LyricsView { track_id: t.id, synced, plain, source });
    }
    let (body, footer) = ytm.fetch_lyrics(&t.id).await.ok()??;
    Some(LyricsView { track_id: t.id, synced: None, plain: body, source: footer })
}

impl Player {
    fn ytm_cookie_missing(&self) -> bool {
        self.library.lock().map(|l| l.auth_cookie.is_none()).unwrap_or(true)
    }
}

fn main() -> iced::Result {
    updater::cleanup_old_binary();
    // GPU renderer (wgpu) by default. The software renderer (tiny-skia) uses less RAM
    // (~61 vs ~153 MB) but its partial-redraw tracking leaves the floating player capsule
    // half-drawn with black blocks where its shadow overlaps other layers.
    // ICED_BACKEND=tiny-skia still selects it (e.g. on machines without a working GPU driver).
    if std::env::var_os("ICED_BACKEND").is_none() {
        std::env::set_var("ICED_BACKEND", "wgpu");
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")))
        .try_init();

    let player = match Player::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("YTM Native başlatılamadı: {e}");
            std::process::exit(1);
        }
    };

    iced::application("YTM Native", App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| Theme::Dark)
        .window(window::Settings {
            size: Size::new(1240.0, 780.0),
            min_size: Some(Size::new(360.0, 150.0)),
            visible: std::env::var("YTM_TEST_HIDDEN").is_err(),
            decorations: false,
            #[cfg(target_os = "windows")]
            platform_specific: window::settings::PlatformSpecific { undecorated_shadow: true, ..Default::default() },
            icon: window::icon::from_file_data(APP_ICON_PNG, None).ok(),
            ..Default::default()
        })
        .default_font(BASE_FONT)
        .antialiasing(true)
        .run_with(move || App::new(player))
}
