use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use futures::FutureExt;
use reqwest::header::USER_AGENT;
use rustypipe::client::{ClientType, RustyPipe, RustyPipeBuilder};
use rustypipe::model::TrackItem;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::model::{HlsSegment, ResolvedStream, StreamKind, StreamSource, Track};

/// Maximum number of pre-resolved stream URLs kept in RAM (supports hover-prefetch + next 2 tracks).
const STREAM_CACHE_CAP: usize = 12;

#[derive(Clone)]
pub struct YtmClient {
    rp: RustyPipe,
    http: reqwest::Client,
    stream_cache: Arc<Mutex<VecDeque<ResolvedStream>>>,
    visitor_data: Arc<Mutex<Option<String>>>,
}

impl YtmClient {
    pub fn new() -> Result<Self> {
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
        })
    }

    /// Check whether a valid YouTube cookie session is stored in RustyPipe.
    pub async fn is_youtube_connected(&self) -> bool {
        Self::safe_rp(self.rp.user_auth_check_cookie()).await.is_ok()
    }

    /// Connect YouTube account using either Netscape cookies.txt content or a raw Cookie header.
    pub async fn connect_with_cookies(&self, raw: &str) -> Result<()> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("Çerez (cookie) verisi boş"));
        }
        if trimmed.contains('\t') || trimmed.starts_with("# Netscape") || trimmed.starts_with("# HTTP") {
            Self::safe_rp(self.rp.user_auth_set_cookie_txt(trimmed)).await?;
        } else {
            Self::safe_rp(self.rp.user_auth_set_cookie(trimmed.to_string())).await?;
        }
        Self::safe_rp(self.rp.user_auth_check_cookie())
            .await
            .context("Çerez geçersiz veya oturum süresi dolmuş")?;
        Ok(())
    }

    /// One-click YouTube account connection by reading browser cookies via yt-dlp.
    pub async fn connect_from_browser(&self, browser: &str) -> Result<()> {
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
        Ok(())
    }

    /// Fetch the user's real YouTube Music listening history (requires connected YouTube account).
    pub async fn fetch_yt_history(&self) -> Result<Vec<Track>> {
        let paginator = Self::safe_rp(self.rp.query().music_history()).await?;
        let mut seen = std::collections::HashSet::new();
        let mut tracks = Vec::new();
        for h in paginator.items {
            if seen.insert(h.item.id.clone()) {
                tracks.push(track_from_item(h.item));
            }
            if tracks.len() >= 30 {
                break;
            }
        }
        Ok(tracks)
    }

    /// Fetch the user's real YouTube Music liked songs (requires connected YouTube account).
    pub async fn fetch_yt_liked(&self) -> Result<Vec<Track>> {
        let pl = Self::safe_rp(self.rp.query().music_liked_tracks()).await?;
        Ok(pl
            .tracks
            .items
            .into_iter()
            .take(50)
            .map(track_from_item)
            .collect())
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
    pub async fn fetch_radio(&self, video_id: &str) -> Result<Vec<Track>> {
        let paginator = Self::safe_rp(self.rp.query().music_radio_track(video_id)).await?;
        let tracks = paginator
            .items
            .into_iter()
            .filter(|t| t.id != video_id)
            .take(25)
            .map(track_from_item)
            .collect();
        Ok(tracks)
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

    /// Fetch personalized Home recommendations in parallel:
    /// 1) Real YouTube Music listening history seeds (only when logged in),
    /// 2) Local recently played + liked seeds,
    /// 3) YouTube Music trending/new & rotated discovery fallback.
    /// When `refresh_nonce > 0` or `exclude_ids` is provided, rotates seeds and
    /// prioritizes unseen tracks so clicking "Yenile" always delivers fresh picks.
    pub async fn fetch_home_feed(
        &self,
        local_seed_ids: &[String],
        exclude_ids: &[String],
        refresh_nonce: u64,
    ) -> (Vec<Track>, Vec<Track>) {
        let yt_history = if self.is_youtube_connected().await {
            self.fetch_yt_history().await.unwrap_or_default()
        } else {
            Vec::new()
        };

        // Build a diverse pool of seed track IDs from YT history + local recent/liked
        let mut seed_pool: Vec<String> = Vec::new();
        let mut pool_seen = std::collections::HashSet::new();
        for t in yt_history.iter().take(16) {
            if pool_seen.insert(t.id.clone()) {
                seed_pool.push(t.id.clone());
            }
        }
        for id in local_seed_ids.iter().take(16) {
            if pool_seen.insert(id.clone()) {
                seed_pool.push(id.clone());
            }
        }

        // Rotate which 2 seeds we pick based on refresh_nonce so each refresh explores new mixes
        let mut seeds: Vec<String> = Vec::new();
        if !seed_pool.is_empty() {
            let len = seed_pool.len();
            let start = ((refresh_nonce as usize) * 2) % len;
            seeds.push(seed_pool[start].clone());
            if len > 1 {
                seeds.push(seed_pool[(start + 1) % len].clone());
            }
        }

        let exclude_set: std::collections::HashSet<&str> =
            exclude_ids.iter().map(|s| s.as_str()).collect();

        let mut fresh_recs = Vec::new();
        let mut fallback_recs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for s in &seeds {
            seen.insert(s.clone());
        }

        if !seeds.is_empty() {
            let futs = seeds.iter().map(|s| self.fetch_radio(s));
            let results = futures::future::join_all(futs).await;
            for res in results {
                if let Ok(radio) = res {
                    for t in radio {
                        if !seen.insert(t.id.clone()) {
                            continue;
                        }
                        if exclude_set.contains(t.id.as_str()) {
                            fallback_recs.push(t);
                        } else {
                            fresh_recs.push(t);
                        }
                    }
                }
            }
        }

        // If user has little/no history or clicked refresh repeatedly and exhausted first-hop radio,
        // pull fresh discovery picks rotated by refresh_nonce.
        if fresh_recs.len() < 12 {
            let discovery_queries = [
                "türkçe pop hit",
                "en çok dinlenen şarkılar 2026",
                "türkçe rap trap hit",
                "chill lofi türkçe indie",
                "türkçe rock alternatif",
                "global top hits music",
                "elektronik deep house mix",
                "akustik sakin şarkılar",
            ];
            let extra = if refresh_nonce == 0 && seeds.is_empty() {
                Self::safe_rp(self.rp.query().music_new_videos())
                    .await
                    .map(|items| items.into_iter().take(20).map(track_from_item).collect::<Vec<_>>())
                    .unwrap_or_default()
            } else {
                let q = discovery_queries[(refresh_nonce as usize) % discovery_queries.len()];
                self.search_songs(q).await.unwrap_or_default()
            };
            for t in extra {
                if !seen.insert(t.id.clone()) {
                    continue;
                }
                if exclude_set.contains(t.id.as_str()) {
                    fallback_recs.push(t);
                } else {
                    fresh_recs.push(t);
                }
            }
        }

        // Shuffle fresh picks lightly using a fast xorshift seeded with nonce + time
        let mut rng_state = refresh_nonce
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64)
                .unwrap_or(42);
        if rng_state == 0 {
            rng_state = 0xDEAD_BEEF_CAFE_BABEu64;
        }
        let mut next_u64 = || {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            rng_state
        };

        if refresh_nonce > 0 && fresh_recs.len() > 1 {
            for i in (1..fresh_recs.len()).rev() {
                let j = (next_u64() as usize) % (i + 1);
                fresh_recs.swap(i, j);
            }
        }

        let mut recs: Vec<Track> = fresh_recs.into_iter().take(18).collect();
        if recs.len() < 18 {
            for t in fallback_recs {
                recs.push(t);
                if recs.len() >= 18 {
                    break;
                }
            }
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

        // Run pure-Rust VISIONOS HLS (~350-450ms!) and RustyPipe iOS loudness (~300ms) in parallel!
        let (vision_res, ios_res) = tokio::join!(
            self.resolve_via_visionos_direct(video_id),
            self.fetch_ios_loudness_and_hls(video_id)
        );

        let resolved = match (vision_res, ios_res) {
            (Ok(mut stream), ios_out) => {
                if let Ok((_, loudness)) = ios_out {
                    stream.loudness_db = stream.loudness_db.or(loudness);
                }
                stream
            }
            (Err(_), Ok((Some(hls_stream), _))) => hls_stream,
            (Err(vision_err), ios_out) => {
                let loudness = ios_out.ok().and_then(|(_, l)| l);
                warn!("Pure-Rust VISIONOS failed for {video_id}: {vision_err}; trying yt-dlp fallback...");
                match self.resolve_via_ytdlp_fast(video_id).await {
                    Ok(mut s) => {
                        s.loudness_db = loudness;
                        s
                    }
                    Err(_) => self.resolve_via_ytdlp_default(video_id, loudness).await?,
                }
            }
        };

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

        let resp = self
            .http
            .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
            .header(USER_AGENT, VISION_UA)
            .header("X-Youtube-Client-Name", "101")
            .header("X-Youtube-Client-Version", "1.02")
            .header("X-Goog-Visitor-Id", &vd)
            .header("Origin", "https://www.youtube.com")
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
        Ok((None, loudness))
    }

    /// Ultra-fast yt-dlp HLS resolution (~2s instead of ~6s) by skipping webpage & JS challenge solver!
    async fn resolve_via_ytdlp_fast(&self, video_id: &str) -> Result<ResolvedStream> {
        let ytdlp_bin = find_ytdlp_bin();
        let watch_url = format!("https://music.youtube.com/watch?v={video_id}");
        let mut cmd = Command::new(&ytdlp_bin);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        let output = cmd
            .args([
                "-f",
                "234/233",
                "--extractor-args",
                "youtube:player_skip=webpage,configs;player_client=visionos",
                "--get-url",
                "--no-playlist",
                "--no-warnings",
                "--",
                &watch_url,
            ])
            .output()
            .await
            .context("yt-dlp çalıştırılamadı")?;

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

    async fn resolve_via_ytdlp_default(
        &self,
        video_id: &str,
        loudness_db: Option<f32>,
    ) -> Result<ResolvedStream> {
        let ytdlp_bin = find_ytdlp_bin();
        let watch_url = format!("https://music.youtube.com/watch?v={video_id}");
        let mut cmd = Command::new(&ytdlp_bin);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let output = cmd
            .args([
                "-f",
                "234/233/bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio",
                "--dump-json",
                "--no-playlist",
                "--no-warnings",
                "--",
                &watch_url,
            ])
            .output()
            .await
            .context("yt-dlp çalıştırılamadı (sistemde kurulu olmayabilir)")?;

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

    // Pick a small thumbnail (~180px) to keep RAM usage minimal.
    let thumbnail_url = item
        .cover
        .iter()
        .min_by_key(|t| t.width.abs_diff(180))
        .or_else(|| item.cover.first())
        .map(|t| t.url.clone());

    Track {
        id: item.id,
        title: item.name,
        artist,
        album: item.album.map(|a| a.name),
        duration_secs: item.duration,
        thumbnail_url,
    }
}
