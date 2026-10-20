//! SS-047: refresh, listed checks, relink, moves and watcher batches against
//! real catalogs, the real image worker and real files. No original is written.
use image::ImageEncoder;
use soundshelf_core::{
    catalog::Catalog,
    image_contracts::{Availability, TechnicalState},
    image_import::ImageImportEngine,
    image_import_preview::{MediaImportPreviews, NativeSelectionKind},
    image_jobs::{ImportError, ImportMode, ImportState, JobOrigin, ScanKind},
    image_relink::ImageRelinks,
    image_sources::{probe_image_source, refresh_image_source, RootState, WatchBatch},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};
static SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    temp: tempfile::TempDir,
    db: Arc<Mutex<Catalog>>,
    root: PathBuf,
    previews: MediaImportPreviews,
    engine: ImageImportEngine,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = dir(temp.path().join("media"));
        let db = Arc::new(Mutex::new(
            Catalog::open(&temp.path().join("db.sqlite")).unwrap(),
        ));
        let engine = ImageImportEngine {
            catalog: db.clone(),
            worker: PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker")),
            tools: None,
        };
        Self {
            temp,
            db,
            root,
            previews: MediaImportPreviews::default(),
            engine,
        }
    }
    fn import(&self, paths: &[PathBuf], kind: NativeSelectionKind, mode: ImportMode) -> String {
        let raw = self.previews.approve_native(paths, kind).unwrap();
        let preview = self
            .previews
            .prepare(&self.db, &uuid::Uuid::new_v4().to_string(), raw, mode)
            .unwrap();
        let job = self.previews.confirm(&self.db, &preview.id).unwrap()[0].clone();
        self.finish(&job.id);
        job.source_id
    }
    fn finish(&self, id: &str) -> soundshelf_core::image_jobs::ImageImportJob {
        for _ in 0..400 {
            let job = self.db.lock().unwrap().image_import_job(id).unwrap();
            if job.terminal() {
                return job;
            }
            self.engine.tick("test", &AtomicBool::new(false)).unwrap();
        }
        panic!("job did not finish")
    }
    fn refresh(&self, source: &str, scan: ScanKind, verify: bool) -> soundshelf_core::image_jobs::ImageImportJob {
        let job = refresh_image_source(&self.db, source, scan, verify).unwrap();
        assert_eq!(job.origin, JobOrigin::Refresh);
        self.finish(&job.id)
    }
    fn asset(&self, source: &str, path: &str) -> Option<soundshelf_core::image_contracts::ImageAsset> {
        let c = self.db.lock().unwrap();
        let id: Option<String> = c
            .db_connection()
            .query_row(
                "SELECT id FROM asset_registry WHERE source_id=?1 AND relative_path=?2",
                [source, path],
                |r| r.get(0),
            )
            .ok();
        id.map(|id| c.image(&id, None).unwrap())
    }
    fn annotate(&self, id: &str, comment: &str) {
        let mut c = self.db.lock().unwrap();
        let image = c.image(id, None).unwrap();
        let mut notes = image.annotations.clone();
        notes.comment = comment.into();
        c.set_image_annotations(id, image.header.content_hash.as_deref(), &notes)
            .unwrap();
    }
}
fn dir(path: PathBuf) -> PathBuf {
    fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}
fn picture(path: &Path, color: u8) {
    let mut b = vec![];
    image::codecs::png::PngEncoder::new(&mut b)
        .write_image(&[color, 64, 128, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    fs::write(path, b).unwrap();
}
/// Stat evidence recorded within 2 s of a write is deliberately untrusted.
fn settle() {
    std::thread::sleep(Duration::from_millis(2100));
}
fn hashes(root: &Path) -> BTreeMap<PathBuf, String> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path().to_owned(),
                blake3::hash(&fs::read(e.path()).unwrap()).to_hex().to_string(),
            )
        })
        .collect()
}

#[test]
fn quick_check_skips_unchanged_bytes_and_exact_verification_catches_what_stats_cannot() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    picture(&f.root.join("a.png"), 10);
    picture(&f.root.join("b.png"), 20);
    settle();
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders, ImportMode::Images);
    let a = f.asset(&source, "a.png").unwrap();
    f.annotate(&a.header.id, "keep me");
    let originals = hashes(&f.root);

    let quick = f.refresh(&source, ScanKind::Listed, false);
    assert_eq!((quick.state, quick.ready, quick.reused), (ImportState::Complete, 2, 2));
    assert!(!quick.verify && quick.scan == ScanKind::Listed);

    // Rewrite a.png with different bytes of the same size and timestamp.
    let meta = fs::metadata(f.root.join("a.png")).unwrap();
    picture(&f.root.join("a.png"), 11);
    assert_eq!(fs::metadata(f.root.join("a.png")).unwrap().len(), meta.len());
    fs::File::options()
        .write(true)
        .open(f.root.join("a.png"))
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(meta.modified().unwrap()))
        .unwrap();
    #[cfg(unix)]
    {
        // A filesystem that also hides ctime (some network/FUSE mounts): forge the
        // stored evidence to equal the new stat, so only bytes can tell them apart.
        use std::os::unix::fs::MetadataExt;
        let now = fs::metadata(f.root.join("a.png")).unwrap();
        f.db.lock().unwrap().db_connection().execute(
            "UPDATE asset_file_stats SET changed=?2,modified=?3,recorded=?4 WHERE asset_id=?1",
            rusqlite::params![
                a.header.id,
                now.ctime() * 1_000_000_000 + now.ctime_nsec(),
                now.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as i64,
                i64::MAX
            ],
        ).unwrap();
        let blind = f.refresh(&source, ScanKind::Listed, false);
        assert_eq!(blind.reused, 2, "a stat-only check cannot see same-stat byte changes");
        assert_eq!(f.asset(&source, "a.png").unwrap().header.content_hash, a.header.content_hash);
    }
    #[cfg(not(unix))]
    {
        // ChangeTime moved, so even the quick check rehashes; still exact below.
        let _ = f.refresh(&source, ScanKind::Listed, false);
    }

    let exact = f.refresh(&source, ScanKind::Listed, true);
    assert_eq!((exact.state, exact.ready), (ImportState::Complete, 2));
    let changed = f.asset(&source, "a.png").unwrap();
    assert_eq!(changed.header.id, a.header.id);
    assert_ne!(changed.header.content_hash, a.header.content_hash);
    assert_eq!(changed.header.technical_state, TechnicalState::Ready);
    assert_eq!(changed.annotations.comment, "keep me");
    let versions: i64 = f.db.lock().unwrap().db_connection().query_row(
        "SELECT count(*) FROM image_asset_versions WHERE asset_id=?1", [&a.header.id], |r| r.get(0)).unwrap();
    assert_eq!(versions, 2, "previous content version is retained for recovery");
    // The previous version's analysis is immutable and still addressable.
    assert!(f.db.lock().unwrap()
        .cached_image_profile(a.header.content_hash.as_deref().unwrap(), &a.profile.as_ref().unwrap().analyzer_version)
        .unwrap().is_some());
    assert!(f.db.lock().unwrap().image(&a.header.id, a.header.content_hash.as_deref()).is_err(),
        "the old version is no longer current, so version-bound previews are refused");
    // Only a.png changed, and only because this test rewrote it.
    let after = hashes(&f.root);
    assert_eq!(after.len(), originals.len());
    assert_eq!(after[&f.root.join("b.png")], originals[&f.root.join("b.png")]);
}

#[test]
fn listed_check_is_scoped_to_members_and_keeps_missing_distinct_from_decode_failure() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    picture(&f.root.join("chosen.png"), 1);
    picture(&f.root.join("sibling.png"), 2);
    let source = f.import(&[f.root.join("chosen.png")], NativeSelectionKind::Files, ImportMode::Images);
    let chosen = f.asset(&source, "chosen.png").unwrap();
    assert_eq!(
        refresh_image_source(&f.db, &source, ScanKind::Discover, false).unwrap_err(),
        ImportError::InvalidArgument,
        "exact-file sources have no discovery scope"
    );
    let check = f.refresh(&source, ScanKind::Listed, true);
    assert_eq!((check.total, check.ready), (1, 1));
    assert!(f.asset(&source, "sibling.png").is_none(), "siblings are never admitted");

    fs::remove_file(f.root.join("chosen.png")).unwrap();
    let check = f.refresh(&source, ScanKind::Listed, false);
    assert_eq!((check.total, check.failed), (1, 1));
    let missing = f.asset(&source, "chosen.png").unwrap();
    assert_eq!(missing.header.availability, Availability::Missing);
    assert_eq!(missing.header.technical_state, TechnicalState::Ready, "absence is not a decode failure");
    assert_eq!(missing.profile, chosen.profile);

    fs::write(f.root.join("chosen.png"), b"not an image").unwrap();
    f.refresh(&source, ScanKind::Listed, true);
    let broken = f.asset(&source, "chosen.png").unwrap();
    assert_eq!(broken.header.availability, Availability::Available);
    assert_eq!(broken.header.technical_state, TechnicalState::Failed);
    let status = f.db.lock().unwrap().image_source_status(&source).unwrap();
    assert_eq!((status.images, status.missing, status.failed), (1, 0, 1));
}

#[test]
fn offline_partial_denied_and_replaced_roots_never_mass_mark_missing() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    fs::create_dir(f.root.join("nested")).unwrap();
    picture(&f.root.join("a.png"), 1);
    picture(&f.root.join("nested/b.png"), 2);
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders, ImportMode::Images);
    let parked = f.temp.path().join("parked");

    // Drive disconnected: offline, not missing, and nothing is queued.
    fs::rename(&f.root, &parked).unwrap();
    let probe = probe_image_source(&f.db, &source).unwrap();
    assert_eq!(probe.root_state, RootState::Offline);
    assert_eq!(refresh_image_source(&f.db, &source, ScanKind::Discover, false).unwrap_err(), ImportError::SourceUnavailable);
    assert_eq!(f.asset(&source, "a.png").unwrap().header.availability, Availability::Offline);
    assert_eq!(f.db.lock().unwrap().image_source_status(&source).unwrap().missing, 0);

    // Reconnected with the same identity: available again, flagged for refresh.
    fs::rename(&parked, &f.root).unwrap();
    let probe = probe_image_source(&f.db, &source).unwrap();
    assert!(probe.reconnected);
    assert_eq!(probe.root_state, RootState::Online);
    assert!(!probe_image_source(&f.db, &source).unwrap().reconnected);

    #[cfg(unix)]
    if unsafe { libc::geteuid() } != 0 {
        use std::os::unix::fs::PermissionsExt;
        // An unreadable subfolder makes discovery partial: its members stay put.
        fs::set_permissions(f.root.join("nested"), fs::Permissions::from_mode(0o000)).unwrap();
        fs::remove_file(f.root.join("a.png")).unwrap();
        let partial = f.refresh(&source, ScanKind::Discover, false);
        fs::set_permissions(f.root.join("nested"), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!partial.discovery_complete);
        assert_eq!(partial.error_code, Some(ImportError::DiscoveryIncomplete));
        assert_eq!(f.asset(&source, "nested/b.png").unwrap().header.availability, Availability::Available);
        assert_eq!(f.asset(&source, "a.png").unwrap().header.availability, Availability::Available,
            "missing members are only inferred from a complete scan");
        // Root access denied is its own state, not offline.
        let guard = f.temp.path().join("guard");
        fs::create_dir(&guard).unwrap();
        fs::rename(&f.root, guard.join("media")).unwrap();
        std::os::unix::fs::symlink(guard.join("media"), &f.root).unwrap();
        fs::set_permissions(&guard, fs::Permissions::from_mode(0o000)).unwrap();
        let denied = probe_image_source(&f.db, &source).unwrap();
        fs::set_permissions(&guard, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_file(&f.root).unwrap();
        fs::rename(guard.join("media"), &f.root).unwrap();
        assert_eq!(denied.root_state, RootState::Inaccessible);
        picture(&f.root.join("a.png"), 1);
    }
    // A complete scan confirms a real deletion.
    fs::remove_file(f.root.join("a.png")).unwrap();
    let full = f.refresh(&source, ScanKind::Discover, false);
    assert!(full.discovery_complete);
    assert_eq!(f.asset(&source, "a.png").unwrap().header.availability, Availability::Missing);
    assert_eq!(f.asset(&source, "nested/b.png").unwrap().header.availability, Availability::Available);

    // A different folder at the same path is a changed identity: relink required.
    fs::rename(&f.root, &parked).unwrap();
    fs::create_dir(&f.root).unwrap();
    let replaced = probe_image_source(&f.db, &source).unwrap();
    assert_eq!(replaced.root_state, RootState::Changed);
    assert_eq!(refresh_image_source(&f.db, &source, ScanKind::Discover, false).unwrap_err(), ImportError::SourceChanged);
    assert_eq!(f.asset(&source, "nested/b.png").unwrap().header.availability, Availability::Offline);
}

#[test]
fn renames_keep_identity_but_copies_and_hard_links_need_explicit_reconciliation() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    picture(&f.root.join("a.png"), 5);
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders, ImportMode::Images);
    let id = f.asset(&source, "a.png").unwrap().header.id;
    f.annotate(&id, "creator work");

    fs::rename(f.root.join("a.png"), f.root.join("renamed.png")).unwrap();
    f.refresh(&source, ScanKind::Discover, false);
    let moved = f.asset(&source, "renamed.png").unwrap();
    assert_eq!(moved.header.id, id);
    assert_eq!(moved.annotations.comment, "creator work");
    assert!(f.asset(&source, "a.png").is_none());

    // Case-only rename: the same OS file under a new spelling keeps its identity
    // on case-insensitive and case-sensitive filesystems alike.
    fs::rename(f.root.join("renamed.png"), f.root.join("RENAMED.png")).unwrap();
    f.refresh(&source, ScanKind::Discover, false);
    assert_eq!(f.asset(&source, "RENAMED.png").unwrap().header.id, id);
    assert!(f.asset(&source, "renamed.png").is_none());

    // A hard link is the same file at two live paths: never adopted.
    fs::hard_link(f.root.join("RENAMED.png"), f.root.join("link.png")).unwrap();
    f.refresh(&source, ScanKind::Discover, false);
    assert_eq!(f.asset(&source, "RENAMED.png").unwrap().header.id, id);
    let link = f.asset(&source, "link.png").unwrap();
    assert_ne!(link.header.id, id);
    fs::remove_file(f.root.join("link.png")).unwrap();
    {
        let mut c = f.db.lock().unwrap();
        let s = c.source(&source).unwrap();
        c.delete_image(&s, &link.header.id, link.header.content_hash.as_deref(), link.annotations.revision).unwrap();
    }

    // A copy has equal bytes but is a different file: no content-hash merge.
    fs::copy(f.root.join("RENAMED.png"), f.root.join("copy.png")).unwrap();
    fs::remove_file(f.root.join("RENAMED.png")).unwrap();
    f.refresh(&source, ScanKind::Discover, false);
    let copy = f.asset(&source, "copy.png").unwrap();
    assert_ne!(copy.header.id, id);
    assert_eq!(f.asset(&source, "RENAMED.png").unwrap().header.availability, Availability::Missing);
    let candidates = f.db.lock().unwrap().image_move_candidates(&source).unwrap();
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!((candidate.missing_id.as_str(), candidate.present_id.as_str()), (id.as_str(), copy.header.id.as_str()));
    assert!(!candidate.same_file);
    assert_eq!(f.db.lock().unwrap().image_source_status(&source).unwrap().move_candidates, 1);

    // Optimistic revisions: a stale or annotated duplicate is a conflict.
    assert_eq!(
        f.db.lock().unwrap().reconcile_image_move(&id, &copy.header.id, 1, 1).unwrap_err(),
        ImportError::Conflict
    );
    f.annotate(&copy.header.id, "duplicate work");
    let present_revision = f.asset(&source, "copy.png").unwrap().annotations.revision;
    assert_eq!(
        f.db.lock().unwrap().reconcile_image_move(&id, &copy.header.id, candidate.missing_revision, present_revision).unwrap_err(),
        ImportError::Conflict,
        "never discard annotations on the other record"
    );
    f.annotate(&copy.header.id, "");
    let present_revision = f.asset(&source, "copy.png").unwrap().annotations.revision;
    let merged = f.db.lock().unwrap()
        .reconcile_image_move(&id, &copy.header.id, candidate.missing_revision, present_revision)
        .unwrap();
    assert_eq!(merged.header.id, id);
    assert_eq!(merged.header.relative_path, "copy.png");
    assert_eq!(merged.header.availability, Availability::Available);
    assert_eq!(merged.annotations.comment, "creator work");
    assert!(f.db.lock().unwrap().image(&copy.header.id, None).is_err());
    assert!(f.db.lock().unwrap().image_move_candidates(&source).unwrap().is_empty());
    // The merged record keeps working with the stat fast path.
    let next = f.refresh(&source, ScanKind::Listed, true);
    assert_eq!((next.ready, next.failed), (1, 0));
}

#[test]
fn relink_keeps_ids_and_analysis_discloses_changes_and_rejects_ambiguous_targets() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut f = Fixture::new();
    let tools = soundshelf_core::media::MediaTools::discover().unwrap();
    assert!(std::process::Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=0.1"])
        .arg(f.root.join("voice.wav"))
        .status()
        .unwrap()
        .success());
    f.engine.tools = Some(tools);
    fs::create_dir(f.root.join("set")).unwrap();
    picture(&f.root.join("set/a.png"), 1);
    picture(&f.root.join("set/b.png"), 2);
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders, ImportMode::Mixed);
    let a = f.asset(&source, "set/a.png").unwrap();
    let b = f.asset(&source, "set/b.png").unwrap();
    f.annotate(&a.header.id, "a notes");
    f.annotate(&b.header.id, "b notes");
    let sound = f.db.lock().unwrap().all_sounds().unwrap()[0].clone();
    let relinks = ImageRelinks::default();
    let copy = |name: &str| {
        let target = f.temp.path().join(name);
        for entry in walkdir::WalkDir::new(&f.root).into_iter().flatten() {
            let to = target.join(entry.path().strip_prefix(&f.root).unwrap());
            if entry.file_type().is_dir() {
                fs::create_dir_all(&to).unwrap();
            } else {
                fs::copy(entry.path(), &to).unwrap();
            }
        }
        target.canonicalize().unwrap()
    };
    let identical = copy("identical");
    let changed = copy("changed");
    picture(&changed.join("set/b.png"), 99);
    let partial = copy("partial");
    fs::remove_file(partial.join("set/a.png")).unwrap();
    let empty = f.temp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let before = [&identical, &changed].map(|p| hashes(p));
    let no = AtomicBool::new(false);

    assert_eq!(relinks.prepare(&f.db, &source, &empty, &no).unwrap_err(), ImportError::NoMatchingFiles);
    let other = f.temp.path().join("other");
    fs::create_dir(&other).unwrap();
    picture(&other.join("x.png"), 3);
    f.import(&[other.clone()], NativeSelectionKind::Folders, ImportMode::Images);
    let nested = other.join("inside");
    fs::create_dir(&nested).unwrap();
    assert_eq!(relinks.prepare(&f.db, &source, &nested, &no).unwrap_err(), ImportError::SourceOverlap);
    assert_eq!(relinks.prepare(&f.db, &source, f.temp.path(), &no).unwrap_err(), ImportError::SourceOverlap);

    // Partial target: absent members are disclosed and must be acknowledged.
    let preview = relinks.prepare(&f.db, &source, &partial, &no).unwrap();
    assert_eq!((preview.members, preview.present, preview.absent), (3, 2, 1));
    assert!(preview.acknowledgement_required);
    assert_eq!(relinks.confirm(&f.db, &preview.id, false).unwrap_err(), ImportError::InvalidArgument);
    // A preview cannot be confirmed against a folder swapped in since.
    fs::rename(&partial, f.temp.path().join("partial-moved")).unwrap();
    fs::create_dir(&partial).unwrap();
    assert_eq!(relinks.confirm(&f.db, &preview.id, true).unwrap_err(), ImportError::SourceChanged);

    // Identical copy: same IDs, analysis reused without decoding, audio ID kept.
    let preview = relinks.prepare(&f.db, &source, &identical, &no).unwrap();
    assert_eq!((preview.present, preview.absent, preview.matched, preview.changed), (3, 0, 2, 0));
    assert!(!preview.acknowledgement_required);
    let job = relinks.confirm(&f.db, &preview.id, false).unwrap();
    assert_eq!((job.scan, job.verify, job.origin, job.source_generation.as_str()), (ScanKind::Listed, true, JobOrigin::Relink, "1"));
    let job = f.finish(&job.id);
    assert_eq!((job.state, job.ready, job.reused), (ImportState::Complete, 3, 3));
    assert_eq!(f.asset(&source, "set/a.png").unwrap().header.id, a.header.id);
    assert_eq!(f.asset(&source, "set/a.png").unwrap().annotations.comment, "a notes");
    assert_eq!(f.asset(&source, "set/a.png").unwrap().header.content_hash, a.header.content_hash);
    let sounds = f.db.lock().unwrap().all_sounds().unwrap();
    assert_eq!((sounds.len(), sounds[0].id.as_str(), sounds[0].status.as_str()), (1, sound.id.as_str(), "ready"));
    assert_eq!(f.db.lock().unwrap().source(&source).unwrap().root, identical.to_string_lossy());

    // Changed copy: one changed member is disclosed, reanalyzed, annotations kept.
    let preview = relinks.prepare(&f.db, &source, &changed, &no).unwrap();
    assert_eq!((preview.matched, preview.changed), (1, 1));
    let job = f.finish(&relinks.confirm(&f.db, &preview.id, true).unwrap().id);
    assert_eq!((job.ready, job.reused), (3, 2));
    let new_b = f.asset(&source, "set/b.png").unwrap();
    assert_eq!(new_b.header.id, b.header.id);
    assert_ne!(new_b.header.content_hash, b.header.content_hash);
    assert_eq!(new_b.annotations.comment, "b notes");
    assert_eq!(new_b.header.technical_state, TechnicalState::Ready);
    assert_eq!(before, [&identical, &changed].map(|p| hashes(p)), "relink never writes originals");
}

#[test]
fn generation_changes_fence_queued_refresh_and_stale_watch_batches() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    picture(&f.root.join("a.png"), 1);
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders, ImportMode::Images);
    f.db.lock().unwrap().set_image_source_watch(&source, true).unwrap();
    let scope = f.db.lock().unwrap().image_watch_scopes().unwrap().remove(0);
    let queued = refresh_image_source(&f.db, &source, ScanKind::Discover, false).unwrap();
    // One active job per source generation; the watcher must wait, not duplicate.
    assert_eq!(
        f.db.lock().unwrap().enqueue_watch_batch(&scope, &WatchBatch::Changes { files: vec!["a.png".into()], folders: vec![], removed: vec![] }).unwrap_err(),
        ImportError::SourceConflict
    );
    let target = f.temp.path().join("moved");
    fs::rename(&f.root, &target).unwrap();
    let relinks = ImageRelinks::default();
    let preview = relinks.prepare(&f.db, &source, &target, &AtomicBool::new(false)).unwrap();
    let relink = relinks.confirm(&f.db, &preview.id, false).unwrap();
    let old = f.db.lock().unwrap().image_import_job(&queued.id).unwrap();
    assert_eq!((old.state, old.error_code), (ImportState::Cancelled, Some(ImportError::SourceChanged)));
    assert_eq!(
        f.db.lock().unwrap().enqueue_watch_batch(&scope, &WatchBatch::Refresh).unwrap_err(),
        ImportError::SourceChanged,
        "a batch observed under the old root cannot act on the new generation"
    );
    assert_eq!(f.finish(&relink.id).state, ImportState::Complete);
    assert_eq!(f.db.lock().unwrap().image_watch_scopes().unwrap()[0].generation, 1);
}

#[test]
fn watch_batches_stay_in_scope_skip_app_owned_paths_and_escalate_safely() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("album/deep")).unwrap();
    fs::create_dir_all(f.root.join(".soundshelf-export-1234/media")).unwrap();
    fs::create_dir_all(f.root.join(".creativeshelf")).unwrap();
    picture(&f.root.join("album/deep/a.png"), 1);
    picture(&f.root.join(".soundshelf-export-1234/media/out.png"), 2);
    picture(&f.root.join(".creativeshelf/cache.png"), 3);
    picture(&f.root.join("half.png.part"), 4);
    let folder = f.import(&[f.root.clone()], NativeSelectionKind::Folders, ImportMode::Images);
    let status = f.db.lock().unwrap().image_source_status(&folder).unwrap();
    assert_eq!(status.images, 1, "export staging, sidecars and partial downloads are not members");
    assert!(!status.watch, "importing never enables watching");
    assert!(f.db.lock().unwrap().image_watch_scopes().unwrap().is_empty());

    let exact_dir = dir(f.temp.path().join("exact"));
    picture(&exact_dir.join("only.png"), 5);
    picture(&exact_dir.join("sibling.png"), 6);
    let exact = f.import(&[exact_dir.join("only.png")], NativeSelectionKind::Files, ImportMode::Images);
    for id in [&folder, &exact] {
        f.db.lock().unwrap().set_image_source_watch(id, true).unwrap();
    }
    let scopes = f.db.lock().unwrap().image_watch_scopes().unwrap();
    let scope = |id: &str| scopes.iter().find(|s| s.source_id == id).unwrap().clone();
    let (folder_scope, exact_scope) = (scope(&folder), scope(&exact));
    assert_eq!(exact_scope.relative(&exact_dir.join("only.png")).as_deref(), Some("only.png"));
    assert_eq!(exact_scope.relative(&exact_dir.join("sibling.png")), None, "exact-file watching never admits siblings");
    for ignored in ["a.png.tmp", ".creativeshelf/x.png", ".soundshelf-export-9/media/x.png", "~$draft.png", "._a.png", "album/.DS_Store"] {
        assert_eq!(folder_scope.relative(&f.root.join(ignored)), None, "{ignored}");
    }
    assert_eq!(folder_scope.relative(&f.temp.path().join("outside.png")), None);

    let c = f.db.lock().unwrap();
    let sibling = WatchBatch::Changes { files: vec!["sibling.png".into()], folders: vec!["dir".into()], removed: vec!["gone.png".into()] };
    assert_eq!(c.enqueue_watch_batch(&exact_scope, &sibling).unwrap(), None);
    let job = c.enqueue_watch_batch(&folder_scope, &WatchBatch::Changes {
        files: vec!["new.png".into(), "half.png.part".into()],
        folders: vec!["album".into(), "album/deep".into()],
        removed: vec!["never-cataloged.png".into()],
    }).unwrap().unwrap();
    assert_eq!((job.origin, job.verify, job.scan), (JobOrigin::Watch, false, ScanKind::Discover));
    c.cancel_image_import(&job.id, 0).unwrap();
    // A deleted folder that held members can only be reconciled by a full scan.
    let job = c.enqueue_watch_batch(&folder_scope, &WatchBatch::Changes { files: vec![], folders: vec![], removed: vec!["album".into()] }).unwrap().unwrap();
    c.cancel_image_import(&job.id, 0).unwrap();
    let wide = WatchBatch::Changes { files: (0..300).map(|i| format!("{i}.png")).collect(), folders: vec![], removed: vec![] };
    let refresh = c.enqueue_watch_batch(&folder_scope, &wide).unwrap().unwrap();
    c.cancel_image_import(&refresh.id, 0).unwrap();
    let refresh_exact = c.enqueue_watch_batch(&exact_scope, &WatchBatch::Refresh).unwrap().unwrap();
    assert_eq!(refresh_exact.scan, ScanKind::Listed);
    let jobs = c.image_import_jobs(None, 50).unwrap();
    drop(c);
    assert_eq!(jobs.iter().filter(|j| j.origin == JobOrigin::Watch && !j.verify).count(), 4);
    f.finish(&refresh_exact.id);
    assert!(f.asset(&exact, "sibling.png").is_none());
}

#[test]
fn watch_preference_and_root_state_persist_and_tampered_backups_are_rejected() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    picture(&f.root.join("a.png"), 1);
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders, ImportMode::Images);
    f.db.lock().unwrap().set_image_source_watch(&source, true).unwrap();
    probe_image_source(&f.db, &source).unwrap();
    let backup = f.temp.path().join("backup.sqlite");
    f.db.lock().unwrap().backup_database(&backup).unwrap();
    f.db.lock().unwrap().set_image_source_watch(&source, false).unwrap();
    f.db.lock().unwrap().restore_database(&backup, &f.temp.path().join("rollback.sqlite")).unwrap();
    let status = f.db.lock().unwrap().image_source_status(&source).unwrap();
    assert!(status.watch);
    assert_eq!(status.root_state, RootState::Online);
    let stats: i64 = f.db.lock().unwrap().db_connection().query_row("SELECT count(*) FROM asset_file_stats", [], |r| r.get(0)).unwrap();
    assert_eq!(stats, 1);

    for tamper in [
        "UPDATE image_source_monitor SET root_state='elsewhere'",
        "UPDATE asset_file_stats SET file_id='../x'",
        "UPDATE image_import_jobs SET origin='agent'",
    ] {
        let bad = f.temp.path().join("bad.sqlite");
        let _ = fs::remove_file(&bad);
        f.db.lock().unwrap().backup_database(&bad).unwrap();
        let db = rusqlite::Connection::open(&bad).unwrap();
        db.execute_batch(&format!("PRAGMA ignore_check_constraints=1; {tamper};")).unwrap();
        drop(db);
        let rollback = f.temp.path().join("unused.sqlite");
        assert!(f.db.lock().unwrap().restore_database(&bad, &rollback).is_err(), "{tamper}");
        assert!(!rollback.exists());
    }
    let reopened = Catalog::open(&f.temp.path().join("db.sqlite")).unwrap();
    assert!(reopened.image_source_status(&source).unwrap().watch, "survives restart");
}

#[test]
fn listed_check_of_100k_members_discovers_within_one_bounded_quantum() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    picture(&f.root.join("a.png"), 1);
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders, ImportMode::Images);
    {
        let c = f.db.lock().unwrap();
        let db = c.db_connection();
        db.execute_batch("BEGIN").unwrap();
        db.execute("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<99999)
            INSERT INTO asset_registry(id,kind,source_id,relative_path) SELECT printf('%08x-0000-4000-8000-%012x',i,i),'image',?1,printf('bulk/%06d.png',i) FROM n",
            [&source]).unwrap();
        db.execute(r#"INSERT INTO image_assets(id,technical_state,availability,enrichment_state,annotation_revision,annotations)
            SELECT id,'pending','available','disabled',1,'{"revision":1,"tags":[],"comment":"","favorite":false,"rating":null,"caption":null}'
            FROM asset_registry WHERE relative_path GLOB 'bulk/*'"#, []).unwrap();
        db.execute_batch("COMMIT").unwrap();
    }
    let job = refresh_image_source(&f.db, &source, ScanKind::Listed, false).unwrap();
    let start = std::time::Instant::now();
    f.engine.tick("test", &AtomicBool::new(false)).unwrap();
    let elapsed = start.elapsed();
    let job = f.db.lock().unwrap().image_import_job(&job.id).unwrap();
    eprintln!("listed discovery of {} members: {elapsed:?}", job.total);
    assert_eq!((job.total, job.discovery_complete, job.error_code), (100_000, true, None));
    assert!(elapsed < soundshelf_core::image_import_preview::DISCOVERY_DEADLINE);
    f.db.lock().unwrap().cancel_image_import(&job.id, 0).unwrap();
    assert_eq!(f.db.lock().unwrap().image_source_status(&source).unwrap().images, 100_000);
}
