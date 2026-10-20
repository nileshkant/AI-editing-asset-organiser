use rusqlite::{params, Connection};
use serde_json::{json, Value};
use soundshelf_core::{
    catalog::{Catalog, Source, SCHEMA_VERSION},
    image_catalog::IMAGE_CAPABILITY_ENABLED,
    image_contracts::{
        Annotations, AssetKind, Availability, EnrichmentState, ImageProfile, TechnicalState,
    },
    source_catalog,
};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
use tempfile::{tempdir, TempDir};

fn fixture() -> (TempDir, Catalog, Source) {
    let dir = tempdir().unwrap();
    let media = dir.path().join("media");
    fs::create_dir(&media).unwrap();
    let c = Catalog::open(&dir.path().join("catalog.sqlite")).unwrap();
    let source = c.add_source(&media).unwrap();
    (dir, c, source)
}
fn digest(c: char) -> String {
    c.to_string().repeat(64)
}
fn profile() -> ImageProfile {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/contracts/image-v1.json")).unwrap();
    serde_json::from_value(fixture["images"][0]["profile"].clone()).unwrap()
}
fn clean(c: &Catalog) {
    let check: String = c
        .db_connection()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    let foreign: i64 = c
        .db_connection()
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(foreign, 0);
}
fn count(c: &Catalog, table: &str) -> i64 {
    c.db_connection()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn registration_cache_and_annotations_keep_independent_identity() {
    let (_dir, mut c, s) = fixture();
    let h = digest('a');
    let first = c.register_image(&s, "图像/first.png", Some(&h)).unwrap();
    assert_eq!(first.header.technical_state, TechnicalState::Pending);
    assert!(first.profile.is_none());
    let ready = c
        .publish_image(&s, &first.header.id, &h, &profile())
        .unwrap();
    assert_eq!(ready.header.technical_state, TechnicalState::Ready);
    let again = c.register_image(&s, "图像/first.png", Some(&h)).unwrap();
    assert_eq!(again, ready);
    let other = c.register_image(&s, "second.png", Some(&h)).unwrap();
    assert_ne!(first.header.id, other.header.id);
    let other = c
        .publish_cached_image(&s, &other.header.id, &h, &profile().analyzer_version)
        .unwrap();
    assert_eq!(other.profile, ready.profile);
    assert_eq!(count(&c, "image_analyses"), 1);
    let mut annotation = ready.annotations.clone();
    annotation.comment = "only first".into();
    annotation.rating = Some(5);
    let edited = c
        .set_image_annotations(&first.header.id, Some(&h), &annotation)
        .unwrap();
    assert_eq!(edited.annotations.revision, 2);
    assert_eq!(
        c.image(&other.header.id, None).unwrap().annotations.comment,
        ""
    );
    assert!(c
        .set_image_annotations(&first.header.id, Some(&h), &annotation)
        .is_err());
    let mut conflicting = profile();
    conflicting.source_color_profile = Some("different facts".into());
    assert!(c
        .publish_image(&s, &other.header.id, &h, &conflicting)
        .is_err());
    assert_eq!(
        c.image(&other.header.id, None).unwrap().profile,
        Some(profile())
    );
    assert_eq!(c.cached_image_profile(&h, "other-analyzer").unwrap(), None);
    clean(&c);
}

#[test]
fn replacement_keeps_user_metadata_and_invalidates_only_content_bound_data() {
    let (_dir, mut c, s) = fixture();
    let old = digest('a');
    let new = digest('b');
    let asset = c.register_image(&s, "first.png", Some(&old)).unwrap();
    let asset = c
        .publish_image(&s, &asset.header.id, &old, &profile())
        .unwrap();
    let mut annotations = asset.annotations.clone();
    annotations.comment = "keep".into();
    annotations.tags = vec!["my character".into()];
    let asset = c
        .set_image_annotations(&asset.header.id, Some(&old), &annotations)
        .unwrap();
    // Seed reserved persisted evidence/recipes; their editing workflows are later tickets.
    let template: Value =
        serde_json::from_str(include_str!("../../../fixtures/contracts/image-v1.json")).unwrap();
    let evidence = template["images"][0]["evidence"].clone();
    let mut recipes = template["images"][0]["recipes"].clone();
    recipes[0]["asset_id"] = json!(asset.header.id);
    c.db_connection().execute("UPDATE image_assets SET evidence=?2,recipes=?3,enrichment_state='complete' WHERE id=?1",
        params![asset.header.id,evidence.to_string(),recipes.to_string()]).unwrap();
    let changed = c.register_image(&s, "first.png", Some(&new)).unwrap();
    assert_eq!(changed.header.id, asset.header.id);
    assert_eq!(changed.annotations, asset.annotations);
    assert_eq!(changed.header.technical_state, TechnicalState::Pending);
    assert_eq!(changed.header.enrichment_state, EnrichmentState::Stale);
    assert!(changed.profile.is_none());
    assert!(changed.evidence.iter().all(|e| e.stale));
    assert!(changed.recipes.iter().all(|r| r.stale));
    assert_eq!(count(&c, "image_asset_versions"), 2);
    assert!(c
        .cached_image_profile(&old, &profile().analyzer_version)
        .unwrap()
        .is_some());
    assert!(c
        .publish_image(&s, &asset.header.id, &old, &profile())
        .is_err());
    assert!(c
        .set_image_annotations(&asset.header.id, Some(&old), &changed.annotations)
        .is_err());
    assert!(c.register_image(&s, "first.png", None).is_err());
    assert_eq!(c.image(&asset.header.id, Some(&new)).unwrap(), changed);
    let renamed = c.register_image(&s, "renamed.png", Some(&new)).unwrap();
    assert_ne!(renamed.header.id, asset.header.id); // No guessed rename/reassignment.
    clean(&c);
}

#[test]
fn registry_enforces_cross_kind_ids_and_paths_in_both_write_directions() {
    let (_dir, mut c, s) = fixture();
    let audio = c.register(&s, "audio.wav", "legacy digest").unwrap();
    assert!(c
        .register_image(&s, "audio.wav", Some(&digest('a')))
        .is_err());
    let image = c
        .register_image(&s, "image.png", Some(&digest('a')))
        .unwrap();
    assert!(c.register(&s, "image.png", "audio replacement").is_err());
    assert_eq!(c.all_sounds().unwrap().len(), 1);
    assert!(c
        .db_connection()
        .execute(
            "INSERT INTO sounds VALUES(?1,?2,'other.wav','Other','hash','pending')",
            params![image.header.id, s.id]
        )
        .is_err());
    assert!(c
        .db_connection()
        .execute(
            "INSERT INTO asset_registry VALUES(?1,'image',?2,'other.png')",
            params![audio, s.id]
        )
        .is_err());
    assert!(c
        .db_connection()
        .execute(
            "UPDATE sounds SET relative_path='image.png' WHERE id=?1",
            [&audio]
        )
        .is_err());
    assert_eq!(c.sound(&audio).unwrap().relative_path, "audio.wav");
    assert!(c
        .image(&audio, None)
        .unwrap_err()
        .to_string()
        .contains("Wrong asset kind"));
    assert!(c.sound(&image.header.id).is_err());
    assert!(c
        .db_connection()
        .execute(
            "UPDATE image_assets SET technical_state='ready' WHERE id=?1",
            [&image.header.id]
        )
        .is_err());
    assert!(c.db_connection().execute("UPDATE image_assets SET analyzer_version='missing',technical_state='ready' WHERE id=?1",[&image.header.id]).is_err());
    assert!(c
        .db_connection()
        .execute(
            "UPDATE asset_registry SET kind='audio' WHERE id=?1",
            [&image.header.id]
        )
        .is_err());
    assert!(c
        .db_connection()
        .execute("DELETE FROM asset_registry WHERE id=?1", [&audio])
        .is_err());
    clean(&c);
}

#[test]
fn file_scope_generation_and_offline_states_remain_distinct() {
    let (_dir, mut c, s) = fixture();
    c.db_connection()
        .execute("UPDATE sources SET scope='files' WHERE id=?1", [&s.id])
        .unwrap();
    c.db_connection()
        .execute(
            "INSERT INTO source_files(source_id,relative_path) VALUES(?1,'only.png')",
            [&s.id],
        )
        .unwrap();
    let source = c.source(&s.id).unwrap();
    assert!(c
        .register_image(&source, "sibling.png", Some(&digest('a')))
        .is_err());
    let asset = c
        .register_image(&source, "only.png", Some(&digest('a')))
        .unwrap();
    let asset = c
        .publish_image(&source, &asset.header.id, &digest('a'), &profile())
        .unwrap();
    c.set_available(&s.id, false).unwrap();
    let offline = c.image(&asset.header.id, None).unwrap();
    assert_eq!(offline.header.availability, Availability::Offline);
    assert_eq!(offline.header.technical_state, TechnicalState::Ready);
    assert!(c
        .publish_image(&source, &asset.header.id, &digest('a'), &profile())
        .is_err());
    // Local metadata can be corrected while media is offline; snapshots remain protected.
    let mut notes = asset.annotations.clone();
    notes.comment = "offline note".into();
    assert!(c
        .set_image_annotations(&asset.header.id, Some(&digest('a')), &notes)
        .is_ok());
    c.set_available(&s.id, true).unwrap();
    c.db_connection()
        .execute(
            "UPDATE sources SET generation=generation+1 WHERE id=?1",
            [&s.id],
        )
        .unwrap();
    assert!(c
        .register_image(&source, "only.png", Some(&digest('b')))
        .is_err());
    assert!(c
        .publish_image(&source, &asset.header.id, &digest('a'), &profile())
        .is_err());
    assert_eq!(
        c.image(&asset.header.id, None).unwrap().header.content_hash,
        Some(digest('a'))
    );
    assert_eq!(count(&c, "image_analyses"), 1);
    clean(&c);
}

#[test]
fn scoped_keyset_pages_are_stable_and_bounded() {
    let (dir, mut c, s) = fixture();
    let other = dir.path().join("private");
    fs::create_dir(&other).unwrap();
    let hidden = c.add_source(&other).unwrap();
    let private = c
        .register_image(&hidden, "private.png", Some(&digest('b')))
        .unwrap();
    let mut ids = vec![];
    for i in 0..7 {
        ids.push(
            c.register_image(&s, &format!("{i}.png"), None)
                .unwrap()
                .header
                .id,
        );
    }
    ids.sort();
    assert!(c.list_images(&[], None, 3).unwrap().items.is_empty());
    assert!(c.image_in_sources(&[], &private.header.id, None).is_err());
    assert!(c
        .image_in_sources(&[s.id.clone()], &private.header.id, None)
        .is_err());
    let mut found = vec![];
    let mut cursor = None;
    loop {
        let page = c
            .list_images(&[s.id.clone()], cursor.as_deref(), 3)
            .unwrap();
        assert!(page.items.len() <= 3);
        found.extend(page.items.into_iter().map(|i| i.header.id));
        cursor = page.next_after;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(found, ids);
    assert!(c.list_images(&[s.id.clone()], None, 0).is_err());
    assert!(c.list_images(&[s.id.clone()], None, 101).is_err());
    assert!(c
        .list_images(&[s.id.clone()], Some("../unsafe"), 3)
        .is_err());
    assert!(c.list_images(&vec![s.id.clone(); 129], None, 3).is_err());
    assert!(!IMAGE_CAPABILITY_ENABLED);
}

#[test]
fn large_path_pages_respect_the_wire_byte_budget_without_skipping_records() {
    let (_dir, mut c, s) = fixture();
    let mut expected = vec![];
    for i in 0..70 {
        let path = format!("{i:03}/{}.png", "x".repeat(32000));
        expected.push(c.register_image(&s, &path, None).unwrap().header.id);
    }
    let first = c.list_images(&[s.id.clone()], None, 100).unwrap();
    assert!(first.items.len() < 70);
    assert!(first.next_after.is_some());
    assert!(serde_json::to_vec(&first).unwrap().len() <= 2 * 1024 * 1024);
    let second = c
        .list_images(&[s.id.clone()], first.next_after.as_deref(), 100)
        .unwrap();
    assert!(second.next_after.is_none());
    let mut actual = first
        .items
        .into_iter()
        .chain(second.items)
        .map(|a| a.header.id)
        .collect::<Vec<_>>();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

#[test]
fn mixed_sidecars_save_as_v2_while_audio_only_sources_stay_v1() {
    let (dir, c, s) = fixture();
    // Establish a real valid old snapshot before image registration.
    let audio = c.register(&s, "old.wav", "a".repeat(64).as_str()).unwrap();
    c.set_status(&audio, "missing").unwrap(); // No fake ready analysis/media is needed.
    let db = Arc::new(Mutex::new(c));
    source_catalog::save(&db, &s).unwrap();
    let sidecar = Path::new(&s.root).join(".creativeshelf/catalog.json");
    assert_eq!(source_catalog::read(Path::new(&s.root)).unwrap().unwrap().schema, source_catalog::SCHEMA);
    fs::write(Path::new(&s.root).join("image.png"), b"image member").unwrap();
    db.lock()
        .unwrap()
        .register_image(&s, "image.png", Some(&digest('a')))
        .unwrap();
    // SS-048 removed the SS-041 guard: the image travels in a v2 snapshot.
    source_catalog::save(&db, &s).unwrap();
    let snapshot = source_catalog::read(Path::new(&s.root)).unwrap().unwrap();
    assert_eq!(snapshot.schema, source_catalog::SCHEMA_V2);
    assert_eq!(snapshot.catalog.images[0].asset.header.relative_path, "image.png");
    assert!(snapshot.catalog.sounds.is_empty(), "confirmed-missing audio is pruned as before");
    assert!(db.lock().unwrap().snapshot_states().unwrap().iter().all(|state| state.source_id != s.id || (!state.dirty && state.error.is_empty())));
    assert_eq!(db.lock().unwrap().export_portable().unwrap().images.len(), 1);
    source_catalog::rebuild(&db, &s).unwrap();
    assert!(!sidecar.exists(), "rebuild moves the previous snapshot to a backup");
    // Image presence in a different source must not break an audio-only sidecar.
    let other = dir.path().join("audio-only");
    fs::create_dir(&other).unwrap();
    let source = db.lock().unwrap().add_source(&other).unwrap();
    source_catalog::save(&db, &source).unwrap();
    assert!(other.join(".creativeshelf/catalog.json").is_file());
    let moved = dir.path().join("moved");
    fs::create_dir(&moved).unwrap();
    assert!(db.lock().unwrap().relink(&s.id, &moved).is_err());
    assert_eq!(db.lock().unwrap().source(&s.id).unwrap().root, s.root);
}

#[test]
fn backup_restore_preserves_images_history_shared_profiles_and_revisions() {
    let (dir, mut c, s) = fixture();
    let h = digest('a');
    let asset = c.register_image(&s, "image.png", Some(&h)).unwrap();
    let asset = c
        .publish_image(&s, &asset.header.id, &h, &profile())
        .unwrap();
    let mut a = asset.annotations.clone();
    a.comment = "backed up".into();
    let asset = c
        .set_image_annotations(&asset.header.id, Some(&h), &a)
        .unwrap();
    let backup = dir.path().join("backup.sqlite");
    c.backup_database(&backup).unwrap();
    let mut changed = asset.annotations.clone();
    changed.comment = "after backup".into();
    c.set_image_annotations(&asset.header.id, Some(&h), &changed)
        .unwrap();
    let rollback = dir.path().join("rollback.sqlite");
    c.restore_database(&backup, &rollback).unwrap();
    assert_eq!(
        c.image(&asset.header.id, None).unwrap().annotations,
        asset.annotations
    );
    assert_eq!(count(&c, "image_asset_versions"), 1);
    assert_eq!(count(&c, "image_analyses"), 1);
    assert_eq!(
        Catalog::open(&rollback)
            .unwrap()
            .image(&asset.header.id, None)
            .unwrap()
            .annotations
            .comment,
        "after backup"
    );
    drop(c);
    let reopened = Catalog::open(&dir.path().join("catalog.sqlite")).unwrap();
    assert_eq!(
        reopened
            .image(&asset.header.id, None)
            .unwrap()
            .annotations
            .comment,
        "backed up"
    );
    clean(&reopened);
}

#[test]
fn malformed_backup_metadata_is_rejected_before_live_data_or_rollback_changes() {
    let (dir, mut c, s) = fixture();
    let asset = c
        .register_image(&s, "image.png", Some(&digest('a')))
        .unwrap();
    let backup = dir.path().join("backup.sqlite");
    c.backup_database(&backup).unwrap();
    let corrupt = Connection::open(&backup).unwrap();
    corrupt
        .execute(
            "UPDATE image_assets SET annotations=json_set(annotations,'$.rating',9)",
            [],
        )
        .unwrap();
    drop(corrupt);
    let rollback = dir.path().join("no-rollback.sqlite");
    assert!(c.restore_database(&backup, &rollback).is_err());
    assert!(!rollback.exists());
    assert_eq!(
        c.image(&asset.header.id, None).unwrap().annotations.rating,
        None
    );
    clean(&c);
}

#[test]
fn incomplete_version_history_and_registry_mismatch_cannot_be_restored() {
    let (dir, mut c, s) = fixture();
    let asset = c
        .register_image(&s, "image.png", Some(&digest('a')))
        .unwrap();
    let audio = c.register(&s, "audio.wav", "old hash").unwrap();
    for mode in ["history", "registry"] {
        let backup = dir.path().join(format!("{mode}.sqlite"));
        c.backup_database(&backup).unwrap();
        let corrupt = Connection::open(&backup).unwrap();
        if mode == "history" {
            corrupt
                .execute("DELETE FROM image_asset_versions", [])
                .unwrap();
        } else {
            corrupt
                .execute(
                    "UPDATE asset_registry SET relative_path='wrong.wav' WHERE id=?1",
                    [&audio],
                )
                .unwrap();
        }
        drop(corrupt);
        assert!(c
            .restore_database(&backup, &dir.path().join(format!("{mode}-rollback.sqlite")))
            .is_err());
        assert!(c.image(&asset.header.id, None).is_ok());
    }
    clean(&c);
}

#[test]
fn metadata_deletion_is_revision_checked_and_never_deletes_original_bytes() {
    let (_dir, mut c, s) = fixture();
    let file = Path::new(&s.root).join("image.png");
    fs::write(&file, b"original untouched fixture").unwrap();
    let before = fs::read(&file).unwrap();
    let image = c
        .register_image(&s, "image.png", Some(&digest('a')))
        .unwrap();
    assert!(c
        .delete_image(&s, &image.header.id, Some(&digest('a')), 99)
        .is_err());
    assert!(c
        .delete_image(&s, &image.header.id, Some(&digest('b')), 1)
        .is_err());
    c.delete_image(&s, &image.header.id, Some(&digest('a')), 1)
        .unwrap();
    assert!(c.image(&image.header.id, None).is_err());
    assert_eq!(count(&c, "image_asset_versions"), 0);
    assert_eq!(count(&c, "asset_registry"), 0);
    assert_eq!(fs::read(&file).unwrap(), before);
    clean(&c);
}

#[test]
fn reserved_element_storage_rejects_audio_parent_on_restore() {
    let (dir, mut c, s) = fixture();
    let audio = c.register(&s, "audio.wav", "hash").unwrap();
    let template: Value =
        serde_json::from_str(include_str!("../../../fixtures/contracts/image-v1.json")).unwrap();
    let id = "99999999-9999-4999-8999-999999999999";
    let lineage = template["images"][1]["lineage"].clone();
    c.db_connection()
        .execute(
            "INSERT INTO asset_registry VALUES(?1,'element',?2,'reserved.png')",
            params![id, s.id],
        )
        .unwrap();
    let a = Annotations {
        revision: 1,
        tags: vec![],
        comment: String::new(),
        favorite: false,
        rating: None,
        caption: None,
    };
    c.db_connection().execute("INSERT INTO image_assets(id,technical_state,availability,enrichment_state,annotation_revision,annotations,lineage)
        VALUES(?1,'pending','available','disabled',1,?2,?3)",params![id,serde_json::to_string(&a).unwrap(),lineage.to_string()]).unwrap();
    assert_eq!(c.image(id, None).unwrap().header.kind, AssetKind::Element);
    c.db_connection()
        .execute(
            "UPDATE image_assets SET lineage=json_set(lineage,'$.parent_id',?2) WHERE id=?1",
            params![id, audio],
        )
        .unwrap();
    assert!(c.image(id, None).is_err());
    let backup = dir.path().join("invalid-lineage.sqlite");
    c.backup_database(&backup).unwrap();
    assert!(c
        .restore_database(&backup, &dir.path().join("rollback.sqlite"))
        .is_err());
}

#[test]
fn newer_schema_is_rejected_without_mutating_its_version() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("future.sqlite");
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    let journal: String = db
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    drop(db);
    let before = fs::read(&path).unwrap();
    assert!(Catalog::open(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.pragma_query_value::<String, _>(None, "journal_mode", |r| r.get(0))
            .unwrap(),
        journal
    );
    assert_eq!(
        db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        SCHEMA_VERSION + 1
    );
}
