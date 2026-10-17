use image::ImageEncoder;
use soundshelf_core::{
    catalog::Catalog,
    image_contracts::{Availability, TechnicalState},
    image_import::ImageImportEngine,
    image_import_preview::{MediaImportPreviews, NativeSelectionKind},
    image_jobs::{ImportError, ImportItemState, ImportMode, ImportState},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Instant,
};
static SERIAL: Mutex<()> = Mutex::new(());
struct Fixture {
    _temp: tempfile::TempDir,
    db: Arc<Mutex<Catalog>>,
    root: PathBuf,
    p: MediaImportPreviews,
    engine: ImageImportEngine,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("media");
        fs::create_dir(&root).unwrap();
        let db = Arc::new(Mutex::new(
            Catalog::open(&temp.path().join("db.sqlite")).unwrap(),
        ));
        let engine = ImageImportEngine {
            catalog: db.clone(),
            worker: PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker")),
            tools: None,
        };
        Self {
            _temp: temp,
            root,
            db,
            p: MediaImportPreviews::default(),
            engine,
        }
    }
    fn select(&self, paths: &[PathBuf], kind: NativeSelectionKind, mode: ImportMode) -> String {
        let raw = self.p.approve_native(paths, kind).unwrap();
        let preview = self
            .p
            .prepare(&self.db, &uuid::Uuid::new_v4().to_string(), raw, mode)
            .unwrap();
        self.p.confirm(&self.db, &preview.id).unwrap()[0].id.clone()
    }
    fn finish(&self, id: &str) {
        for _ in 0..30 {
            if self
                .db
                .lock()
                .unwrap()
                .image_import_job(id)
                .unwrap()
                .terminal()
            {
                return;
            }
            self.engine.tick("test", &AtomicBool::new(false)).unwrap();
        }
        panic!("did not finish")
    }
}
fn picture(path: &Path, color: u8) {
    let mut b = vec![];
    image::codecs::png::PngEncoder::new(&mut b)
        .write_image(
            &[color, 64, 128, 128],
            1,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    fs::write(path, b).unwrap();
}
#[test]
fn actual_worker_exact_unicode_selection_does_not_import_siblings_or_write_originals() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let file = f.root.join("图像 %.png");
    picture(&file, 32);
    picture(&f.root.join("sibling.png"), 99);
    let original = fs::read(&file).unwrap();
    let id = f.select(
        &[file.clone()],
        NativeSelectionKind::Files,
        ImportMode::Images,
    );
    f.finish(&id);
    let c = f.db.lock().unwrap();
    let j = c.image_import_job(&id).unwrap();
    assert_eq!((j.state, j.ready, j.total), (ImportState::Complete, 1, 1));
    let items = c.image_import_items(&id, None, 50).unwrap();
    assert_eq!(items.items[0].state, ImportItemState::Ready);
    let image = c
        .image(items.items[0].asset_id.as_ref().unwrap(), None)
        .unwrap();
    assert_eq!(image.profile.unwrap().width, 1);
    assert_eq!(image.header.technical_state, TechnicalState::Ready);
    assert!(c.all_sounds().unwrap().is_empty());
    assert!(c.active_jobs().unwrap().is_empty());
    assert_eq!(original, fs::read(file).unwrap());
    assert!(!f.root.join(".creativeshelf").exists());
}
#[test]
fn corrupt_item_is_independent_and_reimport_reuses_facts_preserves_annotations_and_original_hashes()
{
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    picture(&f.root.join("a.png"), 32);
    fs::copy(f.root.join("a.png"), f.root.join("b.jpeg")).unwrap();
    fs::write(f.root.join("broken.png"), b"\x89PNG\r\n\x1a\ntruncated").unwrap();
    fs::write(f.root.join("zero.png"), []).unwrap();
    let id = f.select(
        &[f.root.clone()],
        NativeSelectionKind::Folders,
        ImportMode::Images,
    );
    f.finish(&id);
    let mut c = f.db.lock().unwrap();
    let j = c.image_import_job(&id).unwrap();
    assert_eq!(
        (j.state, j.ready, j.failed, j.reused),
        (ImportState::Failed, 2, 2, 1)
    );
    let items = c.image_import_items(&id, None, 50).unwrap().items;
    let a = items
        .iter()
        .find(|i| i.relative_path == "a.png")
        .unwrap()
        .asset_id
        .as_ref()
        .unwrap();
    let b = items.iter().find(|i| i.relative_path == "b.jpeg").unwrap();
    assert!(b
        .warnings
        .contains(&soundshelf_core::image_analysis::AnalysisWarning::ExtensionMismatch));
    let image = c.image(a, None).unwrap();
    let mut notes = image.annotations.clone();
    notes.comment = "my creative notes".into();
    notes.tags = vec!["creator".into()];
    c.set_image_annotations(a, image.header.content_hash.as_deref(), &notes)
        .unwrap();
    let a = a.clone();
    drop(c);
    let retry = f.db.lock().unwrap().retry_image_import(&id, 0).unwrap();
    f.finish(&retry.id);
    let c = f.db.lock().unwrap();
    assert_eq!(c.image_import_job(&retry.id).unwrap().reused, 2);
    let current = c.image(&a, None).unwrap();
    assert_eq!(current.annotations.comment, "my creative notes");
    assert_eq!(current.annotations.tags, vec!["creator"]);
    assert_eq!(current.header.content_hash, image.header.content_hash);
}
#[test]
fn replacement_same_size_and_timestamp_rehashes_preserving_identity() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let path = f.root.join("image.png");
    picture(&path, 30);
    let id = f.select(
        &[path.clone()],
        NativeSelectionKind::Files,
        ImportMode::Images,
    );
    f.finish(&id);
    let mut c = f.db.lock().unwrap();
    let item = c.image_import_items(&id, None, 50).unwrap().items.remove(0);
    let old = c.image(item.asset_id.as_ref().unwrap(), None).unwrap();
    let mut notes = old.annotations.clone();
    notes.comment = "keep".into();
    c.set_image_annotations(&old.header.id, old.header.content_hash.as_deref(), &notes)
        .unwrap();
    drop(c);
    let len = fs::metadata(&path).unwrap().len();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    picture(&path, 31);
    assert_eq!(fs::metadata(&path).unwrap().len(), len);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let retry = f.db.lock().unwrap().retry_image_import(&id, 0).unwrap();
    f.finish(&retry.id);
    let c = f.db.lock().unwrap();
    let current = c.image(&old.header.id, None).unwrap();
    assert_ne!(current.header.content_hash, old.header.content_hash);
    assert_eq!(current.annotations.comment, "keep");
    assert_eq!(c.image_import_job(&retry.id).unwrap().reused, 0);
}
#[test]
fn cancelled_discovery_does_not_publish_and_shutdown_checkpoints_resumable_work() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    picture(&f.root.join("a.png"), 30);
    let id = f.select(
        &[f.root.clone()],
        NativeSelectionKind::Folders,
        ImportMode::Images,
    );
    f.db.lock().unwrap().cancel_image_import(&id, 0).unwrap();
    assert!(f
        .engine
        .tick("test", &AtomicBool::new(false))
        .unwrap()
        .is_none());
    let retry = f.db.lock().unwrap().retry_image_import(&id, 0).unwrap();
    f.engine.tick("test", &AtomicBool::new(true)).unwrap();
    assert_eq!(
        f.db.lock()
            .unwrap()
            .image_import_job(&retry.id)
            .unwrap()
            .state,
        ImportState::Queued
    );
    f.finish(&retry.id);
    assert_eq!(
        f.db.lock()
            .unwrap()
            .image_import_job(&retry.id)
            .unwrap()
            .ready,
        1
    );
}
#[test]
fn root_replacement_and_offline_discovery_do_not_mass_mark_ready_images_missing() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    picture(&f.root.join("a.png"), 30);
    let id = f.select(
        &[f.root.clone()],
        NativeSelectionKind::Folders,
        ImportMode::Images,
    );
    f.finish(&id);
    let image_id =
        f.db.lock()
            .unwrap()
            .image_import_items(&id, None, 50)
            .unwrap()
            .items[0]
            .asset_id
            .clone()
            .unwrap();
    let retry = f.db.lock().unwrap().retry_image_import(&id, 0).unwrap();
    fs::rename(&f.root, f.root.with_file_name("offline")).unwrap();
    f.finish(&retry.id);
    {
        let c = f.db.lock().unwrap();
        assert_eq!(
            c.image_import_job(&retry.id).unwrap().error_code,
            Some(ImportError::SourceUnavailable)
        );
        assert_eq!(
            c.image(&image_id, None).unwrap().header.availability,
            Availability::Offline
        );
        assert_eq!(
            c.image(&image_id, None).unwrap().header.technical_state,
            TechnicalState::Ready
        );
    }
    fs::create_dir(&f.root).unwrap();
    picture(&f.root.join("new.png"), 31);
    let raw =
        f.p.approve_native(&[f.root.clone()], NativeSelectionKind::Folders)
            .unwrap();
    let preview =
        f.p.prepare(
            &f.db,
            &uuid::Uuid::new_v4().to_string(),
            raw,
            ImportMode::Images,
        )
        .unwrap();
    assert!(matches!(
        f.p.confirm(&f.db, &preview.id),
        Err(ImportError::SourceChanged)
    ));
}
#[test]
fn missing_reconciliation_is_scoped_and_partial_symlink_walk_preserves_old_assets() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let chosen = f.root.join("chosen");
    fs::create_dir(&chosen).unwrap();
    picture(&chosen.join("a.png"), 30);
    picture(&f.root.join("sibling.png"), 31);
    let id = f.select(
        &[f.root.clone()],
        NativeSelectionKind::Folders,
        ImportMode::Images,
    );
    f.finish(&id);
    let items =
        f.db.lock()
            .unwrap()
            .image_import_items(&id, None, 50)
            .unwrap()
            .items;
    let a = items
        .iter()
        .find(|i| i.relative_path == "chosen/a.png")
        .unwrap()
        .asset_id
        .clone()
        .unwrap();
    let sibling = items
        .iter()
        .find(|i| i.relative_path == "sibling.png")
        .unwrap()
        .asset_id
        .clone()
        .unwrap();
    fs::remove_file(chosen.join("a.png")).unwrap();
    fs::remove_file(f.root.join("sibling.png")).unwrap();
    picture(&chosen.join("new.png"), 32);
    let scoped = f.select(
        &[chosen.clone()],
        NativeSelectionKind::Folders,
        ImportMode::Images,
    );
    f.finish(&scoped);
    let c = f.db.lock().unwrap();
    assert_eq!(
        c.image(&a, None).unwrap().header.availability,
        Availability::Missing,
        "job={:?} images={:?}",
        c.image_import_job(&scoped),
        c.image_import_items(&scoped, None, 50)
    );
    assert_eq!(
        c.image(&sibling, None).unwrap().header.availability,
        Availability::Available
    );
    drop(c);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("not-present", chosen.join("skip-link")).unwrap();
        fs::remove_file(chosen.join("new.png")).unwrap();
        picture(&chosen.join("other.png"), 33);
        let next = f.select(&[chosen], NativeSelectionKind::Folders, ImportMode::Images);
        f.finish(&next);
        let c = f.db.lock().unwrap();
        let j = c.image_import_job(&next).unwrap();
        assert!(!j.discovery_complete);
        assert_eq!(j.state, ImportState::Failed);
        assert_eq!(j.ready, 1);
    }
}
#[test]
#[ignore = "requires real native FFmpeg and FFprobe"]
fn mixed_import_runs_real_shared_audio_and_image_decoders_without_changing_legacy_jobs() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = Fixture::new();
    let tools = soundshelf_core::media::MediaTools::discover().unwrap();
    assert!(std::process::Command::new(&tools.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.1"
        ])
        .arg(f.root.join("voice.wav"))
        .status()
        .unwrap()
        .success());
    f.engine.tools = Some(tools);
    picture(&f.root.join("image.png"), 32);
    let start = Instant::now();
    let id = f.select(
        &[f.root.clone()],
        NativeSelectionKind::Folders,
        ImportMode::Mixed,
    );
    f.finish(&id);
    let c = f.db.lock().unwrap();
    let j = c.image_import_job(&id).unwrap();
    assert_eq!((j.state, j.ready), (ImportState::Complete, 2));
    assert_eq!(c.all_sounds().unwrap().len(), 1);
    assert!(c.all_sounds().unwrap()[0].profile.is_some());
    assert!(c.active_jobs().unwrap().is_empty());
    eprintln!("mixed import completed in {:?}", start.elapsed());
}

#[test]
fn source_quantums_rotate_and_generation_changes_terminate_without_stale_publication() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    for i in 0..3 {
        picture(&f.root.join(format!("{i}.png")), i);
    }
    let first = f.select(
        &[f.root.clone()],
        NativeSelectionKind::Folders,
        ImportMode::Images,
    );
    let other = f._temp.path().join("other");
    fs::create_dir(&other).unwrap();
    picture(&other.join("one.png"), 20);
    let second = f.select(&[other], NativeSelectionKind::Folders, ImportMode::Images);
    let flag = AtomicBool::new(false);
    assert_eq!(f.engine.tick("fair", &flag).unwrap().unwrap().id, first);
    assert_eq!(f.engine.tick("fair", &flag).unwrap().unwrap().id, second);
    assert_eq!(f.engine.tick("fair", &flag).unwrap().unwrap().id, first);
    assert_eq!(f.engine.tick("fair", &flag).unwrap().unwrap().id, second);
    {
        let c = f.db.lock().unwrap();
        assert_eq!(c.image_import_job(&first).unwrap().ready, 1);
        assert_eq!(c.image_import_job(&second).unwrap().ready, 1);
        let source = c.image_import_job(&first).unwrap().source_id;
        c.db_connection()
            .execute(
                "UPDATE sources SET generation=generation+1 WHERE id=?1",
                [source],
            )
            .unwrap();
    }
    f.engine.tick("fair", &flag).unwrap();
    f.engine.tick("fair", &flag).unwrap();
    let c = f.db.lock().unwrap();
    let stale = c.image_import_job(&first).unwrap();
    assert_eq!(stale.state, ImportState::Failed);
    assert_eq!(stale.error_code, Some(ImportError::SourceChanged));
    assert_eq!(stale.ready, 1);
}
#[test]
fn cancel_during_actual_decode_fences_asset_cache_and_progress_in_one_transaction() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let file = f.root.join("large.png");
    let pixels = vec![91u8; 4096 * 4096 * 4];
    let mut bytes = vec![];
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&pixels, 4096, 4096, image::ExtendedColorType::Rgba8)
        .unwrap();
    fs::write(&file, &bytes).unwrap();
    drop(pixels);
    let hash = blake3::hash(&bytes).to_hex().to_string();
    drop(bytes);
    let id = f.select(&[file], NativeSelectionKind::Files, ImportMode::Images);
    f.engine.tick("cancel", &AtomicBool::new(false)).unwrap();
    std::thread::scope(|scope| {
        let running = scope.spawn(|| f.engine.tick("cancel", &AtomicBool::new(false)));
        let start = Instant::now();
        while !soundshelf_core::image_analysis::worker_busy() {
            assert!(start.elapsed() < std::time::Duration::from_secs(15));
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        f.db.lock().unwrap().cancel_image_import(&id, 0).unwrap();
        running.join().unwrap().unwrap();
    });
    let c = f.db.lock().unwrap();
    let job = c.image_import_job(&id).unwrap();
    assert_eq!(job.state, ImportState::Cancelled);
    assert_eq!(job.ready, 0);
    assert!(c
        .cached_image_profile(&hash, soundshelf_core::image_analysis::ANALYZER)
        .unwrap()
        .is_none());
    let images = c.list_images(&[job.source_id], None, 50).unwrap();
    assert_eq!(images.items.len(), 1);
    assert_eq!(
        images.items[0].header.technical_state,
        TechnicalState::Pending
    );
}
#[test]
#[ignore = "requires real native FFmpeg and FFprobe; loopback, not physical output qualification"]
fn playback_and_audio_analysis_remain_responsive_during_large_image_decode() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let tools = soundshelf_core::media::MediaTools::discover().unwrap();
    let audio = f._temp.path().join("audio.wav");
    assert!(std::process::Command::new(&tools.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.5"
        ])
        .arg(&audio)
        .status()
        .unwrap()
        .success());
    let player = soundshelf_core::playback::Player::new_loopback(Some(tools.clone()));
    player.play("fixture", &audio, 0.5).unwrap();
    let file = f.root.join("large.png");
    let pixels = vec![91u8; 2048 * 2048 * 4];
    let mut bytes = vec![];
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&pixels, 2048, 2048, image::ExtendedColorType::Rgba8)
        .unwrap();
    fs::write(&file, bytes).unwrap();
    drop(pixels);
    let id = f.select(&[file], NativeSelectionKind::Files, ImportMode::Images);
    f.engine.tick("perf", &AtomicBool::new(false)).unwrap();
    let mut worst = std::time::Duration::ZERO;
    let start = Instant::now();
    std::thread::scope(|scope| {
        let image = scope.spawn(|| f.engine.tick("perf", &AtomicBool::new(false)));
        let audio_job = scope.spawn(|| {
            soundshelf_core::media::analyze(&tools, &audio, Arc::new(AtomicBool::new(false)))
        });
        for _ in 0..20 {
            let before = Instant::now();
            assert!(f.db.lock().unwrap().active_jobs().unwrap().is_empty());
            let _ = player.status();
            worst = worst.max(before.elapsed());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        image.join().unwrap().unwrap();
        assert!(audio_job.join().unwrap().is_ok());
    });
    let sink = player.loopback_sink().unwrap();
    let output = sink.step(4800);
    assert!(output.iter().any(|v| v.abs() > 0.0001));
    player.stop();
    f.finish(&id);
    eprintln!("Concurrent image + shared audio analysis + loopback playback: {:?}; worst catalog/status call {:?}",start.elapsed(),worst);
    assert!(
        worst < std::time::Duration::from_secs(2),
        "catalog/status stalled: {worst:?}"
    );
}
