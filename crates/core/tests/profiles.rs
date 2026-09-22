use soundshelf_core::{
    catalog::{Catalog, Profile},
    media::{measure_pcm, normalize_layout},
};
use std::path::Path;

fn mock_profile() -> Profile {
    Profile {
        duration: 2.5,
        sample_rate: 48000,
        channels: 2,
        frames: 120000,
        peak: 0.707,
        rms: 0.5,
        channel_peaks: vec![0.707, 0.707],
        channel_rms: vec![0.5, 0.5],
        channel_layout: "stereo".into(),
        description: "A 2.50-second recording with moderate level average energy and a relatively even level. Measured across the full recording; the event itself has not been identified by listening.".into(),
        tags: vec!["moderate level".into(), "short".into(), "stereo".into()],
        waveform: vec![[-0.7, 0.7]],
    }
}

#[test]
fn layout_normalization() {
    assert_eq!(normalize_layout(1, None), "mono");
    assert_eq!(normalize_layout(2, None), "stereo");
    assert_eq!(normalize_layout(3, None), "2.1");
    assert_eq!(normalize_layout(4, None), "quadraphonic");
    assert_eq!(normalize_layout(6, None), "5.1 surround");
    assert_eq!(normalize_layout(8, None), "7.1 surround");
    assert_eq!(normalize_layout(12, None), "multichannel");

    assert_eq!(normalize_layout(1, Some("mono")), "mono");
    assert_eq!(normalize_layout(2, Some("stereo")), "stereo");
    assert_eq!(normalize_layout(6, Some("5.1(side)")), "5.1 surround");
    assert_eq!(normalize_layout(8, Some("7.1(wide)")), "7.1 surround");
    assert_eq!(normalize_layout(4, Some("quad(side)")), "quadraphonic");
    assert_eq!(normalize_layout(2, Some("custom_layout")), "custom layout");
}

#[test]
fn synthetic_calibrated_sine_tone() {
    // A 1 kHz sine tone at peak amplitude 0.8:
    // RMS of a sine wave is peak / sqrt(2) = 0.8 / 1.41421356 = ~0.565685
    let rate = 48000u32;
    let frames = 48000usize; // 1 second
    let amplitude = 0.8f32;
    let mut data: Vec<u8> = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        let t = i as f32 / rate as f32;
        let sample = amplitude * (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
        data.extend_from_slice(&sample.to_le_bytes());
    }

    let profile = measure_pcm(&mut &data[..], rate, 1, 1).unwrap();
    assert_eq!(profile.channels, 1);
    assert_eq!(profile.sample_rate, 48000);
    assert_eq!(profile.frames, 48000);
    assert!((profile.duration - 1.0).abs() < 1e-6);
    assert!((profile.peak - 0.8).abs() < 0.005);
    let expected_rms = amplitude / std::f32::consts::SQRT_2;
    assert!((profile.rms - expected_rms).abs() < 0.01);
    assert_eq!(profile.channel_peaks.len(), 1);
    assert_eq!(profile.channel_rms.len(), 1);
    assert!((profile.channel_peaks[0] - 0.8).abs() < 0.005);
    assert!((profile.channel_rms[0] - expected_rms).abs() < 0.01);
    assert_eq!(profile.channel_layout, "mono");
}

#[test]
fn antiphase_stereo_no_energy_cancellation() {
    // Channel 0 = +0.6, Channel 1 = -0.6
    // An algebraic stereo downmix would result in 0.0 (total cancellation).
    // Our per-channel and extrema measurement must retain full 0.6 peak and 0.6 RMS.
    let frames = 1000usize;
    let mut data: Vec<u8> = Vec::with_capacity(frames * 8);
    for _ in 0..frames {
        data.extend_from_slice(&0.6f32.to_le_bytes());
        data.extend_from_slice(&(-0.6f32).to_le_bytes());
    }

    let profile = measure_pcm(&mut &data[..], 48000, 2, 1).unwrap();
    assert_eq!(profile.channels, 2);
    assert!((profile.peak - 0.6).abs() < 1e-6);
    assert!((profile.rms - 0.6).abs() < 1e-6);
    assert_eq!(profile.channel_peaks, vec![0.6, 0.6]);
    assert_eq!(profile.channel_rms, vec![0.6, 0.6]);
    assert_eq!(profile.channel_layout, "stereo");
    // Waveform captures the extrema [-0.6, 0.6]
    assert_eq!(profile.waveform[0], [-0.6, 0.6]);
}

#[test]
fn asymmetric_stereo_tracks_channels_independently() {
    // Channel 0: active audio (peak 0.9, rms 0.9)
    // Channel 1: complete silence (peak 0.0, rms 0.0)
    let frames = 500usize;
    let mut data: Vec<u8> = Vec::with_capacity(frames * 8);
    for _ in 0..frames {
        data.extend_from_slice(&0.9f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
    }

    let profile = measure_pcm(&mut &data[..], 48000, 2, 1).unwrap();
    assert_eq!(profile.channel_peaks[0], 0.9);
    assert_eq!(profile.channel_peaks[1], 0.0);
    assert_eq!(profile.channel_rms[0], 0.9);
    assert_eq!(profile.channel_rms[1], 0.0);
    assert_eq!(profile.peak, 0.9);
    // Overall RMS over 1000 total samples: sqrt((500 * 0.81 + 0) / 1000) = sqrt(0.405) = ~0.6364
    let expected_overall = (0.81f64 / 2.0).sqrt() as f32;
    assert!((profile.rms - expected_overall).abs() < 1e-4);
}

#[test]
fn impulse_and_pronounced_peaks() {
    // 1 single sample at 1.0, 9999 samples at 0.001
    let frames = 10000usize;
    let mut data: Vec<u8> = Vec::with_capacity(frames * 4);
    data.extend_from_slice(&1.0f32.to_le_bytes());
    for _ in 1..frames {
        data.extend_from_slice(&0.001f32.to_le_bytes());
    }

    let profile = measure_pcm(&mut &data[..], 48000, 1, 10).unwrap();
    assert_eq!(profile.peak, 1.0);
    assert!(profile.rms < 0.02);
    // Peak-to-RMS ratio is very high (> 50), which classifies as pronounced peaks
    let crest_ratio = profile.peak / profile.rms.max(0.00001);
    assert!(crest_ratio > 6.0);
}

#[test]
fn pure_silence_profile_valid() {
    let frames = 48000usize;
    let data = vec![0u8; frames * 4];
    let profile = measure_pcm(&mut &data[..], 48000, 1, 1).unwrap();
    assert_eq!(profile.peak, 0.0);
    assert_eq!(profile.rms, 0.0);
    assert_eq!(profile.channel_peaks, vec![0.0]);
    assert_eq!(profile.channel_rms, vec![0.0]);
}

#[test]
fn profile_validation_rejects_corrupt_data_and_underscores() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("audio");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("tone.wav"), b"mock pcm audio").unwrap();

    let mut catalog = Catalog::open(Path::new(":memory:")).unwrap();
    let source = catalog.add_source(&root).unwrap();
    let hash = soundshelf_core::catalog::hash_file(&root.join("tone.wav")).unwrap();
    let id = catalog.register(&source, "tone.wav", &hash).unwrap();

    // Baseline valid profile publishes cleanly
    let valid = mock_profile();
    assert!(catalog.publish(&source, &id, &hash, &valid).is_ok());

    // 1. Tags with underscores must be rejected
    let mut bad_tags = valid.clone();
    bad_tags.tags.push("medium_duration".into());
    assert!(catalog.publish(&source, &id, &hash, &bad_tags).is_err());

    // 2. Mismatched channel peaks count
    let mut bad_peaks = valid.clone();
    bad_peaks.channel_peaks = vec![0.5]; // only 1 peak for 2 channels
    assert!(catalog.publish(&source, &id, &hash, &bad_peaks).is_err());

    // 3. Mismatched channel rms count
    let mut bad_rms = valid.clone();
    bad_rms.channel_rms = vec![0.5, 0.5, 0.5]; // 3 rms for 2 channels
    assert!(catalog.publish(&source, &id, &hash, &bad_rms).is_err());

    // 4. Negative or non-finite peak
    let mut neg_peak = valid.clone();
    neg_peak.channel_peaks[0] = -0.1;
    assert!(catalog.publish(&source, &id, &hash, &neg_peak).is_err());
    let mut nan_peak = valid.clone();
    nan_peak.channel_peaks[0] = f32::NAN;
    assert!(catalog.publish(&source, &id, &hash, &nan_peak).is_err());

    // 5. Empty description
    let mut empty_desc = valid.clone();
    empty_desc.description = "   ".into();
    assert!(catalog.publish(&source, &id, &hash, &empty_desc).is_err());
}

#[test]
fn descriptions_state_provenance_and_no_fabricated_identity() {
    let p = mock_profile();
    assert!(p.description.contains("Measured across the full recording"));
    assert!(p.description.contains("the event itself has not been identified by listening"));
    // Must not fabricate semantic events like "dog", "speech", "explosion"
    assert!(!p.description.contains("dog"));
    assert!(!p.description.contains("explosion"));
    assert!(!p.description.contains("whoosh"));
}
