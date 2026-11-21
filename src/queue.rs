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
}

const MAX_QUEUE_LEN: usize = 100;

impl PlayQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_with(&mut self, track: Track) {
        self.tracks.clear();
        self.tracks.push(track);
        self.current_idx = Some(0);
    }

    pub fn start_with_list(&mut self, tracks: Vec<Track>, start_idx: usize) {
        self.tracks = tracks.into_iter().take(MAX_QUEUE_LEN).collect();
        if self.tracks.is_empty() {
            self.current_idx = None;
        } else {
            self.current_idx = Some(start_idx.min(self.tracks.len() - 1));
        }
    }

    pub fn append_radio(&mut self, radio_tracks: Vec<Track>) {
        for t in radio_tracks {
            if self.tracks.len() >= MAX_QUEUE_LEN {
                break;
            }
            if !self.tracks.iter().any(|existing| existing.id == t.id) {
                self.tracks.push(t);
            }
        }
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
            Some(idx) => self.tracks.len().saturating_sub(idx + 1) < 3,
            None => false,
        }
    }
}
