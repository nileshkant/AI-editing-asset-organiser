use soundshelf_core::{
    catalog::{Catalog, Profile},
    media::MediaTools,
    playback::{LoopbackSink, PlaybackState, Player, SharedAudioState},
};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::{atomic::Ordering, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;

fn test_tools() -> Option<MediaTools> {
    MediaTools::discover()
}

fn create_tone_wav(tools: &MediaTools, path: &Path, duration_secs: f64) {
    let status = Command::new(&tools.ffmpeg)
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={:.2}", duration_secs),
            "-ar",
            "48000",
            "-ac",
            "2",
        ])
        .arg(path)
        .status()
        .expect("ffmpeg tone generation");
    assert!(status.success());
}

fn create_tone_mp3(tools: &MediaTools, path: &Path, duration_secs: f64) {
    let status = Command::new(&tools.ffmpeg)
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=880:duration={:.2}", duration_secs),
            "-ar",
            "48000",
            "-ac",
            "2",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "128k",
        ])
        .arg(path)
        .status()
        .expect("ffmpeg mp3 generation");
    assert!(status.success());
}

fn wait_for_data(sink: &Arc<LoopbackSink>, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if sink.available_samples() > 0 {
            return true;
        }
        thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn test_audible_wav_and_mp3_playback() {
    let tools = match test_tools() {
        Some(t) => t,
        None => {
            eprintln!("Skipping test_audible_wav_and_mp3_playback: ffmpeg not found");
            return;
        }
    };

    let dir = tempdir().unwrap();
    let wav_file = dir.path().join("audible.wav");
    let mp3_file = dir.path().join("audible.mp3");
    create_tone_wav(&tools, &wav_file, 0.5);
    create_tone_mp3(&tools, &mp3_file, 0.5);

    let player = Player::new_loopback(Some(tools));

    // 1. WAV playback
    player.play("wav-1", &wav_file, 0.5).expect("play wav");
    let sink = player.loopback_sink().expect("loopback sink available");

    assert!(wait_for_data(&sink, Duration::from_secs(2)));
    let samples = sink.step(1024);
    assert!(!samples.is_empty());
    assert!(samples.iter().any(|&s| s.abs() > 0.05), "WAV must produce audible non-zero audio samples");

    let status = player.status();
    assert_eq!(status.state, PlaybackState::Playing);
    assert!(status.peak > 0.05, "Peak meter must register audio signal");

    player.stop();
    assert_eq!(player.status().state, PlaybackState::Stopped);

    // 2. MP3 playback
    player.play("mp3-1", &mp3_file, 0.5).expect("play mp3");
    let sink_mp3 = player.loopback_sink().expect("loopback sink available");

    assert!(wait_for_data(&sink_mp3, Duration::from_secs(2)));
    let samples_mp3 = sink_mp3.step(1024);
    assert!(!samples_mp3.is_empty());
    assert!(samples_mp3.iter().any(|&s| s.abs() > 0.05), "MP3 must produce audible non-zero audio samples");

    let status_mp3 = player.status();
    assert_eq!(status_mp3.state, PlaybackState::Playing);
    assert!(status_mp3.peak > 0.05, "Peak meter must register audio signal for MP3");
}

#[test]
fn test_pause_and_resume() {
    let tools = match test_tools() {
        Some(t) => t,
        None => return,
    };

    let dir = tempdir().unwrap();
    let wav_file = dir.path().join("pause_resume.wav");
    create_tone_wav(&tools, &wav_file, 1.0);

    let player = Player::new_loopback(Some(tools));
    player.play("pr-1", &wav_file, 1.0).unwrap();
    let sink = player.loopback_sink().unwrap();

    assert!(wait_for_data(&sink, Duration::from_secs(2)));
    sink.step(4800); // 0.1s worth of frames

    let before_pause = player.status();
    assert_eq!(before_pause.state, PlaybackState::Playing);
    assert!(before_pause.position_seconds > 0.05);

    // Pause
    player.pause();
    let paused_status = player.status();
    assert_eq!(paused_status.state, PlaybackState::Paused);
    let paused_pos = paused_status.position_seconds;

    // Step frames while paused -> should produce pure silence and NOT advance position
    let silence = sink.step(4800);
    assert!(silence.iter().all(|&s| s == 0.0), "Paused sink must output absolute silence");
    assert_eq!(player.status().position_seconds, paused_pos, "Position must not advance while paused");

    // Resume
    player.resume().unwrap();
    assert_eq!(player.status().state, PlaybackState::Playing);

    let active_samples = sink.step(4800);
    assert!(active_samples.iter().any(|&s| s.abs() > 0.05), "Resumed playback must produce audible samples");
    assert!(player.status().position_seconds > paused_pos, "Position must advance after resume");
}

#[test]
fn test_seek_spam_stability() {
    let tools = match test_tools() {
        Some(t) => t,
        None => return,
    };

    let dir = tempdir().unwrap();
    let wav_file = dir.path().join("seek_spam.wav");
    create_tone_wav(&tools, &wav_file, 4.0);

    let player = Player::new_loopback(Some(tools));
    player.play("seek-1", &wav_file, 4.0).unwrap();

    // Dispatch 20 rapid seeks in succession
    for i in 0..20 {
        let target = (i as f64) * 0.15;
        let _ = player.seek(target);
    }

    // Settle on seek to 2.5s
    player.seek(2.5).unwrap();
    let status = player.status();
    assert_eq!(status.state, PlaybackState::Playing);
    assert!((status.position_seconds - 2.5).abs() < 0.1);

    let sink = player.loopback_sink().unwrap();
    assert!(wait_for_data(&sink, Duration::from_secs(2)));
    let samples = sink.step(2048);
    assert!(samples.iter().any(|&s| s.abs() > 0.05), "Playback must continue audibly after seeking");
}

#[test]
fn test_bounded_memory_long_file() {
    let tools = match test_tools() {
        Some(t) => t,
        None => return,
    };

    let dir = tempdir().unwrap();
    let wav_file = dir.path().join("bounded_mem.wav");
    create_tone_wav(&tools, &wav_file, 5.0);

    let player = Player::new_loopback(Some(tools));
    player.play("bounded-1", &wav_file, 5.0).unwrap();

    let sink = player.loopback_sink().unwrap();
    // Do not read for 100ms to allow buffer to fill up
    thread::sleep(Duration::from_millis(100));

    // The ring buffer capacity is fixed (sample_rate * channels * 2 = 192,000 slots)
    // Verify available samples did not exceed capacity
    assert!(sink.available_samples() <= 192000);

    // Read in chunks
    for _ in 0..5 {
        let chunk = sink.step(4800);
        assert!(!chunk.is_empty());
    }
}

#[test]
fn test_underrun_graceful_silence() {
    let shared = Arc::new(SharedAudioState::new(48000, 2, 1.0));
    let (_, consumer) = rtrb::RingBuffer::new(1024);
    let sink = LoopbackSink {
        shared: shared.clone(),
        consumer: Mutex::new(consumer),
        captured: Mutex::new(Vec::new()),
    };

    shared.is_playing.store(true, Ordering::Relaxed);

    // Step when consumer has 0 samples
    let out = sink.step(512);
    assert_eq!(out.len(), 1024); // 512 frames * 2 channels
    assert!(out.iter().all(|&s| s == 0.0), "Underrun must gracefully output silence");
    assert!(shared.underrun_count.load(Ordering::Relaxed) > 0);
}

#[test]
fn test_natural_end_detection() {
    let tools = match test_tools() {
        Some(t) => t,
        None => return,
    };

    let dir = tempdir().unwrap();
    let wav_file = dir.path().join("short.wav");
    create_tone_wav(&tools, &wav_file, 0.25);

    let player = Player::new_loopback(Some(tools));
    player.play("short-1", &wav_file, 0.25).unwrap();
    let sink = player.loopback_sink().unwrap();

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        sink.step(2048);
        let status = player.status();
        if status.state == PlaybackState::Finished {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    let final_status = player.status();
    assert_eq!(final_status.state, PlaybackState::Finished, "Player must transition to Finished at EOF");
    assert!((final_status.position_seconds - 0.25).abs() < 0.05);
}

#[test]
fn test_device_error_visibility() {
    let shared = Arc::new(SharedAudioState::new(48000, 2, 1.0));
    *shared.error_msg.lock().unwrap() = Some("Audio device disconnected".into());

    let tools = test_tools();
    let player = Player::new_loopback(tools);
    // When last_error is set in player:
    let status = player.status();
    assert_eq!(status.state, PlaybackState::Stopped);

    // Attempting to play non-existent file sets visible error:
    let res = player.play("err-1", Path::new("/nonexistent/file.wav"), 1.0);
    assert!(res.is_err());
    let err_status = player.status();
    assert_eq!(err_status.state, PlaybackState::Error);
    assert!(err_status.error.is_some());
    assert!(err_status.error.unwrap().contains("does not exist"));
}

#[test]
fn test_decode_error_file_safety() {
    let tools = match test_tools() {
        Some(t) => t,
        None => return,
    };

    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_catalog.sqlite");
    let mut catalog = Catalog::open(&db_path).unwrap();

    let audio_dir = dir.path().join("audio");
    fs::create_dir(&audio_dir).unwrap();

    let good_file = audio_dir.join("good.wav");
    create_tone_wav(&tools, &good_file, 0.5);

    let source = catalog.add_source(&audio_dir).unwrap();
    let digest = soundshelf_core::catalog::hash_file(&good_file).unwrap();
    let sound_id = catalog.register(&source, "good.wav", &digest).unwrap();

    let prof = Profile {
        duration: 0.5,
        sample_rate: 48000,
        channels: 2,
        frames: 24000,
        peak: 0.8,
        rms: 0.2,
        channel_peaks: vec![0.8, 0.8],
        channel_rms: vec![0.2, 0.2],
        channel_layout: "stereo".into(),
        description: "Test".into(),
        tags: vec!["tone".into()],
        waveform: vec![[-0.8, 0.8]],
    };
    catalog.publish(&source, &sound_id, &digest, &prof).unwrap();

    // Corrupt junk file pretending to be audio
    let corrupt_file = audio_dir.join("corrupt.wav");
    fs::write(&corrupt_file, b"NOT REAL AUDIO JUNK DATA HEADERLESS").unwrap();

    let player = Player::new_loopback(Some(tools));
    let _ = player.play(&sound_id, &corrupt_file, 1.0);

    // Wait a brief moment for decoder to fail
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        if player.status().state == PlaybackState::Error {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let status = player.status();
    assert_eq!(status.state, PlaybackState::Error, "Corrupt file decode must report Error state");

    // CRITICAL: Verify catalog database record was NEVER deleted or modified
    let sound_in_db = catalog.sound(&sound_id).expect("Sound record MUST still exist in database");
    assert_eq!(sound_in_db.id, sound_id);
    assert_eq!(sound_in_db.status, "ready");
    assert_eq!(sound_in_db.profile, Some(prof));
}

#[test]
fn test_volume_and_metering() {
    let tools = match test_tools() {
        Some(t) => t,
        None => return,
    };

    let dir = tempdir().unwrap();
    let wav_file = dir.path().join("vol_test.wav");
    create_tone_wav(&tools, &wav_file, 1.0);

    let player = Player::new_loopback(Some(tools));

    // Full volume (1.0)
    player.set_volume(1.0);
    player.play("vol-1", &wav_file, 1.0).unwrap();
    let sink = player.loopback_sink().unwrap();
    assert!(wait_for_data(&sink, Duration::from_secs(2)));
    let full_samples = sink.step(2048);
    assert!(!full_samples.is_empty());
    let full_peak = player.status().peak;
    assert!(full_peak > 0.05);

    // Half volume (0.5)
    player.set_volume(0.5);
    assert!((player.volume() - 0.5).abs() < 0.01);
    let half_samples = sink.step(2048);
    let half_max = half_samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(half_max > 0.0);
    assert!(half_max < full_peak * 0.75, "Half volume output must be lower than full volume");

    // Mute (0.0)
    player.set_volume(0.0);
    let muted_samples = sink.step(2048);
    assert!(muted_samples.iter().all(|&s| s == 0.0), "Volume 0.0 must output complete silence");
    assert_eq!(player.status().peak, 0.0, "Muted peak must be 0.0");
}

#[test]
fn test_app_quit_cleanup() {
    let tools = match test_tools() {
        Some(t) => t,
        None => return,
    };

    let dir = tempdir().unwrap();
    let wav_file = dir.path().join("quit_test.wav");
    create_tone_wav(&tools, &wav_file, 2.0);

    let player = Box::new(Player::new_loopback(Some(tools)));
    player.play("quit-1", &wav_file, 2.0).unwrap();

    let sink = player.loopback_sink().unwrap();
    assert!(wait_for_data(&sink, Duration::from_secs(2)));

    // Drop the player: must kill child and join decoder thread immediately
    let start = Instant::now();
    drop(player);
    assert!(start.elapsed() < Duration::from_millis(500), "Player drop must terminate cleanly and promptly");
}
