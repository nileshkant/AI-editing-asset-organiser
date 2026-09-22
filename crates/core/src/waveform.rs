use crate::{invalid, media::{run_stream, MediaTools}, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{create_dir_all, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::AtomicBool,
        Arc,
    },
    time::Duration,
};

pub const SSWF_MAGIC: &[u8; 4] = b"SSWF";
pub const SSWF_FORMAT_VERSION: u32 = 1;
pub const SSWF_ALGORITHM_VERSION: u32 = 1;
pub const DEFAULT_BASE_BUCKET: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChannelBucket {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyramidLevel {
    pub frames_per_bucket: u32,
    pub channels: Vec<Vec<ChannelBucket>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaveformPyramid {
    pub sample_rate: u32,
    pub channels: u16,
    pub total_frames: u64,
    pub base_bucket: u32,
    pub levels: Vec<PyramidLevel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaveformResponse {
    pub channels: Vec<Vec<ChannelBucket>>,
    pub start_frame: u64,
    pub end_frame: u64,
    pub frames_per_point: f64,
    pub total_frames: u64,
    pub sample_rate: u32,
    pub channels_count: u16,
}

impl WaveformPyramid {
    pub fn build(
        reader: &mut dyn Read,
        sample_rate: u32,
        channels: u16,
        base_bucket: u32,
    ) -> Result<Self> {
        if sample_rate == 0 || channels == 0 {
            return Err(invalid("Invalid sample rate or channel count for waveform"));
        }
        let base_bucket = base_bucket.max(16);
        let ch_count = channels as usize;
        let frame_bytes = ch_count * 4;
        let mut bytes = vec![0u8; frame_bytes];

        let mut channel_mins = vec![f32::INFINITY; ch_count];
        let mut channel_maxs = vec![f32::NEG_INFINITY; ch_count];
        let mut channel_sums = vec![0.0f64; ch_count];

        let mut level0_channels: Vec<Vec<ChannelBucket>> = vec![Vec::new(); ch_count];
        let mut total_frames = 0u64;
        let mut bucket_count = 0u32;

        loop {
            let mut have = 0;
            while have < frame_bytes {
                let n = reader.read(&mut bytes[have..])?;
                if n == 0 {
                    break;
                }
                have += n;
            }
            if have == 0 {
                break;
            }
            if have != frame_bytes {
                return Err(invalid("Truncated decoded PCM in waveform builder"));
            }

            for (ch, raw) in bytes.chunks_exact(4).enumerate() {
                let sample = f32::from_le_bytes(raw.try_into().unwrap());
                if !sample.is_finite() {
                    return Err(invalid("Non-finite sample in waveform builder"));
                }
                channel_mins[ch] = channel_mins[ch].min(sample);
                channel_maxs[ch] = channel_maxs[ch].max(sample);
                channel_sums[ch] += (sample as f64).powi(2);
            }

            total_frames += 1;
            bucket_count += 1;

            if bucket_count == base_bucket {
                for ch in 0..ch_count {
                    let rms = (channel_sums[ch] / bucket_count as f64).sqrt() as f32;
                    level0_channels[ch].push(ChannelBucket {
                        min: channel_mins[ch],
                        max: channel_maxs[ch],
                        rms,
                    });
                    channel_mins[ch] = f32::INFINITY;
                    channel_maxs[ch] = f32::NEG_INFINITY;
                    channel_sums[ch] = 0.0;
                }
                bucket_count = 0;
            }
        }

        if total_frames == 0 {
            return Err(invalid("No audio frames to build waveform"));
        }

        if bucket_count > 0 {
            for ch in 0..ch_count {
                let rms = (channel_sums[ch] / bucket_count as f64).sqrt() as f32;
                level0_channels[ch].push(ChannelBucket {
                    min: channel_mins[ch],
                    max: channel_maxs[ch],
                    rms,
                });
            }
        }

        let mut levels = Vec::new();
        levels.push(PyramidLevel {
            frames_per_bucket: base_bucket,
            channels: level0_channels,
        });

        // Downsample by factor of 4 for subsequent levels
        let mut cur_frames_per_bucket = base_bucket;
        while levels.last().unwrap().channels[0].len() > 4 && levels.len() < 6 {
            let prev = levels.last().unwrap();
            cur_frames_per_bucket = cur_frames_per_bucket.saturating_mul(4);
            let mut next_channels: Vec<Vec<ChannelBucket>> = vec![Vec::new(); ch_count];

            for ch in 0..ch_count {
                let prev_buckets = &prev.channels[ch];
                for chunk in prev_buckets.chunks(4) {
                    let min = chunk.iter().map(|b| b.min).fold(f32::INFINITY, f32::min);
                    let max = chunk.iter().map(|b| b.max).fold(f32::NEG_INFINITY, f32::max);
                    let rms = (chunk.iter().map(|b| (b.rms as f64).powi(2)).sum::<f64>()
                        / chunk.len() as f64)
                        .sqrt() as f32;
                    next_channels[ch].push(ChannelBucket { min, max, rms });
                }
            }

            levels.push(PyramidLevel {
                frames_per_bucket: cur_frames_per_bucket,
                channels: next_channels,
            });
        }

        Ok(Self {
            sample_rate,
            channels,
            total_frames,
            base_bucket,
            levels,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        for level in &self.levels {
            payload.extend_from_slice(&level.frames_per_bucket.to_le_bytes());
            let count = level.channels.first().map(|c| c.len()).unwrap_or(0) as u32;
            payload.extend_from_slice(&count.to_le_bytes());
            for ch in &level.channels {
                for b in ch {
                    payload.extend_from_slice(&b.min.to_le_bytes());
                    payload.extend_from_slice(&b.max.to_le_bytes());
                    payload.extend_from_slice(&b.rms.to_le_bytes());
                }
            }
        }

        let hash = blake3::hash(&payload);

        let mut bytes = Vec::with_capacity(64 + payload.len());
        bytes.extend_from_slice(SSWF_MAGIC);
        bytes.extend_from_slice(&SSWF_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&SSWF_ALGORITHM_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.sample_rate.to_le_bytes());
        bytes.extend_from_slice(&self.channels.to_le_bytes());
        bytes.push(self.levels.len() as u8);
        bytes.push(0u8); // reserved
        bytes.extend_from_slice(&self.base_bucket.to_le_bytes());
        bytes.extend_from_slice(&self.total_frames.to_le_bytes());
        bytes.extend_from_slice(hash.as_bytes()); // 32 bytes hash
        bytes.extend_from_slice(&payload);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 64 {
            return Err(invalid("Truncated waveform cache header"));
        }
        if &bytes[0..4] != SSWF_MAGIC {
            return Err(invalid("Invalid waveform magic bytes"));
        }
        let format_version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if format_version != SSWF_FORMAT_VERSION {
            return Err(invalid("Unsupported waveform format version"));
        }
        let _algo_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let sample_rate = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let channels = u16::from_le_bytes(bytes[16..18].try_into().unwrap());
        let levels_count = bytes[18] as usize;
        let base_bucket = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let total_frames = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let expected_hash = &bytes[32..64];

        let payload = &bytes[64..];
        let computed_hash = blake3::hash(payload);
        if computed_hash.as_bytes() != expected_hash {
            return Err(invalid("Waveform cache hash mismatch or corrupt data"));
        }

        let mut offset = 0;
        let mut levels = Vec::with_capacity(levels_count);
        let ch_count = channels as usize;

        for _ in 0..levels_count {
            if offset + 8 > payload.len() {
                return Err(invalid("Truncated level header in waveform cache"));
            }
            let frames_per_bucket =
                u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
            let count =
                u32::from_le_bytes(payload[offset + 4..offset + 8].try_into().unwrap()) as usize;
            offset += 8;

            let bucket_bytes = count * 12;
            let mut level_channels = vec![Vec::with_capacity(count); ch_count];

            for ch in 0..ch_count {
                if offset + bucket_bytes > payload.len() {
                    return Err(invalid("Truncated channel data in waveform cache"));
                }
                let ch_data = &payload[offset..offset + bucket_bytes];
                offset += bucket_bytes;

                for chunk in ch_data.chunks_exact(12) {
                    let min = f32::from_le_bytes(chunk[0..4].try_into().unwrap());
                    let max = f32::from_le_bytes(chunk[4..8].try_into().unwrap());
                    let rms = f32::from_le_bytes(chunk[8..12].try_into().unwrap());
                    level_channels[ch].push(ChannelBucket { min, max, rms });
                }
            }

            levels.push(PyramidLevel {
                frames_per_bucket,
                channels: level_channels,
            });
        }

        Ok(Self {
            sample_rate,
            channels,
            total_frames,
            base_bucket,
            levels,
        })
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            create_dir_all(parent)?;
        }
        let temp_path = path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
        let bytes = self.to_bytes();
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(temp_path, path)?;
        Ok(())
    }

    pub fn read_from_file(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }

    pub fn query_window(
        &self,
        start_frame: Option<u64>,
        end_frame: Option<u64>,
        max_points: Option<usize>,
    ) -> WaveformResponse {
        let max_points = max_points.unwrap_or(1000).clamp(32, 4000);
        let start = start_frame.unwrap_or(0).min(self.total_frames);
        let end = end_frame
            .unwrap_or(self.total_frames)
            .max(start + 1)
            .min(self.total_frames);
        let span = end.saturating_sub(start).max(1);
        let desired_frames_per_point = span as f64 / max_points as f64;

        // Find pyramid level closest to desired resolution
        let mut chosen_level = &self.levels[0];
        for level in &self.levels {
            if (level.frames_per_bucket as f64) <= desired_frames_per_point {
                chosen_level = level;
            } else {
                break;
            }
        }

        let fpb = chosen_level.frames_per_bucket as f64;
        let start_idx = ((start as f64) / fpb).floor() as usize;
        let end_idx = (((end as f64) / fpb).ceil() as usize).max(start_idx + 1);

        let ch_count = self.channels as usize;
        let mut response_channels: Vec<Vec<ChannelBucket>> = vec![Vec::new(); ch_count];

        for ch in 0..ch_count {
            let src = &chosen_level.channels[ch];
            let actual_start = start_idx.min(src.len());
            let actual_end = end_idx.min(src.len());
            let slice = &src[actual_start..actual_end];

            if slice.len() <= max_points {
                response_channels[ch] = slice.to_vec();
            } else {
                // Downsample slice to max_points
                let step = slice.len() as f64 / max_points as f64;
                let mut downsampled = Vec::with_capacity(max_points);
                for i in 0..max_points {
                    let s = ((i as f64) * step).floor() as usize;
                    let e = (((i + 1) as f64) * step).ceil() as usize;
                    let sub = &slice[s.min(slice.len())..e.min(slice.len())];
                    if !sub.is_empty() {
                        let min = sub.iter().map(|b| b.min).fold(f32::INFINITY, f32::min);
                        let max = sub.iter().map(|b| b.max).fold(f32::NEG_INFINITY, f32::max);
                        let rms = (sub.iter().map(|b| (b.rms as f64).powi(2)).sum::<f64>()
                            / sub.len() as f64)
                            .sqrt() as f32;
                        downsampled.push(ChannelBucket { min, max, rms });
                    }
                }
                response_channels[ch] = downsampled;
            }
        }

        let actual_points = response_channels[0].len().max(1);
        let frames_per_point = span as f64 / actual_points as f64;

        WaveformResponse {
            channels: response_channels,
            start_frame: start,
            end_frame: end,
            frames_per_point,
            total_frames: self.total_frames,
            sample_rate: self.sample_rate,
            channels_count: self.channels,
        }
    }
}

pub struct WaveformService {
    cache_dir: PathBuf,
}

impl WaveformService {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    pub fn cache_path(&self, hash: &str) -> PathBuf {
        self.cache_dir.join(format!("{hash}.sswf"))
    }

    pub fn load_or_build(
        &self,
        hash: &str,
        sound_path: &Path,
        sample_rate: u32,
        channels: u16,
        tools: &MediaTools,
        cancel: Arc<AtomicBool>,
    ) -> Result<WaveformPyramid> {
        let path = self.cache_path(hash);
        if path.is_file() {
            if let Ok(pyramid) = WaveformPyramid::read_from_file(&path) {
                if pyramid.sample_rate == sample_rate && pyramid.channels == channels {
                    return Ok(pyramid);
                }
            }
        }

        // Cache missing or corrupt: build from decoded audio
        tools.validate()?;
        let mut decode = Command::new(&tools.ffmpeg);
        decode
            .args(["-v", "error", "-nostdin", "-threads", "1", "-i"])
            .arg(sound_path)
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-f",
                "f32le",
                "-acodec",
                "pcm_f32le",
                "pipe:1",
            ]);

        let pyramid = run_stream(decode, cancel, Duration::from_secs(7200), |r| {
            WaveformPyramid::build(r, sample_rate, channels, DEFAULT_BASE_BUCKET)
        })?;

        let _ = pyramid.save_to_file(&path);
        Ok(pyramid)
    }
}
