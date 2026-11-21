use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

use crate::backend::AppController;

/// Default Discord Application Client ID (YouTube Music Rich Presence)
pub const DEFAULT_DISCORD_CLIENT_ID: &str = "1163164207358025798";

/// Default local HTTP port for OBS Browser Source overlays & Stream Deck API (matches ytmdesktop2)
pub const OBS_API_PORT: u16 = 13091;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricLineDto {
    pub time_secs: f64,
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct LrcLibResponse {
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

/// Parse standard LRC format (`[mm:ss.xx] Lyric text`) into sorted `LyricLineDto` items.
pub fn parse_lrc(raw: &str) -> Vec<LyricLineDto> {
    let mut lines = Vec::new();
    for row in raw.lines() {
        let trimmed = row.trim();
        if !trimmed.starts_with('[') {
            continue;
        }
        if let Some(close_idx) = trimmed.find(']') {
            let tag = &trimmed[1..close_idx];
            let text = trimmed[close_idx + 1..].trim();
            if let Some(colon_idx) = tag.find(':') {
                let mins_str = &tag[..colon_idx];
                let secs_str = &tag[colon_idx + 1..];
                if let (Ok(mins), Ok(secs)) = (mins_str.parse::<f64>(), secs_str.parse::<f64>()) {
                    let time_secs = (mins * 60.0 + secs).max(0.0);
                    lines.push(LyricLineDto {
                        time_secs,
                        text: text.to_string(),
                    });
                }
            }
        }
    }
    lines.sort_by(|a, b| {
        a.time_secs
            .partial_cmp(&b.time_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    lines
}

/// Query LRCLIB (`https://lrclib.net/api/search`) for time-synced karaoke lyrics!
pub async fn fetch_lrclib_lyrics(
    http: &reqwest::Client,
    title: &str,
    artist: &str,
) -> Option<(Option<Vec<LyricLineDto>>, String, String)> {
    let clean_title = title
        .split(['(', '['])
        .next()
        .unwrap_or(title)
        .trim();
    let clean_artist = artist
        .split([',', '&', '•'])
        .next()
        .unwrap_or(artist)
        .trim();

    let resp = http
        .get("https://lrclib.net/api/search")
        .query(&[("track_name", clean_title), ("artist_name", clean_artist)])
        .header("User-Agent", "YTM-Native/1.0 (https://github.com/Rbg4445/Ytmnative)")
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let items: Vec<LrcLibResponse> = resp.json().await.ok()?;
    // Prefer an item that has syncedLyrics
    if let Some(synced_hit) = items.iter().find(|i| {
        i.synced_lyrics
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }) {
        let raw_lrc = synced_hit.synced_lyrics.as_deref().unwrap_or_default();
        let parsed = parse_lrc(raw_lrc);
        if !parsed.is_empty() {
            let plain = parsed
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            return Some((
                Some(parsed),
                plain,
                "Source: LRCLIB (Time-Synced Lyrics)".to_string(),
            ));
        }
    }

    if let Some(plain_hit) = items.into_iter().find(|i| {
        i.plain_lyrics
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }) {
        let plain = plain_hit.plain_lyrics.unwrap_or_default();
        return Some((None, plain, "Source: LRCLIB".to_string()));
    }

    None
}

/// Extract a 11-char YouTube Video ID from any `music.youtube.com`, `youtube.com`, `youtu.be`, or `ytmd://` URL.
pub fn extract_video_id_from_url(input: &str) -> Option<String> {
    let s = input.trim();
    if !s.contains("://") && !s.starts_with("music.youtube.com") && !s.starts_with("youtube.com") && !s.starts_with("youtu.be") {
        return None;
    }
    if let Some(pos) = s.find("v=") {
        let after = &s[pos + 2..];
        let id: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if id.len() == 11 {
            return Some(id);
        }
    }
    for prefix in ["youtu.be/", "/shorts/", "/embed/", "/watch/"] {
        if let Some(pos) = s.find(prefix) {
            let after = &s[pos + prefix.len()..];
            let id: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if id.len() == 11 {
                return Some(id);
            }
        }
    }
    None
}

// ============================================================================
// PURE-RUST DISCORD RICH PRESENCE (Zero External Crates, Windows & Linux IPC)
// ============================================================================

pub struct DiscordRpcClient {
    #[cfg(windows)]
    pipe: Option<std::fs::File>,
    #[cfg(unix)]
    sock: Option<std::os::unix::net::UnixStream>,
    last_key: String,
}

impl Default for DiscordRpcClient {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordRpcClient {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            pipe: None,
            #[cfg(unix)]
            sock: None,
            last_key: String::new(),
        }
    }

    fn is_connected(&self) -> bool {
        #[cfg(windows)]
        {
            self.pipe.is_some()
        }
        #[cfg(unix)]
        {
            self.sock.is_some()
        }
        #[cfg(not(any(windows, unix)))]
        {
            false
        }
    }

    fn disconnect(&mut self) {
        #[cfg(windows)]
        {
            self.pipe = None;
        }
        #[cfg(unix)]
        {
            self.sock = None;
        }
    }

    fn try_connect(&mut self) -> bool {
        if self.is_connected() {
            return true;
        }
        #[cfg(windows)]
        {
            for i in 0..4 {
                let path = format!(r"\\.\pipe\discord-ipc-{i}");
                if let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                    self.pipe = Some(file);
                    let handshake = serde_json::json!({
                        "v": 1,
                        "client_id": DEFAULT_DISCORD_CLIENT_ID
                    })
                    .to_string();
                    if self.send_frame(0, handshake.as_bytes()).is_ok() {
                        let _ = self.read_frame();
                        debug!("Connected to Discord Rich Presence pipe: {path}");
                        return true;
                    } else {
                        self.pipe = None;
                    }
                }
            }
            false
        }
        #[cfg(unix)]
        {
            let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
            for i in 0..4 {
                for base in [&runtime_dir, "/tmp"] {
                    let path = format!("{base}/discord-ipc-{i}");
                    if let Ok(stream) = std::os::unix::net::UnixStream::connect(&path) {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                        self.sock = Some(stream);
                        let handshake = serde_json::json!({
                            "v": 1,
                            "client_id": DEFAULT_DISCORD_CLIENT_ID
                        })
                        .to_string();
                        if self.send_frame(0, handshake.as_bytes()).is_ok() {
                            let _ = self.read_frame();
                            return true;
                        } else {
                            self.sock = None;
                        }
                    }
                }
            }
            false
        }
        #[cfg(not(any(windows, unix)))]
        {
            false
        }
    }

    fn send_frame(&mut self, op: u32, payload: &[u8]) -> Result<()> {
        let mut buf = Vec::with_capacity(8 + payload.len());
        buf.extend_from_slice(&op.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(payload);

        #[cfg(windows)]
        if let Some(ref mut p) = self.pipe {
            p.write_all(&buf)?;
            p.flush()?;
            return Ok(());
        }
        #[cfg(unix)]
        if let Some(ref mut s) = self.sock {
            s.write_all(&buf)?;
            s.flush()?;
            return Ok(());
        }
        Err(anyhow!("Discord IPC not connected"))
    }

    fn read_frame(&mut self) -> Result<()> {
        let mut hdr = [0u8; 8];
        #[cfg(windows)]
        if let Some(ref mut p) = self.pipe {
            p.read_exact(&mut hdr)?;
            let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
            if len <= 65536 {
                let mut body = vec![0u8; len];
                p.read_exact(&mut body)?;
            }
            return Ok(());
        }
        #[cfg(unix)]
        if let Some(ref mut s) = self.sock {
            s.read_exact(&mut hdr)?;
            let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
            if len <= 65536 {
                let mut body = vec![0u8; len];
                s.read_exact(&mut body)?;
            }
            return Ok(());
        }
        Err(anyhow!("Not connected"))
    }

    pub fn update_presence(
        &mut self,
        enabled: bool,
        track: Option<(&str, &str, &str, Option<&str>)>,
        is_playing: bool,
        position_secs: f64,
        duration_secs: f64,
    ) {
        if !enabled {
            if !self.last_key.is_empty() {
                self.clear_activity();
                self.last_key.clear();
            }
            return;
        }

        let Some((id, title, artist, thumb)) = track else {
            if !self.last_key.is_empty() {
                self.clear_activity();
                self.last_key.clear();
            }
            return;
        };

        let key = format!("{id}:{is_playing}:{}", (position_secs as u64) / 15);
        if key == self.last_key {
            return;
        }

        if !self.try_connect() {
            return;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let timestamps = if is_playing && duration_secs > 1.0 {
            let start = now.saturating_sub(position_secs as u64);
            let end = start + (duration_secs as u64);
            serde_json::json!({ "start": start, "end": end })
        } else {
            serde_json::Value::Null
        };

        let state_str = if is_playing {
            format!("by {artist}")
        } else {
            format!("by {artist} (Paused)")
        };

        let large_img = thumb.unwrap_or("https://music.youtube.com/img/on_platform_logo_dark.svg");
        let watch_url = format!("https://music.youtube.com/watch?v={id}");

        let mut activity = serde_json::json!({
            "type": 2,
            "details": title,
            "state": state_str,
            "assets": {
                "large_image": large_img,
                "large_text": "YTM Native"
            },
            "buttons": [
                {
                    "label": "Listen on YouTube Music",
                    "url": watch_url
                }
            ]
        });

        if !timestamps.is_null() {
            activity["timestamps"] = timestamps;
        }

        let pkt = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": {
                "pid": std::process::id(),
                "activity": activity
            },
            "nonce": format!("ytm-{now}")
        })
        .to_string();

        if self.send_frame(1, pkt.as_bytes()).is_ok() {
            let _ = self.read_frame();
            self.last_key = key;
        } else {
            self.disconnect();
        }
    }

    pub fn clear_activity(&mut self) {
        if !self.is_connected() {
            return;
        }
        let pkt = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": {
                "pid": std::process::id(),
                "activity": serde_json::Value::Null
            },
            "nonce": "ytm-clear"
        })
        .to_string();
        if self.send_frame(1, pkt.as_bytes()).is_ok() {
            let _ = self.read_frame();
        } else {
            self.disconnect();
        }
    }
}

// ============================================================================
// PURE-RUST RFC 1321 MD5 + LAST.FM SCROBBLER (Zero External Crates!)
// ============================================================================

pub fn md5_hex(input: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
        5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
        4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
        6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
        0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
        0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
        0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
        0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
        0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
        0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
    ];

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    let bit_len = (input.len() as u64) * 8;
    let mut msg = input.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, w) in chunk.chunks_exact(4).enumerate() {
            m[i] = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out.iter().map(|b| format!("{b:02x}")).collect()
}

fn lastfm_sign(params: &[(&str, &str)], secret: &str) -> String {
    let mut sorted = params.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut s = String::new();
    for (k, v) in sorted {
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(secret);
    md5_hex(s.as_bytes())
}

pub async fn lastfm_update_now_playing(
    http: &reqwest::Client,
    api_key: &str,
    secret: &str,
    session_key: &str,
    artist: &str,
    track: &str,
) {
    if api_key.is_empty() || secret.is_empty() || session_key.is_empty() {
        return;
    }
    let params = [
        ("api_key", api_key),
        ("artist", artist),
        ("method", "track.updateNowPlaying"),
        ("sk", session_key),
        ("track", track),
    ];
    let sig = lastfm_sign(&params, secret);
    let _ = http
        .post("https://ws.audioscrobbler.com/2.0/")
        .form(&[
            ("method", "track.updateNowPlaying"),
            ("api_key", api_key),
            ("artist", artist),
            ("track", track),
            ("sk", session_key),
            ("api_sig", sig.as_str()),
            ("format", "json"),
        ])
        .timeout(Duration::from_secs(5))
        .send()
        .await;
}

pub async fn lastfm_scrobble(
    http: &reqwest::Client,
    api_key: &str,
    secret: &str,
    session_key: &str,
    artist: &str,
    track: &str,
    timestamp: u64,
) {
    if api_key.is_empty() || secret.is_empty() || session_key.is_empty() {
        return;
    }
    let ts = timestamp.to_string();
    let params = [
        ("api_key", api_key),
        ("artist", artist),
        ("method", "track.scrobble"),
        ("sk", session_key),
        ("timestamp", ts.as_str()),
        ("track", track),
    ];
    let sig = lastfm_sign(&params, secret);
    let _ = http
        .post("https://ws.audioscrobbler.com/2.0/")
        .form(&[
            ("method", "track.scrobble"),
            ("api_key", api_key),
            ("artist", artist),
            ("track", track),
            ("timestamp", ts.as_str()),
            ("sk", session_key),
            ("api_sig", sig.as_str()),
            ("format", "json"),
        ])
        .timeout(Duration::from_secs(5))
        .send()
        .await;
}

// ============================================================================
// OBS BROWSER SOURCE OVERLAY & STREAM DECK COMPANION SERVER (127.0.0.1:13091)
// ============================================================================

const OBS_EMBED_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8" />
<title>YTM Native — OBS Now Playing Overlay</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; font-family: 'Segoe UI', -apple-system, BlinkMacSystemFont, sans-serif; }
  body { background: transparent; color: #fff; overflow: hidden; }
  .card {
    display: flex; align-items: center; gap: 14px;
    padding: 12px 16px; border-radius: 14px;
    background: rgba(12, 12, 16, 0.84);
    backdrop-filter: blur(18px);
    border: 1px solid rgba(255, 255, 255, 0.14);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.55);
    max-width: 460px; position: relative; overflow: hidden;
  }
  .art { width: 64px; height: 64px; border-radius: 10px; object-fit: cover; background: #1e1e24; flex-shrink: 0; }
  .meta { flex: 1; min-width: 0; }
  .title { font-size: 16px; font-weight: 700; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .artist { font-size: 13px; color: #b4b4c0; margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .time { font-size: 11px; color: #8e8e9a; margin-top: 6px; font-variant-numeric: tabular-nums; }
  .bar-bg { position: absolute; left: 0; right: 0; bottom: 0; height: 3.5px; background: rgba(255,255,255,0.12); }
  .bar-fill { height: 100%; background: #FF0033; width: 0%; transition: width 0.4s linear; box-shadow: 0 0 10px #FF0033; }
  /* Layout variants matching ytmdesktop2 */
  body.layout-compact .card { padding: 8px 12px; max-width: 340px; }
  body.layout-compact .art { width: 46px; height: 46px; }
  body.layout-stack .card { flex-direction: column; text-align: center; max-width: 240px; padding: 16px; }
  body.layout-stack .art { width: 176px; height: 176px; border-radius: 12px; }
  body.layout-text .card { background: transparent; border: none; box-shadow: none; padding: 4px 8px; }
  body.layout-text .art, body.layout-text .bar-bg, body.layout-text .time { display: none; }
  body.layout-badge .card { padding: 7px 12px; border-radius: 999px; max-width: 320px; }
  body.layout-badge .art { width: 36px; height: 36px; border-radius: 50%; }
  body.layout-badge .time { display: none; }
</style>
</head>
<body>
  <div id="wrap" class="card">
    <img id="art" class="art" src="" alt="" />
    <div class="meta">
      <div id="title" class="title">YTM Native</div>
      <div id="artist" class="artist">Waiting for playback...</div>
      <div id="time" class="time">0:00 / 0:00</div>
    </div>
    <div class="bar-bg"><div id="bar" class="bar-fill"></div></div>
  </div>
<script>
  const params = new URLSearchParams(location.search);
  const layout = params.get("layout") || "default";
  document.body.classList.add("layout-" + layout);
  function fmt(s) { s = Math.max(0, Math.floor(s || 0)); return Math.floor(s/60) + ":" + String(s%60).padStart(2,"0"); }
  async function poll() {
    try {
      const r = await fetch("/api/v1/state");
      const d = await r.json();
      if (d.current) {
        document.getElementById("title").textContent = d.current.title;
        document.getElementById("artist").textContent = d.current.artist;
        if (d.current.thumbnail_url && document.getElementById("art").src !== d.current.thumbnail_url) {
          document.getElementById("art").src = d.current.thumbnail_url;
        }
        const dur = d.duration_secs || d.current.duration_secs || 0;
        const pos = d.position_secs || 0;
        document.getElementById("time").textContent = fmt(pos) + " / " + fmt(dur);
        document.getElementById("bar").style.width = (dur > 0 ? Math.min(100, (pos / dur) * 100) : 0) + "%";
      }
    } catch (_) {}
  }
  poll();
  setInterval(poll, 1000);
</script>
</body>
</html>"#;

pub fn spawn_obs_api_server(ctrl: Arc<AppController>) {
    tauri::async_runtime::spawn(async move {
        let addr = format!("127.0.0.1:{OBS_API_PORT}");
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => {
                info!("OBS Overlay & Companion API listening on http://{addr}");
                l
            }
            Err(e) => {
                warn!("Could not bind OBS API port {addr}: {e}");
                return;
            }
        };

        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            let ctrl = Arc::clone(&ctrl);
            tauri::async_runtime::spawn(async move {
                let mut buf = [0u8; 4096];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("GET");
                let raw_path = parts.next().unwrap_or("/");
                let path = raw_path.split('?').next().unwrap_or("/");

                let enabled = ctrl
                    .library
                    .lock()
                    .map(|l| l.obs_api_enabled)
                    .unwrap_or(true);

                if !enabled {
                    let resp = "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 16\r\n\r\nOBS API Disabled";
                    let _ = stream.write_all(resp.as_bytes()).await;
                    return;
                }

                match (method, path) {
                    ("GET", "/") | ("GET", "/embed/now-playing") => {
                        let body = OBS_EMBED_HTML.as_bytes();
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes()).await;
                        let _ = stream.write_all(body).await;
                    }
                    ("GET", "/api/v1/state") => {
                        let dto = ctrl.playback_dto();
                        let json = serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string());
                        let body = json.as_bytes();
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes()).await;
                        let _ = stream.write_all(body).await;
                    }
                    ("POST", "/api/v1/command") | ("GET", "/api/v1/command") => {
                        if raw_path.contains("next") || req.contains("\"next\"") {
                            ctrl.advance_next(false);
                        } else if raw_path.contains("prev") || req.contains("\"prev\"") {
                            ctrl.step_prev();
                        } else if raw_path.contains("toggle") || raw_path.contains("play") || raw_path.contains("pause") || req.contains("\"toggle\"") {
                            ctrl.toggle_pause();
                        }
                        let body = b"{\"ok\":true}";
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes()).await;
                        let _ = stream.write_all(body).await;
                    }
                    _ => {
                        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found";
                        let _ = stream.write_all(resp.as_bytes()).await;
                    }
                }
            });
        }
    });
}
