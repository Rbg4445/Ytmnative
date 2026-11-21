use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::model::{ArtistSummary, Playlist, Track};

const MAX_RECENTLY_PLAYED: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryData {
    #[serde(default)]
    pub recently_played: Vec<Track>,
    #[serde(default)]
    pub liked_tracks: Vec<Track>,
    #[serde(default)]
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub followed_artists: Vec<ArtistSummary>,
    #[serde(default)]
    pub auth_cookie: Option<String>,
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// Optional GitHub repository ("owner/repo") used for VDS-free updates via GitHub Releases.
    #[serde(default)]
    pub update_repo: Option<String>,
    /// Last played track so the player bar restores on startup (ytmdesktop-style persistence).
    #[serde(default)]
    pub last_track: Option<Track>,
    /// Optional user Custom CSS injected into the UI (ytmdesktop feature).
    #[serde(default)]
    pub custom_css: String,
    /// Whether the startup intro video has already been shown once.
    #[serde(default)]
    pub intro_seen: bool,
    /// UI language code (defaults to English "en").
    #[serde(default = "default_language")]
    pub language: String,
    /// Discord Rich Presence enabled (ytmdesktop2 feature).
    #[serde(default = "default_true")]
    pub discord_rpc_enabled: bool,
    /// Custom Discord Application Client ID override (defaults to official YouTube Music App ID).
    #[serde(default)]
    pub discord_client_id: String,
    /// OBS Browser Source & Stream Deck Companion API on 127.0.0.1:13091 (ytmdesktop2 feature).
    #[serde(default = "default_true")]
    pub obs_api_enabled: bool,
    /// Last.fm scrobbling enabled (ytmdesktop2 feature).
    #[serde(default)]
    pub lastfm_enabled: bool,
    #[serde(default)]
    pub lastfm_api_key: String,
    #[serde(default)]
    pub lastfm_secret: String,
    #[serde(default)]
    pub lastfm_session_key: String,
    /// Active built-in theme preset name (ytmdesktop2 feature).
    #[serde(default)]
    pub theme_preset: String,
}

fn default_volume() -> f32 {
    0.85
}

fn default_language() -> String {
    "en".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for LibraryData {
    fn default() -> Self {
        Self {
            recently_played: Vec::new(),
            liked_tracks: Vec::new(),
            playlists: Vec::new(),
            followed_artists: Vec::new(),
            auth_cookie: None,
            volume: default_volume(),
            update_repo: None,
            last_track: None,
            custom_css: String::new(),
            intro_seen: false,
            language: default_language(),
            discord_rpc_enabled: true,
            discord_client_id: String::new(),
            obs_api_enabled: true,
            lastfm_enabled: false,
            lastfm_api_key: String::new(),
            lastfm_secret: String::new(),
            lastfm_session_key: String::new(),
            theme_preset: String::new(),
        }
    }
}

impl LibraryData {
    pub fn storage_path() -> PathBuf {
        #[cfg(windows)]
        let base = std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        #[cfg(not(windows))]
        let base = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|_| std::env::temp_dir());

        base.join("ytm-native").join("library.json")
    }

    pub fn load() -> Self {
        let path = Self::storage_path();
        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                warn!("Failed to parse library.json, starting fresh: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let path = Self::storage_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = fs::write(&path, json) {
                    warn!("Failed to save library.json: {e}");
                }
            }
            Err(e) => warn!("Failed to serialize library.json: {e}"),
        }
    }

    pub fn record_played(&mut self, track: &Track) {
        self.last_track = Some(track.clone());
        self.recently_played.retain(|t| t.id != track.id);
        self.recently_played.insert(0, track.clone());
        self.recently_played.truncate(MAX_RECENTLY_PLAYED);
        self.save();
    }

    pub fn is_liked(&self, track_id: &str) -> bool {
        self.liked_tracks.iter().any(|t| t.id == track_id)
    }

    pub fn toggle_like(&mut self, track: &Track) -> bool {
        if let Some(pos) = self.liked_tracks.iter().position(|t| t.id == track.id) {
            self.liked_tracks.remove(pos);
            self.save();
            false
        } else {
            self.liked_tracks.insert(0, track.clone());
            self.save();
            true
        }
    }

    pub fn create_playlist(&mut self, name: String) -> String {
        let trimmed = name.trim();
        let final_name = if trimmed.is_empty() {
            format!("Çalma Listem #{}", self.playlists.len() + 1)
        } else {
            trimmed.to_string()
        };
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let id = format!("pl_{ts}");
        self.playlists.push(Playlist {
            id: id.clone(),
            name: final_name,
            tracks: Vec::new(),
        });
        self.save();
        id
    }

    pub fn delete_playlist(&mut self, playlist_id: &str) {
        self.playlists.retain(|p| p.id != playlist_id);
        self.save();
    }

    pub fn add_track_to_playlist(&mut self, playlist_id: &str, track: &Track) -> bool {
        if let Some(pl) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
            if !pl.tracks.iter().any(|t| t.id == track.id) {
                pl.tracks.push(track.clone());
                self.save();
                return true;
            }
        }
        false
    }

    pub fn remove_track_from_playlist(&mut self, playlist_id: &str, track_id: &str) {
        if let Some(pl) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
            pl.tracks.retain(|t| t.id != track_id);
            self.save();
        }
    }

    pub fn toggle_follow_artist(&mut self, artist: &ArtistSummary) -> bool {
        if let Some(pos) = self.followed_artists.iter().position(|a| a.id == artist.id) {
            self.followed_artists.remove(pos);
            self.save();
            false
        } else {
            self.followed_artists.insert(0, artist.clone());
            self.save();
            true
        }
    }
}
