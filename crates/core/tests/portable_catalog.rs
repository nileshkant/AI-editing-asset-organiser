use soundshelf_core::{
    catalog::{hash_file, Catalog, ClipRecipe, Profile, SCHEMA_VERSION},
    portable::*,
    search::SearchQuery,
};
use std::{collections::BTreeMap, fs, path::Path};
use tempfile::tempdir;

fn memory() -> Catalog {
    Catalog::open(Path::new(":memory:")).unwrap()
}
fn profile() -> Profile {
    Profile {
        duration: 1.0,
        sample_rate: 48000,
        channels: 1,
        frames: 48000,
        peak: 0.8,
        rms: 0.2,
        channel_peaks: vec![0.8],
        channel_rms: vec![0.2],
        channel_layout: "mono".into(),
        description: "Measured test audio".into(),
        tags: vec!["impact".into()],
        waveform: vec![[-0.8, 0.8]],
    }
}
fn fixture(root: &Path) -> (Catalog, String, String) {
    fs::write(root.join("音声.wav"), b"original immutable bytes").unwrap();
    let mut catalog = memory();
    let source = catalog.add_source(root).unwrap();
    let hash = hash_file(&root.join("音声.wav")).unwrap();
    let id = catalog.register(&source, "音声.wav", &hash).unwrap();
    catalog.publish(&source, &id, &hash, &profile()).unwrap();
    catalog
        .annotate(&id, &["お気に入り".into()], "Scene 你好", true)
        .unwrap();
    catalog
        .create_clip(
            &id,
            "First half",
            &ClipRecipe {
                asset_id: id.clone(),
                asset_version_id: hash,
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
    catalog
        .save_search(
            "Favorite impacts",
            &SearchQuery {
                text: "impact".into(),
                source_ids: vec![source.id.clone()],
                favorites_only: true,
                ..Default::default()
            },
        )
        .unwrap();
    (catalog, source.id, id)
}
#[test]
fn roundtrip_relocated_unicode_profiles_annotations_clips_and_search_without_media_reads() {
    let root = tempdir().unwrap();
    let (catalog, source, id) = fixture(root.path());
    let data = catalog.export_portable().unwrap();
    let text = data.json().unwrap();
    assert!(!text.contains(root.path().to_str().unwrap()));
    for forbidden in [
        "data_directory",
        "credentials",
        "lease_owner",
        "destination_id",
        "local_path",
    ] {
        assert!(!text.contains(forbidden));
    }
    let moved = tempdir().unwrap(); // Intentionally empty: import must not read/hash/decode media.
    let mut target = memory();
    let roots = BTreeMap::from([(source.clone(), moved.path().to_str().unwrap().to_string())]);
    let report = target
        .import_portable(
            &prepare_import(&text, &roots, false).unwrap(),
            &roots,
            root.path(),
        )
        .unwrap();
    assert_eq!(report.sounds, 1);
    assert_eq!(report.offline_sources, 0);
    assert_eq!(target.sound(&id).unwrap(), catalog.sound(&id).unwrap());
    assert_eq!(target.all_clips().unwrap(), catalog.all_clips().unwrap());
    assert_eq!(
        target.saved_searches().unwrap(),
        catalog.saved_searches().unwrap()
    );
    assert_eq!(target.search(&SearchQuery::default()).unwrap().total, 1);
    assert_eq!(
        target.cached_profile(&data.sounds[0].content_hash).unwrap(),
        Some(profile())
    );
    assert_eq!(
        target
            .db_connection()
            .query_row("SELECT count(*) FROM jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        fs::read(root.path().join("音声.wav")).unwrap(),
        b"original immutable bytes"
    );
}
#[test]
fn unmapped_root_offline_metadata_survives_and_verified_relink_restores_identity() {
    let root = tempdir().unwrap();
    let (catalog, source, id) = fixture(root.path());
    let data = catalog.export_portable().unwrap();
    let mut target = memory();
    let offline = tempdir().unwrap();
    let report = target
        .import_portable(&data, &BTreeMap::new(), offline.path())
        .unwrap();
    assert_eq!(report.offline_sources, 1);
    assert!(!target.source(&source).unwrap().available);
    assert_eq!(target.sound(&id).unwrap().comment, "Scene 你好");
    assert!(target.resolve(&id).is_err());
    target.relink(&source, root.path()).unwrap();
    assert_eq!(
        target.resolve(&id).unwrap(),
        root.path().join("音声.wav").canonicalize().unwrap()
    );
}
#[test]
fn collision_after_inserting_source_rolls_back_every_change() {
    let a = tempdir().unwrap();
    let (mut target, _, _) = fixture(a.path());
    let before = target.export_portable().unwrap().json().unwrap();
    let b = tempdir().unwrap();
    let (catalog, _, _) = fixture(b.path());
    let mut data = catalog.export_portable().unwrap();
    // Force saved-search unique-name collision at the end of the transaction.
    data.saved_searches[0].id = "different-search".into();
    let roots = BTreeMap::from([(
        data.sources[0].id.clone(),
        b.path().to_str().unwrap().into(),
    )]);
    assert!(target.import_portable(&data, &roots, b.path()).is_err());
    assert_eq!(target.export_portable().unwrap().json().unwrap(), before);
}
#[test]
fn rejects_schema_paths_orphans_invalid_profiles_and_scope_broadening() {
    let root = tempdir().unwrap();
    let (c, _, _) = fixture(root.path());
    let data = c.export_portable().unwrap();
    let mut bad = data.clone();
    bad.schema = "soundshelf-catalog/v999".into();
    assert!(bad.validate().is_err());
    for path in ["../secret", "C:/secret", "/etc/passwd", "a\\b", "a//b"] {
        let mut bad = data.clone();
        bad.sounds[0].relative_path = path.into();
        assert!(bad.validate().is_err());
    }
    let mut bad = data.clone();
    bad.sounds[0].source_id = "unknown".into();
    assert!(bad.validate().is_err());
    let mut bad = data.clone();
    bad.sounds[0].profile.as_mut().unwrap().rms = -1.0;
    assert!(bad.validate().is_err());
    let mut bad = data.clone();
    bad.sources[0].scope = "files".into();
    assert!(bad.validate().is_err());
    let mut bad = data.clone();
    bad.sounds.push(bad.sounds[0].clone());
    assert!(bad.validate().is_err());
    let roots = BTreeMap::from([(
        data.sources[0].id.clone(),
        root.path().join("missing").to_str().unwrap().into(),
    )]);
    assert!(memory()
        .import_portable(&data, &roots, root.path())
        .is_err());
    assert!(preview("{\"schema\":\"unknown\"}", false).is_err());
}
#[test]
fn file_scope_roundtrip_preserves_only_explicit_members() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("selected.wav"), b"chosen").unwrap();
    fs::write(root.path().join("private.wav"), b"private").unwrap();
    let mut c = memory();
    let (source, relative) = c.select_file(&root.path().join("selected.wav")).unwrap();
    let hash = hash_file(&root.path().join("selected.wav")).unwrap();
    let id = c.register(&source, &relative, &hash).unwrap();
    c.publish(&source, &id, &hash, &profile()).unwrap();
    let data = c.export_portable().unwrap();
    assert_eq!(data.sources[0].files, vec!["selected.wav"]);
    let mut imported = memory();
    imported
        .import_portable(
            &data,
            &BTreeMap::from([(source.id.clone(), root.path().to_str().unwrap().into())]),
            root.path(),
        )
        .unwrap();
    assert!(imported
        .validate_source_entry(&source.id, "private.wav")
        .is_err());
    assert_eq!(
        imported.resolve(&id).unwrap(),
        root.path().join("selected.wav").canonicalize().unwrap()
    );
}
fn legacy_text(hash: Option<String>) -> String {
    serde_json::json!({"schema":LEGACY_SCHEMA,"sources":[{"id":"main","name":"Legacy","local_path":"/secret/machine"}],"total_sounds":1,"sounds":[{"id":"old-path-id","source_id":"main","relative_path":"音声.wav","title":"Old sound","semantic_description":"Wind from filename","audio_profile":{"duration_sec":800,"analyzed_seconds":20,"rms_dbfs":-33},"user_tags":["mine"],"comment":"Keep","favorite":true,"content_hash":hash}]}).to_string()
}
#[test]
fn legacy_opt_in_count_hash_and_coverage_annotations_survive_reanalysis() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("音声.wav"), b"old audio").unwrap();
    let roots = BTreeMap::from([("main".into(), root.path().to_str().unwrap().into())]);
    assert!(preview(&legacy_text(None), false).is_err());
    assert!(prepare_import(&legacy_text(None), &BTreeMap::new(), true).is_err());
    let mut incorrect: serde_json::Value = serde_json::from_str(&legacy_text(None)).unwrap();
    incorrect["total_sounds"] = 2.into();
    assert!(preview(&incorrect.to_string(), true).is_err());
    assert!(prepare_import(&legacy_text(Some("incorrect".into())), &roots, true).is_err());
    let hash = hash_file(&root.path().join("音声.wav")).unwrap();
    let data = prepare_import(&legacy_text(Some(hash.clone())), &roots, true).unwrap();
    assert_eq!(
        data.legacy_evidence[0].provenance,
        "legacy_filename_inference"
    );
    assert_eq!(data.legacy_evidence[0].analyzed_seconds, Some(20.0));
    assert_eq!(data.sounds[0].content_hash, hash);
    assert!(data.sounds[0].profile.is_none());
    assert_eq!(data.sounds[0].status, "pending");
    let mut c = memory();
    c.import_portable(&data, &roots, root.path()).unwrap();
    let s = c.source("main").unwrap();
    c.publish(&s, &data.sounds[0].id, &hash, &profile())
        .unwrap();
    let restored = c.sound(&data.sounds[0].id).unwrap();
    assert_eq!(restored.comment, "Keep");
    assert_eq!(restored.user_tags, vec!["mine"]);
    assert!(restored.favorite);
    assert_eq!(
        c.export_portable().unwrap().legacy_evidence,
        data.legacy_evidence
    );
    assert_eq!(
        fs::read(root.path().join("音声.wav")).unwrap(),
        b"old audio"
    );
}
#[cfg(unix)]
#[test]
fn legacy_symlink_escape_does_not_read_unselected_file() {
    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("音声.wav"), b"secret").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("音声.wav"),
        root.path().join("音声.wav"),
    )
    .unwrap();
    assert!(prepare_import(
        &legacy_text(None),
        &BTreeMap::from([("main".into(), root.path().to_str().unwrap().into())]),
        true
    )
    .is_err());
}
#[test]
fn catalog_files_refuse_overwrite_and_markdown_escapes_html() {
    let root = tempdir().unwrap();
    let (c, _, _) = fixture(root.path());
    let mut data = c.export_portable().unwrap();
    data.sounds[0].comment = "<script>unsafe</script>".into();
    assert!(data.markdown().contains("&lt;script&gt;"));
    assert!(!data.markdown().contains("<script>"));
    let path = root.path().join("catalog.json");
    write_document(&path, &data.json().unwrap()).unwrap();
    assert!(write_document(&path, "replace").is_err());
    assert!(preview(&read_document(&path).unwrap(), false).is_ok());
}
#[test]
fn v5_upgrade_preserves_metadata_and_backup_failure_is_transactional() {
    let root = tempdir().unwrap();
    let path = root.path().join("v5.sqlite");
    let old = include_str!("fixtures/schema_v5.sql");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch(old).unwrap();
    connection.pragma_update(None, "user_version", 5).unwrap();
    connection.execute_batch("INSERT INTO sources(id,name,root) VALUES('s','Keep','/offline');INSERT INTO sounds(id,source_id,relative_path,title,content_hash,status) VALUES('a','s','a.wav','Keep','hash','pending');INSERT INTO annotations(sound_id,comment) VALUES('a','preserved');").unwrap();
    drop(connection);
    let c = Catalog::open(&path).unwrap();
    assert_eq!(c.sound("a").unwrap().comment, "preserved");
    assert_eq!(
        c.db_connection()
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
    let backup = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().contains("pre-v8"))
        .unwrap();
    let db = rusqlite::Connection::open(backup).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        5
    );
    let broken = root.path().join("broken.sqlite");
    let db = rusqlite::Connection::open(&broken).unwrap();
    db.execute_batch(old).unwrap();
    db.execute_batch("CREATE TABLE legacy_evidence(dummy TEXT); PRAGMA user_version=5;")
        .unwrap();
    drop(db);
    assert!(Catalog::open(&broken).is_err());
    let db = rusqlite::Connection::open(broken).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        5
    );
}

#[test]
fn large_legacy_source_uses_durable_scoped_scan_without_256_file_selection_limit() {
    let root = tempdir().unwrap();
    let mut sounds = vec![];
    for i in 0..300 {
        let path = format!("{i}.wav");
        fs::write(root.path().join(&path), b"same content").unwrap();
        sounds.push(serde_json::json!({"id":format!("old-{i}"),"source_id":"legacy","title":format!("Sound {i}"),"relative_path":path,"description":"Filename inference"}));
    }
    fs::write(root.path().join("private.wav"), b"not selected").unwrap();
    let text = serde_json::json!({"schema":LEGACY_SCHEMA,"total_sounds":300,"sources":[{"id":"legacy","name":"Legacy"}],"sounds":sounds}).to_string();
    let roots = BTreeMap::from([("legacy".into(), root.path().to_str().unwrap().into())]);
    let data = prepare_import(&text, &roots, true).unwrap();
    let mut c = memory();
    c.import_portable(&data, &roots, root.path()).unwrap();
    let job = c.enqueue_scan("legacy").unwrap();
    assert_eq!(job.paths, None);
    let source = c.source("legacy").unwrap();
    assert_eq!(source.scope, "files");
    assert_eq!(source.files.len(), 300);
    assert!(c.validate_source_entry("legacy", "private.wav").is_err());
}
