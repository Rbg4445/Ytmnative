use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::model::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// Bounded playback queue with history, upcoming tracks, repeat, and shuffle.
#[derive(Debug, Default)]
pub struct PlayQueue {
    tracks: Vec<Track>,
    current_idx: Option<usize>,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    /// Song keys (normalized title + artist) and video ids heard recently, kept across queues,
    /// so autoplay never brings back the same song (or another video of it) right away.
    recent: VecDeque<String>,
    /// The track the current radio/autoplay chain started from (second refill seed).
    origin_seed: Option<String>,
}

const MAX_QUEUE_LEN: usize = 100;
/// Played tracks kept before the current one when the queue has to make room.
const KEEP_PLAYED: usize = 20;
const RECENT_MEMORY: usize = 200;
/// Refill autoplay when fewer than this many tracks are left after the current one.
const REFILL_THRESHOLD: usize = 5;

/// "Song identity" used to spot duplicates that have different video ids:
/// official audio vs music video vs lyric video, "(Remastered)", "[Official Video]", etc.
pub fn song_key(track: &Track) -> String {
    let mut title = String::with_capacity(track.title.len());
    let mut depth = 0i32;
    for ch in track.title.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 => title.push(ch),
            _ => {}
        }
    }
    let mut title = title.to_lowercase();
    // Cut common suffixes like "- Official Video", "| Lyrics"
    for sep in [" - ", " | ", " – ", " — "] {
        if let Some(pos) = title.find(sep) {
            let tail = &title[pos + sep.len()..];
            if ["official", "video", "audio", "lyric", "lyrics", "klip", "music", "visualizer", "remaster", "live", "canlı"]
                .iter()
                .any(|w| tail.contains(w))
            {
                title.truncate(pos);
            }
        }
    }
    let title: String = title
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let artist = track
        .artist
        .split([',', '&'])
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    format!("{artist}::{title}")
}

impl PlayQueue {
    pub fn new() -> Self {
        Self::default()
    }

    fn remember(&mut self, track: &Track) {
        for key in [track.id.clone(), song_key(track)] {
            if let Some(pos) = self.recent.iter().position(|k| *k == key) {
                self.recent.remove(pos);
            }
            self.recent.push_back(key);
        }
        while self.recent.len() > RECENT_MEMORY * 2 {
            self.recent.pop_front();
        }
    }

    /// Called whenever a track actually starts playing.
    pub fn note_played(&mut self, track: &Track) {
        self.remember(track);
    }

    pub fn start_with(&mut self, track: Track) {
        self.origin_seed = Some(track.id.clone());
        self.tracks.clear();
        self.tracks.push(track);
        self.current_idx = Some(0);
    }

    pub fn start_with_list(&mut self, tracks: Vec<Track>, start_idx: usize) {
        self.tracks = tracks.into_iter().take(MAX_QUEUE_LEN).collect();
        if self.tracks.is_empty() {
            self.current_idx = None;
            self.origin_seed = None;
        } else {
            let idx = start_idx.min(self.tracks.len() - 1);
            self.current_idx = Some(idx);
            self.origin_seed = Some(self.tracks[idx].id.clone());
        }
    }

    pub fn origin_seed(&self) -> Option<String> {
        self.origin_seed.clone()
    }

    /// Drop the oldest already-played tracks so new autoplay tracks always fit.
    fn make_room(&mut self, needed: usize) {
        let Some(idx) = self.current_idx else { return };
        let overflow = (self.tracks.len() + needed).saturating_sub(MAX_QUEUE_LEN);
        let removable = idx.saturating_sub(KEEP_PLAYED);
        let n = overflow.min(removable);
        if n > 0 {
            self.tracks.drain(0..n);
            self.current_idx = Some(idx - n);
        }
    }

    /// Append autoplay/radio tracks, skipping anything already queued or heard recently
    /// (same video id OR same song as another video). Returns how many were added.
    pub fn append_radio(&mut self, radio_tracks: Vec<Track>) -> usize {
        let mut known: std::collections::HashSet<String> = self.recent.iter().cloned().collect();
        for t in &self.tracks {
            known.insert(t.id.clone());
            known.insert(song_key(t));
        }
        let fresh: Vec<Track> = radio_tracks
            .into_iter()
            .filter(|t| {
                let key = song_key(t);
                let new = !known.contains(&t.id) && !known.contains(&key);
                if new {
                    known.insert(t.id.clone());
                    known.insert(key);
                }
                new
            })
            .collect();
        self.make_room(fresh.len());
        let mut added = 0;
        for t in fresh {
            if self.tracks.len() >= MAX_QUEUE_LEN {
                break;
            }
            self.tracks.push(t);
            added += 1;
        }
        added
    }

    pub fn current(&self) -> Option<&Track> {
        self.current_idx.and_then(|i| self.tracks.get(i))
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current_idx
    }

    pub fn peek_next_ids(&self, count: usize) -> Vec<String> {
        let Some(idx) = self.current_idx else {
            return Vec::new();
        };
        self.tracks
            .iter()
            .skip(idx + 1)
            .take(count)
            .map(|t| t.id.clone())
            .collect()
    }

    pub fn peek_next(&self) -> Option<&Track> {
        let next = self.current_idx? + 1;
        self.tracks.get(next)
    }

    pub fn cycle_repeat(&mut self) -> RepeatMode {
        self.repeat = match self.repeat {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        self.repeat
    }

    pub fn toggle_shuffle(&mut self) -> bool {
        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.shuffle_upcoming();
        }
        self.shuffle
    }

    /// Fisher-Yates shuffle on upcoming tracks after `current_idx` without extra deps.
    fn shuffle_upcoming(&mut self) {
        let Some(idx) = self.current_idx else {
            return;
        };
        let start = idx + 1;
        if start + 1 >= self.tracks.len() {
            return;
        }
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e3779b97f4a7c15);
        for i in (start + 1..self.tracks.len()).rev() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let j = start + ((seed >> 33) as usize % (i - start + 1));
            self.tracks.swap(i, j);
        }
    }

    pub fn advance_next(&mut self, from_auto_end: bool) -> Option<&Track> {
        let idx = self.current_idx?;
        if from_auto_end && self.repeat == RepeatMode::One {
            return self.tracks.get(idx);
        }
        if idx + 1 < self.tracks.len() {
            self.current_idx = Some(idx + 1);
            self.tracks.get(idx + 1)
        } else if self.repeat == RepeatMode::All && !self.tracks.is_empty() {
            self.current_idx = Some(0);
            self.tracks.first()
        } else {
            None
        }
    }

    pub fn step_prev(&mut self) -> Option<&Track> {
        let idx = self.current_idx?;
        if idx > 0 {
            self.current_idx = Some(idx - 1);
            self.tracks.get(idx - 1)
        } else {
            self.tracks.first()
        }
    }

    pub fn select_index(&mut self, idx: usize) -> Option<&Track> {
        if idx < self.tracks.len() {
            self.current_idx = Some(idx);
            self.tracks.get(idx)
        } else {
            None
        }
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn needs_refill(&self) -> bool {
        match self.current_idx {
            Some(idx) => self.tracks.len().saturating_sub(idx + 1) < REFILL_THRESHOLD,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(id: &str, title: &str, artist: &str) -> Track {
        Track {
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            artist_id: None,
            album: None,
            duration_secs: None,
            thumbnail_url: None,
        }
    }

    #[test]
    fn same_song_different_videos_share_a_key() {
        let a = tr("1", "Blinding Lights", "The Weeknd");
        let b = tr("2", "Blinding Lights (Official Video)", "The Weeknd");
        let c = tr("3", "The Weeknd - Blinding Lights [Lyrics]", "The Weeknd");
        let d = tr("4", "Blinding Lights - Official Audio", "The Weeknd, Someone");
        assert_eq!(song_key(&a), song_key(&b));
        assert_eq!(song_key(&a), song_key(&d));
        // "Artist - Title" uploads keep their full title (the tail is the song, not a suffix)
        assert_ne!(song_key(&a), song_key(&c));
        assert_ne!(song_key(&a), song_key(&tr("5", "Save Your Tears", "The Weeknd")));
    }

    #[test]
    fn radio_skips_queued_and_recently_heard_songs() {
        let mut q = PlayQueue::new();
        let first = tr("1", "Song A", "X");
        q.start_with(first.clone());
        q.note_played(&first);
        let added = q.append_radio(vec![
            tr("1", "Song A", "X"),                  // same id
            tr("9", "Song A (Official Video)", "X"), // same song, other video
            tr("2", "Song B", "Y"),
            tr("2", "Song B", "Y"), // duplicate inside the batch
            tr("3", "Song C", "Z"),
        ]);
        assert_eq!(added, 2);

        // Song B played earlier, then a NEW queue starts: B must not come back via autoplay.
        let b = tr("2", "Song B", "Y");
        q.note_played(&b);
        q.start_with(tr("7", "Song D", "W"));
        assert_eq!(q.append_radio(vec![b, tr("8", "Song E", "V")]), 1);
    }

    #[test]
    fn full_queue_drops_old_played_tracks_instead_of_stopping() {
        let mut q = PlayQueue::new();
        let list: Vec<Track> = (0..MAX_QUEUE_LEN).map(|i| tr(&format!("id{i}"), &format!("T{i}"), "A")).collect();
        q.start_with_list(list, MAX_QUEUE_LEN - 2);
        let added = q.append_radio((0..10).map(|i| tr(&format!("new{i}"), &format!("N{i}"), "B")).collect());
        assert_eq!(added, 10);
        assert!(q.tracks().len() <= MAX_QUEUE_LEN);
        assert_eq!(q.current().unwrap().id, format!("id{}", MAX_QUEUE_LEN - 2));
    }
}
