use soundshelf_core::{catalog::Catalog,library::scan,media::{MediaTools,run_stream}};
use std::{fs,path::PathBuf,process::Command,sync::{Arc,Mutex,atomic::AtomicBool},time::{Duration,Instant}};
use tempfile::tempdir;
fn tools()->MediaTools{MediaTools{ffmpeg:PathBuf::from(std::env::var("SOUNDSHELF_FFMPEG").expect("set SOUNDSHELF_FFMPEG")),ffprobe:PathBuf::from(std::env::var("SOUNDSHELF_FFPROBE").expect("set SOUNDSHELF_FFPROBE"))}}
#[test]#[ignore="requires explicit FFmpeg fixture tools"]
fn full_scan_reuses_analysis_after_move_and_hides_deleted_file(){
    let tools=tools();let base=tempdir().unwrap();let root=base.path().join("audio");fs::create_dir(&root).unwrap();
    let status=Command::new(&tools.ffmpeg).args(["-v","error","-f","lavfi","-i","sine=frequency=440:duration=0.25","-ar","48000"]).arg(root.join("tone.wav")).status().unwrap();assert!(status.success());
    let catalog=Arc::new(Mutex::new(Catalog::open(&base.path().join("db.sqlite")).unwrap()));let source=catalog.lock().unwrap().add_source(&root).unwrap();
    let first=scan(catalog.clone(),source.clone(),&tools,Arc::new(AtomicBool::new(false)),"job-1".into(),|_|{}).unwrap();assert_eq!(first.completed,1);assert_eq!(first.reused,0);assert_eq!(first.failed,0);
    let before=catalog.lock().unwrap().all_sounds().unwrap().remove(0);assert_eq!(before.profile.as_ref().unwrap().frames,12000);
    let moved=base.path().join("moved");fs::rename(root,&moved).unwrap();let source=catalog.lock().unwrap().relink(&source.id,&moved).unwrap();
    let second=scan(catalog.clone(),source.clone(),&tools,Arc::new(AtomicBool::new(false)),"job-2".into(),|_|{}).unwrap();assert_eq!(second.reused,1);assert_eq!(catalog.lock().unwrap().sound(&before.id).unwrap().profile,before.profile);
    fs::remove_file(moved.join("tone.wav")).unwrap();scan(catalog.clone(),source,&tools,Arc::new(AtomicBool::new(false)),"job-3".into(),|_|{}).unwrap();assert_eq!(catalog.lock().unwrap().sound(&before.id).unwrap().status,"missing");
}
#[test]#[ignore="requires explicit FFmpeg fixture tools"]
fn stalled_media_process_is_killed_and_reaped(){let tools=tools();let mut command=Command::new(tools.ffmpeg);command.args(["-v","error","-re","-f","lavfi","-i","sine=duration=10","-f","f32le","pipe:1"]);let start=Instant::now();let result=run_stream(command,Arc::new(AtomicBool::new(false)),Duration::from_millis(200),|r|{std::io::copy(r,&mut std::io::sink())?;Ok(())});assert!(result.is_err());assert!(start.elapsed()<Duration::from_secs(3));}

#[test]#[ignore="requires explicit FFmpeg fixture tools"]
fn real_ffmpeg_measured_profiles_and_layouts() {
    let tools = tools();
    let base = tempdir().unwrap();

    // 1. Stereo file
    let stereo_file = base.path().join("stereo.wav");
    let status = Command::new(&tools.ffmpeg)
        .args(["-v","error","-f","lavfi","-i","sine=frequency=440:duration=0.5","-ac","2","-ar","48000"])
        .arg(&stereo_file)
        .status().unwrap();
    assert!(status.success());
    let p_stereo = soundshelf_core::media::analyze(&tools, &stereo_file, Arc::new(AtomicBool::new(false))).unwrap();
    assert_eq!(p_stereo.channels, 2);
    assert_eq!(p_stereo.sample_rate, 48000);
    assert_eq!(p_stereo.frames, 24000);
    assert_eq!(p_stereo.channel_layout, "stereo");
    assert_eq!(p_stereo.channel_peaks.len(), 2);
    assert_eq!(p_stereo.channel_rms.len(), 2);
    assert!(p_stereo.tags.contains(&"stereo".to_string()));
    assert!(!p_stereo.tags.iter().any(|t| t.contains('_')));
    assert!(p_stereo.description.contains("Measured across the full recording"));

    // 2. 5.1 Surround file
    let surround_file = base.path().join("surround.wav");
    let status = Command::new(&tools.ffmpeg)
        .args(["-v","error","-f","lavfi","-i","sine=frequency=220:duration=0.25","-ac","6","-ar","48000"])
        .arg(&surround_file)
        .status().unwrap();
    assert!(status.success());
    let p_surround = soundshelf_core::media::analyze(&tools, &surround_file, Arc::new(AtomicBool::new(false))).unwrap();
    assert_eq!(p_surround.channels, 6);
    assert_eq!(p_surround.channel_layout, "5.1 surround");
    assert_eq!(p_surround.channel_peaks.len(), 6);
    assert_eq!(p_surround.channel_rms.len(), 6);
    assert!(p_surround.tags.contains(&"5.1 surround".to_string()));
    assert!(!p_surround.tags.iter().any(|t| t.contains('_')));

    // 3. Pure silence file
    let silence_file = base.path().join("silence.wav");
    let status = Command::new(&tools.ffmpeg)
        .args(["-v","error","-f","lavfi","-i","anullsrc=r=48000:cl=mono","-t","0.2"])
        .arg(&silence_file)
        .status().unwrap();
    assert!(status.success());
    let p_silence = soundshelf_core::media::analyze(&tools, &silence_file, Arc::new(AtomicBool::new(false))).unwrap();
    assert_eq!(p_silence.channels, 1);
    assert_eq!(p_silence.channel_layout, "mono");
    assert_eq!(p_silence.peak, 0.0);
    assert_eq!(p_silence.rms, 0.0);
    assert!(p_silence.tags.contains(&"silence".to_string()));
    assert!(p_silence.tags.contains(&"quiet".to_string()));
    assert!(p_silence.description.contains("silent recording"));
    assert!(!p_silence.tags.iter().any(|t| t.contains('_')));
}

