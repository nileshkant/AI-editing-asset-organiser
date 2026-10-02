#![cfg(feature = "packaged-media-qualification")]
use soundshelf_core::{catalog::{hash_file, Catalog, ClipRecipe}, export::{render, ExportFormat, ExportOptions}, media::{analyze, MediaTools}, playback::Player, waveform::WaveformService};
use std::{fs, path::PathBuf, process::Command, sync::{Arc, atomic::AtomicBool}, thread, time::{Duration, Instant}};
fn flag() -> Arc<AtomicBool> { Arc::new(AtomicBool::new(false)) }
#[test]
fn actual_audio_only_packaged_tools_cover_import_waveform_preview_and_lossless_export() {
    let generator = MediaTools::discover().expect("Explicit full fixture generator required");
    let tools = MediaTools { ffmpeg: PathBuf::from(std::env::var_os("CREATIVESHELF_PACKAGED_FFMPEG").expect("Packaged FFmpeg required")), ffprobe: PathBuf::from(std::env::var_os("CREATIVESHELF_PACKAGED_FFPROBE").expect("Packaged FFprobe required")) };
    tools.validate().unwrap();
    let temp = tempfile::tempdir().unwrap(); let root = temp.path().canonicalize().unwrap();
    let media = root.join("originals"); fs::create_dir(&media).unwrap();
    for (name, codec) in [("tone.wav", "pcm_s24le"), ("tone.mp3", "libmp3lame"), ("tone.flac", "flac"), ("tone.m4a", "aac"), ("tone.ogg", "vorbis"), ("tone.opus", "libopus"), ("tone.aac", "aac"), ("tone.aiff", "pcm_s16be"), ("tone.wma", "wmav2"), ("tone.caf", "pcm_s16le")] {
        let path = media.join(name);
        assert!(Command::new(&generator.ffmpeg).args(["-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=0.4", "-ar", "48000", "-ac", "2", "-c:a", codec, "-strict", "-2"]).arg(&path).status().unwrap().success(), "fixture generator: {name}");
        let before = hash_file(&path).unwrap();
        let profile = analyze(&tools, &path, flag()).unwrap_or_else(|e| panic!("packaged import {name}: {e}"));
        assert_eq!(profile.channels, 2, "{name}"); assert_eq!(profile.sample_rate, 48000, "{name}"); assert!(profile.frames > 10000 && profile.frames < 48000 && profile.peak > 0.01, "{name}");
        let cache = root.join("cache"); fs::create_dir_all(&cache).unwrap();
        let waveform = WaveformService::new(cache).load_or_build(&before, &path, profile.sample_rate, profile.channels, &tools, flag()).unwrap();
        assert_eq!(waveform.total_frames, profile.frames, "{name}");
        assert_eq!(hash_file(&path).unwrap(), before, "source changed: {name}");
    }
    let path = media.join("tone.wav"); let before = hash_file(&path).unwrap();
    let profile = analyze(&tools, &path, flag()).unwrap();
    let mut catalog = Catalog::open(&root.join("fixture.sqlite")).unwrap(); let source = catalog.add_source(&media).unwrap();
    let id = catalog.register(&source, "tone.wav", &before).unwrap(); catalog.publish(&source, &id, &before, &profile).unwrap();
    let recipe = ClipRecipe { asset_id: id.clone(), asset_version_id: before.clone(), source_sample_rate_hz: 48000, start_frame: "2400".into(), end_frame: "12000".into(), channel_policy: "preserve".into(), gain_db: -3.0, fade_in_ms: 10, fade_out_ms: 20 };
    let clip = catalog.create_clip(&id, "Packaged tool qualification", &recipe).unwrap();
    let journal = root.join("journal"); fs::create_dir(&journal).unwrap();
    for (format, extension) in [(ExportFormat::Wav, "wav"), (ExportFormat::Flac, "flac")] {
        let (_, output) = render(&tools, &path, &clip, &profile, &root.join(format!("export.{extension}")), ExportOptions { format, sample_rate: Some(44100), ..Default::default() }, flag(), &journal).unwrap();
        assert_eq!(output.sample_rate, 44100); assert_eq!(output.frames, 8820); assert_eq!(output.channels, 2);
    }
    let player = Player::new_loopback(Some(tools)); player.play("fixture", &path, 0.4).unwrap();
    let sink = player.loopback_sink().unwrap(); let start = Instant::now(); let mut nonzero = false;
    while start.elapsed() < Duration::from_secs(3) { if sink.step(1024).iter().any(|v| v.abs() > 0.01) { nonzero = true; break; } thread::sleep(Duration::from_millis(10)); }
    player.stop(); assert!(nonzero, "Packaged preview must decode PCM into the loopback sink (not physical audio evidence)");
    assert_eq!(hash_file(&path).unwrap(), before);
}
