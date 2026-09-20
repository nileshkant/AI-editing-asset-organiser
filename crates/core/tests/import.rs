use soundshelf_core::{catalog::Catalog,library::scan,media::{MediaTools,run_stream}};
use std::{fs,path::PathBuf,process::Command,sync::{Arc,Mutex,atomic::AtomicBool},time::{Duration,Instant}};
use tempfile::tempdir;
fn tools()->MediaTools{MediaTools{ffmpeg:PathBuf::from(std::env::var("SOUNDSHELF_FFMPEG").expect("set SOUNDSHELF_FFMPEG")),ffprobe:PathBuf::from(std::env::var("SOUNDSHELF_FFPROBE").expect("set SOUNDSHELF_FFPROBE"))}}
#[test]#[ignore="requires explicit FFmpeg fixture tools"]
fn full_scan_reuses_analysis_after_move_and_hides_deleted_file(){
    let tools=tools();let base=tempdir().unwrap();let root=base.path().join("audio");fs::create_dir(&root).unwrap();
    let status=Command::new(&tools.ffmpeg).args(["-v","error","-f","lavfi","-i","sine=frequency=440:duration=0.25","-ar","48000"]).arg(root.join("tone.wav")).status().unwrap();assert!(status.success());
    let catalog=Arc::new(Mutex::new(Catalog::open(&base.path().join("db.sqlite")).unwrap()));let source=catalog.lock().unwrap().add_source(&root).unwrap();
    let first=scan(catalog.clone(),source.clone(),&tools,Arc::new(AtomicBool::new(false)),|_|{}).unwrap();assert_eq!(first.completed,1);assert_eq!(first.reused,0);assert_eq!(first.failed,0);
    let before=catalog.lock().unwrap().all_sounds().unwrap().remove(0);assert_eq!(before.profile.as_ref().unwrap().frames,12000);
    let moved=base.path().join("moved");fs::rename(root,&moved).unwrap();let source=catalog.lock().unwrap().relink(&source.id,&moved).unwrap();
    let second=scan(catalog.clone(),source.clone(),&tools,Arc::new(AtomicBool::new(false)),|_|{}).unwrap();assert_eq!(second.reused,1);assert_eq!(catalog.lock().unwrap().sound(&before.id).unwrap().profile,before.profile);
    fs::remove_file(moved.join("tone.wav")).unwrap();scan(catalog.clone(),source,&tools,Arc::new(AtomicBool::new(false)),|_|{}).unwrap();assert_eq!(catalog.lock().unwrap().sound(&before.id).unwrap().status,"missing");
}
#[test]#[ignore="requires explicit FFmpeg fixture tools"]
fn stalled_media_process_is_killed_and_reaped(){let tools=tools();let mut command=Command::new(tools.ffmpeg);command.args(["-v","error","-re","-f","lavfi","-i","sine=duration=10","-f","f32le","pipe:1"]);let start=Instant::now();let result=run_stream(command,Arc::new(AtomicBool::new(false)),Duration::from_millis(200),|r|{std::io::copy(r,&mut std::io::sink())?;Ok(())});assert!(result.is_err());assert!(start.elapsed()<Duration::from_secs(3));}
