use soundshelf_core::{
    catalog::{contained, hash_file, valid_relative, Catalog, Profile, Source},
    search::{search, SearchQuery},
};
use std::{fs, path::Path};
use tempfile::tempdir;

fn sample_profile() -> Profile {
    Profile {
        duration: 2.5,
        sample_rate: 48000,
        channels: 2,
        frames: 120000,
        peak: 0.9,
        rms: 0.25,
        channel_peaks: vec![0.9, 0.85],
        channel_rms: vec![0.25, 0.22],
        channel_layout: "stereo".into(),
        description: "Measured stereo test recording".into(),
        tags: vec!["stereo".into(), "clean".into()],
        waveform: vec![[-0.9, 0.9], [-0.85, 0.85]],
    }
}

fn test_catalog() -> Catalog {
    Catalog::open(Path::new(":memory:")).unwrap()
}

fn add_sound(
    c: &mut Catalog,
    source: &Source,
    root: &Path,
    relative: &str,
    content: &[u8],
) -> (String, String) {
    let full_path = root.join(relative);
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&full_path, content).unwrap();
    let hash = hash_file(&full_path).unwrap();
    let id = c.register(source, relative, &hash).unwrap();
    c.publish(source, &id, &hash, &sample_profile()).unwrap();
    (id, hash)
}

#[test]
fn missing_sounds_absent_from_search_and_queries() {
    let base = tempdir().unwrap();
    let root_a = base.path().join("source_a");
    let root_b = base.path().join("source_b");
    fs::create_dir(&root_a).unwrap();
    fs::create_dir(&root_b).unwrap();

    let mut c = test_catalog();
    let source_a = c.add_source(&root_a).unwrap();
    let source_b = c.add_source(&root_b).unwrap();

    // Sound 1: ready in source A
    let (id1, _) = add_sound(&mut c, &source_a, &root_a, "ready.wav", b"ready audio");
    c.annotate(&id1, &["tag1".into()], "first sound", true).unwrap();

    // Sound 2: missing in source A
    let (id2, _) = add_sound(&mut c, &source_a, &root_a, "missing.wav", b"missing audio");
    c.annotate(&id2, &["tag2".into()], "second sound", true).unwrap();
    c.set_status(&id2, "missing").unwrap();

    // Sound 3: ready in source B, but source B is offline
    let (id3, _) = add_sound(&mut c, &source_b, &root_b, "offline.wav", b"offline audio");
    c.annotate(&id3, &["tag3".into()], "third sound", true).unwrap();
    c.set_available(&source_b.id, false).unwrap();

    // 1. ready_sound checks
    assert!(c.ready_sound(&id1).is_ok());
    let err_missing = c.ready_sound(&id2).unwrap_err();
    assert_eq!(err_missing.to_string(), "Sound is not ready");
    let err_offline = c.ready_sound(&id3).unwrap_err();
    assert_eq!(err_offline.to_string(), "Source is offline");

    // 2. resolve checks
    assert!(c.resolve(&id1).is_ok());
    assert_eq!(c.resolve(&id2).unwrap_err().to_string(), "Sound is not ready");
    assert_eq!(c.resolve(&id3).unwrap_err().to_string(), "Source is offline");

    // 3. annotate checks
    assert!(c.annotate(&id1, &["updated".into()], "new comment", true).is_ok());
    assert_eq!(
        c.annotate(&id2, &["updated".into()], "new comment", true).unwrap_err().to_string(),
        "Cannot annotate missing sound"
    );
    assert_eq!(
        c.annotate(&id3, &["updated".into()], "new comment", true).unwrap_err().to_string(),
        "Cannot annotate sound in offline source"
    );

    // 4. search checks: online sources list only contains available sources
    let online: Vec<String> = c
        .sources()
        .unwrap()
        .into_iter()
        .filter(|s| s.available)
        .map(|s| s.id)
        .collect();
    assert_eq!(online, vec![source_a.id.clone()]);

    let all = c.all_sounds().unwrap();
    assert_eq!(all.len(), 3);

    // Empty search query -> only ready sound 1 is returned
    let res = search(all.clone(), &SearchQuery::default(), &online).unwrap();
    assert_eq!(res.total, 1);
    assert_eq!(res.items.len(), 1);
    assert_eq!(res.items[0].id, id1);

    // Favorites query -> only ready sound 1 (even though id2 and id3 are favorited)
    let res = search(
        all.clone(),
        &SearchQuery {
            favorites_only: true,
            ..Default::default()
        },
        &online,
    )
    .unwrap();
    assert_eq!(res.total, 1);
    assert_eq!(res.items[0].id, id1);

    // Tag query -> id2's tag "tag2" returns 0 results
    let res = search(
        all.clone(),
        &SearchQuery {
            tags: vec!["tag2".into()],
            ..Default::default()
        },
        &online,
    )
    .unwrap();
    assert_eq!(res.total, 0);

    // Explicit source filter targeting offline source_b returns 0 results
    let res = search(
        all,
        &SearchQuery {
            source_ids: vec![source_b.id],
            ..Default::default()
        },
        &online,
    )
    .unwrap();
    assert_eq!(res.total, 0);
}

#[test]
fn volume_unplug_and_restoration_lifecycle() {
    let base = tempdir().unwrap();
    let root = base.path().join("media");
    fs::create_dir(&root).unwrap();

    let mut c = test_catalog();
    let source = c.add_source(&root).unwrap();
    let (id1, _) = add_sound(&mut c, &source, &root, "sound1.wav", b"content 1");
    let (id2, _) = add_sound(&mut c, &source, &root, "sound2.wav", b"content 2");

    // Both are ready and resolvable
    assert!(c.ready_sound(&id1).is_ok());
    assert!(c.ready_sound(&id2).is_ok());
    assert!(c.resolve(&id1).is_ok());

    // Volume unplugged: mark unavailable
    c.set_available(&source.id, false).unwrap();

    // Unavailable mount is distinct from deletion:
    // Sounds remain "ready" in DB, NOT deleted, NOT marked missing!
    assert_eq!(c.sound(&id1).unwrap().status, "ready");
    assert_eq!(c.sound(&id2).unwrap().status, "ready");
    assert_eq!(c.all_sounds().unwrap().len(), 2);

    // But ready_sound and resolve fail
    assert_eq!(c.ready_sound(&id1).unwrap_err().to_string(), "Source is offline");
    assert_eq!(c.resolve(&id1).unwrap_err().to_string(), "Source is offline");

    // Search excludes offline sounds
    let online: Vec<String> = c.sources().unwrap().into_iter().filter(|s| s.available).map(|s| s.id).collect();
    let res = search(c.all_sounds().unwrap(), &SearchQuery::default(), &online).unwrap();
    assert_eq!(res.total, 0);

    // Volume re-mounted / restored: mark available again
    c.set_available(&source.id, true).unwrap();
    assert!(c.ready_sound(&id1).is_ok());
    assert!(c.resolve(&id1).is_ok());
    let online: Vec<String> = c.sources().unwrap().into_iter().filter(|s| s.available).map(|s| s.id).collect();
    let res = search(c.all_sounds().unwrap(), &SearchQuery::default(), &online).unwrap();
    assert_eq!(res.total, 2);
}

#[test]
fn reconciliation_marks_deleted_file_missing_and_resurrection_reuses_profile() {
    let base = tempdir().unwrap();
    let root = base.path().join("media");
    fs::create_dir(&root).unwrap();

    let mut c = test_catalog();
    let source = c.add_source(&root).unwrap();
    let (id_kept, _) = add_sound(&mut c, &source, &root, "kept.wav", b"keep me");
    let (id_deleted, hash_deleted) = add_sound(&mut c, &source, &root, "deleted.wav", b"delete me");

    // Reconcile with complete scan where deleted.wav is absent
    c.reconcile(&source, &["kept.wav".into()], true).unwrap();

    // kept.wav remains ready
    assert_eq!(c.sound(&id_kept).unwrap().status, "ready");
    assert!(c.ready_sound(&id_kept).is_ok());

    // deleted.wav is marked missing
    assert_eq!(c.sound(&id_deleted).unwrap().status, "missing");
    assert_eq!(c.ready_sound(&id_deleted).unwrap_err().to_string(), "Sound is not ready");

    // Absent from search
    let online: Vec<String> = c.sources().unwrap().into_iter().filter(|s| s.available).map(|s| s.id).collect();
    let res = search(c.all_sounds().unwrap(), &SearchQuery::default(), &online).unwrap();
    assert_eq!(res.total, 1);
    assert_eq!(res.items[0].id, id_kept);

    // Resurrection: file is restored with same content
    let resurrected_id = c.register(&source, "deleted.wav", &hash_deleted).unwrap();
    assert_eq!(resurrected_id, id_deleted);
    // Profile is already cached, so status is immediately ready!
    let sound = c.sound(&id_deleted).unwrap();
    assert_eq!(sound.status, "ready");
    assert!(sound.profile.is_some());
    assert!(c.ready_sound(&id_deleted).is_ok());

    // Both appear in search again
    let res = search(c.all_sounds().unwrap(), &SearchQuery::default(), &online).unwrap();
    assert_eq!(res.total, 2);
}

#[test]
fn relink_reuses_identity_with_zero_decode_jobs() {
    let base = tempdir().unwrap();
    let old_root = base.path().join("old_location");
    let new_root = base.path().join("new_location");
    fs::create_dir(&old_root).unwrap();

    let mut c = test_catalog();
    let source = c.add_source(&old_root).unwrap();
    let (id, hash) = add_sound(&mut c, &source, &old_root, "tone.wav", b"tone bytes");
    c.annotate(&id, &["relink_tag".into()], "stay intact", true).unwrap();

    // Move directory on disk
    fs::rename(&old_root, &new_root).unwrap();

    // Relink source
    let relinked = c.relink(&source.id, &new_root).unwrap();
    assert_eq!(relinked.generation, 1);
    assert!(relinked.available);

    // Profile remains cached and register yields the same ID
    let same_id = c.register(&relinked, "tone.wav", &hash).unwrap();
    assert_eq!(same_id, id);
    let cached = c.cached_profile(&hash).unwrap();
    assert_eq!(cached, Some(sample_profile()));

    // Sound metadata survived relink
    let sound = c.ready_sound(&id).unwrap();
    assert_eq!(sound.user_tags, vec!["relink tag"]);
    assert_eq!(sound.comment, "stay intact");
    assert!(sound.favorite);

    // Resolves to new location
    let resolved = c.resolve(&id).unwrap();
    assert_eq!(resolved, new_root.join("tone.wav").canonicalize().unwrap());
}

#[test]
fn wrong_relink_destination_is_atomic() {
    let base = tempdir().unwrap();
    let old_root = base.path().join("source");
    let bad_root = base.path().join("bad_dest");
    fs::create_dir(&old_root).unwrap();
    fs::create_dir(&bad_root).unwrap();

    let mut c = test_catalog();
    let source = c.add_source(&old_root).unwrap();
    let (id, _) = add_sound(&mut c, &source, &old_root, "tone.wav", b"original");

    // Write different content in bad_root
    fs::write(bad_root.join("tone.wav"), b"modified content").unwrap();

    // Relink must fail atomically
    let err = c.relink(&source.id, &bad_root).unwrap_err();
    assert_eq!(err.to_string(), "Replacement differs from this library. No changes made.");

    // Source generation and path untouched
    let current_source = c.source(&source.id).unwrap();
    assert_eq!(current_source.generation, 0);
    assert_eq!(current_source.root, old_root.canonicalize().unwrap().to_str().unwrap());
    assert!(c.resolve(&id).is_ok());

    // Relinking to a file instead of directory fails
    let file_path = bad_root.join("tone.wav");
    let err = c.relink(&source.id, &file_path).unwrap_err();
    assert_eq!(err.to_string(), "Replacement must be a folder");
}

#[test]
fn one_changed_file_alone_reanalyzed() {
    let base = tempdir().unwrap();
    let root = base.path().join("multi");
    fs::create_dir(&root).unwrap();

    let mut c = test_catalog();
    let source = c.add_source(&root).unwrap();
    let (id1, h1) = add_sound(&mut c, &source, &root, "file1.wav", b"file 1 audio");
    let (id2, _) = add_sound(&mut c, &source, &root, "file2.wav", b"file 2 audio");
    c.annotate(&id2, &["keep_user_tag".into()], "my annotation", true).unwrap();

    // Modify file2 only
    let new_h2 = "new_hash_file2";
    let reg_id2 = c.register(&source, "file2.wav", new_h2).unwrap();
    assert_eq!(reg_id2, id2);

    // file1 is unchanged and cached
    assert!(c.cached_profile(&h1).unwrap().is_some());
    assert_eq!(c.sound(&id1).unwrap().status, "ready");

    // file2 is pending analysis for new hash
    let s2 = c.sound(&id2).unwrap();
    assert_eq!(s2.status, "pending");
    assert!(s2.profile.is_none());
    // Annotations are preserved!
    assert_eq!(s2.user_tags, vec!["keep user tag"]);
    assert_eq!(s2.comment, "my annotation");
    assert!(s2.favorite);

    // While pending, ready_sound and resolve reject
    assert_eq!(c.ready_sound(&id2).unwrap_err().to_string(), "Sound is not ready");
    assert_eq!(c.resolve(&id2).unwrap_err().to_string(), "Sound is not ready");

    // Publish new profile for file2
    c.publish(&source, &id2, new_h2, &sample_profile()).unwrap();
    assert_eq!(c.sound(&id2).unwrap().status, "ready");
    assert!(c.ready_sound(&id2).is_ok());
    assert!(c.resolve(&id2).is_ok());
}

#[test]
fn partial_scan_never_mass_deletes() {
    let base = tempdir().unwrap();
    let root = base.path().join("partial");
    fs::create_dir(&root).unwrap();

    let mut c = test_catalog();
    let source = c.add_source(&root).unwrap();
    let (id1, _) = add_sound(&mut c, &source, &root, "a.wav", b"a");
    let (id2, _) = add_sound(&mut c, &source, &root, "b.wav", b"b");

    // Incomplete scan: complete = false, seen = []
    c.reconcile(&source, &[], false).unwrap();

    // Neither sound marked missing!
    assert_eq!(c.sound(&id1).unwrap().status, "ready");
    assert_eq!(c.sound(&id2).unwrap().status, "ready");
    assert!(c.ready_sound(&id1).is_ok());
    assert!(c.ready_sound(&id2).is_ok());
}

#[test]
fn stale_worker_scan_rejection() {
    let base = tempdir().unwrap();
    let old_root = base.path().join("old");
    let new_root = base.path().join("new");
    fs::create_dir(&old_root).unwrap();

    let mut c = test_catalog();
    let stale_source = c.add_source(&old_root).unwrap();
    let (id, h) = add_sound(&mut c, &stale_source, &old_root, "sound.wav", b"sound");

    // Move and relink (increments generation to 1)
    fs::rename(&old_root, &new_root).unwrap();
    let _ = c.relink(&stale_source.id, &new_root).unwrap();

    // Worker with stale_source (generation 0) attempts actions:
    let err_reg = c.register(&stale_source, "sound.wav", &h).unwrap_err();
    assert_eq!(err_reg.to_string(), "Source moved; discard stale scan");

    let err_pub = c.publish(&stale_source, &id, &h, &sample_profile()).unwrap_err();
    assert_eq!(err_pub.to_string(), "Media or source changed during analysis");

    let err_rec = c.reconcile(&stale_source, &["sound.wav".into()], true).unwrap_err();
    assert_eq!(err_rec.to_string(), "Source moved; discard stale scan");
}

#[test]
fn overlapping_roots_rejected_at_add_and_relink() {
    let base = tempdir().unwrap();
    let root_a = base.path().join("source_a");
    let nested = root_a.join("nested");
    let root_b = base.path().join("source_b");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir(&root_b).unwrap();

    let mut c = test_catalog();
    let source_a = c.add_source(&root_a).unwrap();
    let source_b = c.add_source(&root_b).unwrap();

    // Adding nested under existing source is rejected
    assert!(c.add_source(&nested).is_err());

    // Adding duplicate root is idempotent
    assert_eq!(c.add_source(&root_a).unwrap().id, source_a.id);

    // Relinking source_b to overlap source_a is rejected
    let err = c.relink(&source_b.id, &nested).unwrap_err();
    assert_eq!(err.to_string(), "Replacement overlaps another source");
    assert_eq!(c.source(&source_b.id).unwrap().root, root_b.canonicalize().unwrap().to_str().unwrap());
}

#[test]
fn paths_and_symlink_security() {
    for bad in ["", "../secret", "/tmp/a", "a/../b", "a//b", "C:\\x", "a\\b", "./a"] {
        assert!(valid_relative(bad).is_err(), "Should reject {bad}");
    }
    assert!(valid_relative("valid/sub/path.wav").is_ok());

    #[cfg(unix)]
    {
        let base = tempdir().unwrap();
        let root = base.path().join("root");
        let outside = base.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.wav"), b"secret").unwrap();

        std::os::unix::fs::symlink(outside.join("secret.wav"), root.join("link.wav")).unwrap();
        assert!(contained(&root, "link.wav").is_err());
    }
}
