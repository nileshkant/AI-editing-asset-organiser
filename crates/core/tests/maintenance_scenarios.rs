use soundshelf_core::{catalog::Catalog, maintenance::Preferences, waveform::WaveformService};
use std::fs;
#[test]
fn wal_backup_restore_retains_metadata_preferences_and_rollback() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("live.sqlite");
    let mut db = Catalog::open(&path).unwrap();
    let source = db.add_source(temp.path()).unwrap();
    let id = db.register(&source, "a.wav", "hash").unwrap();
    db.annotate(&id, &["rain".into()], "before", true).unwrap();
    db.set_preferences(&Preferences {
        output_device: Some("Output".into()),
        imports_paused: true,
    })
    .unwrap();
    let backup = temp.path().join("backup.sqlite");
    db.backup_database(&backup).unwrap();
    db.annotate(&id, &["thunder".into()], "after", false)
        .unwrap();
    let rollback = temp.path().join("rollback.sqlite");
    db.restore_database(&backup, &rollback).unwrap();
    assert_eq!(db.sound(&id).unwrap().comment, "before");
    assert!(db.preferences().unwrap().imports_paused);
    assert_eq!(
        Catalog::open(&rollback)
            .unwrap()
            .sound(&id)
            .unwrap()
            .comment,
        "after"
    );
    assert!(db.snapshot_states().unwrap()[0].dirty);
}
#[test]
fn backup_collision_and_invalid_restore_do_not_change_live_data() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(&temp.path().join("live.sqlite")).unwrap();
    let source = db.add_source(temp.path()).unwrap();
    let backup = temp.path().join("backup.sqlite");
    db.backup_database(&backup).unwrap();
    let before = fs::read(&backup).unwrap();
    assert!(db.backup_database(&backup).is_err());
    assert_eq!(fs::read(&backup).unwrap(), before);
    let foreign = Catalog::open(&temp.path().join("foreign.sqlite")).unwrap();
    foreign
        .db_connection()
        .execute_batch(
            "CREATE TRIGGER hostile AFTER INSERT ON sounds BEGIN DELETE FROM annotations; END;",
        )
        .unwrap();
    assert!(db
        .restore_database(
            &temp.path().join("foreign.sqlite"),
            &temp.path().join("rollback.sqlite")
        )
        .is_err());
    assert_eq!(db.sources().unwrap()[0].id, source.id);
    assert!(!temp.path().join("rollback.sqlite").exists());
    foreign
        .db_connection()
        .execute_batch("DROP TRIGGER hostile; PRAGMA user_version=999;")
        .unwrap();
    assert!(db
        .restore_database(
            &temp.path().join("foreign.sqlite"),
            &temp.path().join("rollback.sqlite")
        )
        .is_err());
}
#[test]
fn active_import_prevents_restore_and_backup_failure_preserves_database() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(&temp.path().join("live.sqlite")).unwrap();
    let source = db.add_source(temp.path()).unwrap();
    let backup = temp.path().join("backup.sqlite");
    db.backup_database(&backup).unwrap();
    db.enqueue_scan(&source.id).unwrap();
    assert!(db
        .restore_database(&backup, &temp.path().join("rollback.sqlite"))
        .is_err());
    assert!(!temp.path().join("rollback.sqlite").exists());
    assert!(db
        .backup_database(&temp.path().join("absent/backup.sqlite"))
        .is_err());
    assert_eq!(db.sources().unwrap().len(), 1);
}
#[test]
fn report_allowlist_excludes_catalog_names_paths_comments_and_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let db = Catalog::open(&temp.path().join("live.sqlite")).unwrap();
    let source = db.add_source(temp.path()).unwrap();
    let id = db.register(&source, "SECRET_FILENAME.wav", "hash").unwrap();
    db.annotate(&id, &["SECRET_TAG".into()], "Bearer SECRET_TOKEN", false)
        .unwrap();
    let report = db.support_report(true).unwrap();
    assert!(!report.contains("SECRET"));
    assert!(!report.contains(temp.path().to_str().unwrap()));
    let fields: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(fields["sound_count"], 1);
    assert_eq!(fields["telemetry"], false);
}
#[test]
fn purge_only_generated_waveforms_never_media_or_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let cache = WaveformService::new(temp.path().into());
    fs::write(
        temp.path().join(format!("{}.sswf", "a".repeat(64))),
        b"cache",
    )
    .unwrap();
    fs::write(temp.path().join("original.wav"), b"keep").unwrap();
    fs::write(temp.path().join("unknown.sswf"), b"keep").unwrap();
    assert_eq!(cache.purge_cache().unwrap(), 1);
    assert_eq!(fs::read(temp.path().join("original.wav")).unwrap(), b"keep");
    assert!(temp.path().join("unknown.sswf").exists());
}
#[test]
fn v7_migration_preserves_catalog_and_backs_up_before_preferences_schema() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("v7.sqlite");
    let db = Catalog::open(&path).unwrap();
    let source = db.add_source(temp.path()).unwrap();
    db.db_connection()
        .execute_batch("DROP TRIGGER analyses_search_insert; DROP TRIGGER analyses_search_update; DROP TABLE search_profiles; DROP TABLE app_settings; PRAGMA user_version=7;")
        .unwrap();
    drop(db);
    let db = Catalog::open(&path).unwrap();
    assert_eq!(db.sources().unwrap()[0].id, source.id);
    assert_eq!(db.preferences().unwrap(), Preferences::default());
    assert!(fs::read_dir(temp.path()).unwrap().any(|p| p
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(&format!("pre-v{}",soundshelf_core::catalog::SCHEMA_VERSION))));
}

#[test]
fn restore_rejects_inconsistent_derived_search_metadata_before_rollback() {
    let temp = tempfile::tempdir().unwrap();
    let mut live = Catalog::open(&temp.path().join("live.sqlite")).unwrap();
    let source = live.add_source(temp.path()).unwrap();
    let id = live.register(&source, "rain.wav", "hash").unwrap();
    live.annotate(&id, &[], "keep", true).unwrap();
    let backup = temp.path().join("backup.sqlite");
    live.backup_database(&backup).unwrap();
    let altered = rusqlite::Connection::open(&backup).unwrap();
    altered.execute("INSERT INTO analyses VALUES('hash',?1,'{}')",[soundshelf_core::catalog::ANALYZER]).unwrap();
    altered.execute("UPDATE search_profiles SET profile='different'",[]).unwrap();
    drop(altered);
    let rollback = temp.path().join("rollback.sqlite");
    assert!(live.restore_database(&backup, &rollback).is_err());
    assert!(!rollback.exists());
    assert_eq!(live.sound(&id).unwrap().comment,"keep");
}
