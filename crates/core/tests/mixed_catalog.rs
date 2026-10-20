//! SS-048: mixed audio/image portable catalogs, folder snapshots and backups.
use serde_json::Value;
use soundshelf_core::{
    catalog::{hash_file, Catalog, Profile, Source},
    image_annotations::AnnotationPatch,
    image_contracts::{Availability, ImageProfile, TechnicalState},
    portable::{self, PORTABLE_SCHEMA, PORTABLE_SCHEMA_V2},
    source_catalog,
};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tempfile::{tempdir, TempDir};

fn audio_profile() -> Profile {
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
fn image_profile() -> ImageProfile {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/contracts/image-v1.json")).unwrap();
    serde_json::from_value(fixture["images"][0]["profile"].clone()).unwrap()
}
struct Library {
    dir: TempDir,
    db: Arc<Mutex<Catalog>>,
    source: Source,
    sound: String,
    image: String,
    plain: String,
}
fn tags(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}
/// One folder with a sound and two ready images; one image is annotated, one
/// tag is renamed with an alias, and the annotated image has two versions.
fn library() -> Library {
    let dir = tempdir().unwrap();
    let root = dir.path().join("media");
    fs::create_dir_all(root.join("图像")).unwrap();
    fs::write(root.join("tone.wav"), b"audio metadata fixture").unwrap();
    fs::write(root.join("图像/fox.png"), b"first image bytes").unwrap();
    fs::write(root.join("plain.png"), b"second image bytes").unwrap();
    let mut c = Catalog::open(&dir.path().join("catalog.sqlite")).unwrap();
    let source = c.add_source(&root).unwrap();
    let hash = hash_file(&root.join("tone.wav")).unwrap();
    let sound = c.register(&source, "tone.wav", &hash).unwrap();
    c.publish(&source, &sound, &hash, &audio_profile()).unwrap();
    c.annotate(&sound, &["rain".into()], "audio note", true).unwrap();
    let old = "e".repeat(64);
    let image = c.register_image(&source, "图像/fox.png", Some(&old)).unwrap().header.id;
    let current = hash_file(&root.join("图像/fox.png")).unwrap();
    c.register_image(&source, "图像/fox.png", Some(&current)).unwrap();
    c.publish_image(&source, &image, &current, &image_profile()).unwrap();
    let plain_hash = hash_file(&root.join("plain.png")).unwrap();
    let plain = c.register_image(&source, "plain.png", Some(&plain_hash)).unwrap().header.id;
    c.publish_image(&source, &plain, &plain_hash, &image_profile()).unwrap();
    c.edit_image_annotations(
        &image,
        1,
        &AnnotationPatch {
            add_tags: tags(&["Brand blue", "night"]),
            comment: Some("<b>keep</b> as text".into()),
            favorite: Some(true),
            rating: Some(Some(4)),
            ..Default::default()
        },
    )
    .unwrap();
    c.rename_image_tag("Brand blue", "Brand navy", None, true).unwrap();
    Library { dir, db: Arc::new(Mutex::new(c)), source, sound, image, plain }
}
fn root(l: &Library) -> std::path::PathBuf {
    l.dir.path().join("media")
}
fn media_hashes(root: &Path) -> BTreeMap<String, String> {
    ["tone.wav", "图像/fox.png", "plain.png"]
        .iter()
        .filter(|p| root.join(p).exists())
        .map(|p| (p.to_string(), hash_file(&root.join(p)).unwrap()))
        .collect()
}
fn fresh() -> (TempDir, Catalog) {
    let dir = tempdir().unwrap();
    let c = Catalog::open(&dir.path().join("target.sqlite")).unwrap();
    (dir, c)
}
fn jobs(c: &Catalog) -> i64 {
    c.db_connection()
        .query_row("SELECT count(*) FROM image_import_jobs", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn mixed_portable_round_trip_keeps_identities_versions_annotations_and_aliases() {
    let l = library();
    let before = media_hashes(&root(&l));
    let c = l.db.lock().unwrap();
    let exported = c.export_portable().unwrap();
    assert_eq!(exported.schema, PORTABLE_SCHEMA_V2);
    let text = exported.json().unwrap();
    assert!(!text.contains(&root(&l).to_string_lossy().to_string()), "no absolute roots");
    for forbidden in ["asset_file_stats", "image_source_monitor", "image_annotation_actions", "lease_owner", "root_stamp"] {
        assert!(!text.contains(forbidden), "{forbidden} is machine-local");
    }
    let preview = portable::preview(&text, false).unwrap();
    assert_eq!((preview.sounds, preview.images), (1, 2));
    let original_image = c.image(&l.image, None).unwrap();
    let original_sound = c.sound(&l.sound).unwrap();
    drop(c);

    let (_t, mut target) = fresh();
    let moved = tempdir().unwrap();
    let roots = BTreeMap::from([(l.source.id.clone(), moved.path().to_string_lossy().into_owned())]);
    let data = portable::prepare_import(&text, &roots, false).unwrap();
    let report = target.import_portable(&data, &roots, moved.path()).unwrap();
    assert_eq!((report.sounds, report.images, report.offline_sources), (1, 2, 0));
    let image = target.image(&l.image, None).unwrap();
    assert_eq!(image, original_image, "identity, profile, annotations and revision");
    assert_eq!(image.annotations.tags, ["Brand navy", "night"]);
    assert_eq!(image.annotations.revision, original_image.annotations.revision);
    assert_eq!(target.sound(&l.sound).unwrap().comment, original_sound.comment);
    let versions: i64 = target
        .db_connection()
        .query_row("SELECT count(*) FROM image_asset_versions WHERE asset_id=?1", [&l.image], |r| r.get(0))
        .unwrap();
    assert_eq!(versions, 2, "previous content version recovery history travels");
    let alias: String = target
        .db_connection()
        .query_row("SELECT canonical FROM image_tag_aliases WHERE alias='Brand blue'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(alias, "Brand navy");
    // A second export of the imported library is identical apart from ordering-free JSON.
    let again: Value = serde_json::from_str(&target.export_portable().unwrap().json().unwrap()).unwrap();
    let first: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(again["images"], first["images"]);
    // Aliases keep resolving after transfer.
    let next = target
        .edit_image_annotations(&l.plain, 1, &AnnotationPatch { add_tags: tags(&["brand blue"]), ..Default::default() })
        .unwrap();
    assert_eq!(next.annotations.tags, ["Brand navy"]);
    // Watching is never carried over; stats are rebuilt on the first check.
    let watched: i64 = target
        .db_connection()
        .query_row("SELECT count(*) FROM image_source_monitor WHERE watch=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(watched, 0);
    assert_eq!(jobs(&target), 0, "metadata import starts no hashing or decoding");
    assert_eq!(media_hashes(&root(&l)), before, "original media is never written");
}

#[test]
fn audio_only_libraries_still_export_v1_for_older_readers() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("audio");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a.wav"), b"x").unwrap();
    let c = Catalog::open(&dir.path().join("c.sqlite")).unwrap();
    let source = c.add_source(&root).unwrap();
    let hash = hash_file(&root.join("a.wav")).unwrap();
    c.register(&source, "a.wav", &hash).unwrap();
    let exported = c.export_portable().unwrap();
    assert_eq!(exported.schema, PORTABLE_SCHEMA);
    let text = exported.json().unwrap();
    assert!(!text.contains("\"images\"") && !text.contains("image_tag_aliases"));
}

#[test]
fn unsupported_newer_and_malformed_mixed_catalogs_fail_without_changes() {
    let l = library();
    let text = l.db.lock().unwrap().export_portable().unwrap().json().unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    let reject = |value: &Value, expected: &str| {
        let error = portable::preview(&value.to_string(), false).unwrap_err().to_string();
        assert!(error.contains(expected), "{error} should contain {expected}");
    };
    let mut newer = doc.clone();
    newer["schema"] = "soundshelf-catalog/v3".into();
    reject(&newer, "newer CreativeShelf");
    let mut downgraded = doc.clone();
    downgraded["schema"] = "soundshelf-catalog/v1".into();
    reject(&downgraded, "require the soundshelf-catalog/v2 schema");
    let mut unknown = doc.clone();
    unknown["images"][0]["asset"]["header"]["future_required"] = true.into();
    assert!(portable::preview(&unknown.to_string(), false).is_err());
    let mut kind = doc.clone();
    kind["images"][0]["asset"]["header"]["kind"] = "video".into();
    assert!(portable::preview(&kind.to_string(), false).is_err());
    let mut orphan = doc.clone();
    orphan["images"][0]["asset"]["header"]["source_id"] = "missing".into();
    reject(&orphan, "unknown source");
    let mut collision = doc.clone();
    collision["images"][0]["asset"]["header"]["id"] = collision["images"][1]["asset"]["header"]["id"].clone();
    reject(&collision, "duplicate identity");
    let mut audio_id = doc.clone();
    audio_id["images"][0]["asset"]["header"]["id"] = doc["sounds"][0]["id"].clone();
    assert!(portable::preview(&audio_id.to_string(), false).is_err());
    let mut history = doc.clone();
    history["images"][0]["versions"] = Value::Array(vec![]);
    reject(&history, "version history");
    let mut tag = doc.clone();
    tag["images"][0]["asset"]["annotations"]["tags"] = Value::from(vec!["  not normalized "]);
    assert!(portable::preview(&tag.to_string(), false).is_err());
    let mut chain = doc.clone();
    chain["image_tag_aliases"] = serde_json::json!([{"alias":"a","canonical":"b"},{"alias":"b","canonical":"c"}]);
    reject(&chain, "chain");
    let mut path = doc.clone();
    path["images"][0]["asset"]["header"]["relative_path"] = "../escape.png".into();
    assert!(portable::preview(&path.to_string(), false).is_err());

    // A database-level conflict after sources and sounds were inserted rolls back everything.
    let (_t, mut target) = fresh();
    let mut conflicting = doc.clone();
    let profile = &mut conflicting["images"][1]["asset"]["profile"];
    profile["width"] = 1601.into();
    conflicting["images"][1]["asset"]["header"]["content_hash"] = doc["images"][0]["asset"]["header"]["content_hash"].clone();
    conflicting["images"][1]["versions"] = doc["images"][0]["versions"].clone();
    let data: portable::PortableCatalog = serde_json::from_value(conflicting.clone()).unwrap();
    assert!(data.validate().is_err(), "conflicting analyses for one digest");
    let data: portable::PortableCatalog = serde_json::from_value(doc.clone()).unwrap();
    target
        .db_connection()
        .execute("INSERT INTO image_tag_aliases VALUES('brand blue','Brand blue','Other',0)", [])
        .unwrap();
    let error = target.import_portable(&data, &BTreeMap::new(), _t.path()).unwrap_err();
    assert!(error.to_string().contains("alias conflicts"), "{error}");
    for table in ["sources", "sounds", "image_assets", "asset_registry"] {
        let n: i64 = target
            .db_connection()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{table} rolled back");
    }
}

#[test]
fn unmapped_sources_import_offline_and_keep_image_facts() {
    let l = library();
    let text = l.db.lock().unwrap().export_portable().unwrap().json().unwrap();
    let (dir, mut target) = fresh();
    let data = portable::prepare_import(&text, &BTreeMap::new(), false).unwrap();
    let report = target.import_portable(&data, &BTreeMap::new(), dir.path()).unwrap();
    assert_eq!(report.offline_sources, 1);
    let image = target.image(&l.image, None).unwrap();
    assert_eq!(image.header.availability, Availability::Offline);
    assert_eq!(image.header.technical_state, TechnicalState::Ready);
    assert!(image.profile.is_some());
    // Exporting again records the stored member state, not the derived offline view.
    let again = target.export_portable().unwrap();
    assert_eq!(again.images[0].asset.header.availability, Availability::Available);
}

#[test]
fn mixed_folder_snapshot_warm_import_trusts_stats_and_flags_only_changes() {
    let l = library();
    source_catalog::save(&l.db, &l.source).unwrap();
    let snapshot = source_catalog::read(&root(&l)).unwrap().unwrap();
    assert_eq!(snapshot.schema, source_catalog::SCHEMA_V2);
    assert_eq!(snapshot.catalog.images.len(), 2);
    assert!(snapshot.catalog.image_tag_aliases.is_empty(), "aliases are library-wide");
    assert_eq!(snapshot.fingerprints.len(), 3);
    let state = &l.db.lock().unwrap().snapshot_states().unwrap()[0];
    assert!(!state.dirty && state.error.is_empty());

    // Same-stat replacement: bytes differ, size and mtime are restored. A warm
    // import cannot see it, which proves it neither hashes nor decodes.
    let fox = root(&l).join("图像/fox.png");
    let modified = fs::metadata(&fox).unwrap().modified().unwrap();
    fs::write(&fox, b"FIRST image bytes").unwrap();
    fs::File::options().write(true).open(&fox).unwrap().set_modified(modified).unwrap();
    // A size change is visible from stat alone.
    fs::write(root(&l).join("plain.png"), b"second image bytes, edited").unwrap();

    let target = Arc::new(Mutex::new(Catalog::open(&l.dir.path().join("other.sqlite")).unwrap()));
    let started = Instant::now();
    let imported = source_catalog::import_folder(&target, &root(&l)).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(imported.id, l.source.id);
    let c = target.lock().unwrap();
    let fox = c.image(&l.image, None).unwrap();
    assert_eq!(fox.header.technical_state, TechnicalState::Ready, "unchanged stat keeps verified facts");
    assert_eq!(fox.annotations.tags, ["Brand navy", "night"]);
    let plain = c.image(&l.plain, None).unwrap();
    assert_eq!(plain.header.technical_state, TechnicalState::Pending);
    assert!(plain.profile.is_some(), "cached analysis stays linked until the check");
    assert_eq!(c.pending_members(&l.source.id).unwrap(), ["plain.png"]);
    assert_eq!(jobs(&c), 0, "import itself starts no work");
    drop(c);
    let job = soundshelf_core::image_sources::enqueue_snapshot_checks(&target, &l.source.id)
        .unwrap()
        .unwrap();
    let paths: String = target
        .lock()
        .unwrap()
        .db_connection()
        .query_row("SELECT paths FROM image_import_jobs WHERE id=?1", [&job.id], |r| r.get(0))
        .unwrap();
    assert_eq!(paths, r#"["plain.png"]"#, "only the changed member is checked");
    eprintln!("warm mixed folder import: {elapsed:?}");
    assert!(elapsed < Duration::from_secs(5));
}

#[test]
fn missing_members_and_newer_sidecars_are_never_overwritten() {
    let l = library();
    source_catalog::save(&l.db, &l.source).unwrap();
    {
        let other = library();
        source_catalog::save(&other.db, &other.source).unwrap();
        fs::remove_file(root(&other).join("plain.png")).unwrap();
        let target = Arc::new(Mutex::new(Catalog::open(&other.dir.path().join("other.sqlite")).unwrap()));
        source_catalog::import_folder(&target, &root(&other)).unwrap();
        let plain = target.lock().unwrap().image(&other.plain, None).unwrap();
        assert_eq!(plain.header.availability, Availability::Missing);
        assert_eq!(plain.header.technical_state, TechnicalState::Ready, "missing keeps facts");
    }

    // Backup, then edit and save a newer snapshot; restoring the backup must
    // not overwrite the newer sidecar in the background.
    let backup = l.dir.path().join("backup.sqlite");
    l.db.lock().unwrap().backup_database(&backup).unwrap();
    l.db.lock()
        .unwrap()
        .edit_image_annotations(&l.image, 3, &AnnotationPatch { comment: Some("newer note".into()), ..Default::default() })
        .unwrap();
    source_catalog::save(&l.db, &l.source).unwrap();
    let newer = fs::read(root(&l).join(".creativeshelf/catalog.json")).unwrap();
    let summary = l.db.lock().unwrap().inspect_backup(&backup).unwrap();
    assert_eq!((summary.sounds, summary.images, summary.tag_aliases), (1, 2, 1));
    l.db.lock()
        .unwrap()
        .restore_previewed(&backup, &l.dir.path().join("rollback.sqlite"), &summary)
        .unwrap();
    let error = source_catalog::save(&l.db, &l.source).unwrap_err();
    assert!(error.to_string().contains("revision changed"), "{error}");
    assert_eq!(fs::read(root(&l).join(".creativeshelf/catalog.json")).unwrap(), newer);
    assert!(String::from_utf8(newer).unwrap().contains("newer note"));
}

#[test]
fn restore_preview_is_bound_to_the_previewed_bytes() {
    let l = library();
    let backup = l.dir.path().join("backup.sqlite");
    l.db.lock().unwrap().backup_database(&backup).unwrap();
    let summary = l.db.lock().unwrap().inspect_backup(&backup).unwrap();
    assert_eq!(summary.schema_version, soundshelf_core::catalog::SCHEMA_VERSION);
    let mut bytes = fs::read(&backup).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&backup, bytes).unwrap();
    let before = l.db.lock().unwrap().image(&l.image, None).unwrap();
    let error = l
        .db
        .lock()
        .unwrap()
        .restore_previewed(&backup, &l.dir.path().join("rollback.sqlite"), &summary)
        .unwrap_err();
    assert!(error.to_string().contains("changed since its preview"));
    assert_eq!(l.db.lock().unwrap().image(&l.image, None).unwrap(), before);
    assert!(!l.dir.path().join("rollback.sqlite").exists(), "nothing ran");
}

#[test]
fn wal_backup_captures_committed_image_edits_from_another_connection() {
    let l = library();
    let path = l.dir.path().join("catalog.sqlite");
    let mut writer = Catalog::open(&path).unwrap();
    let mode: String = writer.db_connection().query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    writer
        .edit_image_annotations(&l.plain, 1, &AnnotationPatch { add_tags: tags(&["from wal"]), ..Default::default() })
        .unwrap();
    let backup = l.dir.path().join("wal.sqlite");
    l.db.lock().unwrap().backup_database(&backup).unwrap();
    drop(writer);
    let (_t, mut target) = fresh();
    let summary = target.inspect_backup(&backup).unwrap();
    target.restore_previewed(&backup, &_t.path().join("rb.sqlite"), &summary).unwrap();
    assert_eq!(target.image(&l.plain, None).unwrap().annotations.tags, ["from wal"], "journal mode {mode}");
}

#[cfg(unix)]
#[test]
fn read_only_folder_keeps_working_and_reports_the_snapshot_error() {
    use std::os::unix::fs::PermissionsExt;
    let l = library();
    source_catalog::save(&l.db, &l.source).unwrap();
    let sidecar = root(&l).join(".creativeshelf");
    let before = fs::read(sidecar.join("catalog.json")).unwrap();
    l.db.lock()
        .unwrap()
        .edit_image_annotations(&l.plain, 1, &AnnotationPatch { add_tags: tags(&["later"]), ..Default::default() })
        .unwrap();
    fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o500)).unwrap();
    let probe = sidecar.join("probe");
    let writable = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    let result = source_catalog::save(&l.db, &l.source);
    fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o700)).unwrap();
    if writable {
        eprintln!("running with privileges that ignore directory permissions; skipped");
        return;
    }
    let error = result.unwrap_err();
    l.db.lock().unwrap().snapshot_error(&l.source.id, &error.to_string()).unwrap();
    assert_eq!(fs::read(sidecar.join("catalog.json")).unwrap(), before);
    let state = &l.db.lock().unwrap().snapshot_states().unwrap()[0];
    assert!(state.dirty && !state.error.is_empty());
    assert!(l.db.lock().unwrap().image(&l.plain, None).is_ok(), "library stays usable");
}

#[test]
fn bounded_mixed_round_trip_at_scale() {
    // 10,000 images and 1,000 sounds through export, JSON, validation and import.
    let dir = tempdir().unwrap();
    let root = dir.path().join("big");
    fs::create_dir(&root).unwrap();
    let mut c = Catalog::open(&dir.path().join("big.sqlite")).unwrap();
    let source = c.add_source(&root).unwrap();
    for i in 0..1000 {
        let hash = format!("{:064x}", i + 1);
        let id = c.register(&source, &format!("a/{i}.wav"), &hash).unwrap();
        c.publish(&source, &id, &hash, &audio_profile()).unwrap();
    }
    let profile = image_profile();
    let start = Instant::now();
    // Seeding speed is not under test; durability of the fixture is irrelevant.
    c.db_connection().execute_batch("PRAGMA synchronous=OFF").unwrap();
    for i in 0..10_000 {
        let hash = format!("{:064x}", 1_000_000 + i);
        let image = c.register_image(&source, &format!("i/{i}.png"), Some(&hash)).unwrap();
        c.publish_image(&source, &image.header.id, &hash, &profile).unwrap();
    }
    eprintln!("seed: {:?}", start.elapsed());
    let started = Instant::now();
    let text = c.export_portable().unwrap().json().unwrap();
    let export = started.elapsed();
    let started = Instant::now();
    let preview = portable::preview(&text, false).unwrap();
    let validate = started.elapsed();
    assert_eq!(preview.images, 10_000);
    let (_t, mut target) = fresh();
    let data = portable::prepare_import(&text, &BTreeMap::new(), false).unwrap();
    let started = Instant::now();
    target.import_portable(&data, &BTreeMap::new(), _t.path()).unwrap();
    let import = started.elapsed();
    eprintln!(
        "10k images + 1k sounds: {} bytes, export {export:?}, preview {validate:?}, import {import:?}",
        text.len()
    );
    assert!((text.len() as u64) < portable::MAX_BYTES);
}
