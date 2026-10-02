use soundshelf_core::{
    catalog::{hash_file, Catalog, ClipRecipe, Profile},
    jobs::now_secs,
    library::{scan, scan_paths},
    media::MediaTools,
};
use std::{
    fs,
    path::Path,
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use tempfile::tempdir;
fn profile() -> Profile {
    Profile {
        duration: 1.,
        sample_rate: 48000,
        channels: 1,
        frames: 48000,
        peak: 0.8,
        rms: 0.2,
        channel_peaks: vec![0.8],
        channel_rms: vec![0.2],
        channel_layout: "mono".into(),
        description: "Measured".into(),
        tags: vec![],
        waveform: vec![[-0.8, 0.8]],
    }
}
fn ready(c: &mut Catalog, path: &Path) -> String {
    let (s, p) = c.select_file(path).unwrap();
    let h = hash_file(path).unwrap();
    let id = c.register(&s, &p, &h).unwrap();
    c.publish(&s, &id, &h, &profile()).unwrap();
    id
}
#[test]
fn explicit_membership_deduplicates_paths_but_retains_content_identities() {
    let d = tempdir().unwrap();
    let a = d.path().join("a.wav");
    let b = d.path().join("b.wav");
    fs::write(&a, b"same").unwrap();
    fs::write(&b, b"same").unwrap();
    fs::write(d.path().join("sibling.wav"), b"unselected").unwrap();
    let mut c = Catalog::open(Path::new(":memory:")).unwrap();
    let id = ready(&mut c, &a);
    assert_eq!(ready(&mut c, &a), id);
    let other = ready(&mut c, &b);
    assert_ne!(id, other);
    assert_eq!(c.all_sounds().unwrap().len(), 2);
    let source = c.sources().unwrap().remove(0);
    assert_eq!(source.scope, "files");
    assert_eq!(source.files.len(), 2);
    assert!(c.register(&source, "sibling.wav", "hash").is_err());
    assert!(c.add_source(d.path()).is_err());
    c.annotate(&id, &["mine".into()], "note", true).unwrap();
    assert_eq!(c.sound(&other).unwrap().comment, "");
    assert!(c.convert_source_to_folder(&source.id, false).is_err());
    let folder = c.convert_source_to_folder(&source.id, true).unwrap();
    assert_eq!(folder.id, source.id);
    assert_eq!(c.sound(&id).unwrap().comment, "note");
    assert!(c.register(&source, "a.wav", "hash").is_err());
}
#[test]
fn confirmed_removal_deletes_metadata_and_recipes_but_preserves_original() {
    let d = tempdir().unwrap();
    let file = d.path().join("a.wav");
    fs::write(&file, b"same").unwrap();
    let mut c = Catalog::open(Path::new(":memory:")).unwrap();
    let id = ready(&mut c, &file);
    let sound = c.sound(&id).unwrap();
    let clip = c
        .create_clip(
            &id,
            "clip",
            &ClipRecipe {
                asset_id: id.clone(),
                asset_version_id: sound.content_hash.clone(),
                source_sample_rate_hz: 48000,
                start_frame: "0".into(),
                end_frame: "100".into(),
                channel_policy: "preserve".into(),
                gain_db: 0.,
                fade_in_ms: 0,
                fade_out_ms: 0,
            },
        )
        .unwrap();
    let s = c.sources().unwrap().remove(0);
    assert!(c.remove_source(&s.id, false).is_err());
    assert!(c.sound(&id).is_ok());
    c.remove_source(&s.id, true).unwrap();
    assert!(c.sound(&id).is_err());
    assert!(c.get_clip(&clip.id).is_err());
    assert_eq!(fs::read(&file).unwrap(), b"same");
}
#[test]
fn rename_and_parent_move_preserve_source_sound_profile_and_annotations() {
    let base = tempdir().unwrap();
    let root = base.path().join("old");
    fs::create_dir(&root).unwrap();
    let old = root.join("a.wav");
    fs::write(&old, b"same").unwrap();
    let mut c = Catalog::open(Path::new(":memory:")).unwrap();
    let id = ready(&mut c, &old);
    let before = c.sound(&id).unwrap();
    c.annotate(&id, &["tag".into()], "keep", true).unwrap();
    let renamed = root.join("renamed.wav");
    fs::rename(&old, &renamed).unwrap();
    c.set_status(&id, "missing").unwrap();
    let s = c.relink_file(&id, &renamed).unwrap();
    assert_eq!(s.id, before.source_id);
    assert_eq!(c.sound(&id).unwrap().status, "ready");
    let moved = base.path().join("new");
    fs::rename(root, &moved).unwrap();
    let s = c.relink_file(&id, &moved.join("renamed.wav")).unwrap();
    assert_eq!(s.id, before.source_id);
    let after = c.sound(&id).unwrap();
    assert_eq!(after.profile, before.profile);
    assert_eq!(after.comment, "keep");
    assert!(after.favorite);
    assert_eq!(s.files.len(), 1);
    assert!(c.resolve(&id).is_ok());
    fs::write(moved.join("different.wav"), b"wrong").unwrap();
    assert!(c.relink_file(&id, &moved.join("different.wav")).is_err());
    assert_eq!(c.sound(&id).unwrap(), after);
}
#[test]
fn collision_rolls_back_and_target_reconciliation_never_marks_siblings_missing() {
    let d = tempdir().unwrap();
    let a = d.path().join("a.wav");
    let b = d.path().join("b.wav");
    fs::write(&a, b"same").unwrap();
    fs::write(&b, b"same").unwrap();
    let mut c = Catalog::open(Path::new(":memory:")).unwrap();
    let first = ready(&mut c, &a);
    let second = ready(&mut c, &b);
    assert!(c.relink_file(&first, &b).is_err());
    let source = c.sources().unwrap().remove(0);
    assert_eq!(source.files.len(), 2);
    c.reconcile_paths(&source, &[], Some(&["a.wav".into()]), false)
        .unwrap();
    assert_eq!(c.sound(&first).unwrap().status, "ready");
    c.reconcile_paths(&source, &[], Some(&["a.wav".into()]), true)
        .unwrap();
    assert_eq!(c.sound(&first).unwrap().status, "missing");
    assert_eq!(c.sound(&second).unwrap().status, "ready");
    c.set_available(&source.id, false).unwrap();
    assert!(c
        .source(&source.id)
        .unwrap()
        .files
        .iter()
        .all(|f| f.status == "offline"));
}
#[test]
fn jobs_recover_exact_targets_deduplicate_and_bound_queue() {
    let d = tempdir().unwrap();
    let media = d.path().join("media");
    fs::create_dir(&media).unwrap();
    let db = d.path().join("db.sqlite");
    let c = Catalog::open(&db).unwrap();
    fs::write(media.join("a.wav"), b"a").unwrap();
    let (s, p) = c.select_file(&media.join("a.wav")).unwrap();
    let job = c
        .enqueue_paths(&s.id, Some(vec![p.clone(), p.clone()]))
        .unwrap();
    assert_eq!(
        job.id,
        c.enqueue_paths(&s.id, Some(vec![p.clone()])).unwrap().id
    );
    drop(c);
    let c = Catalog::open(&db).unwrap();
    let recovered = c.recover_jobs(now_secs()).unwrap();
    assert_eq!(recovered[0].paths, Some(vec![p]));
    for i in 1..32 {
        let path = media.join(format!("{i}.wav"));
        fs::write(&path, b"a").unwrap();
        let (s, p) = c.select_file(&path).unwrap();
        c.enqueue_paths(&s.id, Some(vec![p])).unwrap();
    }
    fs::write(media.join("overflow.wav"), b"a").unwrap();
    let (s, p) = c.select_file(&media.join("overflow.wav")).unwrap();
    assert!(c
        .enqueue_paths(&s.id, Some(vec![p]))
        .unwrap_err()
        .to_string()
        .contains("queue is full"));
    c.cancel_queued().unwrap();
    assert!(c.recover_jobs(now_secs()).unwrap().is_empty());
    assert!(c
        .claim_job(&job.id, "worker", now_secs())
        .unwrap()
        .is_none());
}
#[test]
fn v4_upgrade_preserves_catalog_and_creates_recoverable_backup() {
    let historical = include_str!("fixtures/schema_v4.sql").replace("\r\n", "\n");
    for v4 in [historical.clone(), historical.replace('\n', "\r\n")] {
        let d = tempdir().unwrap();
        let db = d.path().join("db.sqlite");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(&v4).unwrap();
        conn.pragma_update(None, "user_version", 4).unwrap();
        conn.execute(
            "INSERT INTO sources(id,name,root) VALUES('old','Original','/original')",
            [],
        )
        .unwrap();
        conn.execute_batch("INSERT INTO sounds(id,source_id,relative_path,title,content_hash,status) VALUES('asset','old','a.wav','Original','hash','ready');
        INSERT INTO annotations(sound_id,tags,comment,favorite) VALUES('asset','[\"custom\"]','preserved',1);
        INSERT INTO jobs(id,source_id,kind,state,status,created_at,updated_at) VALUES('job','old','scan','queued','queued',0,0);
        INSERT INTO clips(id,sound_id,name,asset_version_id,source_sample_rate_hz,start_frame,end_frame,created_at,updated_at) VALUES('clip','asset','Keep','hash',48000,'0','100',0,0);").unwrap();
        conn.execute(
            "INSERT INTO analyses(content_hash,analyzer,profile) VALUES('hash',?1,?2)",
            rusqlite::params![
                soundshelf_core::catalog::ANALYZER,
                serde_json::to_string(&profile()).unwrap()
            ],
        )
        .unwrap();
        drop(conn);
        let c = Catalog::open(&db).unwrap();
        assert_eq!(c.source("old").unwrap().scope, "folder");
        let sound = c.sound("asset").unwrap();
        assert_eq!(sound.profile, Some(profile()));
        assert_eq!(sound.comment, "preserved");
        assert!(sound.favorite);
        assert_eq!(sound.user_tags, vec!["custom"]);
        assert_eq!(c.get_clip("clip").unwrap().recipe.end_frame, "100");
        assert_eq!(c.job("job").unwrap().paths, None);
        let backups = fs::read_dir(d.path())
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("pre-v5")
                    .then_some(p)
            })
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        let backup = rusqlite::Connection::open(&backups[0]).unwrap();
        assert_eq!(
            backup
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            4
        );
        assert_eq!(
            backup
                .query_row("SELECT name FROM sources WHERE id='old'", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "Original"
        );
    }
}
#[cfg(unix)]
#[test]
fn selected_symlink_escape_and_unsupported_extension_do_not_create_scopes() {
    let d = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("a.wav"), b"a").unwrap();
    std::os::unix::fs::symlink(outside.path().join("a.wav"), d.path().join("link.wav")).unwrap();
    fs::write(d.path().join("no.txt"), b"text").unwrap();
    let c = Catalog::open(Path::new(":memory:")).unwrap();
    assert!(c.select_file(&d.path().join("link.wav")).is_err());
    assert!(c.select_file(&d.path().join("no.txt")).is_err());
    assert!(c.sources().unwrap().is_empty());
}
fn tools() -> MediaTools {
    MediaTools {
        ffmpeg: std::env::var("SOUNDSHELF_FFMPEG").unwrap().into(),
        ffprobe: std::env::var("SOUNDSHELF_FFPROBE").unwrap().into(),
    }
}
#[test]
#[ignore = "requires explicit FFmpeg fixture tools"]
fn real_selected_scan_ignores_siblings_reuses_content_and_isolates_failures() {
    let t = tools();
    let d = tempdir().unwrap();
    let root = d.path().join("folder.with.dots");
    fs::create_dir(&root).unwrap();
    let first = root.join("a.wav");
    assert!(std::process::Command::new(&t.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=0.1",
            "-ar",
            "48000"
        ])
        .arg(&first)
        .status()
        .unwrap()
        .success());
    fs::copy(&first, root.join("b.wav")).unwrap();
    fs::copy(&first, root.join("extensionless")).unwrap();
    fs::copy(&first, root.join("sibling.wav")).unwrap();
    fs::write(root.join("broken.wav"), b"corrupt").unwrap();
    let c = Arc::new(Mutex::new(
        Catalog::open(&d.path().join("db.sqlite")).unwrap(),
    ));
    let (s, p) = c.lock().unwrap().select_file(&first).unwrap();
    let result = scan_paths(
        c.clone(),
        s,
        &t,
        Arc::new(AtomicBool::new(false)),
        "one".into(),
        Some(vec![p]),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.completed, 1);
    assert_eq!(c.lock().unwrap().all_sounds().unwrap().len(), 1);
    for name in ["b.wav", "extensionless", "broken.wav"] {
        c.lock().unwrap().select_file(&root.join(name)).unwrap();
    }
    let s = c.lock().unwrap().sources().unwrap().remove(0);
    let result = scan(
        c.clone(),
        s.clone(),
        &t,
        Arc::new(AtomicBool::new(false)),
        "many".into(),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.completed, 4);
    assert_eq!(result.reused, 3);
    assert_eq!(result.failed, 1);
    assert!(result.errors[0].contains("broken.wav"));
    assert_eq!(c.lock().unwrap().all_sounds().unwrap().len(), 4);
    fs::remove_file(&first).unwrap();
    let result = scan(
        c.clone(),
        s.clone(),
        &t,
        Arc::new(AtomicBool::new(true)),
        "cancel".into(),
        |_| {},
    );
    assert!(result.is_err());
    assert_eq!(
        c.lock()
            .unwrap()
            .all_sounds()
            .unwrap()
            .iter()
            .find(|s| s.relative_path == "a.wav")
            .unwrap()
            .status,
        "ready"
    );
    scan(
        c.clone(),
        s.clone(),
        &t,
        Arc::new(AtomicBool::new(false)),
        "missing".into(),
        |_| {},
    )
    .unwrap();
    let source = c.lock().unwrap().source(&s.id).unwrap();
    assert_eq!(
        source
            .files
            .iter()
            .find(|f| f.relative_path == "a.wav")
            .unwrap()
            .status,
        "missing"
    );
    assert_eq!(
        source
            .files
            .iter()
            .find(|f| f.relative_path == "b.wav")
            .unwrap()
            .status,
        "ready"
    );
    let folder = c
        .lock()
        .unwrap()
        .convert_source_to_folder(&s.id, true)
        .unwrap();
    let result = scan(
        c.clone(),
        folder,
        &t,
        Arc::new(AtomicBool::new(false)),
        "recursive".into(),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.completed, 4);
    assert_eq!(c.lock().unwrap().all_sounds().unwrap().len(), 5);
}
#[test]
fn failed_upgrade_rolls_back_schema_and_preserves_backup() {
    let d = tempdir().unwrap();
    let path = d.path().join("broken.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE sources(id TEXT PRIMARY KEY,name TEXT,root TEXT,generation INTEGER,available INTEGER);CREATE TABLE source_files(dummy TEXT);PRAGMA user_version=4;").unwrap();
    drop(conn);
    assert!(Catalog::open(&path).is_err());
    let conn = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert!(conn.prepare("SELECT scope FROM sources").is_err());
    assert!(fs::read_dir(d.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("pre-v5")));
}
#[test]
fn targeted_import_within_folder_preserves_unselected_sounds() {
    let d = tempdir().unwrap();
    fs::write(d.path().join("a.wav"), b"a").unwrap();
    fs::write(d.path().join("b.wav"), b"b").unwrap();
    let mut c = Catalog::open(Path::new(":memory:")).unwrap();
    let s = c.add_source(d.path()).unwrap();
    let a = ready(&mut c, &d.path().join("a.wav"));
    let b = ready(&mut c, &d.path().join("b.wav"));
    let selected = c.select_file(&d.path().join("a.wav")).unwrap();
    assert_eq!(selected.0.id, s.id);
    c.reconcile_paths(&s, &["a.wav".into()], Some(&["a.wav".into()]), true)
        .unwrap();
    assert_eq!(c.sound(&a).unwrap().status, "ready");
    assert_eq!(c.sound(&b).unwrap().status, "ready");
    assert_eq!(c.sources().unwrap().len(), 1);
}
#[test]
fn reconciliation_of_ten_thousand_paths_preserves_existing_entries() {
    let d = tempdir().unwrap();
    let mut c = Catalog::open(Path::new(":memory:")).unwrap();
    let source = c.add_source(d.path()).unwrap();
    let paths = (0..10000).map(|i| format!("{i}.wav")).collect::<Vec<_>>();
    for path in &paths {
        c.register(&source, path, "hash").unwrap();
    }
    let legacy_start = std::time::Instant::now();
    assert_eq!(
        paths.iter().filter(|p| !paths[..9999].contains(p)).count(),
        1
    );
    let legacy = legacy_start.elapsed();
    let start = std::time::Instant::now();
    c.reconcile_paths(&source, &paths[..9999], None, true)
        .unwrap();
    println!(
        "10,000 entries: legacy Vec membership {:?}; hash-set reconciliation including SQL {:?}",
        legacy,
        start.elapsed()
    );
    let sounds = c.all_sounds().unwrap();
    assert_eq!(sounds.iter().filter(|s| s.status == "missing").count(), 1);
    assert_eq!(
        sounds.iter().filter(|s| s.status == "pending").count(),
        9999
    );
}
#[test]
fn relink_snapshot_rejects_catalog_changes_during_unlocked_hashing() {
    let d = tempdir().unwrap();
    let a = d.path().join("a.wav");
    fs::write(&a, b"same").unwrap();
    let mut c = Catalog::open(Path::new(":memory:")).unwrap();
    let id = ready(&mut c, &a);
    let plan = c.prepare_file_relink(&id, &a).unwrap();
    let source = c.sources().unwrap().remove(0);
    c.convert_source_to_folder(&source.id, true).unwrap();
    let verified = plan.verify().unwrap();
    assert!(c
        .commit_file_relink(verified)
        .unwrap_err()
        .to_string()
        .contains("changed during relink"));
    assert_eq!(c.sound(&id).unwrap().relative_path, "a.wav");
}
#[test]
fn selected_parent_relink_checks_only_members_and_preserves_recipes() {
    let d = tempdir().unwrap();
    let root = d.path().join("old");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a.wav"), b"a").unwrap();
    fs::write(root.join("b.wav"), b"b").unwrap();
    fs::write(root.join("sibling.wav"), b"unselected").unwrap();
    let mut c = Catalog::open(Path::new(":memory:")).unwrap();
    let a = ready(&mut c, &root.join("a.wav"));
    let b = ready(&mut c, &root.join("b.wav"));
    let sound = c.sound(&a).unwrap();
    let clip = c
        .create_clip(
            &a,
            "keep",
            &ClipRecipe {
                asset_id: a.clone(),
                asset_version_id: sound.content_hash.clone(),
                source_sample_rate_hz: 48000,
                start_frame: "0".into(),
                end_frame: "100".into(),
                channel_policy: "preserve".into(),
                gain_db: 0.,
                fade_in_ms: 0,
                fade_out_ms: 0,
            },
        )
        .unwrap();
    let s = c.sources().unwrap().remove(0);
    c.annotate(&a, &["tag".into()], "note", true).unwrap();
    let moved = d.path().join("new");
    fs::rename(root, &moved).unwrap();
    fs::write(moved.join("sibling.wav"), b"changed unrelated").unwrap();
    c.set_available(&s.id, false).unwrap();
    let relinked = c.relink(&s.id, &moved).unwrap();
    assert_eq!(relinked.id, s.id);
    assert!(relinked.available);
    assert_eq!(relinked.files.len(), 2);
    assert_eq!(c.sound(&b).unwrap().source_id, s.id);
    assert_eq!(c.sound(&a).unwrap().comment, "note");
    let kept = c.get_clip(&clip.id).unwrap();
    assert_eq!(kept.recipe, clip.recipe);
    assert!(!kept.is_stale);
    assert_eq!(fs::read(moved.join("a.wav")).unwrap(), b"a");
}
#[test]
fn nested_file_selections_work_in_either_order_without_duplicate_identity() {
    for nested_first in [true, false] {
        let d = tempdir().unwrap();
        fs::create_dir(d.path().join("child")).unwrap();
        let parent = d.path().join("a.wav");
        let nested = d.path().join("child/b.wav");
        fs::write(&parent, b"a").unwrap();
        fs::write(&nested, b"b").unwrap();
        let mut c = Catalog::open(Path::new(":memory:")).unwrap();
        let files = if nested_first {
            [&nested, &parent]
        } else {
            [&parent, &nested]
        };
        let first = ready(&mut c, files[0]);
        let second = ready(&mut c, files[1]);
        assert_eq!(ready(&mut c, files[0]), first);
        assert_eq!(ready(&mut c, files[1]), second);
        assert_eq!(c.all_sounds().unwrap().len(), 2);
        assert!(c.sources().unwrap().iter().all(|s| s.scope == "files"));
        if nested_first {
            let s = c.source(&c.sound(&second).unwrap().source_id).unwrap();
            assert!(c.convert_source_to_folder(&s.id, true).is_err());
            assert_eq!(c.source(&s.id).unwrap().scope, "files");
        }
    }
}
