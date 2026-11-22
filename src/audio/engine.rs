use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use reqwest::header::USER_AGENT;
use rtrb::{Consumer, Producer, RingBuffer};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;
use tracing::{debug, error, info, warn};

use crate::audio::stream::HttpAudioStream;
use crate::model::{HlsSegment, ResolvedStream, StreamKind};

enum AudioCmd {
    Play(ResolvedStream),
    Pause,
    Resume,
    Stop,
    Seek(f64),
    Shutdown,
}

/// Lock-free snapshot of playback state read by the UI thread.
#[derive(Debug, Clone, Default)]
pub struct AudioSnapshot {
    pub is_playing: bool,
    pub is_paused: bool,
    pub is_buffering: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub volume: f32,
    pub track_end_seq: u64,
    pub last_error: Option<String>,
}

struct SharedState {
    is_playing: AtomicBool,
    is_paused: AtomicBool,
    is_buffering: AtomicBool,
    position_ms: AtomicU64,
    duration_ms: AtomicU64,
    volume_bits: AtomicU32,
    norm_gain_bits: AtomicU32,
    flush_ring: AtomicBool,
    track_end_seq: AtomicU64,
    last_error: Mutex<Option<String>>,
    /// Cancel flag for the currently active HTTP stream so skip/stop is instant.
    stream_cancel: Mutex<Arc<AtomicBool>>,
    /// Command queue + Condvar so the decode thread sleeps at 0% CPU when idle/paused.
    cmd_queue: Mutex<VecDeque<AudioCmd>>,
    cmd_cv: Condvar,
}

impl SharedState {
    fn new() -> Self {
        Self {
            is_playing: AtomicBool::new(false),
            is_paused: AtomicBool::new(false),
            is_buffering: AtomicBool::new(false),
            position_ms: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            volume_bits: AtomicU32::new(0.85_f32.to_bits()),
            norm_gain_bits: AtomicU32::new(1.0_f32.to_bits()),
            flush_ring: AtomicBool::new(false),
            track_end_seq: AtomicU64::new(0),
            last_error: Mutex::new(None),
            stream_cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            cmd_queue: Mutex::new(VecDeque::new()),
            cmd_cv: Condvar::new(),
        }
    }

    fn set_error(&self, msg: Option<String>) {
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = msg;
        }
    }

    fn cancel_active_stream(&self) {
        if let Ok(guard) = self.stream_cancel.lock() {
            guard.store(true, Ordering::Relaxed);
        }
    }

    fn new_stream_cancel(&self) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut guard) = self.stream_cancel.lock() {
            guard.store(true, Ordering::Relaxed);
            *guard = Arc::clone(&flag);
        }
        flag
    }

    fn push_cmd(&self, cmd: AudioCmd) {
        if matches!(cmd, AudioCmd::Play(_) | AudioCmd::Stop | AudioCmd::Shutdown) {
            self.cancel_active_stream();
        }
        if let Ok(mut q) = self.cmd_queue.lock() {
            // If a new Play arrives, drop stale queued Play/Seek commands to stay snappy.
            if matches!(cmd, AudioCmd::Play(_) | AudioCmd::Stop | AudioCmd::Shutdown) {
                q.clear();
            }
            q.push_back(cmd);
        }
        self.cmd_cv.notify_one();
    }

    fn try_pop_cmd(&self) -> Option<AudioCmd> {
        self.cmd_queue.lock().ok()?.pop_front()
    }

    fn wait_cmd(&self) -> AudioCmd {
        let mut q = self.cmd_queue.lock().expect("cmd_queue poisoned");
        loop {
            if let Some(cmd) = q.pop_front() {
                return cmd;
            }
            q = self.cmd_cv.wait(q).expect("cmd_cv wait poisoned");
        }
    }
}

pub struct AudioEngine {
    shared: Arc<SharedState>,
    worker: Option<JoinHandle<()>>,
}

impl AudioEngine {
    pub fn new() -> Result<Self> {
        let shared = Arc::new(SharedState::new());
        let worker_shared = Arc::clone(&shared);

        let worker = thread::Builder::new()
            .name("ytm-audio-worker".to_string())
            .spawn(move || audio_worker_main(worker_shared))
            .context("failed to spawn audio worker thread")?;

        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    pub fn play(&self, stream: ResolvedStream) {
        self.shared.set_error(None);
        self.shared.is_buffering.store(true, Ordering::Relaxed);
        self.shared.push_cmd(AudioCmd::Play(stream));
    }

    pub fn pause(&self) {
        self.shared.push_cmd(AudioCmd::Pause);
    }

    pub fn resume(&self) {
        self.shared.push_cmd(AudioCmd::Resume);
    }

    pub fn toggle_pause(&self) {
        if self.shared.is_paused.load(Ordering::Relaxed) {
            self.resume();
        } else if self.shared.is_playing.load(Ordering::Relaxed) {
            self.pause();
        }
    }

    pub fn stop(&self) {
        self.shared.push_cmd(AudioCmd::Stop);
    }

    pub fn seek(&self, seconds: f64) {
        self.shared.push_cmd(AudioCmd::Seek(seconds.max(0.0)));
    }

    pub fn set_volume(&self, volume: f32) {
        let clamped = volume.clamp(0.0, 1.0);
        self.shared
            .volume_bits
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.shared.volume_bits.load(Ordering::Relaxed))
    }

    pub fn snapshot(&self) -> AudioSnapshot {
        let last_error = self.shared.last_error.lock().ok().and_then(|g| g.clone());
        AudioSnapshot {
            is_playing: self.shared.is_playing.load(Ordering::Relaxed),
            is_paused: self.shared.is_paused.load(Ordering::Relaxed),
            is_buffering: self.shared.is_buffering.load(Ordering::Relaxed),
            position_secs: self.shared.position_ms.load(Ordering::Relaxed) as f64 / 1000.0,
            duration_secs: self.shared.duration_ms.load(Ordering::Relaxed) as f64 / 1000.0,
            volume: f32::from_bits(self.shared.volume_bits.load(Ordering::Relaxed)),
            track_end_seq: self.shared.track_end_seq.load(Ordering::Relaxed),
            last_error,
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.shared.push_cmd(AudioCmd::Shutdown);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

struct OutputSink {
    stream: Stream,
    sample_rate: u32,
    channels: usize,
    producer: Producer<f32>,
}

fn init_output_sink(shared: &Arc<SharedState>) -> Result<OutputSink> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("Varsayılan ses çıkış aygıtı bulunamadı"))?;

    let default_cfg = device
        .default_output_config()
        .context("Ses aygıtı yapılandırması alınamadı")?;

    let sample_rate = default_cfg.sample_rate().0;
    let channels = default_cfg.channels() as usize;
    let sample_format = default_cfg.sample_format();
    let config: StreamConfig = default_cfg.into();

    // ~0.6s ring buffer in f32 (~230 KB at 48kHz stereo).
    let ring_cap = (sample_rate as usize * channels * 3) / 5;
    let (producer, mut consumer) = RingBuffer::<f32>::new(ring_cap.max(4096));

    let err_fn = |err| error!("cpal stream error: {err}");

    let stream = match sample_format {
        SampleFormat::F32 => {
            let s = Arc::clone(shared);
            device.build_output_stream(
                &config,
                move |data: &mut [f32], _| fill_output_f32(data, &mut consumer, &s),
                err_fn,
                None,
            )?
        }
        SampleFormat::I16 => {
            let s = Arc::clone(shared);
            device.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    let gain = effective_gain(&s);
                    drain_if_flushed(&mut consumer, &s);
                    for sample in data.iter_mut() {
                        let v = consumer.pop().unwrap_or(0.0) * gain;
                        *sample = (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    }
                },
                err_fn,
                None,
            )?
        }
        SampleFormat::U16 => {
            let s = Arc::clone(shared);
            device.build_output_stream(
                &config,
                move |data: &mut [u16], _| {
                    let gain = effective_gain(&s);
                    drain_if_flushed(&mut consumer, &s);
                    for sample in data.iter_mut() {
                        let v = consumer.pop().unwrap_or(0.0) * gain;
                        *sample = ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16;
                    }
                },
                err_fn,
                None,
            )?
        }
        other => return Err(anyhow!("Desteklenmeyen ses örnekleme formatı: {other:?}")),
    };

    // Start paused so idle CPU is 0%.
    let _ = stream.pause();

    info!("Audio output initialized: {sample_rate} Hz, {channels} ch, {sample_format:?}");
    Ok(OutputSink {
        stream,
        sample_rate,
        channels,
        producer,
    })
}

#[inline]
fn effective_gain(shared: &SharedState) -> f32 {
    let vol = f32::from_bits(shared.volume_bits.load(Ordering::Relaxed));
    let norm = f32::from_bits(shared.norm_gain_bits.load(Ordering::Relaxed));
    // Perceptual volume curve (vol^2) feels much more natural than linear.
    (vol * vol) * norm
}

#[inline]
fn drain_if_flushed(consumer: &mut Consumer<f32>, shared: &SharedState) {
    if shared.flush_ring.swap(false, Ordering::AcqRel) {
        let n = consumer.slots();
        if n > 0 {
            if let Ok(chunk) = consumer.read_chunk(n) {
                chunk.commit_all();
            }
        }
    }
}

fn fill_output_f32(data: &mut [f32], consumer: &mut Consumer<f32>, shared: &SharedState) {
    drain_if_flushed(consumer, shared);
    let gain = effective_gain(shared);
    for sample in data.iter_mut() {
        let v = consumer.pop().unwrap_or(0.0) * gain;
        *sample = v.clamp(-1.0, 1.0);
    }
}

fn audio_worker_main(shared: Arc<SharedState>) {
    let mut sink = match init_output_sink(&shared) {
        Ok(s) => Some(s),
        Err(e) => {
            error!("Audio output init failed: {e}");
            shared.set_error(Some(format!("Ses aygıtı başlatılamadı: {e}")));
            None
        }
    };

    loop {
        // Sleep on Condvar when completely idle -> 0.0% CPU.
        let cmd = shared.wait_cmd();
        match cmd {
            AudioCmd::Shutdown => break,
            AudioCmd::Stop => {
                if let Some(s) = sink.as_ref() {
                    let _ = s.stream.pause();
                }
                shared.is_playing.store(false, Ordering::Relaxed);
                shared.is_paused.store(false, Ordering::Relaxed);
                shared.is_buffering.store(false, Ordering::Relaxed);
                shared.position_ms.store(0, Ordering::Relaxed);
            }
            AudioCmd::Play(mut stream_info) => {
                // Lazy re-init if audio device was temporarily unavailable.
                if sink.is_none() {
                    match init_output_sink(&shared) {
                        Ok(s) => sink = Some(s),
                        Err(e) => {
                            shared.is_buffering.store(false, Ordering::Relaxed);
                            shared.set_error(Some(format!("Ses aygıtı hatası: {e}")));
                            continue;
                        }
                    }
                }
                let Some(active_sink) = sink.as_mut() else {
                    continue;
                };

                loop {
                    match play_stream(active_sink, &shared, &stream_info) {
                        Ok(PlayExit::NextTrack(next_stream)) => {
                            stream_info = next_stream;
                            continue;
                        }
                        Ok(PlayExit::Ended) => {
                            let _ = active_sink.stream.pause();
                            shared.is_playing.store(false, Ordering::Relaxed);
                            shared.is_paused.store(false, Ordering::Relaxed);
                            shared.is_buffering.store(false, Ordering::Relaxed);
                            shared.track_end_seq.fetch_add(1, Ordering::AcqRel);
                            break;
                        }
                        Ok(PlayExit::Stopped) => {
                            let _ = active_sink.stream.pause();
                            shared.is_playing.store(false, Ordering::Relaxed);
                            shared.is_paused.store(false, Ordering::Relaxed);
                            shared.is_buffering.store(false, Ordering::Relaxed);
                            break;
                        }
                        Ok(PlayExit::Shutdown) => return,
                        Err(e) => {
                            let _ = active_sink.stream.pause();
                            shared.is_playing.store(false, Ordering::Relaxed);
                            shared.is_paused.store(false, Ordering::Relaxed);
                            shared.is_buffering.store(false, Ordering::Relaxed);
                            warn!("Playback error for {}: {e}", stream_info.track_id);
                            shared.set_error(Some(format!("Çalma hatası: {e}")));
                            break;
                        }
                    }
                }
            }
            AudioCmd::Pause | AudioCmd::Resume | AudioCmd::Seek(_) => {}
        }
    }
}

enum PlayExit {
    Ended,
    Stopped,
    Shutdown,
    NextTrack(ResolvedStream),
}

fn hint_for_mime(mime: &str) -> Hint {
    let mut hint = Hint::new();
    let lower = mime.to_ascii_lowercase();
    if lower.contains("aac") && !lower.contains("mp4") {
        hint.with_extension("aac");
    } else if lower.contains("mp4") || lower.contains("m4a") {
        hint.with_extension("m4a");
    } else if lower.contains("mpeg") || lower.contains("mp3") {
        hint.with_extension("mp3");
    } else if lower.contains("webm") || lower.contains("matroska") {
        hint.with_extension("webm");
    }
    hint
}

fn play_stream(
    sink: &mut OutputSink,
    shared: &Arc<SharedState>,
    info: &ResolvedStream,
) -> Result<PlayExit> {
    shared.is_buffering.store(true, Ordering::Relaxed);
    shared.is_paused.store(false, Ordering::Relaxed);
    shared.position_ms.store(0, Ordering::Relaxed);
    shared
        .norm_gain_bits
        .store(info.normalization_gain().to_bits(), Ordering::Relaxed);
    shared.flush_ring.store(true, Ordering::Release);

    let cancel_flag = shared.new_stream_cancel();

    match &info.kind {
        StreamKind::Hls { segments } => {
            play_hls_stream(sink, shared, info, segments, cancel_flag)
        }
        StreamKind::Direct {
            url,
            content_length,
        } => play_direct_stream(sink, shared, info, url, *content_length, cancel_flag),
    }
}

fn play_hls_stream(
    sink: &mut OutputSink,
    shared: &Arc<SharedState>,
    info: &ResolvedStream,
    segments: &[HlsSegment],
    cancel_flag: Arc<AtomicBool>,
) -> Result<PlayExit> {
    if segments.is_empty() {
        return Err(anyhow!("HLS segment listesi boş"));
    }

    let total_dur = info
        .duration_secs
        .unwrap_or_else(|| segments.iter().map(|s| s.duration_secs).sum());
    shared
        .duration_ms
        .store((total_dur * 1000.0) as u64, Ordering::Relaxed);

    // Precompute segment start timestamps for O(1) seek
    let mut seg_starts = Vec::with_capacity(segments.len());
    let mut acc = 0.0_f64;
    for s in segments {
        seg_starts.push(acc);
        acc += s.duration_secs;
    }

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .pool_max_idle_per_host(2)
        .build()?;

    let mut seg_idx: usize = 0;
    let mut skip_until_secs: Option<f64> = None;
    let mut paused = false;
    let mut started_sink = false;
    let mut resampled: Vec<f32> = Vec::with_capacity(8192);
    // Next segment downloading in the background while the current one plays,
    // so segment boundaries never wait on the network.
    let mut prefetched: Option<(usize, thread::JoinHandle<Result<Vec<u8>>>)> = None;
    let spawn_segment_fetch = |idx: usize| {
        let client = client.clone();
        let url = segments[idx].url.clone();
        let ua = info.user_agent.clone();
        let cancel = Arc::clone(&cancel_flag);
        thread::spawn(move || fetch_segment_bytes(&client, &url, ua.as_deref(), &cancel))
    };

    while seg_idx < segments.len() {
        if cancel_flag.load(Ordering::Relaxed) {
            if let Some(cmd) = shared.try_pop_cmd() {
                return match cmd {
                    AudioCmd::Shutdown => Ok(PlayExit::Shutdown),
                    AudioCmd::Play(next) => Ok(PlayExit::NextTrack(next)),
                    _ => Ok(PlayExit::Stopped),
                };
            }
            return Ok(PlayExit::Stopped);
        }

        // Handle commands before fetching next segment
        let maybe_cmd = if paused {
            Some(shared.wait_cmd())
        } else {
            shared.try_pop_cmd()
        };
        if let Some(cmd) = maybe_cmd {
            match cmd {
                AudioCmd::Shutdown => return Ok(PlayExit::Shutdown),
                AudioCmd::Stop => {
                    shared.flush_ring.store(true, Ordering::Release);
                    return Ok(PlayExit::Stopped);
                }
                AudioCmd::Play(next) => {
                    shared.flush_ring.store(true, Ordering::Release);
                    return Ok(PlayExit::NextTrack(next));
                }
                AudioCmd::Pause => {
                    paused = true;
                    shared.is_paused.store(true, Ordering::Relaxed);
                    shared.is_playing.store(false, Ordering::Relaxed);
                    let _ = sink.stream.pause();
                    continue;
                }
                AudioCmd::Resume => {
                    paused = false;
                    shared.is_paused.store(false, Ordering::Relaxed);
                    shared.is_playing.store(true, Ordering::Relaxed);
                    let _ = sink.stream.play();
                    continue;
                }
                AudioCmd::Seek(target_secs) => {
                    let target = target_secs.clamp(0.0, total_dur);
                    seg_idx = find_segment_for_time(&seg_starts, target);
                    skip_until_secs = Some(target);
                    shared.flush_ring.store(true, Ordering::Release);
                    shared
                        .position_ms
                        .store((target * 1000.0) as u64, Ordering::Relaxed);
                    continue;
                }
            }
        }

        // Take the ~115 KB segment from the background download if it is the right one
        // (after a seek it may not be), otherwise fetch it now. Then start the next one.
        let seg_bytes = match prefetched.take() {
            Some((idx, handle)) if idx == seg_idx => match handle.join() {
                Ok(Ok(bytes)) => bytes,
                _ => fetch_segment_bytes(
                    &client,
                    &segments[seg_idx].url,
                    info.user_agent.as_deref(),
                    &cancel_flag,
                )?,
            },
            _ => fetch_segment_bytes(
                &client,
                &segments[seg_idx].url,
                info.user_agent.as_deref(),
                &cancel_flag,
            )?,
        };
        if seg_idx + 1 < segments.len() {
            prefetched = Some((seg_idx + 1, spawn_segment_fetch(seg_idx + 1)));
        }

        let mss = MediaSourceStream::new(Box::new(Cursor::new(seg_bytes)), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("aac");

        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .with_context(|| format!("HLS segment #{seg_idx} çözümlenemedi"))?;

        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow!("HLS segmentinde ses izi bulunamadı"))?;

        let track_id = track.id;
        let time_base = track.codec_params.time_base;
        let src_sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .context("HLS AAC decoder başlatılamadı")?;

        if !started_sink {
            shared.is_buffering.store(false, Ordering::Relaxed);
            shared.is_playing.store(true, Ordering::Relaxed);
            sink.stream.play().context("Ses çıkış akışı başlatılamadı")?;
            started_sink = true;
        }

        let seg_base_secs = seg_starts[seg_idx];
        let mut decoded_frames_in_seg: u64 = 0;
        let mut sample_buf: Option<SampleBuffer<f32>> = None;
        let mut seek_jump: Option<f64> = None;

        loop {
            let maybe_cmd = if paused {
                Some(shared.wait_cmd())
            } else {
                shared.try_pop_cmd()
            };
            if let Some(cmd) = maybe_cmd {
                match cmd {
                    AudioCmd::Shutdown => return Ok(PlayExit::Shutdown),
                    AudioCmd::Stop => {
                        shared.flush_ring.store(true, Ordering::Release);
                        return Ok(PlayExit::Stopped);
                    }
                    AudioCmd::Play(next) => {
                        shared.flush_ring.store(true, Ordering::Release);
                        return Ok(PlayExit::NextTrack(next));
                    }
                    AudioCmd::Pause => {
                        paused = true;
                        shared.is_paused.store(true, Ordering::Relaxed);
                        shared.is_playing.store(false, Ordering::Relaxed);
                        let _ = sink.stream.pause();
                        continue;
                    }
                    AudioCmd::Resume => {
                        paused = false;
                        shared.is_paused.store(false, Ordering::Relaxed);
                        shared.is_playing.store(true, Ordering::Relaxed);
                        let _ = sink.stream.play();
                        continue;
                    }
                    AudioCmd::Seek(target_secs) => {
                        seek_jump = Some(target_secs.clamp(0.0, total_dur));
                        break;
                    }
                }
            }

            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(SymphoniaError::ResetRequired) => {
                    decoder.reset();
                    continue;
                }
                Err(e) => {
                    debug!("HLS segment #{seg_idx} read ended: {e}");
                    break;
                }
            };

            if packet.track_id() != track_id {
                continue;
            }

            let decoded = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
                Err(e) => return Err(anyhow!("HLS ses çözme hatası: {e}")),
            };

            let spec = *decoded.spec();
            let src_channels = spec.channels.count().max(1);
            let actual_src_rate = spec.rate.max(src_sample_rate);

            let pkt_secs = if let Some(tb) = time_base {
                let t = tb.calc_time(packet.ts());
                seg_base_secs + t.seconds as f64 + t.frac
            } else {
                seg_base_secs + (decoded_frames_in_seg as f64 / actual_src_rate as f64)
            };

            let sb = sample_buf
                .get_or_insert_with(|| SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
            sb.copy_interleaved_ref(decoded);
            let samples = sb.samples();
            decoded_frames_in_seg += (samples.len() / src_channels) as u64;

            if let Some(target) = skip_until_secs {
                if pkt_secs + 0.05 < target {
                    continue;
                }
                skip_until_secs = None;
            }

            shared
                .position_ms
                .store((pkt_secs * 1000.0) as u64, Ordering::Relaxed);

            convert_and_resample(
                samples,
                src_channels,
                actual_src_rate,
                sink.channels,
                sink.sample_rate,
                &mut resampled,
            );

            let mut offset = 0;
            while offset < resampled.len() {
                if let Some(cmd) = shared.try_pop_cmd() {
                    match cmd {
                        AudioCmd::Shutdown => return Ok(PlayExit::Shutdown),
                        AudioCmd::Stop => {
                            shared.flush_ring.store(true, Ordering::Release);
                            return Ok(PlayExit::Stopped);
                        }
                        AudioCmd::Play(next) => {
                            shared.flush_ring.store(true, Ordering::Release);
                            return Ok(PlayExit::NextTrack(next));
                        }
                        AudioCmd::Pause => {
                            paused = true;
                            shared.is_paused.store(true, Ordering::Relaxed);
                            shared.is_playing.store(false, Ordering::Relaxed);
                            let _ = sink.stream.pause();
                            break;
                        }
                        AudioCmd::Seek(target_secs) => {
                            seek_jump = Some(target_secs.clamp(0.0, total_dur));
                            break;
                        }
                        AudioCmd::Resume => {}
                    }
                }

                let avail = sink.producer.slots();
                if avail == 0 {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }

                let to_write = avail.min(resampled.len() - offset);
                if let Ok(mut chunk) = sink.producer.write_chunk(to_write) {
                    let (first, second) = chunk.as_mut_slices();
                    let n1 = first.len();
                    first.copy_from_slice(&resampled[offset..offset + n1]);
                    if !second.is_empty() {
                        second.copy_from_slice(&resampled[offset + n1..offset + to_write]);
                    }
                    chunk.commit_all();
                    offset += to_write;
                }
            }

            if seek_jump.is_some() {
                break;
            }
        }

        if let Some(target) = seek_jump {
            seg_idx = find_segment_for_time(&seg_starts, target);
            skip_until_secs = Some(target);
            shared.flush_ring.store(true, Ordering::Release);
            shared
                .position_ms
                .store((target * 1000.0) as u64, Ordering::Relaxed);
        } else {
            seg_idx += 1;
        }
    }

    // Let ring buffer finish playing last ~0.6s cleanly
    let drain_deadline = std::time::Instant::now() + Duration::from_millis(650);
    while std::time::Instant::now() < drain_deadline {
        if let Some(cmd) = shared.try_pop_cmd() {
            match cmd {
                AudioCmd::Shutdown => return Ok(PlayExit::Shutdown),
                AudioCmd::Stop => return Ok(PlayExit::Stopped),
                AudioCmd::Play(next) => return Ok(PlayExit::NextTrack(next)),
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(20));
    }

    Ok(PlayExit::Ended)
}

fn find_segment_for_time(seg_starts: &[f64], target_secs: f64) -> usize {
    match seg_starts.binary_search_by(|s| {
        s.partial_cmp(&target_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    }) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

fn fetch_segment_bytes(
    client: &reqwest::blocking::Client,
    url: &str,
    ua: Option<&str>,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<u8>> {
    let mut last_err = None;
    for attempt in 0..3 {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("stream cancelled"));
        }
        let mut req = client.get(url);
        if let Some(u) = ua {
            req = req.header(USER_AGENT, u);
        }
        match req.send() {
            Ok(resp) if resp.status().is_success() => match resp.bytes() {
                Ok(b) => return Ok(b.to_vec()),
                Err(e) => last_err = Some(anyhow!("HLS segment gövdesi okunamadı: {e}")),
            },
            Ok(resp) => {
                let st = resp.status();
                last_err = Some(anyhow!("HLS segment HTTP {st}"));
                if st.as_u16() == 403 || st.as_u16() == 404 {
                    break;
                }
            }
            Err(e) => last_err = Some(anyhow!("HLS segment isteği hatası: {e}")),
        }
        thread::sleep(Duration::from_millis(150 * (attempt + 1)));
    }
    Err(last_err.unwrap_or_else(|| anyhow!("HLS segment indirilemedi")))
}

fn play_direct_stream(
    sink: &mut OutputSink,
    shared: &Arc<SharedState>,
    info: &ResolvedStream,
    url: &str,
    content_length: Option<u64>,
    cancel_flag: Arc<AtomicBool>,
) -> Result<PlayExit> {
    let http_stream = HttpAudioStream::open(
        url.to_string(),
        content_length,
        info.user_agent.clone(),
        Arc::clone(&cancel_flag),
    )?;

    let mss = MediaSourceStream::new(Box::new(http_stream), Default::default());
    let hint = hint_for_mime(&info.mime);

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .context("Ses akışı formatı tanınamadı")?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("Akışta geçerli ses izi bulunamadı"))?;

    let track_id = track.id;
    let time_base = track.codec_params.time_base;
    let src_sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let duration_secs = info.duration_secs.or_else(|| {
        let frames = track.codec_params.n_frames?;
        let tb = time_base?;
        let t = tb.calc_time(frames);
        Some(t.seconds as f64 + t.frac)
    });

    if let Some(d) = duration_secs {
        shared
            .duration_ms
            .store((d * 1000.0) as u64, Ordering::Relaxed);
    }

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("Ses çözücü (decoder) başlatılamadı")?;

    shared.is_buffering.store(false, Ordering::Relaxed);
    shared.is_playing.store(true, Ordering::Relaxed);
    sink.stream
        .play()
        .context("Ses çıkış akışı başlatılamadı")?;

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut resampled: Vec<f32> = Vec::with_capacity(8192);
    let mut paused = false;

    loop {
        // 1. Handle any pending commands (if paused, block on Condvar -> 0% CPU!).
        let maybe_cmd = if paused {
            Some(shared.wait_cmd())
        } else {
            shared.try_pop_cmd()
        };

        if let Some(cmd) = maybe_cmd {
            match cmd {
                AudioCmd::Shutdown => return Ok(PlayExit::Shutdown),
                AudioCmd::Stop => {
                    shared.flush_ring.store(true, Ordering::Release);
                    return Ok(PlayExit::Stopped);
                }
                AudioCmd::Play(next) => {
                    shared.flush_ring.store(true, Ordering::Release);
                    return Ok(PlayExit::NextTrack(next));
                }
                AudioCmd::Pause => {
                    paused = true;
                    shared.is_paused.store(true, Ordering::Relaxed);
                    shared.is_playing.store(false, Ordering::Relaxed);
                    let _ = sink.stream.pause();
                    continue;
                }
                AudioCmd::Resume => {
                    paused = false;
                    shared.is_paused.store(false, Ordering::Relaxed);
                    shared.is_playing.store(true, Ordering::Relaxed);
                    let _ = sink.stream.play();
                    continue;
                }
                AudioCmd::Seek(target_secs) => {
                    let target_time = Time::from(target_secs);
                    match format.seek(
                        SeekMode::Coarse,
                        SeekTo::Time {
                            time: target_time,
                            track_id: Some(track_id),
                        },
                    ) {
                        Ok(_) => {
                            decoder.reset();
                            shared.flush_ring.store(true, Ordering::Release);
                            shared
                                .position_ms
                                .store((target_secs * 1000.0) as u64, Ordering::Relaxed);
                            debug!("Seeked to {target_secs:.2}s");
                        }
                        Err(e) => {
                            warn!("Seek failed: {e}");
                        }
                    }
                    continue;
                }
            }
        }

        // 2. Read next packet from progressive HTTP stream.
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Normal EOF — let remaining ring buffer samples play out cleanly.
                let drain_deadline = std::time::Instant::now() + Duration::from_millis(700);
                while std::time::Instant::now() < drain_deadline {
                    if let Some(cmd) = shared.try_pop_cmd() {
                        match cmd {
                            AudioCmd::Shutdown => return Ok(PlayExit::Shutdown),
                            AudioCmd::Stop => return Ok(PlayExit::Stopped),
                            AudioCmd::Play(next) => return Ok(PlayExit::NextTrack(next)),
                            _ => {}
                        }
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                return Ok(PlayExit::Ended);
            }
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::Interrupted => {
                // Cancelled due to Stop/Play/Shutdown
                if let Some(cmd) = shared.try_pop_cmd() {
                    return match cmd {
                        AudioCmd::Shutdown => Ok(PlayExit::Shutdown),
                        AudioCmd::Play(next) => Ok(PlayExit::NextTrack(next)),
                        _ => Ok(PlayExit::Stopped),
                    };
                }
                return Ok(PlayExit::Stopped);
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(anyhow!("Akış okuma hatası: {e}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        if let Some(tb) = time_base {
            let t = tb.calc_time(packet.ts());
            let ms = ((t.seconds as f64 + t.frac) * 1000.0) as u64;
            shared.position_ms.store(ms, Ordering::Relaxed);
        }

        // 3. Decode packet to interleaved f32.
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(msg)) => {
                debug!("Skipping corrupt audio frame: {msg}");
                continue;
            }
            Err(SymphoniaError::IoError(_)) => continue,
            Err(e) => return Err(anyhow!("Ses çözme hatası: {e}")),
        };

        let spec = *decoded.spec();
        let src_channels = spec.channels.count().max(1);
        let actual_src_rate = spec.rate.max(src_sample_rate);

        let sb = sample_buf.get_or_insert_with(|| {
            SampleBuffer::<f32>::new(decoded.capacity() as u64, spec)
        });
        sb.copy_interleaved_ref(decoded);
        let samples = sb.samples();

        // 4. Convert channel count + linear resample if needed.
        convert_and_resample(
            samples,
            src_channels,
            actual_src_rate,
            sink.channels,
            sink.sample_rate,
            &mut resampled,
        );

        // 5. Push into lock-free ring buffer while staying responsive to commands.
        let mut offset = 0;
        while offset < resampled.len() {
            if let Some(cmd) = shared.try_pop_cmd() {
                match cmd {
                    AudioCmd::Shutdown => return Ok(PlayExit::Shutdown),
                    AudioCmd::Stop => {
                        shared.flush_ring.store(true, Ordering::Release);
                        return Ok(PlayExit::Stopped);
                    }
                    AudioCmd::Play(next) => {
                        shared.flush_ring.store(true, Ordering::Release);
                        return Ok(PlayExit::NextTrack(next));
                    }
                    AudioCmd::Pause => {
                        paused = true;
                        shared.is_paused.store(true, Ordering::Relaxed);
                        shared.is_playing.store(false, Ordering::Relaxed);
                        let _ = sink.stream.pause();
                        break;
                    }
                    AudioCmd::Seek(target_secs) => {
                        let target_time = Time::from(target_secs);
                        if format
                            .seek(
                                SeekMode::Coarse,
                                SeekTo::Time {
                                    time: target_time,
                                    track_id: Some(track_id),
                                },
                            )
                            .is_ok()
                        {
                            decoder.reset();
                            shared.flush_ring.store(true, Ordering::Release);
                            shared
                                .position_ms
                                .store((target_secs * 1000.0) as u64, Ordering::Relaxed);
                        }
                        break;
                    }
                    AudioCmd::Resume => {}
                }
            }

            let avail = sink.producer.slots();
            if avail == 0 {
                thread::sleep(Duration::from_millis(5));
                continue;
            }

            let to_write = avail.min(resampled.len() - offset);
            if let Ok(mut chunk) = sink.producer.write_chunk(to_write) {
                let (first, second) = chunk.as_mut_slices();
                let n1 = first.len();
                first.copy_from_slice(&resampled[offset..offset + n1]);
                if !second.is_empty() {
                    second.copy_from_slice(&resampled[offset + n1..offset + to_write]);
                }
                chunk.commit_all();
                offset += to_write;
            }
        }
    }
}

/// High-quality 4-point Cubic Catmull-Rom resampler + channel mapper into a reusable buffer (zero allocations).
/// Eliminates the high-frequency roll-off & aliasing shimmer of linear interpolation when resampling 44.1 kHz -> 48 kHz.
fn convert_and_resample(
    src: &[f32],
    src_ch: usize,
    src_rate: u32,
    dst_ch: usize,
    dst_rate: u32,
    out: &mut Vec<f32>,
) {
    out.clear();
    let src_frames = src.len() / src_ch;
    if src_frames == 0 {
        return;
    }

    if src_rate == dst_rate && src_ch == dst_ch {
        out.extend_from_slice(src);
        return;
    }

    let ratio = src_rate as f64 / dst_rate as f64;
    let dst_frames = ((src_frames as f64) / ratio).round().max(1.0) as usize;
    out.reserve(dst_frames * dst_ch);
    let max_idx = src_frames - 1;

    for df in 0..dst_frames {
        let pos = df as f64 * ratio;
        let i1 = (pos as usize).min(max_idx);
        let i0 = i1.saturating_sub(1);
        let i2 = (i1 + 1).min(max_idx);
        let i3 = (i1 + 2).min(max_idx);
        let t = (pos - i1 as f64) as f32;
        let t2 = t * t;
        let t3 = t2 * t;

        for ch in 0..dst_ch {
            let s_ch = if src_ch == 1 { 0 } else { ch.min(src_ch - 1) };
            let y0 = src[i0 * src_ch + s_ch];
            let y1 = src[i1 * src_ch + s_ch];
            let y2 = src[i2 * src_ch + s_ch];
            let y3 = src[i3 * src_ch + s_ch];

            // Catmull-Rom cubic spline interpolation
            let a0 = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
            let a1 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let a2 = -0.5 * y0 + 0.5 * y2;
            let a3 = y1;
            let sample = ((a0 * t3) + (a1 * t2) + (a2 * t) + a3).clamp(-1.0, 1.0);
            out.push(sample);
        }
    }
}

/// Result of headless smoke-test decoding.
#[derive(Debug, Clone)]
pub struct DecodeStats {
    pub sample_rate: u32,
    pub channels: usize,
    pub packets_decoded: usize,
    pub decoded_secs: f64,
    pub rms_level: f32,
}

/// Headless verification helper used by `examples/smoke.rs`:
/// Opens the stream (HLS segment or progressive HTTP), probes with symphonia, and decodes `max_seconds` of audio.
pub fn probe_and_decode_sample(info: &ResolvedStream, max_seconds: f64) -> Result<DecodeStats> {
    let cancel = Arc::new(AtomicBool::new(false));
    let (mss, hint) = match &info.kind {
        StreamKind::Hls { segments } => {
            let first_seg = segments
                .first()
                .ok_or_else(|| anyhow!("HLS segment listesi boş"))?;
            let client = reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .build()?;
            let bytes =
                fetch_segment_bytes(&client, &first_seg.url, info.user_agent.as_deref(), &cancel)?;
            let mut h = Hint::new();
            h.with_extension("aac");
            (
                MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default()),
                h,
            )
        }
        StreamKind::Direct {
            url,
            content_length,
        } => {
            let http_stream = HttpAudioStream::open(
                url.clone(),
                *content_length,
                info.user_agent.clone(),
                cancel,
            )?;
            (
                MediaSourceStream::new(Box::new(http_stream), Default::default()),
                hint_for_mime(&info.mime),
            )
        }
    };

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .context("Symphonia probe failed")?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("No audio track found"))?;

    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("Symphonia decoder init failed")?;

    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut channels = 2usize;
    let mut packets_decoded = 0usize;
    let mut total_frames = 0usize;
    let mut sum_sq = 0.0_f64;
    let mut total_samples = 0usize;
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    while (total_frames as f64 / sample_rate as f64) < max_seconds {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(anyhow!("Packet read error: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(anyhow!("Decode error: {e}")),
        };

        let spec = *decoded.spec();
        sample_rate = spec.rate;
        channels = spec.channels.count().max(1);
        let sb = sample_buf.get_or_insert_with(|| {
            SampleBuffer::<f32>::new(decoded.capacity() as u64, spec)
        });
        sb.copy_interleaved_ref(decoded);
        let samples = sb.samples();

        for &s in samples {
            sum_sq += (s as f64) * (s as f64);
        }
        total_samples += samples.len();
        total_frames += samples.len() / channels;
        packets_decoded += 1;
    }

    let rms = if total_samples > 0 {
        (sum_sq / total_samples as f64).sqrt() as f32
    } else {
        0.0
    };

    Ok(DecodeStats {
        sample_rate,
        channels,
        packets_decoded,
        decoded_secs: total_frames as f64 / sample_rate as f64,
        rms_level: rms,
    })
}
