use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE, USER_AGENT};
use symphonia::core::io::MediaSource;

/// 256 KB per HTTP Range chunk — fast startup (<100ms) with minimal HTTP overhead.
const CHUNK_SIZE: u64 = 256 * 1024;
/// Hard RAM cap: 24 chunks * 256 KB = 6 MB max per active audio stream.
const MAX_CACHED_CHUNKS: usize = 24;

const STREAM_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Progressive, seekable HTTP audio stream with a strictly bounded 6 MB RAM cache.
/// Playback begins as soon as the first 256 KB chunk arrives without waiting for full download.
pub struct HttpAudioStream {
    client: Client,
    url: String,
    user_agent: String,
    use_query_range: bool,
    pos: u64,
    total_len: Option<u64>,
    chunks: HashMap<u64, Vec<u8>>,
    lru_order: VecDeque<u64>,
    cancel: Arc<AtomicBool>,
}

impl HttpAudioStream {
    pub fn open(
        url: String,
        known_len: Option<u64>,
        user_agent: Option<String>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(2)
            .build()?;

        let use_query_range = false;
        let mut stream = Self {
            client,
            url,
            user_agent: user_agent.unwrap_or_else(|| STREAM_USER_AGENT.to_string()),
            use_query_range,
            pos: 0,
            total_len: known_len,
            chunks: HashMap::with_capacity(MAX_CACHED_CHUNKS),
            lru_order: VecDeque::with_capacity(MAX_CACHED_CHUNKS),
            cancel,
        };

        // Fetch chunk 0 immediately to validate URL, discover exact length, and prime decoder.
        stream.fetch_chunk(0)?;
        Ok(stream)
    }

    fn fetch_chunk(&mut self, chunk_idx: u64) -> Result<()> {
        if self.chunks.contains_key(&chunk_idx) {
            return Ok(());
        }
        if self.cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("stream cancelled"));
        }

        let start = chunk_idx * CHUNK_SIZE;
        if let Some(total) = self.total_len {
            if start >= total {
                return Ok(());
            }
        }
        let end = match self.total_len {
            Some(total) => (start + CHUNK_SIZE - 1).min(total.saturating_sub(1)),
            None => start + CHUNK_SIZE - 1,
        };

        let range_header = format!("bytes={start}-{end}");
        let mut last_err = None;

        for attempt in 0..3 {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("stream cancelled"));
            }

            let req = if self.use_query_range {
                let sep = if self.url.contains('?') { '&' } else { '?' };
                let ranged_url = format!("{}{sep}range={start}-{end}", self.url);
                self.client
                    .get(&ranged_url)
                    .header(USER_AGENT, &self.user_agent)
            } else {
                self.client
                    .get(&self.url)
                    .header(RANGE, &range_header)
                    .header(USER_AGENT, &self.user_agent)
            };

            let resp = req.send();

            match resp {
                Ok(r) => {
                    let status = r.status();
                    if status.as_u16() == 416 {
                        // Range Not Satisfiable -> EOF
                        return Ok(());
                    }
                    if !status.is_success() {
                        last_err = Some(anyhow!("HTTP {status} for range {range_header}"));
                        if status.as_u16() == 403 || status.as_u16() == 404 {
                            // Don't retry hard auth/not-found errors
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(150 * (attempt + 1)));
                        continue;
                    }

                    if self.total_len.is_none() {
                        if let Some(cr) = r.headers().get(CONTENT_RANGE).and_then(|v| v.to_str().ok()) {
                            if let Some(total_str) = cr.rsplit('/').next() {
                                if let Ok(total) = total_str.parse::<u64>() {
                                    self.total_len = Some(total);
                                }
                            }
                        } else if status.as_u16() == 200 {
                            if let Some(cl) = r.headers().get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()) {
                                if let Ok(total) = cl.parse::<u64>() {
                                    self.total_len = Some(total);
                                }
                            }
                        }
                    }

                    match r.bytes() {
                        Ok(bytes) => {
                            if bytes.is_empty() {
                                return Ok(());
                            }
                            self.insert_chunk(chunk_idx, bytes.to_vec());
                            return Ok(());
                        }
                        Err(e) => {
                            last_err = Some(anyhow!("failed reading body: {e}"));
                        }
                    }
                }
                Err(e) => {
                    last_err = Some(anyhow!("HTTP request failed: {e}"));
                }
            }

            std::thread::sleep(Duration::from_millis(150 * (attempt + 1)));
        }

        Err(last_err.unwrap_or_else(|| anyhow!("failed to fetch chunk {chunk_idx}")))
    }

    fn insert_chunk(&mut self, chunk_idx: u64, data: Vec<u8>) {
        if self.chunks.len() >= MAX_CACHED_CHUNKS {
            if let Some(oldest) = self.lru_order.pop_front() {
                self.chunks.remove(&oldest);
            }
        }
        self.chunks.insert(chunk_idx, data);
        self.lru_order.push_back(chunk_idx);
    }
}

impl Read for HttpAudioStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stream cancelled"));
        }
        if let Some(total) = self.total_len {
            if self.pos >= total {
                return Ok(0);
            }
        }

        let chunk_idx = self.pos / CHUNK_SIZE;
        let offset_in_chunk = (self.pos % CHUNK_SIZE) as usize;

        if !self.chunks.contains_key(&chunk_idx) {
            self.fetch_chunk(chunk_idx)
                .map_err(|e| io::Error::other(e.to_string()))?;
        }

        let Some(chunk) = self.chunks.get(&chunk_idx) else {
            return Ok(0);
        };

        if offset_in_chunk >= chunk.len() {
            return Ok(0);
        }

        let avail = chunk.len() - offset_in_chunk;
        let to_copy = avail.min(buf.len());
        buf[..to_copy].copy_from_slice(&chunk[offset_in_chunk..offset_in_chunk + to_copy]);
        self.pos += to_copy as u64;
        Ok(to_copy)
    }
}

impl Seek for HttpAudioStream {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos: i64 = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::Current(delta) => self.pos as i64 + delta,
            SeekFrom::End(delta) => {
                let total = self.total_len.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::Unsupported, "unknown stream length")
                })?;
                total as i64 + delta
            }
        };

        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start of stream",
            ));
        }

        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

impl MediaSource for HttpAudioStream {
    fn is_seekable(&self) -> bool {
        self.total_len.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.total_len
    }
}
