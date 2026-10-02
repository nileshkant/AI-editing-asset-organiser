use soundshelf_core::{
    catalog::{Catalog, Profile},
    library::{import_scan, scan},
    media::MediaTools,
    source_catalog::{self},
};
use std::{
    fs,
    path::Path,
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use tempfile::tempdir;
fn profile() -> Profile {
    Profile {
        duration: 1.0,
        sample_rate: 100,
        channels: 1,
        frames: 100,
        peak: 0.5,
        rms: 0.2,
        channel_peaks: vec![0.5],
        channel_rms: vec![0.2],
        channel_layout: "mono".into(),
        description: "Measured fixture".into(),
        tags: vec!["mono".into()],
        waveform: vec![[0.0, 0.5]],
    }
}
fn memory() -> Arc<Mutex<Catalog>> {
    Arc::new(Mutex::new(Catalog::open(Path::new(":memory:")).unwrap()))
}
fn seed(
    root: &Path,
) -> (
    Arc<Mutex<Catalog>>,
    soundshelf_core::catalog::Source,
    String,
) {
    let db = memory();
    fs::write(
        root.join("tone.wav"),
        b"metadata fixture, not a decoder fixture",
    )
    .unwrap();
    let mut c = db.lock().unwrap();
    let source = c.add_source(root).unwrap();
    let hash = soundshelf_core::catalog::hash_file(&root.join("tone.wav")).unwrap();
    let id = c.register(&source, "tone.wav", &hash).unwrap();
    c.publish(&source, &id, &hash, &profile()).unwrap();
    c.annotate(&id, &["rain hint".into()], "Keep annotations", true)
        .unwrap();
    drop(c);
    source_catalog::save(&db, &source).unwrap();
    (db, source, id)
}
#[test]
fn portable_relocation_same_library_edits_and_missing_pruning_without_decoders() {
    let root = tempdir().unwrap();
    let (db, source, id) = seed(root.path());
    let snapshot = source_catalog::read(root.path()).unwrap().unwrap();
    assert_eq!(snapshot.catalog.sounds[0].id, id);
    assert!(snapshot.catalog.saved_searches.is_empty());
    db.lock()
        .unwrap()
        .annotate(&id, &["edited".into()], "local wins", false)
        .unwrap();
    let same = source_catalog::import_folder(&db, root.path()).unwrap();
    assert_eq!(same.id, source.id);
    assert_eq!(db.lock().unwrap().sound(&id).unwrap().comment, "local wins");
    source_catalog::save(&db, &source).unwrap();
    let moved = tempdir().unwrap();
    fs::rename(root.path().join("tone.wav"), moved.path().join("tone.wav")).unwrap();
    fs::rename(
        root.path().join(".creativeshelf"),
        moved.path().join(".creativeshelf"),
    )
    .unwrap();
    let target = memory();
    let imported = source_catalog::import_folder(&target, moved.path()).unwrap();
    assert_eq!(imported.id, source.id);
    assert_eq!(
        target.lock().unwrap().sound(&id).unwrap().comment,
        "local wins"
    );
    let bogus = MediaTools {
        ffmpeg: "/no-decoder".into(),
        ffprobe: "/no-probe".into(),
    };
    source_catalog::take_import_io();
    let p = import_scan(
        target.clone(),
        imported.clone(),
        &bogus,
        Arc::new(AtomicBool::new(false)),
        "warm".into(),
        |_| {},
    )
    .unwrap();
    assert_eq!(p.reused, 1);
    assert_eq!(p.failed, 0);
    assert_eq!(
        source_catalog::take_import_io(),
        source_catalog::ImportIo::default()
    ); // Would fail immediately if scan/hash/decode pipeline were used.
    fs::remove_file(moved.path().join("tone.wav")).unwrap();
    let p = import_scan(
        target.clone(),
        imported,
        &bogus,
        Arc::new(AtomicBool::new(false)),
        "deleted".into(),
        |_| {},
    )
    .unwrap();
    assert_eq!(p.failed, 0);
    assert_eq!(target.lock().unwrap().sound(&id).unwrap().status, "missing");
    assert!(source_catalog::read(moved.path())
        .unwrap()
        .unwrap()
        .catalog
        .sounds
        .is_empty());
}
#[test]
fn malformed_catalog_and_symlink_escape_fail_without_database_mutation() {
    let root = tempdir().unwrap();
    let (_, _, _) = seed(root.path());
    let target = memory();
    let mut snapshot = source_catalog::read(root.path()).unwrap().unwrap();
    snapshot.catalog.sounds[0].relative_path = "../escape.wav".into();
    fs::write(
        root.path().join(".creativeshelf/catalog.json"),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    assert!(source_catalog::import_folder(&target, root.path()).is_err());
    assert!(target.lock().unwrap().sources().unwrap().is_empty());
    #[cfg(unix)]
    {
        let other = tempdir().unwrap();
        fs::write(other.path().join("outside.wav"), b"x").unwrap();
        std::os::unix::fs::symlink(other.path(), root.path().join("link")).unwrap();
        assert!(source_catalog::fingerprint(root.path(), "link/outside.wav").is_err());
    }
}
#[test]
fn competing_revision_preserves_prior_snapshot_and_explicit_rebuild_retains_backup() {
    let root = tempdir().unwrap();
    let (db, source, _) = seed(root.path());
    let mut other = source_catalog::read(root.path()).unwrap().unwrap();
    other.revision = uuid::Uuid::new_v4().to_string();
    let bytes = serde_json::to_vec(&other).unwrap();
    fs::write(root.path().join(".creativeshelf/catalog.json"), &bytes).unwrap();
    assert!(source_catalog::save(&db, &source).is_err());
    assert_eq!(
        fs::read(root.path().join(".creativeshelf/catalog.json")).unwrap(),
        bytes
    );
    source_catalog::rebuild(&db, &source).unwrap();
    assert!(fs::read_dir(root.path().join(".creativeshelf"))
        .unwrap()
        .any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("catalog-backup-")));
    source_catalog::save(&db, &source).unwrap();
    assert!(source_catalog::read(root.path()).unwrap().is_some());
}
#[test]
fn access_error_and_concurrent_writer_do_not_prune_or_overwrite() {
    let root = tempdir().unwrap();
    let (db, source, _) = seed(root.path());
    let before = fs::read(root.path().join(".creativeshelf/catalog.json")).unwrap();
    let f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.path().join(".creativeshelf/catalog.lock"))
        .unwrap();
    f.lock().unwrap();
    assert!(source_catalog::save(&db, &source).is_err());
    assert_eq!(
        fs::read(root.path().join(".creativeshelf/catalog.json")).unwrap(),
        before
    );
    drop(f);
    fs::remove_file(root.path().join("tone.wav")).unwrap();
    fs::create_dir(root.path().join("tone.wav")).unwrap();
    assert!(source_catalog::fingerprint(root.path(), "tone.wav").is_err());
    assert!(source_catalog::import_folder(&db, root.path()).is_err());
    assert_eq!(
        fs::read(root.path().join(".creativeshelf/catalog.json")).unwrap(),
        before
    );
}
#[test]
fn dirty_state_survives_restart_and_v6_migration_backs_up() {
    let root = tempdir().unwrap();
    let (db, source, id) = seed(root.path());
    assert!(!db.lock().unwrap().snapshot_states().unwrap()[0].dirty);
    db.lock()
        .unwrap()
        .annotate(&id, &[], "changed", true)
        .unwrap();
    assert!(db.lock().unwrap().snapshot_states().unwrap()[0].dirty);
    source_catalog::save(&db, &source).unwrap();
    let dbroot = tempdir().unwrap();
    let path = dbroot.path().join("library.sqlite");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch(include_str!("../src/schema.sql")).unwrap();
    c.pragma_update(None, "user_version", 6).unwrap();
    drop(c);
    let c = Catalog::open(&path).unwrap();
    assert_eq!(
        c.db_connection()
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        8
    );
    assert!(fs::read_dir(dbroot.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("pre-v8")));
}
#[test]
#[ignore = "requires explicit FFmpeg fixture tools"]
fn real_import_creates_snapshot_warm_import_skips_unlisted_and_refresh_adds_it() {
    let tools = MediaTools::discover().unwrap();
    let root = tempdir().unwrap();
    let make = |path: &Path, frequency: &str| {
        assert!(std::process::Command::new(&tools.ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency={frequency}:duration=0.1")
            ])
            .arg(path)
            .status()
            .unwrap()
            .success());
    };
    make(&root.path().join("rain.wav"), "440");
    let db = memory();
    let source = source_catalog::import_folder(&db, root.path()).unwrap();
    let first = scan(
        db.clone(),
        source.clone(),
        &tools,
        Arc::new(AtomicBool::new(false)),
        "first".into(),
        |_| {},
    )
    .unwrap();
    assert_eq!(first.completed, 1);
    assert!(source_catalog::read(root.path()).unwrap().is_some());
    make(&root.path().join("new.wav"), "880");
    let target = memory();
    let imported = source_catalog::import_folder(&target, root.path()).unwrap();
    let warm = import_scan(
        target.clone(),
        imported.clone(),
        &tools,
        Arc::new(AtomicBool::new(false)),
        "warm".into(),
        |_| {},
    )
    .unwrap();
    assert_eq!(warm.total, 1);
    assert_eq!(warm.reused, 1);
    assert_eq!(target.lock().unwrap().all_sounds().unwrap().len(), 1);
    let fresh = scan(
        target.clone(),
        imported.clone(),
        &tools,
        Arc::new(AtomicBool::new(false)),
        "refresh".into(),
        |_| {},
    )
    .unwrap();
    assert_eq!(fresh.completed, 2);
    assert!(fresh.errors.is_empty(), "{:?}", fresh.errors);
    fs::remove_file(root.path().join("rain.wav")).unwrap();
    make(&root.path().join("rain.wav"), "220");
    let changed = import_scan(
        target.clone(),
        imported,
        &tools,
        Arc::new(AtomicBool::new(false)),
        "changed".into(),
        |_| {},
    )
    .unwrap();
    assert_eq!(changed.reused, 1);
    assert_eq!(changed.completed, 2);
    assert!(changed.errors.is_empty(), "{:?}", changed.errors);
}

#[test]
fn clip_history_roundtrip_and_offline_root_preserves_previous_snapshot() {
    let root = tempdir().unwrap();
    let (db, source, id) = seed(root.path());
    let mut c = db.lock().unwrap();
    let sound = c.sound(&id).unwrap();
    let mut recipe = soundshelf_core::catalog::ClipRecipe {
        asset_id: id.clone(),
        asset_version_id: sound.content_hash,
        source_sample_rate_hz: 100,
        start_frame: "0".into(),
        end_frame: "50".into(),
        channel_policy: "preserve".into(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let first = c.create_clip(&id, "Rain cue", &recipe).unwrap();
    recipe.end_frame = "75".into();
    c.update_clip(&first.id, "Long rain cue", &recipe, 1)
        .unwrap();
    drop(c);
    source_catalog::save(&db, &source).unwrap();
    let before = fs::read(root.path().join(".creativeshelf/catalog.json")).unwrap();
    let target = memory();
    source_catalog::import_folder(&target, root.path()).unwrap();
    let c = target.lock().unwrap();
    let count: i64 = c
        .db_connection()
        .query_row(
            "SELECT count(*) FROM clip_revisions WHERE clip_id=?1",
            [&first.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(c.all_clips().unwrap()[0].revision, 2);
    drop(c);
    let renamed = root.path().with_extension("offline-fixture");
    fs::rename(root.path(), &renamed).unwrap();
    let tools = MediaTools {
        ffmpeg: "/absent".into(),
        ffprobe: "/absent".into(),
    };
    assert!(import_scan(
        target,
        source,
        &tools,
        Arc::new(AtomicBool::new(false)),
        "offline".into(),
        |_| {}
    )
    .is_err());
    assert_eq!(
        fs::read(renamed.join(".creativeshelf/catalog.json")).unwrap(),
        before
    );
    fs::rename(renamed, root.path()).unwrap();
}

#[test]
fn cancellation_and_oversized_snapshot_preserve_database_and_disk() {
    let root = tempdir().unwrap();
    let (db, source, _) = seed(root.path());
    let before = fs::read(root.path().join(".creativeshelf/catalog.json")).unwrap();
    let tools = MediaTools {
        ffmpeg: "/absent".into(),
        ffprobe: "/absent".into(),
    };
    assert!(import_scan(
        db.clone(),
        source,
        &tools,
        Arc::new(AtomicBool::new(true)),
        "cancel".into(),
        |_| {}
    )
    .is_err());
    assert_eq!(
        fs::read(root.path().join(".creativeshelf/catalog.json")).unwrap(),
        before
    );
    let mut snapshot = source_catalog::read(root.path()).unwrap().unwrap();
    snapshot.catalog.clips.clear();
    snapshot.clip_history.clear();
    snapshot.fingerprints.clear();
    snapshot.catalog.sounds.clear();
    snapshot.schema = "future/v999".into();
    fs::write(
        root.path().join(".creativeshelf/catalog.json"),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    let target = memory();
    assert!(source_catalog::import_folder(&target, root.path()).is_err());
    assert!(target.lock().unwrap().sources().unwrap().is_empty());
}

#[test]
fn ten_thousand_members_reuse_metadata_without_walk_hash_or_decode() {
    let root = tempdir().unwrap();
    let (db, source, _) = seed(root.path());
    {
        let c = db.lock().unwrap();
        let original = c.all_sounds().unwrap().remove(0);
        for n in 0..9999 {
            let relative = format!("fixture-{n}.wav");
            fs::hard_link(root.path().join("tone.wav"), root.path().join(&relative)).unwrap();
            c.register(&source, &relative, &original.content_hash)
                .unwrap();
        }
    }
    source_catalog::save(&db, &source).unwrap();
    let target = memory();
    let source = source_catalog::import_folder(&target, root.path()).unwrap();
    source_catalog::take_import_io();
    let tools = MediaTools {
        ffmpeg: "/no-decoder".into(),
        ffprobe: "/no-probe".into(),
    };
    let result = import_scan(
        target,
        source,
        &tools,
        Arc::new(AtomicBool::new(false)),
        "large".into(),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.reused, 10000);
    assert_eq!(
        source_catalog::take_import_io(),
        source_catalog::ImportIo::default()
    );
}

#[test]
#[cfg(unix)]
fn read_only_snapshot_failure_is_retryable_and_member_access_denial_never_prunes() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempdir().unwrap();
    let (db, source, id) = seed(root.path());
    let path = root.path().join(".creativeshelf/catalog.json");
    let before = fs::read(&path).unwrap();
    fs::set_permissions(
        root.path().join(".creativeshelf"),
        fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    let error = source_catalog::save(&db, &source).unwrap_err().to_string();
    db.lock()
        .unwrap()
        .snapshot_error(&source.id, &error)
        .unwrap();
    assert!(db.lock().unwrap().snapshot_states().unwrap()[0].dirty);
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::set_permissions(
        root.path().join(".creativeshelf"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    source_catalog::save(&db, &source).unwrap();
    assert!(!db.lock().unwrap().snapshot_states().unwrap()[0].dirty);
    let before = fs::read(&path).unwrap();
    fs::set_permissions(
        root.path().join("tone.wav"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    assert!(source_catalog::import_folder(&db, root.path()).is_err());
    assert_eq!(db.lock().unwrap().sound(&id).unwrap().status, "ready");
    assert_eq!(fs::read(path).unwrap(), before);
    fs::set_permissions(
        root.path().join("tone.wav"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
}

#[test]
fn changed_content_cannot_be_acknowledged_by_a_metadata_only_save() {
    let root = tempdir().unwrap();
    let (db, source, _) = seed(root.path());
    let path = root.path().join(".creativeshelf/catalog.json");
    let before = fs::read(&path).unwrap();
    fs::write(root.path().join("tone.wav"), b"changed original content").unwrap();
    assert!(source_catalog::save(&db, &source).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn byte_limit_and_non_regular_catalog_fail_before_import() {
    let root = tempdir().unwrap();
    let (_, _, _) = seed(root.path());
    let path = root.path().join(".creativeshelf/catalog.json");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(soundshelf_core::portable::MAX_BYTES + 1)
        .unwrap();
    let target = memory();
    assert!(source_catalog::import_folder(&target, root.path()).is_err());
    assert!(target.lock().unwrap().sources().unwrap().is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        fs::remove_file(&path).unwrap();
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let started = std::time::Instant::now();
        assert!(source_catalog::read(root.path()).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}

#[test]
fn restored_database_does_not_overwrite_newer_source_annotations() {
    let root=tempdir().unwrap();let dbroot=tempdir().unwrap();let (db,source,id)=seed(root.path());
    let backup=dbroot.path().join("backup.sqlite");db.lock().unwrap().backup_database(&backup).unwrap();
    db.lock().unwrap().annotate(&id,&["newer".into()],"Newer source annotation",false).unwrap();
    source_catalog::save(&db,&source).unwrap();
    db.lock().unwrap().restore_database(&backup,&dbroot.path().join("rollback.sqlite")).unwrap();
    assert!(source_catalog::save(&db,&source).is_err());
    assert_eq!(source_catalog::read(root.path()).unwrap().unwrap().catalog.sounds[0].comment,"Newer source annotation");
}
