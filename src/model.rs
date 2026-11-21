use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Lightweight track metadata kept in search results, queue, and local library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub artist_id: Option<String>,
    pub album: Option<String>,
    pub duration_secs: Option<u32>,
    pub thumbnail_url: Option<String>,
}

/// Artist summary card for search results, sidebar library, and artist pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistSummary {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub subscriber_count: Option<u64>,
}

/// Full artist profile page payload returned when clicking an artist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistDetailDto {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub description: Option<String>,
    pub subscriber_count: Option<u64>,
    pub tracks: Vec<Track>,
    pub similar_artists: Vec<ArtistSummary>,
}

/// Combined search results containing both matching artists and tracks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchBundleDto {
    pub artists: Vec<ArtistSummary>,
    pub tracks: Vec<Track>,
}

/// User-created playlist persisted in local storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
}

impl Track {
    pub fn duration_string(&self) -> String {
        match self.duration_secs {
            Some(secs) => format_seconds(secs as f64),
            None => "--:--".to_string(),
        }
    }
}

/// Source used to resolve the audio stream URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSource {
    RustyPipe,
    YtDlp,
}

impl std::fmt::Display for StreamSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamSource::RustyPipe => write!(f, "rustypipe"),
            StreamSource::YtDlp => write!(f, "yt-dlp"),
        }
    }
}

/// Single HLS audio segment (~7s, ~115 KB ADTS AAC).
#[derive(Debug, Clone)]
pub struct HlsSegment {
    pub url: String,
    pub duration_secs: f64,
}

/// Transport backing a resolved audio stream.
#[derive(Debug, Clone)]
pub enum StreamKind {
    Direct {
        url: String,
        content_length: Option<u64>,
    },
    Hls {
        segments: Vec<HlsSegment>,
    },
}

/// Resolved audio stream ready for low-RAM progressive playback.
#[derive(Debug, Clone)]
pub struct ResolvedStream {
    pub track_id: String,
    pub kind: StreamKind,
    pub mime: String,
    pub bitrate: u32,
    pub duration_secs: Option<f64>,
    /// YouTube loudness in dB relative to target (-14 LUFS). Positive means louder than target.
    pub loudness_db: Option<f32>,
    /// Matching User-Agent for HTTP requests.
    pub user_agent: Option<String>,
    pub source: StreamSource,
    pub resolved_at: Instant,
}

impl ResolvedStream {
    /// YouTube stream URLs typically expire after ~6 hours; treat as fresh for 2 hours.
    pub fn is_fresh(&self) -> bool {
        self.resolved_at.elapsed().as_secs() < 2 * 3600
    }

    /// Official-app style loudness normalization gain:
    /// Only attenuates tracks louder than target (`loudness_db > 0.0`), never boosts (prevents clipping).
    pub fn normalization_gain(&self) -> f32 {
        match self.loudness_db {
            Some(db) if db > 0.0 => 10.0_f32.powf(-db.min(20.0) / 20.0),
            _ => 1.0,
        }
    }
}

/// Format seconds as `M:SS` or `H:MM:SS`.
pub fn format_seconds(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
