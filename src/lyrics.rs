//! Lyrics: LRCLIB time-synced lyrics + LRC parsing. UI-agnostic (used by Tauri and iced UIs).

use std::time::Duration;

use serde::{Deserialize, Serialize};

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
