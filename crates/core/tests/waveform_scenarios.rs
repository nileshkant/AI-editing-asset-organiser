use soundshelf_core::waveform::WaveformPyramid;
use tempfile::tempdir;

fn generate_stereo_pcm(frames: usize, left_fn: impl Fn(usize) -> f32, right_fn: impl Fn(usize) -> f32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(frames * 8);
    for i in 0..frames {
        bytes.extend_from_slice(&left_fn(i).to_le_bytes());
        bytes.extend_from_slice(&right_fn(i).to_le_bytes());
    }
    bytes
}

#[test]
fn impulse_sample_mapping() {
    let frames = 10240;
    let impulse_frame = 2500;
    let data = generate_stereo_pcm(
        frames,
        |i| if i == impulse_frame { 0.95 } else { 0.01 },
        |i| if i == impulse_frame { -0.85 } else { -0.01 },
    );

    let pyramid = WaveformPyramid::build(&mut &data[..], 48000, 2, 256).unwrap();
    assert_eq!(pyramid.sample_rate, 48000);
    assert_eq!(pyramid.channels, 2);
    assert_eq!(pyramid.total_frames, frames as u64);

    // Impulse is at frame 2500. With bucket 256, bucket index is 2500 / 256 = 9
    let target_bucket = impulse_frame / 256;
    let l0_left = &pyramid.levels[0].channels[0];
    let l0_right = &pyramid.levels[0].channels[1];

    assert!((l0_left[target_bucket].max - 0.95).abs() < 1e-5);
    assert!((l0_right[target_bucket].min - (-0.85)).abs() < 1e-5);

    // Other buckets should NOT have the impulse
    for (idx, b) in l0_left.iter().enumerate() {
        if idx != target_bucket {
            assert!(b.max < 0.1);
        }
    }
}

#[test]
fn stereo_separate_antiphase() {
    // Left channel is +0.75 DC, Right channel is -0.75 DC (anti-phase stereo)
    let frames = 5120;
    let data = generate_stereo_pcm(frames, |_| 0.75, |_| -0.75);

    let pyramid = WaveformPyramid::build(&mut &data[..], 44100, 2, 256).unwrap();
    assert_eq!(pyramid.channels, 2);

    let left = &pyramid.levels[0].channels[0];
    let right = &pyramid.levels[0].channels[1];

    // Left channel must show positive amplitude
    for b in left {
        assert!((b.min - 0.75).abs() < 1e-4);
        assert!((b.max - 0.75).abs() < 1e-4);
        assert!((b.rms - 0.75).abs() < 1e-4);
    }

    // Right channel must show negative amplitude
    for b in right {
        assert!((b.min - (-0.75)).abs() < 1e-4);
        assert!((b.max - (-0.75)).abs() < 1e-4);
        assert!((b.rms - 0.75).abs() < 1e-4);
    }
}

#[test]
fn pyramid_downsampling_math() {
    let frames = 4096;
    let data = generate_stereo_pcm(
        frames,
        |i| ((i as f32) / 100.0).sin() * 0.8,
        |i| ((i as f32) / 50.0).cos() * 0.6,
    );

    let pyramid = WaveformPyramid::build(&mut &data[..], 48000, 2, 256).unwrap();
    assert!(pyramid.levels.len() >= 2);

    let l0 = &pyramid.levels[0];
    let l1 = &pyramid.levels[1];

    assert_eq!(l0.frames_per_bucket, 256);
    assert_eq!(l1.frames_per_bucket, 1024);

    // L1 bucket 0 must be the aggregation of L0 buckets 0..4
    for ch in 0..2 {
        let expected_min = l0.channels[ch][0..4].iter().map(|b| b.min).fold(f32::INFINITY, f32::min);
        let expected_max = l0.channels[ch][0..4].iter().map(|b| b.max).fold(f32::NEG_INFINITY, f32::max);
        let expected_rms = (l0.channels[ch][0..4].iter().map(|b| (b.rms as f64).powi(2)).sum::<f64>() / 4.0).sqrt() as f32;

        let actual = l1.channels[ch][0];
        assert!((actual.min - expected_min).abs() < 1e-5);
        assert!((actual.max - expected_max).abs() < 1e-5);
        assert!((actual.rms - expected_rms).abs() < 1e-4);
    }
}

#[test]
fn cache_serialization_and_recovery() {
    let frames = 5000;
    let data = generate_stereo_pcm(frames, |_| 0.4, |_| -0.3);
    let original = WaveformPyramid::build(&mut &data[..], 48000, 2, 256).unwrap();

    let bytes = original.to_bytes();
    assert!(bytes.starts_with(b"SSWF"));
    assert!(bytes.len() > 64);

    let loaded = WaveformPyramid::from_bytes(&bytes).unwrap();
    assert_eq!(original, loaded);

    // Corrupted payload should fail validation
    let mut corrupted = bytes.clone();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0xff;
    assert!(WaveformPyramid::from_bytes(&corrupted).is_err());

    // Truncated header should fail validation
    assert!(WaveformPyramid::from_bytes(&bytes[..30]).is_err());

    // Wrong magic should fail validation
    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert!(WaveformPyramid::from_bytes(&bad_magic).is_err());
}

#[test]
fn disk_cache_file_save_and_read() {
    let dir = tempdir().unwrap();
    let cache_file = dir.path().join("test_sound.sswf");

    let frames = 2048;
    let data = generate_stereo_pcm(frames, |_| 0.5, |_| -0.5);
    let pyramid = WaveformPyramid::build(&mut &data[..], 48000, 2, 256).unwrap();

    pyramid.save_to_file(&cache_file).unwrap();
    assert!(cache_file.exists());

    let reloaded = WaveformPyramid::read_from_file(&cache_file).unwrap();
    assert_eq!(pyramid, reloaded);
}

#[test]
fn bounded_points_query_for_long_file() {
    // 500,000 frames (~10.4 seconds at 48kHz)
    let frames = 100_000;
    let data = generate_stereo_pcm(frames, |i| if i % 1000 == 0 { 0.9 } else { 0.1 }, |_| 0.2);
    let pyramid = WaveformPyramid::build(&mut &data[..], 48000, 2, 256).unwrap();

    // Query full file with target points = 800
    let resp = pyramid.query_window(None, None, Some(800));
    assert_eq!(resp.channels_count, 2);
    assert_eq!(resp.total_frames, frames as u64);
    assert_eq!(resp.start_frame, 0);
    assert_eq!(resp.end_frame, frames as u64);
    assert!(resp.channels[0].len() <= 800);
    assert!(resp.channels[1].len() <= 800);
    assert!(!resp.channels[0].is_empty());

    // Peak at frame % 1000 == 0 must be captured in the downsampled view
    let max_peak = resp.channels[0].iter().map(|b| b.max).fold(0.0f32, f32::max);
    assert!((max_peak - 0.9).abs() < 1e-4);
}

#[test]
fn high_zoom_query_window() {
    let frames = 50_000;
    let data = generate_stereo_pcm(frames, |i| if i == 5000 { 0.99 } else { 0.05 }, |_| 0.1);
    let pyramid = WaveformPyramid::build(&mut &data[..], 48000, 2, 256).unwrap();

    // Zoom into frame 4800..5200 (400 frames window)
    let resp = pyramid.query_window(Some(4800), Some(5200), Some(200));
    assert_eq!(resp.start_frame, 4800);
    assert_eq!(resp.end_frame, 5200);
    assert!(resp.channels[0].len() <= 200);

    // Window contains frame 5000, so peak 0.99 must be present!
    let max_peak = resp.channels[0].iter().map(|b| b.max).fold(0.0f32, f32::max);
    assert!((max_peak - 0.99).abs() < 1e-4);
}

#[test]
fn missing_cache_rebuild() {
    let tools = match soundshelf_core::media::MediaTools::discover() {
        Some(t) if t.validate().is_ok() => t,
        _ => return,
    };
    let dir = tempdir().unwrap();
    let audio_path = dir.path().join("tone.wav");

    // Generate valid WAV tone
    let status = std::process::Command::new(&tools.ffmpeg)
        .args([
            "-y", "-f", "lavfi", "-i", "sine=frequency=1000:duration=1",
            "-ac", "2", "-ar", "48000",
        ])
        .arg(&audio_path)
        .status()
        .unwrap();
    assert!(status.success());

    let cache_dir = dir.path().join("cache");
    let service = soundshelf_core::waveform::WaveformService::new(cache_dir.clone());
    let hash = "test-tone-hash-123";

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // First load: cache does not exist, triggers rebuild
    let p1 = service.load_or_build(hash, &audio_path, 48000, 2, &tools, cancel.clone()).unwrap();
    assert_eq!(p1.sample_rate, 48000);
    assert_eq!(p1.channels, 2);
    assert!(service.cache_path(hash).exists());

    // Delete cache file to simulate missing cache
    std::fs::remove_file(service.cache_path(hash)).unwrap();
    assert!(!service.cache_path(hash).exists());

    // Second load: missing cache rebuilds automatically
    let p2 = service.load_or_build(hash, &audio_path, 48000, 2, &tools, cancel).unwrap();
    assert_eq!(p1, p2);
    assert!(service.cache_path(hash).exists());
}
