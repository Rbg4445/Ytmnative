use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use futures::future::{BoxFuture, Shared};
use futures::{FutureExt, StreamExt};
use reqwest::header::USER_AGENT;
use rustypipe::client::{ClientType, RustyPipe, RustyPipeBuilder, RustyPipeQuery};
use rustypipe::model::{ArtistItem, TrackItem};
use sha1::{Digest, Sha1};
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::model::{
    ArtistDetailDto, ArtistSummary, HlsSegment, Playlist, ResolvedStream, SearchBundleDto,
    StreamKind, StreamSource, Track,
};

/// Everything pulled from the signed-in YouTube Music account in one sync.
pub struct AccountSnapshot {
    pub liked: Option<Vec<Track>>,
    pub playlists: Option<Vec<Playlist>>,
    pub artists: Option<Vec<ArtistSummary>>,
}

/// Maximum number of pre-resolved stream URLs kept in RAM (supports hover-prefetch + next 2 tracks).
const STREAM_CACHE_CAP: usize = 12;

/// A stream resolution that is already running; later callers for the same video await it.
type InflightResolve = Shared<BoxFuture<'static, std::result::Result<ResolvedStream, String>>>;

#[derive(Clone)]
pub struct YtmClient {
    rp: RustyPipe,
    http: reqwest::Client,
    stream_cache: Arc<Mutex<VecDeque<ResolvedStream>>>,
    visitor_data: Arc<Mutex<Option<String>>>,
    raw_cookie: Arc<Mutex<Option<String>>>,
    /// Last known result of the account check; decides whether radio/mix requests are sent
    /// with the user's session (personalized) or anonymously.
    auth_ok: Arc<AtomicBool>,
    /// Resolutions in progress (hover/upcoming prefetch + click on the same song share one).
    inflight: Arc<std::sync::Mutex<HashMap<String, InflightResolve>>>,
}

impl YtmClient {
    pub fn new(initial_cookie: Option<String>) -> Result<Self> {
        let mut builder = RustyPipeBuilder::new();
        let rp_dir = persistent_rp_dir();
        let _ = std::fs::create_dir_all(&rp_dir);
        if let Some(dir_str) = rp_dir.to_str() {
            builder = builder.storage_dir(dir_str);
        }
        let rp = builder.build().context("failed to initialize RustyPipe client")?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(4)
            .build()?;
        Ok(Self {
            rp,
            http,
            stream_cache: Arc::new(Mutex::new(VecDeque::with_capacity(STREAM_CACHE_CAP))),
            visitor_data: Arc::new(Mutex::new(None)),
            raw_cookie: Arc::new(Mutex::new(initial_cookie)),
            auth_ok: Arc::new(AtomicBool::new(false)),
            inflight: Arc::new(std::sync::Mutex::new(HashMap::new())),
        })
    }

    pub async fn get_raw_cookie(&self) -> Option<String> {
        self.raw_cookie.lock().await.clone()
    }

    /// Check whether a valid YouTube cookie session is available.
    /// If RustyPipe's own cache lost the cookie but library.json still has it, re-apply it.
    pub async fn is_youtube_connected(&self) -> bool {
        let mut ok = Self::safe_rp(self.rp.user_auth_check_cookie()).await.is_ok();
        if !ok {
            let saved = self.raw_cookie.lock().await.clone();
            if let Some(cookie) = saved.filter(|c| !c.trim().is_empty()) {
                ok = Self::safe_rp(self.rp.user_auth_set_cookie(cookie)).await.is_ok()
                    && Self::safe_rp(self.rp.user_auth_check_cookie()).await.is_ok();
            }
        }
        self.auth_ok.store(ok, Ordering::Relaxed);
        ok
    }

    /// Query that carries the user's session when an account is connected, so radios and
    /// mixes are personalized like on music.youtube.com (anonymous otherwise).
    fn user_query(&self) -> RustyPipeQuery {
        let q = self.rp.query();
        if self.auth_ok.load(Ordering::Relaxed) {
            q.authenticated()
        } else {
            q
        }
    }

    /// Connect YouTube account using either Netscape cookies.txt content or a raw Cookie header.
    pub async fn connect_with_cookies(&self, raw: &str) -> Result<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("Çerez (cookie) verisi boş"));
        }
        let header_cookie = if trimmed.contains('\t')
            || trimmed.starts_with("# Netscape")
            || trimmed.starts_with("# HTTP")
        {
            Self::safe_rp(self.rp.user_auth_set_cookie_txt(trimmed)).await?;
            parse_netscape_to_cookie_header(trimmed)
        } else {
            Self::safe_rp(self.rp.user_auth_set_cookie(trimmed.to_string())).await?;
            trimmed.to_string()
        };
        Self::safe_rp(self.rp.user_auth_check_cookie())
            .await
            .context("Çerez geçersiz veya oturum süresi dolmuş")?;
        {
            let mut guard = self.raw_cookie.lock().await;
            *guard = Some(header_cookie.clone());
        }
        self.auth_ok.store(true, Ordering::Relaxed);
        Ok(header_cookie)
    }

    /// One-click YouTube account connection by reading browser cookies via yt-dlp.
    pub async fn connect_from_browser(&self, browser: &str) -> Result<String> {
        let ytdlp_bin = find_ytdlp_bin();
        let tmp_cookie = std::env::temp_dir().join(format!("ytm_ck_{}.txt", std::process::id()));

        let mut cmd = Command::new(&ytdlp_bin);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        let output = cmd
            .args([
                "--cookies-from-browser",
                browser,
                "--cookies",
                tmp_cookie.to_str().unwrap_or("cookies.txt"),
                "--skip-download",
                "--no-warnings",
                "--playlist-items",
                "0",
                "https://music.youtube.com/",
            ])
            .output()
            .await
            .context("yt-dlp ile tarayıcı çerezleri okunamadı")?;

        let cookie_text = std::fs::read_to_string(&tmp_cookie).ok();
        let _ = std::fs::remove_file(&tmp_cookie);

        let text = cookie_text.ok_or_else(|| {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow!(
                "Tarayıcı ({browser}) çerezleri alınamadı (tarayıcı açıkken kilitli olabilir; kapatıp dene veya çerez yapıştır). Detay: {}",
                err.trim()
            )
        })?;

        self.connect_with_cookies(&text).await
    }

    /// Disconnect saved YouTube account cookies.
    pub async fn disconnect_youtube(&self) -> Result<()> {
        let _ = Self::safe_rp(self.rp.user_auth_remove_cookie()).await;
        let mut guard = self.raw_cookie.lock().await;
        *guard = None;
        self.auth_ok.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// Fetch the user's real YouTube Music listening history (requires connected YouTube account).
    pub async fn fetch_yt_history(&self) -> Result<Vec<Track>> {
        let q = self.rp.query().authenticated();
        let mut paginator = Self::safe_rp(q.music_history()).await?;
        let _ = Self::safe_rp(paginator.extend_limit(&q, 60)).await;
        let mut seen = std::collections::HashSet::new();
        let mut tracks = Vec::new();
        for h in paginator.items {
            if seen.insert(h.item.id.clone()) {
                tracks.push(track_from_item(h.item));
            }
            if tracks.len() >= 40 {
                break;
            }
        }
        Ok(tracks)
    }

    /// Fetch ALL of the user's liked songs (follows pagination; previously only the first 50).
    pub async fn fetch_yt_liked(&self) -> Result<Vec<Track>> {
        let q = self.rp.query().authenticated();
        let mut pl = Self::safe_rp(q.music_liked_tracks()).await?;
        if let Err(e) = Self::safe_rp(pl.tracks.extend_limit(&q, 2000)).await {
            warn!("Liked songs: could not load all pages: {e}");
        }
        Ok(pl.tracks.items.into_iter().map(track_from_item).collect())
    }

    /// Fetch the playlists in the user's YouTube Music library, with their tracks.
    /// Ids are prefixed with "yt:" so they never collide with local playlists.
    pub async fn fetch_yt_playlists(&self) -> Result<Vec<Playlist>> {
        let q = self.rp.query().authenticated();
        let mut saved = Self::safe_rp(q.music_saved_playlists()).await?;
        let _ = Self::safe_rp(saved.extend_limit(&q, 200)).await;

        let items: Vec<_> = saved
            .items
            .into_iter()
            // "LM" = liked songs (synced separately), "SE" = saved episodes
            .filter(|p| !p.is_podcast && p.id != "LM" && p.id != "SE")
            .take(80)
            .collect();

        let jobs = items.into_iter().map(|item| {
            let q = q.clone();
            async move {
                let mut pl = match Self::safe_rp(q.music_playlist(&item.id)).await {
                    Ok(pl) => pl,
                    Err(e) => {
                        warn!("Playlist {} could not be loaded: {e}", item.id);
                        return None;
                    }
                };
                let _ = Self::safe_rp(pl.tracks.extend_limit(&q, 500)).await;
                Some(Playlist {
                    id: format!("yt:{}", item.id),
                    name: item.name,
                    tracks: pl.tracks.items.into_iter().map(track_from_item).collect(),
                    source: Some("youtube".to_string()),
                })
            }
        });
        let loaded: Vec<Option<Playlist>> = futures::stream::iter(jobs).buffered(4).collect().await;
        Ok(loaded.into_iter().flatten().collect())
    }

    /// Fetch the artists in the user's YouTube Music library.
    pub async fn fetch_yt_artists(&self) -> Result<Vec<ArtistSummary>> {
        let q = self.rp.query().authenticated();
        let mut saved = Self::safe_rp(q.music_saved_artists()).await?;
        let _ = Self::safe_rp(saved.extend_limit(&q, 300)).await;
        Ok(saved.items.into_iter().map(artist_from_item).collect())
    }

    /// Pull liked songs, library playlists and library artists in parallel.
    /// Each part is `None` when it failed, so a partial failure never wipes existing data.
    pub async fn fetch_account_snapshot(&self) -> AccountSnapshot {
        let (liked, playlists, artists) = tokio::join!(
            self.fetch_yt_liked(),
            self.fetch_yt_playlists(),
            self.fetch_yt_artists()
        );
        for (what, err) in [
            ("liked", liked.as_ref().err()),
            ("playlists", playlists.as_ref().err()),
            ("artists", artists.as_ref().err()),
        ] {
            if let Some(e) = err {
                warn!("Account sync ({what}) failed: {e}");
            }
        }
        AccountSnapshot {
            liked: liked.ok(),
            playlists: playlists.ok(),
            artists: artists.ok(),
        }
    }

    /// Execute a RustyPipe future with panic protection (guards against upstream offline panics).
    async fn safe_rp<T, F>(fut: F) -> Result<T>
    where
        F: std::future::Future<Output = std::result::Result<T, rustypipe::error::Error>>,
    {
        match AssertUnwindSafe(fut).catch_unwind().await {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(e)) => Err(anyhow!("RustyPipe error: {e}")),
            Err(payload) => {
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "internal panic (possible network/offline error)".to_string()
                };
                warn!("Caught panic inside RustyPipe: {msg}");
                Err(anyhow!("Bağlantı/API hatası (çökme önlendi): {msg}"))
            }
        }
    }

    /// Search YouTube Music songs.
    pub async fn search_songs(&self, query: &str) -> Result<Vec<Track>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let res = Self::safe_rp(self.rp.query().music_search_tracks(q)).await?;
        let tracks = res
            .items
            .items
            .into_iter()
            .take(25)
            .map(track_from_item)
            .collect();
        Ok(tracks)
    }

    /// Combined search returning both matching Artists and Tracks in parallel (~300ms).
    pub async fn search_all(&self, query: &str) -> Result<SearchBundleDto> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(SearchBundleDto {
                artists: Vec::new(),
                tracks: Vec::new(),
            });
        }

        let q1 = self.rp.query();
        let q2 = self.rp.query();
        let (artists_res, tracks_res) = tokio::join!(
            Self::safe_rp(q1.music_search_artists(q)),
            Self::safe_rp(q2.music_search_tracks(q))
        );

        let mut artists: Vec<ArtistSummary> = artists_res
            .ok()
            .map(|r| {
                r.items
                    .items
                    .into_iter()
                    .take(8)
                    .map(artist_from_item)
                    .collect()
            })
            .unwrap_or_default();

        let tracks: Vec<Track> = tracks_res
            .map(|r| {
                r.items
                    .items
                    .into_iter()
                    .take(25)
                    .map(track_from_item)
                    .collect()
            })
            .unwrap_or_default();

        // Fallback: if artist search returned empty, synthesize artist cards from the top tracks' artist_id
        if artists.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for t in &tracks {
                if let Some(aid) = &t.artist_id {
                    if seen.insert(aid.clone()) {
                        artists.push(ArtistSummary {
                            id: aid.clone(),
                            name: t.artist.split(',').next().unwrap_or(&t.artist).trim().to_string(),
                            avatar_url: t.thumbnail_url.clone(),
                            subscriber_count: None,
                        });
                        if artists.len() >= 5 {
                            break;
                        }
                    }
                }
            }
        }

        Ok(SearchBundleDto { artists, tracks })
    }

    /// Fetch an artist's profile page, top songs, and similar artists.
    pub async fn fetch_artist(&self, artist_id: &str, artist_name_hint: Option<&str>) -> Result<ArtistDetailDto> {
        let id = artist_id.trim();
        if !id.is_empty() {
            if let Ok(artist) = Self::safe_rp(self.rp.query().music_artist(id, false)).await {
                let avatar_url = artist
                    .header_image
                    .iter()
                    .max_by_key(|t| t.width)
                    .or_else(|| artist.header_image.first())
                    .map(|t| t.url.clone());

                let mut tracks: Vec<Track> = artist
                    .tracks
                    .into_iter()
                    .take(35)
                    .map(track_from_item)
                    .collect();

                // If the artist page shelf had very few tracks, supplement via search or tracks_playlist_id
                if tracks.len() < 10 {
                    if let Some(pl_id) = &artist.tracks_playlist_id {
                        if let Ok(pl) = Self::safe_rp(self.rp.query().music_playlist(pl_id)).await {
                            let mut seen: std::collections::HashSet<String> =
                                tracks.iter().map(|t| t.id.clone()).collect();
                            for item in pl.tracks.items.into_iter().take(35) {
                                if seen.insert(item.id.clone()) {
                                    tracks.push(track_from_item(item));
                                }
                            }
                        }
                    }
                }
                if tracks.is_empty() {
                    if let Ok(extra) = self.search_songs(&artist.name).await {
                        tracks = extra;
                    }
                }

                let similar_artists = artist
                    .similar_artists
                    .into_iter()
                    .take(10)
                    .map(artist_from_item)
                    .collect();

                return Ok(ArtistDetailDto {
                    id: artist.id,
                    name: artist.name,
                    avatar_url,
                    description: artist.description,
                    subscriber_count: artist.subscriber_count,
                    tracks,
                    similar_artists,
                });
            }
        }

        // Fallback if artist_id wasn't a browseable channel ID: search by artist name
        let query_name = artist_name_hint
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(id);
        let tracks = self.search_songs(query_name).await.unwrap_or_default();
        let avatar_url = tracks.first().and_then(|t| t.thumbnail_url.clone());
        Ok(ArtistDetailDto {
            id: id.to_string(),
            name: query_name.to_string(),
            avatar_url,
            description: None,
            subscriber_count: None,
            tracks,
            similar_artists: Vec::new(),
        })
    }

    /// Ultra-fast (~50ms) YouTube autocomplete search suggestions.
    pub async fn search_suggestions(&self, query: &str) -> Result<Vec<String>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let resp = self
            .http
            .get("https://suggestqueries-clients6.google.com/complete/search")
            .query(&[("client", "firefox"), ("ds", "yt"), ("hl", "tr"), ("q", q)])
            .timeout(Duration::from_secs(2))
            .send()
            .await?;

        let text = resp.text().await?;
        let parsed: serde_json::Value = serde_json::from_str(&text)?;
        let mut out = Vec::new();
        if let Some(arr) = parsed.get(1).and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    let s = s.trim();
                    if !s.is_empty() && !out.iter().any(|x: &String| x.eq_ignore_ascii_case(s)) {
                        out.push(s.to_string());
                    }
                }
                if out.len() >= 8 {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Fetch YouTube Music auto-radio queue for a seed track ID.
    /// Uses the signed-in session when available, so the mix follows the user's taste.
    pub async fn fetch_radio(&self, video_id: &str) -> Result<Vec<Track>> {
        let q = self.user_query();
        let mut paginator = match Self::safe_rp(q.music_radio_track(video_id)).await {
            Ok(p) => p,
            // A stale session must not break playback: retry anonymously.
            Err(_) if self.auth_ok.load(Ordering::Relaxed) => {
                Self::safe_rp(self.rp.query().music_radio_track(video_id)).await?
            }
            Err(e) => return Err(e),
        };
        let _ = Self::safe_rp(paginator.extend_limit(&q, 40)).await;
        let tracks = paginator
            .items
            .into_iter()
            .filter(|t| t.id != video_id)
            .take(40)
            .map(track_from_item)
            .collect();
        Ok(tracks)
    }

    /// An artist "mix": YouTube's artist radio when the artist page offers one,
    /// otherwise a radio seeded from the artist's top song.
    pub async fn fetch_artist_mix(&self, artist_id: &str, artist_name: Option<&str>) -> Result<Vec<Track>> {
        let q = self.user_query();
        let id = artist_id.trim();
        if !id.is_empty() {
            if let Ok(artist) = Self::safe_rp(q.music_artist(id, false)).await {
                if let Some(radio_id) = artist.radio_id.as_deref() {
                    if let Ok(mut pag) = Self::safe_rp(q.music_radio(radio_id)).await {
                        let _ = Self::safe_rp(pag.extend_limit(&q, 40)).await;
                        if !pag.items.is_empty() {
                            return Ok(pag.items.into_iter().take(50).map(track_from_item).collect());
                        }
                    }
                }
                if let Some(top) = artist.tracks.into_iter().next() {
                    let mut out = vec![track_from_item(top.clone())];
                    out.extend(self.fetch_radio(&top.id).await.unwrap_or_default());
                    return Ok(out);
                }
            }
        }
        let name = artist_name.unwrap_or(id);
        let top = self
            .search_songs(name)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Sanatçı için şarkı bulunamadı"))?;
        let mut out = vec![top.clone()];
        out.extend(self.fetch_radio(&top.id).await.unwrap_or_default());
        Ok(out)
    }

    /// Fetch YouTube Music song lyrics (body, source footer).
    pub async fn fetch_lyrics(&self, video_id: &str) -> Result<Option<(String, String)>> {
        let details = Self::safe_rp(self.rp.query().music_details(video_id)).await?;
        let Some(lyrics_id) = details.lyrics_id else {
            return Ok(None);
        };
        let lyrics = Self::safe_rp(self.rp.query().music_lyrics(&lyrics_id)).await?;
        Ok(Some((lyrics.body, lyrics.footer)))
    }

    /// Build Home recommendations from the user's own taste:
    /// 1) seeds = YouTube Music history (when signed in), then local recent + liked songs;
    /// 2) 4 seeds (rotated by `refresh_nonce`) each start a radio; when signed in these radios
    ///    are personalized by YouTube;
    /// 3) the radios are interleaved so no single seed dominates the list.
    /// Generic fallbacks (charts / new releases) are used ONLY when there are no seeds at all;
    /// the old fixed genre searches produced unrelated songs and are gone.
    pub async fn fetch_home_feed(
        &self,
        local_seed_ids: &[String],
        exclude_ids: &[String],
        refresh_nonce: u64,
    ) -> (Vec<Track>, Vec<Track>) {
        const SEEDS_PER_REFRESH: usize = 4;
        const MAX_RECS: usize = 30;

        let yt_history = if self.is_youtube_connected().await {
            self.fetch_yt_history().await.unwrap_or_default()
        } else {
            Vec::new()
        };

        // Seed pool: history first (most recent taste), then local recent/liked.
        let mut seed_pool: Vec<String> = Vec::new();
        let mut pool_seen = std::collections::HashSet::new();
        for id in yt_history
            .iter()
            .take(20)
            .map(|t| &t.id)
            .chain(local_seed_ids.iter().take(20))
        {
            if pool_seen.insert(id.clone()) {
                seed_pool.push(id.clone());
            }
        }

        let mut seeds: Vec<String> = Vec::new();
        if !seed_pool.is_empty() {
            let len = seed_pool.len();
            let start = (refresh_nonce as usize * SEEDS_PER_REFRESH) % len;
            for i in 0..SEEDS_PER_REFRESH.min(len) {
                seeds.push(seed_pool[(start + i) % len].clone());
            }
        }

        let exclude_set: std::collections::HashSet<&str> =
            exclude_ids.iter().map(|s| s.as_str()).collect();
        // Songs already shown in "listen again" are pushed to the back, not repeated up front.
        let history_set: std::collections::HashSet<&str> =
            yt_history.iter().map(|t| t.id.as_str()).collect();

        let mut seen: std::collections::HashSet<String> = seeds.iter().cloned().collect();
        let mut fresh_recs = Vec::new();
        let mut fallback_recs = Vec::new();

        if !seeds.is_empty() {
            let radios: Vec<Vec<Track>> = futures::future::join_all(seeds.iter().map(|s| self.fetch_radio(s)))
                .await
                .into_iter()
                .map(|r| r.unwrap_or_default())
                .collect();

            // Round-robin interleave: 1st of each radio, then 2nd of each, ...
            let longest = radios.iter().map(|r| r.len()).max().unwrap_or(0);
            for i in 0..longest {
                for radio in &radios {
                    let Some(t) = radio.get(i) else { continue };
                    if !seen.insert(t.id.clone()) {
                        continue;
                    }
                    if exclude_set.contains(t.id.as_str()) || history_set.contains(t.id.as_str()) {
                        fallback_recs.push(t.clone());
                    } else {
                        fresh_recs.push(t.clone());
                    }
                }
            }
        } else {
            // Brand-new user with no history and no likes: popular music as a starting point.
            let charts = Self::safe_rp(self.rp.query().music_charts(None)).await.ok();
            let mut extra: Vec<Track> = charts
                .map(|c| {
                    c.top_tracks
                        .into_iter()
                        .chain(c.trending_tracks)
                        .map(track_from_item)
                        .collect()
                })
                .unwrap_or_default();
            if extra.is_empty() {
                extra = Self::safe_rp(self.rp.query().music_new_videos())
                    .await
                    .map(|items| items.into_iter().map(track_from_item).collect())
                    .unwrap_or_default();
            }
            for t in extra {
                if seen.insert(t.id.clone()) {
                    if exclude_set.contains(t.id.as_str()) {
                        fallback_recs.push(t);
                    } else {
                        fresh_recs.push(t);
                    }
                }
            }
        }

        let mut recs: Vec<Track> = fresh_recs.into_iter().take(MAX_RECS).collect();
        for t in fallback_recs {
            if recs.len() >= MAX_RECS {
                break;
            }
            recs.push(t);
        }

        (yt_history, recs)
    }

    /// Check cache, otherwise resolve audio stream in ~400ms using 100% pure Rust (VISIONOS HLS)!
    /// Falls back to yt-dlp only if the direct Innertube call fails.
    pub async fn resolve_stream(&self, video_id: &str) -> Result<ResolvedStream> {
        {
            let cache = self.stream_cache.lock().await;
            if let Some(hit) = cache.iter().find(|s| s.track_id == video_id && s.is_fresh()) {
                debug!("Stream cache hit for {video_id}");
                return Ok(hit.clone());
            }
        }

        // Join an identical resolution that is already running (e.g. started by hover or
        // upcoming-track prefetch) instead of doing all the network work twice.
        let fut = {
            let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(running) = map.get(video_id) {
                running.clone()
            } else {
                let me = self.clone();
                let vid = video_id.to_string();
                let fut: InflightResolve = async move {
                    let res = me.resolve_uncached(&vid).await.map_err(|e| e.to_string());
                    me.inflight
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&vid);
                    res
                }
                .boxed()
                .shared();
                map.insert(video_id.to_string(), fut.clone());
                fut
            }
        };
        fut.await.map_err(|e| anyhow!(e))
    }

    /// Race the HLS resolvers (VISIONOS, iOS) and use the FIRST one that succeeds.
    /// (Previously all resolvers were awaited together, so every song waited for the slowest
    /// server, up to the 15 s request timeout.)
    /// Direct progressive URLs (ANDROID_VR / iOS m4a) are NOT raced: YouTube answers them with
    /// HTTP 403 after the first ~1 MB without a PO token, so the song would stop mid-way.
    /// They are only a fallback, then yt-dlp as the very last resort.
    async fn resolve_uncached(&self, video_id: &str) -> Result<ResolvedStream> {
        let started = Instant::now();
        let ios_hls = async {
            match self.fetch_ios_loudness_and_hls(video_id).await? {
                (Some(stream), _) if matches!(stream.kind, StreamKind::Hls { .. }) => Ok(stream),
                _ => Err(anyhow!("iOS: no HLS stream")),
            }
        };
        let racers: Vec<BoxFuture<'_, Result<ResolvedStream>>> = vec![
            self.resolve_via_visionos_direct(video_id).boxed(),
            ios_hls.boxed(),
        ];

        // A hung request (each one may take up to the 15 s client timeout, and VISIONOS chains
        // several) must not hold the song hostage: give the HLS race 8 s, then fall back.
        let race = tokio::time::timeout(Duration::from_secs(8), futures::future::select_ok(racers))
            .await
            .unwrap_or_else(|_| Err(anyhow!("HLS resolvers timed out after 8 s")));
        let resolved = match race {
            Ok((stream, _still_running)) => stream,
            Err(hls_err) => {
                warn!("HLS resolvers failed for {video_id}: {hls_err}; trying direct stream...");
                match self.resolve_via_tv_or_android_vr(video_id).await {
                    Ok(s) => s,
                    Err(_) => match self.resolve_via_ytdlp_fast(video_id).await {
                        Ok(s) => s,
                        Err(_) => self.resolve_via_ytdlp_default(video_id, None).await?,
                    },
                }
            }
        };
        debug!("Resolved {video_id} in {} ms", started.elapsed().as_millis());

        self.cache_stream(resolved.clone()).await;
        Ok(resolved)
    }

    /// Pre-warm visitorData on startup so even the very first track click resolves in ~400ms.
    pub async fn warmup(&self) {
        let _ = self.get_visitor_data().await;
    }

    /// Fetch and cache YouTube Innertube `visitorData` once per session (~250ms first time, 0ms after).
    async fn get_visitor_data(&self) -> Result<String> {
        {
            let guard = self.visitor_data.lock().await;
            if let Some(vd) = guard.as_ref() {
                return Ok(vd.clone());
            }
        }

        let body = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": "2.20260708.00.00",
                    "hl": "en"
                }
            },
            "videoId": "dQw4w9WgXcQ"
        });
        let resp = self
            .http
            .post("https://www.youtube.com/youtubei/v1/next?prettyPrint=false")
            .json(&body)
            .send()
            .await
            .context("visitorData isteği başarısız")?;
        let json: serde_json::Value = resp.json().await?;
        let vd = json
            .get("responseContext")
            .and_then(|rc| rc.get("visitorData"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("visitorData alınamadı"))?
            .to_string();

        let mut guard = self.visitor_data.lock().await;
        *guard = Some(vd.clone());
        Ok(vd)
    }

    /// Pure-Rust sub-second (~350-450ms) Innertube VISIONOS player call returning unthrottled HLS AAC!
    async fn resolve_via_visionos_direct(&self, video_id: &str) -> Result<ResolvedStream> {
        let vd = self.get_visitor_data().await?;
        const VISION_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

        let body = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "VISIONOS",
                    "clientVersion": "1.02",
                    "deviceMake": "Apple",
                    "deviceModel": "RealityDevice17,1",
                    "userAgent": VISION_UA,
                    "osName": "visionOS",
                    "osVersion": "26.5.23O471",
                    "hl": "en",
                    "visitorData": vd
                }
            },
            "videoId": video_id,
            "playbackContext": {
                "contentPlaybackContext": {
                    "html5Preference": "HTML5_PREF_WANTS"
                }
            },
            "contentCheckOk": true,
            "racyCheckOk": true
        });

        let mut req = self
            .http
            .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
            .header(USER_AGENT, VISION_UA)
            .header("X-Youtube-Client-Name", "101")
            .header("X-Youtube-Client-Version", "1.02")
            .header("X-Goog-Visitor-Id", &vd)
            .header("Origin", "https://www.youtube.com");

        if let Some(cookie) = self.get_raw_cookie().await {
            if let Some(auth) = compute_sapisidhash(&cookie, "https://www.youtube.com") {
                req = req.header("Authorization", auth);
            }
            req = req.header("Cookie", cookie);
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .context("VISIONOS player isteği başarısız")?;

        let json: serde_json::Value = resp.json().await?;
        let status = json
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");
        if status != "OK" {
            // Invalidate visitorData on login/bot error so next call refreshes it
            if status == "LOGIN_REQUIRED" {
                let mut guard = self.visitor_data.lock().await;
                *guard = None;
            }
            return Err(anyhow!("VISIONOS playabilityStatus={status}"));
        }

        let hls_url = json
            .get("streamingData")
            .and_then(|sd| sd.get("hlsManifestUrl"))
            .and_then(|u| u.as_str())
            .ok_or_else(|| anyhow!("VISIONOS yanıtında hlsManifestUrl yok"))?;

        let loudness_db = json
            .get("playerConfig")
            .and_then(|pc| pc.get("audioConfig"))
            .and_then(|ac| ac.get("loudnessDb"))
            .and_then(|v| v.as_f64())
            .map(|v| v as f32);

        let segments = self.fetch_hls_audio_segments(hls_url, Some(VISION_UA)).await?;
        if segments.is_empty() {
            return Err(anyhow!("VISIONOS HLS playlist boş"));
        }
        let total_dur: f64 = segments.iter().map(|s| s.duration_secs).sum();

        Ok(ResolvedStream {
            track_id: video_id.to_string(),
            kind: StreamKind::Hls { segments },
            mime: "audio/aac".to_string(),
            bitrate: 128_000,
            duration_secs: Some(total_dur),
            loudness_db,
            user_agent: Some(VISION_UA.to_string()),
            source: StreamSource::RustyPipe,
            resolved_at: Instant::now(),
        })
    }

    async fn fetch_ios_loudness_and_hls(
        &self,
        video_id: &str,
    ) -> Result<(Option<ResolvedStream>, Option<f32>)> {
        let ios_query = self.rp.query();
        let ios_ua = ios_query.user_agent(ClientType::Ios).into_owned();
        let player = Self::safe_rp(ios_query.player_from_client(video_id, ClientType::Ios)).await?;
        let loudness = player.audio_streams.iter().find_map(|s| s.loudness_db);
        let duration_secs = if player.details.duration > 0 {
            Some(player.details.duration as f64)
        } else {
            player
                .audio_streams
                .iter()
                .find_map(|s| s.duration_ms.map(|ms| ms as f64 / 1000.0))
        };

        if let Some(hls_url) = player.hls_manifest_url.as_deref() {
            if let Ok(segments) = self.fetch_hls_audio_segments(hls_url, Some(&ios_ua)).await {
                if !segments.is_empty() {
                    let total_dur: f64 = segments.iter().map(|s| s.duration_secs).sum();
                    return Ok((
                        Some(ResolvedStream {
                            track_id: video_id.to_string(),
                            kind: StreamKind::Hls { segments },
                            mime: "audio/aac".to_string(),
                            bitrate: 128_000,
                            duration_secs: duration_secs.or(Some(total_dur)),
                            loudness_db: loudness,
                            user_agent: Some(ios_ua),
                            source: StreamSource::RustyPipe,
                            resolved_at: Instant::now(),
                        }),
                        loudness,
                    ));
                }
            }
        }

        // Also check direct M4A/AAC stream from RustyPipe iOS player (~300ms)!
        if let Some(best_m4a) = player
            .audio_streams
            .iter()
            .filter(|s| s.format == rustypipe::model::AudioFormat::M4a)
            .max_by_key(|s| s.bitrate)
        {
            return Ok((
                Some(ResolvedStream {
                    track_id: video_id.to_string(),
                    kind: StreamKind::Direct {
                        url: best_m4a.url.clone(),
                        content_length: Some(best_m4a.size),
                    },
                    mime: "audio/mp4".to_string(),
                    bitrate: best_m4a.bitrate,
                    duration_secs,
                    loudness_db: loudness,
                    user_agent: Some(ios_ua),
                    source: StreamSource::RustyPipe,
                    resolved_at: Instant::now(),
                }),
                loudness,
            ));
        }
        Ok((None, loudness))
    }

    /// Secondary pure-Rust Innertube fallback using ANDROID_VR & TV_SIMPLY (~350ms, no bot challenge).
    async fn resolve_via_tv_or_android_vr(&self, video_id: &str) -> Result<ResolvedStream> {
        let vd = self.get_visitor_data().await.unwrap_or_default();
        const VR_UA: &str = "com.google.android.apps.youtube.vr.oculus/1.60.19 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip";

        let body = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "ANDROID_VR",
                    "clientVersion": "1.60.19",
                    "deviceMake": "Oculus",
                    "deviceModel": "Quest 3",
                    "osName": "Android",
                    "osVersion": "12L",
                    "androidSdkVersion": 32,
                    "hl": "en",
                    "visitorData": vd
                }
            },
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true
        });

        let mut req = self
            .http
            .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
            .header(USER_AGENT, VR_UA)
            .header("X-Youtube-Client-Name", "28")
            .header("X-Youtube-Client-Version", "1.60.19")
            .header("X-Goog-Visitor-Id", &vd);

        if let Some(cookie) = self.get_raw_cookie().await {
            if let Some(auth) = compute_sapisidhash(&cookie, "https://www.youtube.com") {
                req = req.header("Authorization", auth);
            }
            req = req.header("Cookie", cookie);
        }

        let resp = req.json(&body).send().await?;
        let json: serde_json::Value = resp.json().await?;
        if json
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            != Some("OK")
        {
            return Err(anyhow!("ANDROID_VR not OK"));
        }

        let loudness_db = json
            .get("playerConfig")
            .and_then(|pc| pc.get("audioConfig"))
            .and_then(|ac| ac.get("loudnessDb"))
            .and_then(|v| v.as_f64())
            .map(|v| v as f32);

        let duration_secs = json
            .get("videoDetails")
            .and_then(|vd| vd.get("lengthSeconds"))
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse::<f64>().ok());

        if let Some(formats) = json
            .get("streamingData")
            .and_then(|sd| sd.get("adaptiveFormats"))
            .and_then(|af| af.as_array())
        {
            let mut best: Option<(String, u32, Option<u64>)> = None;
            for f in formats {
                let mime = f.get("mimeType").and_then(|m| m.as_str()).unwrap_or("");
                if !mime.starts_with("audio/mp4") {
                    continue;
                }
                let Some(url) = f.get("url").and_then(|u| u.as_str()) else {
                    continue;
                };
                let br = f.get("bitrate").and_then(|b| b.as_u64()).unwrap_or(128_000) as u32;
                let cl = f
                    .get("contentLength")
                    .and_then(|c| c.as_str())
                    .and_then(|c| c.parse::<u64>().ok());
                if best.as_ref().map(|b| br > b.1).unwrap_or(true) {
                    best = Some((url.to_string(), br, cl));
                }
            }
            if let Some((url, bitrate, content_length)) = best {
                return Ok(ResolvedStream {
                    track_id: video_id.to_string(),
                    kind: StreamKind::Direct {
                        url,
                        content_length,
                    },
                    mime: "audio/mp4".to_string(),
                    bitrate,
                    duration_secs,
                    loudness_db,
                    user_agent: Some(VR_UA.to_string()),
                    source: StreamSource::RustyPipe,
                    resolved_at: Instant::now(),
                });
            }
        }

        Err(anyhow!("ANDROID_VR adaptive audio not found"))
    }

    /// Ultra-fast yt-dlp HLS resolution (~2s instead of ~6s) by skipping webpage & JS challenge solver!
    async fn resolve_via_ytdlp_fast(&self, video_id: &str) -> Result<ResolvedStream> {
        let ytdlp_bin = find_ytdlp_bin();
        let watch_url = format!("https://music.youtube.com/watch?v={video_id}");
        let tmp_ck = self.write_temp_netscape_cookies("fast").await;
        let mut cmd = Command::new(&ytdlp_bin);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        cmd.args([
            "-f",
            "234/233",
            "--extractor-args",
            "youtube:player_skip=webpage,configs;player_client=visionos,android_vr,ios",
            "--get-url",
            "--no-playlist",
            "--no-warnings",
        ]);
        if let Some(ref ck_path) = tmp_ck {
            if let Some(s) = ck_path.to_str() {
                cmd.args(["--cookies", s]);
            }
        }
        let output = cmd
            .args(["--", &watch_url])
            .output()
            .await
            .context("yt-dlp çalıştırılamadı")?;

        if let Some(ck_path) = tmp_ck {
            let _ = std::fs::remove_file(ck_path);
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("yt-dlp fast hata: {}", stderr.trim()));
        }

        let url = String::from_utf8_lossy(&output.stdout)
            .lines()
            .find(|l| l.starts_with("https://"))
            .ok_or_else(|| anyhow!("yt-dlp fast URL döndürmedi"))?
            .trim()
            .to_string();

        let segments = self.fetch_hls_audio_segments(&url, None).await?;
        if segments.is_empty() {
            return Err(anyhow!("HLS playlist boş döndü"));
        }
        let total_dur: f64 = segments.iter().map(|s| s.duration_secs).sum();
        Ok(ResolvedStream {
            track_id: video_id.to_string(),
            kind: StreamKind::Hls { segments },
            mime: "audio/aac".to_string(),
            bitrate: 128_000,
            duration_secs: Some(total_dur),
            loudness_db: None,
            user_agent: None,
            source: StreamSource::YtDlp,
            resolved_at: Instant::now(),
        })
    }

    /// Force-bypass cache and re-resolve.
    pub async fn resolve_stream_fresh(&self, video_id: &str) -> Result<ResolvedStream> {
        {
            let mut cache = self.stream_cache.lock().await;
            cache.retain(|s| s.track_id != video_id);
        }
        self.resolve_stream(video_id).await
    }

    /// Prefetch a track's stream URL in the background for instant transitions.
    pub async fn prefetch_stream(&self, video_id: &str) {
        {
            let cache = self.stream_cache.lock().await;
            if cache.iter().any(|s| s.track_id == video_id && s.is_fresh()) {
                return;
            }
        }
        if let Err(e) = self.resolve_stream(video_id).await {
            debug!("Prefetch failed for {video_id}: {e}");
        }
    }

    async fn cache_stream(&self, stream: ResolvedStream) {
        let mut cache = self.stream_cache.lock().await;
        cache.retain(|s| s.track_id != stream.track_id);
        if cache.len() >= STREAM_CACHE_CAP {
            cache.pop_front();
        }
        cache.push_back(stream);
    }

    /// Fetch either an HLS master playlist or media playlist and extract audio segments.
    async fn fetch_hls_audio_segments(
        &self,
        manifest_url: &str,
        ua: Option<&str>,
    ) -> Result<Vec<HlsSegment>> {
        let text = self.fetch_text(manifest_url, ua).await?;

        // If this is a master playlist containing EXT-X-MEDIA:TYPE=AUDIO, pick the best audio playlist URI.
        if text.contains("#EXT-X-MEDIA:") {
            let mut audio_uris: Vec<&str> = Vec::new();
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with("#EXT-X-MEDIA:") && line.contains("TYPE=AUDIO") {
                    if let Some(uri) = extract_m3u8_attr(line, "URI") {
                        audio_uris.push(uri);
                    }
                }
            }
            // Prefer highest-quality audio track: 256k Premium (itag 141/235) -> 130k HQ (itag 234) -> last audio track.
            let chosen = audio_uris
                .iter()
                .find(|u| u.contains("/itag/141/") || u.contains("/itag/235/"))
                .or_else(|| audio_uris.iter().find(|u| u.contains("/itag/234/")))
                .or_else(|| audio_uris.last())
                .copied()
                .ok_or_else(|| anyhow!("HLS master playlist içinde ses akışı bulunamadı"))?;

            let media_text = self.fetch_text(chosen, ua).await?;
            return Ok(parse_media_playlist(&media_text));
        }

        Ok(parse_media_playlist(&text))
    }

    async fn fetch_text(&self, url: &str, ua: Option<&str>) -> Result<String> {
        let mut req = self.http.get(url);
        if let Some(u) = ua {
            req = req.header(USER_AGENT, u);
        }
        let resp = req.send().await.context("HLS manifest isteği başarısız")?;
        if !resp.status().is_success() {
            return Err(anyhow!("HLS manifest HTTP {}", resp.status()));
        }
        Ok(resp.text().await?)
    }

    async fn write_temp_netscape_cookies(&self, tag: &str) -> Option<PathBuf> {
        let raw = self.get_raw_cookie().await?;
        if raw.trim().is_empty() {
            return None;
        }
        let path = std::env::temp_dir().join(format!(
            "ytm_auth_{}_{}.txt",
            tag,
            std::process::id()
        ));
        let mut lines = vec!["# Netscape HTTP Cookie File".to_string()];
        for pair in raw.split(';') {
            let pair = pair.trim();
            if let Some((k, v)) = pair.split_once('=') {
                let k = k.trim();
                let v = v.trim();
                if !k.is_empty() {
                    lines.push(format!(".youtube.com\tTRUE\t/\tTRUE\t2147483647\t{k}\t{v}"));
                }
            }
        }
        std::fs::write(&path, lines.join("\n")).ok()?;
        Some(path)
    }

    async fn resolve_via_ytdlp_default(
        &self,
        video_id: &str,
        loudness_db: Option<f32>,
    ) -> Result<ResolvedStream> {
        let ytdlp_bin = find_ytdlp_bin();
        let watch_url = format!("https://music.youtube.com/watch?v={video_id}");
        let tmp_ck = self.write_temp_netscape_cookies("def").await;
        let mut cmd = Command::new(&ytdlp_bin);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        cmd.args([
            "-f",
            "234/233/bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio",
            "--extractor-args",
            "youtube:player_client=visionos,android_vr,ios,tv",
            "--dump-json",
            "--no-playlist",
            "--no-warnings",
        ]);
        if let Some(ref ck_path) = tmp_ck {
            if let Some(s) = ck_path.to_str() {
                cmd.args(["--cookies", s]);
            }
        }
        let output = cmd
            .args(["--", &watch_url])
            .output()
            .await
            .context("yt-dlp çalıştırılamadı (sistemde kurulu olmayabilir)")?;

        if let Some(ck_path) = tmp_ck {
            let _ = std::fs::remove_file(ck_path);
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("yt-dlp hata verdi: {}", stderr.trim()));
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("yt-dlp JSON çıktısı ayrıştırılamadı")?;

        let url = json
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("yt-dlp JSON içinde 'url' alanı yok"))?
            .to_string();

        let protocol = json.get("protocol").and_then(|v| v.as_str()).unwrap_or("");
        let duration_secs = json.get("duration").and_then(|v| v.as_f64());
        let user_agent = json
            .get("http_headers")
            .and_then(|h| h.get("User-Agent"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if protocol.contains("m3u8") || url.contains("hls_playlist") || url.contains(".m3u8") {
            let segments = self
                .fetch_hls_audio_segments(&url, user_agent.as_deref())
                .await?;
            if segments.is_empty() {
                return Err(anyhow!("yt-dlp HLS playlist boş döndü"));
            }
            return Ok(ResolvedStream {
                track_id: video_id.to_string(),
                kind: StreamKind::Hls { segments },
                mime: "audio/aac".to_string(),
                bitrate: 128_000,
                duration_secs,
                loudness_db,
                user_agent,
                source: StreamSource::YtDlp,
                resolved_at: Instant::now(),
            });
        }

        let ext = json.get("ext").and_then(|v| v.as_str()).unwrap_or("m4a");
        let mime = match ext {
            "m4a" | "mp4" => "audio/mp4".to_string(),
            "mp3" => "audio/mpeg".to_string(),
            "webm" => "audio/webm".to_string(),
            other => format!("audio/{other}"),
        };
        let bitrate = json
            .get("abr")
            .and_then(|v| v.as_f64())
            .map(|kbps| (kbps * 1000.0) as u32)
            .unwrap_or(128_000);
        let content_length = json.get("filesize").and_then(|v| v.as_u64());

        Ok(ResolvedStream {
            track_id: video_id.to_string(),
            kind: StreamKind::Direct {
                url,
                content_length,
            },
            mime,
            bitrate,
            duration_secs,
            loudness_db,
            user_agent,
            source: StreamSource::YtDlp,
            resolved_at: Instant::now(),
        })
    }
}

fn extract_m3u8_attr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=\"");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn parse_media_playlist(text: &str) -> Vec<HlsSegment> {
    let mut segments = Vec::new();
    let mut pending_dur = 7.0_f64;

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let num = rest.split(',').next().unwrap_or("7.0").trim();
            if let Ok(d) = num.parse::<f64>() {
                pending_dur = d;
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            segments.push(HlsSegment {
                url: line.to_string(),
                duration_secs: pending_dur,
            });
        }
    }

    segments
}

fn persistent_rp_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    #[cfg(not(windows))]
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("ytm-native").join("rp_cache")
}

fn find_ytdlp_bin() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
            let winget_link = PathBuf::from(local_app)
                .join("Microsoft")
                .join("WinGet")
                .join("Links")
                .join("yt-dlp.exe");
            if winget_link.exists() {
                return winget_link;
            }
        }
    }
    PathBuf::from("yt-dlp")
}

fn track_from_item(item: TrackItem) -> Track {
    let artist = if item.artists.is_empty() {
        "Bilinmeyen Sanatçı".to_string()
    } else {
        item.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let artist_id = item
        .artist_id
        .or_else(|| item.artists.iter().find_map(|a| a.id.clone()));

    // Pick a crisp thumbnail (~320px) while keeping RAM usage minimal.
    let thumbnail_url = item
        .cover
        .iter()
        .min_by_key(|t| t.width.abs_diff(320))
        .or_else(|| item.cover.first())
        .map(|t| t.url.clone());

    Track {
        id: item.id,
        title: item.name,
        artist,
        artist_id,
        album: item.album.map(|a| a.name),
        duration_secs: item.duration,
        thumbnail_url,
    }
}

fn artist_from_item(item: ArtistItem) -> ArtistSummary {
    let avatar_url = item
        .avatar
        .iter()
        .min_by_key(|t| t.width.abs_diff(320))
        .or_else(|| item.avatar.first())
        .map(|t| t.url.clone());

    ArtistSummary {
        id: item.id,
        name: item.name,
        avatar_url,
        subscriber_count: item.subscriber_count,
    }
}

fn parse_netscape_to_cookie_header(txt: &str) -> String {
    let mut pairs = Vec::new();
    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() >= 7 && cols[0].contains("youtube.com") {
            pairs.push(format!("{}={}", cols[5], cols[6]));
        }
    }
    pairs.join("; ")
}

fn compute_sapisidhash(cookie: &str, origin: &str) -> Option<String> {
    let sapisid = cookie.split(';').find_map(|c| {
        let c = c.trim();
        c.strip_prefix("SAPISID=")
            .or_else(|| c.strip_prefix("__Secure-3PAPISID="))
            .or_else(|| c.strip_prefix("__Secure-1PAPISID="))
    })?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let mut hasher = Sha1::new();
    hasher.update(format!("{now} {sapisid} {origin}").as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Some(format!("SAPISIDHASH {now}_{hex}"))
}
