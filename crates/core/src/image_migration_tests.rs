//! Disposable migration failure/rollback fixtures; never open the user's catalog.
use crate::{
    catalog::{self, Catalog, ClipRecipe, Profile, SCHEMA_VERSION},
    maintenance::Preferences,
    search::SearchQuery,
};
use rusqlite::{params, Connection};
use std::{
    fs,
    path::Path,
    sync::{Arc, Barrier},
    thread,
};

fn v9(path: &Path) -> Catalog {
    let db = Connection::open(path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    for schema in [
        include_str!("schema.sql"),
        include_str!("source_catalog_schema.sql"),
        include_str!("settings_schema.sql"),
        include_str!("search_profile_schema.sql"),
    ] {
        db.execute_batch(schema).unwrap();
    }
    db.pragma_update(None, "user_version", 9).unwrap();
    Catalog { db }
}
fn original_version(db: &Connection) -> u32 {
    db.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}
fn no_registry(db: &Connection) {
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='asset_registry'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn populated_v9_upgrade_preserves_audio_clips_memberships_jobs_preferences_and_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.sqlite");
    let media = dir.path().join("media");
    fs::create_dir(&media).unwrap();
    let original = b"unchanged synthetic bytes";
    fs::write(media.join("voice.wav"), original).unwrap();
    let mut old = v9(&path);
    let source = old.add_source(&media).unwrap();
    let id = old
        .register(&source, "voice.wav", "old content identity")
        .unwrap();
    let profile = Profile {
        duration: 1.0,
        sample_rate: 48000,
        channels: 1,
        frames: 48000,
        peak: 0.5,
        rms: 0.2,
        channel_peaks: vec![0.5],
        channel_rms: vec![0.2],
        channel_layout: "mono".into(),
        description: "Synthetic metadata".into(),
        tags: vec!["impact".into()],
        waveform: vec![[-0.5, 0.5]],
    };
    old.publish(&source, &id, "old content identity", &profile)
        .unwrap();
    old.annotate(&id, &["mine".into()], "preserved", true)
        .unwrap();
    let clip = old
        .create_clip(
            &id,
            "test",
            &ClipRecipe {
                asset_id: id.clone(),
                asset_version_id: "old content identity".into(),
                source_sample_rate_hz: 48000,
                start_frame: "0".into(),
                end_frame: "24000".into(),
                channel_policy: "preserve".into(),
                gain_db: 0.0,
                fade_in_ms: 0,
                fade_out_ms: 0,
            },
        )
        .unwrap();
    old.save_search(
        "saved",
        &SearchQuery {
            text: "impact".into(),
            ..Default::default()
        },
    )
    .unwrap();
    old.set_preferences(&Preferences {
        output_device: Some("Output".into()),
        imports_paused: true,
    })
    .unwrap();
    old.db
        .execute(
            "INSERT INTO source_files(source_id,relative_path) VALUES(?1,'voice.wav')",
            [&source.id],
        )
        .unwrap();
    let job = old.enqueue_scan(&source.id).unwrap();
    let sound = old.sound(&id).unwrap();
    let sources = old.sources().unwrap();
    let searches = old.saved_searches().unwrap();
    let preferences = old.preferences().unwrap();
    let jobs = serde_json::to_value(old.job(&job.id).unwrap()).unwrap();
    let dirty: i64 = old
        .db
        .query_row(
            "SELECT dirty FROM source_catalog_state WHERE source_id=?1",
            [&source.id],
            |r| r.get(0),
        )
        .unwrap();
    drop(old);
    let upgraded = Catalog::open(&path).unwrap();
    assert_eq!(original_version(&upgraded.db), SCHEMA_VERSION);
    assert_eq!(upgraded.sound(&id).unwrap(), sound);
    assert_eq!(upgraded.get_clip(&clip.id).unwrap(), clip);
    assert_eq!(upgraded.sources().unwrap(), sources);
    assert_eq!(upgraded.saved_searches().unwrap(), searches);
    assert_eq!(upgraded.preferences().unwrap(), preferences);
    assert_eq!(
        serde_json::to_value(upgraded.job(&job.id).unwrap()).unwrap(),
        jobs
    );
    assert_eq!(
        upgraded
            .db
            .query_row(
                "SELECT dirty FROM source_catalog_state WHERE source_id=?1",
                [&source.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        dirty
    );
    let registry: (String, String, String) = upgraded
        .db
        .query_row(
            "SELECT id,kind,relative_path FROM asset_registry WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(registry, (id.clone(), "audio".into(), "voice.wav".into()));
    let backup = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().contains(&format!("pre-v{}-", SCHEMA_VERSION)))
        .unwrap();
    let backup = Connection::open(&backup).unwrap();
    assert_eq!(original_version(&backup), 9);
    no_registry(&backup);
    let previous = Catalog { db: backup };
    assert_eq!(previous.sound(&id).unwrap(), sound);
    assert_eq!(previous.get_clip(&clip.id).unwrap(), clip);
    assert_eq!(fs::read(media.join("voice.wav")).unwrap(), original);
}

#[test]
fn non_uuid_audio_id_and_long_legacy_path_are_preserved_without_reinterpretation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.sqlite");
    let old = v9(&path);
    let source = old.add_source(dir.path()).unwrap();
    let relative = "x".repeat(33000);
    old.db
        .execute(
            "INSERT INTO sounds VALUES('legacy:path-id',?1,?2,'Legacy','not-a-digest','pending')",
            params![source.id, relative],
        )
        .unwrap();
    drop(old);
    let c = Catalog::open(&path).unwrap();
    assert_eq!(c.sound("legacy:path-id").unwrap().relative_path, relative);
    assert_eq!(
        c.db.query_row("SELECT id FROM asset_registry", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "legacy:path-id"
    );
}

#[test]
fn migration_midpoint_failure_rolls_back_backfill_triggers_and_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.sqlite");
    let old = v9(&path);
    let s = old.add_source(dir.path()).unwrap();
    let id = old.register(&s, "audio.wav", "hash").unwrap();
    old.annotate(&id, &[], "preserve", true).unwrap();
    old.db
        .execute_batch("CREATE TABLE image_contents(collision TEXT)")
        .unwrap();
    drop(old);
    assert!(Catalog::open(&path).is_err());
    let original = Connection::open(&path).unwrap();
    assert_eq!(original_version(&original), 9);
    no_registry(&original);
    assert_eq!(
        original
            .query_row("SELECT comment FROM annotations", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "preserve"
    );
    assert_eq!(
        original
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='registry_sound_insert'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert!(fs::read_dir(dir.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(&format!("pre-v{}-", SCHEMA_VERSION))));
}

#[test]
fn sqlite_full_during_image_schema_creation_is_transactionally_rolled_back() {
    let mut old = v9(Path::new(":memory:"));
    let pages: i64 = old
        .db
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .unwrap();
    old.db.pragma_update(None, "max_page_count", pages).unwrap();
    let tx = old.db.transaction().unwrap();
    let result = tx.execute_batch(include_str!("image_schema.sql"));
    assert!(
        matches!(result,Err(rusqlite::Error::SqliteFailure(ref e,_)) if e.code==rusqlite::ErrorCode::DiskFull)
    );
    drop(tx);
    no_registry(&old.db);
    assert_eq!(original_version(&old.db), 9);
    assert_eq!(
        old.db
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[test]
fn backup_failures_never_replace_existing_files_or_mutate_old_schema() {
    let dir = tempfile::tempdir().unwrap();
    let old = v9(&dir.path().join("old.sqlite"));
    let existing = dir.path().join("existing.sqlite");
    fs::write(&existing, b"keep existing").unwrap();
    assert!(catalog::upgrade_backup(&old.db, &existing).is_err());
    assert_eq!(fs::read(&existing).unwrap(), b"keep existing");
    assert!(catalog::upgrade_backup(&old.db, &dir.path().join("absent/backup.sqlite")).is_err());
    assert_eq!(original_version(&old.db), 9);
    no_registry(&old.db);
}

#[cfg(unix)]
#[test]
fn real_backup_write_failure_cancels_upgrade_before_schema_mutation() {
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        drop(v9(&path));
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result = Catalog::open(&path);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        let db = Connection::open(&path).unwrap();
        assert_eq!(original_version(&db), 9);
        no_registry(&db);
    }
}

#[test]
fn concurrent_open_upgrade_is_serialized_or_fails_without_partial_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.sqlite");
    drop(v9(&path));
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = vec![];
    for _ in 0..2 {
        let path = path.clone();
        let barrier = barrier.clone();
        threads.push(thread::spawn(move || {
            barrier.wait();
            Catalog::open(&path).is_ok()
        }));
    }
    barrier.wait();
    assert!(threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>()
        .contains(&true));
    let c = Catalog::open(&path).unwrap();
    assert_eq!(original_version(&c.db), SCHEMA_VERSION);
    assert_eq!(
        c.db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        c.db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn fresh_and_v5_upgraded_image_schemas_match() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("v5.sqlite");
    let db = Connection::open(&old).unwrap();
    db.execute_batch(include_str!("../tests/fixtures/schema_v5.sql"))
        .unwrap();
    db.pragma_update(None, "user_version", 5).unwrap();
    drop(db);
    let upgraded = Catalog::open(&old).unwrap();
    let fresh = Catalog::open(Path::new(":memory:")).unwrap();
    fn schema(c: &Catalog) -> Vec<(String, String)> {
        let mut q=c.db.prepare("SELECT name,sql FROM sqlite_master WHERE name LIKE 'image_%' OR name LIKE 'registry_%' OR name LIKE 'asset_registry%' ORDER BY name").unwrap();
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }
    assert_eq!(schema(&upgraded), schema(&fresh));
}
