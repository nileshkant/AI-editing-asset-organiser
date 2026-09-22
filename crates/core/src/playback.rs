use crate::{invalid, media::MediaTools, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Finished,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaybackStatus {
    pub sound_id: Option<String>,
    pub state: PlaybackState,
    pub position_seconds: f64,
    pub duration_seconds: f64,
    pub volume: f32,
    pub peak: f32,
    pub error: Option<String>,
}

pub struct SharedAudioState {
    pub is_playing: AtomicBool,
    pub volume_bits: AtomicU32,
    pub played_frames: AtomicU64,
    pub decoded_frames: AtomicU64,
    pub peak_bits: AtomicU32,
    pub underrun_count: AtomicU64,
    pub sample_rate: u32,
    pub channels: u16,
    pub decoder_eof: AtomicBool,
    pub error_msg: Mutex<Option<String>>,
}

impl SharedAudioState {
    pub fn new(sample_rate: u32, channels: u16, volume: f32) -> Self {
        Self {
            is_playing: AtomicBool::new(false),
            volume_bits: AtomicU32::new(volume.to_bits()),
            played_frames: AtomicU64::new(0),
            decoded_frames: AtomicU64::new(0),
            peak_bits: AtomicU32::new(0),
            underrun_count: AtomicU64::new(0),
            sample_rate,
            channels,
            decoder_eof: AtomicBool::new(false),
            error_msg: Mutex::new(None),
        }
    }
}

pub enum OutputSink {
    Cpal(cpal::Stream),
    Loopback(Arc<LoopbackSink>),
}

pub struct LoopbackSink {
    pub shared: Arc<SharedAudioState>,
    pub consumer: Mutex<Consumer<f32>>,
    pub captured: Mutex<Vec<f32>>,
}

impl LoopbackSink {
    pub fn available_samples(&self) -> usize {
        self.consumer.lock().map(|c| c.slots()).unwrap_or(0)
    }

    pub fn step(&self, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * self.shared.channels as usize);
        let volume = f32::from_bits(self.shared.volume_bits.load(Ordering::Relaxed));
        let is_playing = self.shared.is_playing.load(Ordering::Relaxed);
        let channels = self.shared.channels as usize;
        let mut max_abs: f32 = 0.0;
        let mut frames_consumed = 0u64;

        if let Ok(mut cons) = self.consumer.lock() {
            for _ in 0..frames {
                let mut underrun = false;
                for _ in 0..channels {
                    if is_playing {
                        match cons.pop() {
                            Ok(val) => {
                                let sample = val * volume;
                                out.push(sample);
                                let abs = sample.abs();
                                if abs > max_abs { max_abs = abs; }
                            }
                            Err(_) => {
                                out.push(0.0);
                                underrun = true;
                            }
                        }
                    } else {
                        out.push(0.0);
                    }
                }
                if underrun {
                    self.shared.underrun_count.fetch_add(1, Ordering::Relaxed);
                } else if is_playing {
                    frames_consumed += 1;
                }
            }
        }

        self.shared.played_frames.fetch_add(frames_consumed, Ordering::Relaxed);
        self.shared.peak_bits.store(max_abs.to_bits(), Ordering::Relaxed);

        if let Ok(mut cap) = self.captured.lock() {
            cap.extend_from_slice(&out);
        }
        out
    }
}

struct ActiveTrack {
    sound_id: String,
    path: PathBuf,
    duration_seconds: f64,
    seek_offset_seconds: f64,
    shared: Arc<SharedAudioState>,
    cancel: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    decoder_thread: Option<JoinHandle<()>>,
    sink: OutputSink,
}

pub struct Player {
    tools: Option<MediaTools>,
    volume_bits: AtomicU32,
    active: Mutex<Option<ActiveTrack>>,
    force_loopback: bool,
    last_error: Mutex<Option<String>>,
}

impl Player {
    pub fn new(tools: Option<MediaTools>) -> Self {
        Self {
            tools,
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            active: Mutex::new(None),
            force_loopback: false,
            last_error: Mutex::new(None),
        }
    }

    pub fn new_loopback(tools: Option<MediaTools>) -> Self {
        Self {
            tools,
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            active: Mutex::new(None),
            force_loopback: true,
            last_error: Mutex::new(None),
        }
    }

    pub fn set_volume(&self, volume: f32) {
        let clamped = volume.clamp(0.0, 2.0);
        self.volume_bits.store(clamped.to_bits(), Ordering::Relaxed);
        if let Ok(guard) = self.active.lock() {
            if let Some(track) = guard.as_ref() {
                track.shared.volume_bits.store(clamped.to_bits(), Ordering::Relaxed);
            }
        }
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn play(&self, sound_id: &str, path: &Path, duration_seconds: f64) -> Result<()> {
        self.play_at(sound_id, path, 0.0, duration_seconds, true)
    }

    pub fn play_at(
        &self,
        sound_id: &str,
        path: &Path,
        start_seconds: f64,
        duration_seconds: f64,
        start_playing: bool,
    ) -> Result<()> {
        self.stop();
        *self.last_error.lock().unwrap() = None;

        if !path.is_file() {
            let err = format!("Audio file does not exist: {}", path.display());
            *self.last_error.lock().unwrap() = Some(err.clone());
            return Err(invalid(&err));
        }

        let tools = self.tools.as_ref().ok_or_else(|| {
            let err = "Media tools unavailable for playback decoding";
            *self.last_error.lock().unwrap() = Some(err.into());
            invalid(err)
        })?;

        let volume = self.volume();
        let (sink, shared, producer) = self.create_sink(volume)?;

        let track = self.start_decoder_and_track(
            sound_id,
            path,
            duration_seconds,
            start_seconds,
            tools,
            shared,
            sink,
            producer,
        )?;

        if start_playing {
            track.shared.is_playing.store(true, Ordering::Relaxed);
            if let OutputSink::Cpal(ref stream) = track.sink {
                if let Err(e) = stream.play() {
                    let msg = format!("Failed to start audio stream: {e}");
                    *self.last_error.lock().unwrap() = Some(msg.clone());
                    return Err(invalid(&msg));
                }
            }
        }

        *self.active.lock().unwrap() = Some(track);
        Ok(())
    }

    pub fn pause(&self) {
        if let Ok(guard) = self.active.lock() {
            if let Some(track) = guard.as_ref() {
                track.shared.is_playing.store(false, Ordering::Relaxed);
                if let OutputSink::Cpal(ref stream) = track.sink {
                    let _ = stream.pause();
                }
            }
        }
    }

    pub fn resume(&self) -> Result<()> {
        if let Ok(guard) = self.active.lock() {
            if let Some(track) = guard.as_ref() {
                track.shared.is_playing.store(true, Ordering::Relaxed);
                if let OutputSink::Cpal(ref stream) = track.sink {
                    stream.play().map_err(|e| invalid(&format!("Resume failed: {e}")))?;
                }
            }
        }
        Ok(())
    }

    pub fn stop(&self) {
        let mut guard = self.active.lock().unwrap();
        if let Some(mut track) = guard.take() {
            track.shared.is_playing.store(false, Ordering::Relaxed);
            track.cancel.store(true, Ordering::Relaxed);
            if let OutputSink::Cpal(ref stream) = track.sink {
                let _ = stream.pause();
            }
            if let Ok(mut child_lock) = track.child.lock() {
                if let Some(mut child) = child_lock.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            if let Some(handle) = track.decoder_thread.take() {
                let _ = handle.join();
            }
        }
    }

    pub fn seek(&self, target_seconds: f64) -> Result<()> {
        let (sound_id, path, duration_seconds, is_playing) = {
            let guard = self.active.lock().unwrap();
            match guard.as_ref() {
                Some(t) => (
                    t.sound_id.clone(),
                    t.path.clone(),
                    t.duration_seconds,
                    t.shared.is_playing.load(Ordering::Relaxed),
                ),
                None => return Ok(()),
            }
        };

        let target = target_seconds.max(0.0).min(duration_seconds);
        self.play_at(&sound_id, &path, target, duration_seconds, is_playing)
    }

    pub fn status(&self) -> PlaybackStatus {
        let guard = self.active.lock().unwrap();
        let track = match guard.as_ref() {
            Some(t) => t,
            None => {
                let last_err = self.last_error.lock().unwrap().clone();
                return PlaybackStatus {
                    sound_id: None,
                    state: if last_err.is_some() { PlaybackState::Error } else { PlaybackState::Stopped },
                    position_seconds: 0.0,
                    duration_seconds: 0.0,
                    volume: self.volume(),
                    peak: 0.0,
                    error: last_err,
                };
            }
        };

        if let Ok(err_lock) = track.shared.error_msg.lock() {
            if let Some(e) = err_lock.as_ref() {
                return PlaybackStatus {
                    sound_id: Some(track.sound_id.clone()),
                    state: PlaybackState::Error,
                    position_seconds: 0.0,
                    duration_seconds: track.duration_seconds,
                    volume: self.volume(),
                    peak: 0.0,
                    error: Some(e.clone()),
                };
            }
        }

        let is_playing = track.shared.is_playing.load(Ordering::Relaxed);
        let decoder_eof = track.shared.decoder_eof.load(Ordering::Relaxed);
        let decoded_frames = track.shared.decoded_frames.load(Ordering::Relaxed);
        let played_frames = track.shared.played_frames.load(Ordering::Relaxed);

        let elapsed = (played_frames as f64) / (track.shared.sample_rate as f64);
        let mut position = track.seek_offset_seconds + elapsed;

        if (decoder_eof && played_frames >= decoded_frames && decoded_frames > 0)
            || (decoder_eof && position >= track.duration_seconds.max(0.05))
        {
            track.shared.is_playing.store(false, Ordering::Relaxed);
            return PlaybackStatus {
                sound_id: Some(track.sound_id.clone()),
                state: PlaybackState::Finished,
                position_seconds: track.duration_seconds,
                duration_seconds: track.duration_seconds,
                volume: self.volume(),
                peak: 0.0,
                error: None,
            };
        }

        if position > track.duration_seconds && track.duration_seconds > 0.0 {
            position = track.duration_seconds;
        }

        let peak = f32::from_bits(track.shared.peak_bits.load(Ordering::Relaxed));
        let state = if is_playing { PlaybackState::Playing } else { PlaybackState::Paused };

        PlaybackStatus {
            sound_id: Some(track.sound_id.clone()),
            state,
            position_seconds: position,
            duration_seconds: track.duration_seconds,
            volume: self.volume(),
            peak,
            error: None,
        }
    }

    pub fn loopback_sink(&self) -> Option<Arc<LoopbackSink>> {
        let guard = self.active.lock().unwrap();
        if let Some(track) = guard.as_ref() {
            if let OutputSink::Loopback(ref sink) = track.sink {
                return Some(sink.clone());
            }
        }
        None
    }

    fn create_sink(
        &self,
        volume: f32,
    ) -> Result<(OutputSink, Arc<SharedAudioState>, Producer<f32>)> {
        if self.force_loopback {
            return Ok(self.create_loopback_sink_internal(volume));
        }

        // Try CPAL native audio output
        let host = cpal::default_host();
        let device = match host.default_output_device() {
            Some(d) => d,
            None => {
                return Ok(self.create_loopback_sink_internal(volume));
            }
        };

        let config = match device.default_output_config() {
            Ok(c) => c,
            Err(_) => {
                return Ok(self.create_loopback_sink_internal(volume));
            }
        };

        let sample_rate = config.sample_rate();
        let channels = config.channels();
        let shared = Arc::new(SharedAudioState::new(sample_rate, channels, volume));

        let buffer_capacity = (sample_rate as usize) * (channels as usize) * 2;
        let (producer, mut consumer) = RingBuffer::new(buffer_capacity);

        let callback_shared = shared.clone();
        let err_shared = shared.clone();

        let error_callback = move |err| {
            if let Ok(mut e) = err_shared.error_msg.lock() {
                *e = Some(format!("Audio output stream error: {err}"));
            }
        };

        let stream_config: cpal::StreamConfig = config.into();
        let stream = match device.build_output_stream(
            stream_config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                render_audio(data, &mut consumer, &callback_shared);
            },
            error_callback,
            None,
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Warning: Failed to build CPAL stream ({e}); falling back to loopback sink");
                return Ok(self.create_loopback_sink_internal(volume));
            }
        };

        Ok((OutputSink::Cpal(stream), shared, producer))
    }

    fn create_loopback_sink_internal(
        &self,
        volume: f32,
    ) -> (OutputSink, Arc<SharedAudioState>, Producer<f32>) {
        let sample_rate = 48000;
        let channels = 2;
        let shared = Arc::new(SharedAudioState::new(sample_rate, channels, volume));
        let buffer_capacity = (sample_rate as usize) * (channels as usize) * 2;

        let (producer, consumer) = RingBuffer::new(buffer_capacity);
        let sink = Arc::new(LoopbackSink {
            shared: shared.clone(),
            consumer: Mutex::new(consumer),
            captured: Mutex::new(Vec::new()),
        });

        (OutputSink::Loopback(sink), shared, producer)
    }

    fn start_decoder_and_track(
        &self,
        sound_id: &str,
        path: &Path,
        duration_seconds: f64,
        seek_offset_seconds: f64,
        tools: &MediaTools,
        shared: Arc<SharedAudioState>,
        sink: OutputSink,
        producer: Producer<f32>,
    ) -> Result<ActiveTrack> {
        let cancel = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));

        let thread_handle = spawn_decoder(
            tools.clone(),
            path.to_path_buf(),
            seek_offset_seconds,
            shared.sample_rate,
            shared.channels,
            producer,
            cancel.clone(),
            child.clone(),
            shared.clone(),
        );

        Ok(ActiveTrack {
            sound_id: sound_id.to_string(),
            path: path.to_path_buf(),
            duration_seconds,
            seek_offset_seconds,
            shared,
            cancel,
            child,
            decoder_thread: Some(thread_handle),
            sink,
        })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

// REALTIME SAFE: Zero heap allocations, zero blocking locks
#[inline(always)]
fn render_audio(
    data: &mut [f32],
    consumer: &mut Consumer<f32>,
    shared: &SharedAudioState,
) {
    let volume = f32::from_bits(shared.volume_bits.load(Ordering::Relaxed));
    let is_playing = shared.is_playing.load(Ordering::Relaxed);
    let channels = shared.channels as usize;
    let mut max_abs: f32 = 0.0;
    let mut frames_consumed = 0u64;

    if is_playing {
        let mut idx = 0;
        while idx < data.len() {
            let mut underrun = false;
            for ch in 0..channels {
                if idx + ch < data.len() {
                    match consumer.pop() {
                        Ok(val) => {
                            let out = val * volume;
                            data[idx + ch] = out;
                            let abs = out.abs();
                            if abs > max_abs { max_abs = abs; }
                        }
                        Err(_) => {
                            data[idx + ch] = 0.0;
                            underrun = true;
                        }
                    }
                }
            }
            if underrun {
                shared.underrun_count.fetch_add(1, Ordering::Relaxed);
            } else {
                frames_consumed += 1;
            }
            idx += channels;
        }

        shared.played_frames.fetch_add(frames_consumed, Ordering::Relaxed);
        shared.peak_bits.store(max_abs.to_bits(), Ordering::Relaxed);
    } else {
        data.fill(0.0);
        shared.peak_bits.store(0, Ordering::Relaxed);
    }
}

fn spawn_decoder(
    tools: MediaTools,
    path: PathBuf,
    seek_seconds: f64,
    sample_rate: u32,
    channels: u16,
    mut producer: Producer<f32>,
    cancel: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
    shared: Arc<SharedAudioState>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut cmd = Command::new(&tools.ffmpeg);
        cmd.args([
            "-v", "error",
            "-nostdin",
            "-ss", &format!("{:.3}", seek_seconds),
            "-i",
        ])
        .arg(&path)
        .args([
            "-map", "0:a:0",
            "-vn",
            "-ar", &format!("{}", sample_rate),
            "-ac", &format!("{}", channels),
            "-f", "f32le",
            "-acodec", "pcm_f32le",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        let mut process = match cmd.spawn() {
            Ok(p) => p,
            Err(e) => {
                if let Ok(mut err) = shared.error_msg.lock() {
                    *err = Some(format!("Failed to start audio decoder: {e}"));
                }
                return;
            }
        };

        let mut stdout = match process.stdout.take() {
            Some(out) => out,
            None => {
                let _ = process.kill();
                return;
            }
        };
        let stderr = process.stderr.take();

        *child_slot.lock().unwrap() = Some(process);

        let mut byte_buf = [0u8; 4096];
        let mut total_samples_pushed = 0u64;
        let mut stream_error = false;

        loop {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            match stdout.read(&mut byte_buf) {
                Ok(0) => {
                    break;
                }
                Ok(bytes_read) => {
                    let sample_count = bytes_read / 4;
                    let mut i = 0;
                    while i < sample_count {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let s = f32::from_le_bytes([
                            byte_buf[i * 4],
                            byte_buf[i * 4 + 1],
                            byte_buf[i * 4 + 2],
                            byte_buf[i * 4 + 3],
                        ]);
                        match producer.push(s) {
                            Ok(()) => {
                                i += 1;
                                total_samples_pushed += 1;
                                if total_samples_pushed % (channels as u64) == 0 {
                                    shared.decoded_frames.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Err(_) => {
                                // Buffer full: yield/sleep for consumer to read
                                thread::sleep(Duration::from_millis(5));
                            }
                        }
                    }
                }
                Err(_) => {
                    stream_error = true;
                    break;
                }
            }
        }

        let is_cancelled = cancel.load(Ordering::Relaxed);
        let mut exit_status = None;

        if let Ok(mut lock) = child_slot.lock() {
            if let Some(mut p) = lock.take() {
                if is_cancelled {
                    let _ = p.kill();
                    let _ = p.wait();
                } else {
                    exit_status = p.wait().ok();
                }
            }
        }

        if !is_cancelled {
            if let Some(status) = exit_status {
                if !status.success() && total_samples_pushed == 0 {
                    let mut err_text = String::new();
                    if let Some(mut err_pipe) = stderr {
                        let _ = err_pipe.read_to_string(&mut err_text);
                    }
                    let msg = if err_text.trim().is_empty() {
                        format!("Audio decode failed with status {status}")
                    } else {
                        err_text.trim().to_string()
                    };
                    if let Ok(mut err) = shared.error_msg.lock() {
                        *err = Some(msg);
                    }
                    return;
                }
            }
            if stream_error && total_samples_pushed == 0 {
                if let Ok(mut err) = shared.error_msg.lock() {
                    *err = Some("Audio stream decode read error".to_string());
                }
                return;
            }
            shared.decoder_eof.store(true, Ordering::Relaxed);
        }
    })
}
