//! Bounded, resumable import lane shared by native adapters. No original writes.
use crate::{
    catalog::{contained, Catalog, Source},
    image_analysis::{self, AnalysisError, AnalysisWarning, ImageAnalysisRequest, ImageSelection},
    image_contracts::{ImageFormat, ImageProfile},
    image_import_preview::{candidate, RootStamp, DISCOVERY_DEADLINE, MAX_DEPTH, MAX_FILES},
    image_jobs::{
        self, check_lease, finish_item_with_warnings_on, DiscoveredImport, ImageImportItem,
        ImageImportJob, ImportError as E, ImportItemState, ImportMedia, ImportPhase, Result,
    },
    jobs::now_secs,
    media::MediaTools,
};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};
use walkdir::WalkDir;
const AUDIO_MAX_BYTES: u64 = 512 * 1024 * 1024;
const QUANTUM_DEADLINE: Duration = Duration::from_secs(120);
fn lock(db: &Arc<Mutex<Catalog>>) -> Result<MutexGuard<'_, Catalog>> {
    db.lock().map_err(|_| E::Internal)
}
fn core_error(_: crate::Error) -> E {
    E::Internal
}
fn fence(db: &rusqlite::Connection, j: &ImageImportJob, owner: &str) -> crate::Result<()> {
    check_lease(db, &j.id, owner, now_secs())
        .map(|_| ())
        .map_err(|_| crate::invalid("Image import lease/source changed"))
}
impl From<AnalysisError> for E {
    fn from(e: AnalysisError) -> Self {
        match e {
            AnalysisError::Cancelled => E::Cancelled,
            AnalysisError::TimedOut => E::TimedOut,
            AnalysisError::LimitExceeded => E::LimitExceeded,
            AnalysisError::UnsupportedFormat => E::UnsupportedFormat,
            AnalysisError::AnimationUnsupported => E::UnsupportedAnimation,
            AnalysisError::UnsupportedColorProfile => E::UnsupportedColorProfile,
            AnalysisError::SourceChanged => E::SourceChanged,
            AnalysisError::SourceUnavailable => E::SourceUnavailable,
            AnalysisError::PermissionDenied => E::PermissionDenied,
            AnalysisError::WorkerUnavailable => E::WorkerUnavailable,
            AnalysisError::Busy => E::Busy,
            _ => E::DecodeFailed,
        }
    }
}
pub struct ImageImportEngine {
    pub catalog: Arc<Mutex<Catalog>>,
    pub worker: PathBuf,
    pub tools: Option<MediaTools>,
}
impl ImageImportEngine {
    /// One discovery or one file per claim. Returning work to the queue provides
    /// round-robin fairness between sources; the legacy audio lane is separate.
    pub fn tick(&self, owner: &str, stop: &AtomicBool) -> Result<Option<ImageImportJob>> {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let job = {
            let c = lock(&self.catalog)?;
            if c.preferences().map_err(core_error)?.imports_paused {
                return Ok(None);
            }
            // A restarted app can arrive before the dead owner's lease expires.
            // Revisit expiry during scheduling, not only once during startup.
            c.recover_image_imports(now_secs())?;
            c.claim_next_image_import(owner, now_secs())?
        };
        let Some(job) = job else {
            return Ok(None);
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let done = AtomicBool::new(false);
        let deadline = Instant::now() + QUANTUM_DEADLINE;
        let result = thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(Ordering::Acquire) {
                    let valid = lock(&self.catalog)
                        .and_then(|c| {
                            check_lease(c.db_connection(), &job.id, owner, now_secs()).map(|_| ())
                        })
                        .is_ok();
                    if stop.load(Ordering::Relaxed) || !valid || Instant::now() >= deadline {
                        cancel.store(true, Ordering::Relaxed);
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            });
            let result = self.quantum(&job, owner, cancel.clone());
            done.store(true, Ordering::Release);
            result
        });
        let c = lock(&self.catalog)?;
        if stop.load(Ordering::Relaxed) {
            c.checkpoint_image_import_owner(owner)?;
            return c.image_import_job(&job.id).map(Some);
        }
        if check_lease(c.db_connection(), &job.id, owner, now_secs()).is_ok() {
            match result {
                Ok(()) => c.release_image_import(&job.id, owner)?,
                Err(error) => {
                    c.image_import_finish(
                        &job.id,
                        owner,
                        Some(if Instant::now() >= deadline {
                            E::TimedOut
                        } else {
                            error
                        }),
                    )?;
                }
            }
        } else if c.image_import_job(&job.id)?.state == image_jobs::ImportState::Running {
            // A generation change invalidates publication, but must not strand a
            // stale owned job in an endless expired-lease recovery loop.
            c.db_connection().execute("UPDATE image_import_jobs SET state='failed',phase='complete',lease_owner=NULL,lease_until=NULL,error_code='SOURCE_CHANGED',current='',sequence=sequence+1 WHERE id=?1 AND state='running' AND lease_owner=?2",params![job.id,owner])?;
        }
        c.image_import_job(&job.id).map(Some)
    }
    fn verify_root(&self, job: &ImageImportJob, owner: &str, source: &Source) -> Result<RootStamp> {
        let stamp = RootStamp::read(Path::new(&source.root));
        let stamp = match stamp {
            Ok(s) => s,
            Err(e) => {
                if e == E::SourceUnavailable {
                    let c = lock(&self.catalog)?;
                    check_lease(c.db_connection(), &job.id, owner, now_secs())?;
                    c.db_connection().execute(
                        "UPDATE sources SET available=0 WHERE id=?1 AND generation=?2",
                        params![source.id, source.generation],
                    )?;
                }
                return Err(e);
            }
        };
        if let Some(expected) = &job.root_stamp {
            if !RootStamp::equivalent(expected, &stamp.json()?)? {
                let c = lock(&self.catalog)?;
                check_lease(c.db_connection(), &job.id, owner, now_secs())?;
                c.db_connection().execute(
                    "UPDATE sources SET available=0 WHERE id=?1 AND generation=?2",
                    params![source.id, source.generation],
                )?;
                return Err(E::SourceChanged);
            }
        }
        lock(&self.catalog)?.image_import_set_root(&job.id, owner, &stamp.json()?)?;
        Ok(stamp)
    }
    fn quantum(&self, job: &ImageImportJob, owner: &str, cancel: Arc<AtomicBool>) -> Result<()> {
        check(&cancel)?;
        let source = lock(&self.catalog)?
            .source_header(&job.source_id)
            .map_err(core_error)?;
        let root = self.verify_root(job, owner, &source)?;
        if job.phase == ImportPhase::Discovery {
            return self.discover(job, owner, &source, &root, &cancel);
        }
        let item = lock(&self.catalog)?.image_import_next_item(&job.id, owner)?;
        let Some(item) = item else {
            lock(&self.catalog)?.image_import_finish(
                &job.id,
                owner,
                if job.completed < job.total {
                    Some(E::DiscoveryIncomplete)
                } else {
                    None
                },
            )?;
            return Ok(());
        };
        let result = self.process(job, owner, &source, &item, cancel.clone());
        if let Err(error) = result {
            if cancel.load(Ordering::Relaxed) {
                return Err(E::Cancelled);
            }
            self.verify_root(job, owner, &source)?;
            self.failed_item(job, owner, &source, &item, error)?;
        }
        Ok(())
    }
    fn discover(
        &self,
        job: &ImageImportJob,
        owner: &str,
        source: &Source,
        root: &RootStamp,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let end = Instant::now() + DISCOVERY_DEADLINE;
        let mut batch = vec![];
        let mut complete = true;
        let mut count = 0usize;
        let mut stamps = job.folder_stamps.clone();
        for prefix in &job.folders {
            check(cancel)?;
            let selected = if prefix.is_empty() {
                PathBuf::from(&source.root)
            } else {
                contained_folder(Path::new(&source.root), prefix)?
            };
            let stamp = RootStamp::read(&selected)?;
            if stamps.get(prefix).is_some_and(|old| old != &stamp) {
                return Err(E::SourceChanged);
            }
            stamps.insert(prefix.clone(), stamp.clone());
            let mut walk = WalkDir::new(&selected)
                .follow_links(false)
                .max_open(8)
                .max_depth(MAX_DEPTH + 1)
                .into_iter();
            while let Some(entry) = walk.next() {
                check(cancel)?;
                if Instant::now() >= end {
                    return Err(E::TimedOut);
                }
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => {
                        complete = false;
                        continue;
                    }
                };
                if entry.depth() > MAX_DEPTH {
                    complete = false;
                    if entry.file_type().is_dir() {
                        walk.skip_current_dir();
                    }
                    continue;
                }
                if entry.file_type().is_dir() {
                    if entry.depth() > 0 {
                        match RootStamp::read(entry.path()) {
                            Ok(s) if stamp.same_volume(&s) => {}
                            _ => {
                                complete = false;
                                walk.skip_current_dir();
                            }
                        }
                    }
                    continue;
                }
                if entry.file_type().is_symlink() {
                    complete = false;
                    continue;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                count += 1;
                if count > MAX_FILES {
                    return Err(E::LimitExceeded);
                }
                let relative = entry
                    .path()
                    .strip_prefix(&source.root)
                    .map_err(|_| E::PermissionDenied)?
                    .to_str()
                    .ok_or(E::InvalidArgument)?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                batch.push(DiscoveredImport {
                    relative_path: relative,
                    media_kind: dispatch(entry.path(), job.mode),
                });
                if batch.len() == 32 {
                    lock(&self.catalog)?.image_import_discovered(&job.id, owner, &batch)?;
                    batch.clear();
                }
            }
            if RootStamp::read(&selected)? != stamp {
                return Err(E::SourceChanged);
            }
        }
        if let Some(paths) = &job.paths {
            for relative in paths {
                check(cancel)?;
                if Instant::now() >= end {
                    return Err(E::TimedOut);
                }
                let path = contained(Path::new(&source.root), relative);
                let kind = path
                    .as_ref()
                    .map(|p| dispatch(p, job.mode))
                    .unwrap_or_else(|_| fallback_kind(Path::new(relative), job.mode));
                batch.push(DiscoveredImport {
                    relative_path: relative.clone(),
                    media_kind: kind,
                });
                if batch.len() == 32 {
                    lock(&self.catalog)?.image_import_discovered(&job.id, owner, &batch)?;
                    batch.clear();
                }
            }
        }
        if !batch.is_empty() {
            lock(&self.catalog)?.image_import_discovered(&job.id, owner, &batch)?;
        }
        if &RootStamp::read(Path::new(&source.root))? != root {
            return Err(E::SourceChanged);
        }
        check(cancel)?;
        let c = lock(&self.catalog)?;
        let tx = Transaction::new_unchecked(c.db_connection(), TransactionBehavior::Immediate)?;
        check_lease(&tx, &job.id, owner, now_secs())?;
        let json = serde_json::to_string(&stamps)?;
        if json.len() > 131072 {
            return Err(E::LimitExceeded);
        }
        image_jobs::prune_budget(
            &tx,
            0,
            json.len()
                .saturating_sub(serde_json::to_vec(&job.folder_stamps)?.len()),
        )?;
        tx.execute(
            "UPDATE image_import_jobs SET folder_stamps=?2 WHERE id=?1",
            params![job.id, json],
        )?;
        // Never analyze unseen historical paths after an interrupted rediscovery.
        // History is dropped in bounded chunks; durable assets remain untouched.
        tx.execute(
            "DELETE FROM image_import_items WHERE job_id=?1 AND discovered_epoch!=?2",
            params![job.id, job.epoch],
        )?;
        tx.execute("UPDATE image_import_jobs SET total=(SELECT count(*) FROM image_import_items WHERE job_id=?1) WHERE id=?1",[&job.id])?;
        if complete {
            reconcile_missing(&tx, job, owner)?;
        }
        tx.commit()?;
        c.image_import_discovery_finished(&job.id, owner, complete)?;
        Ok(())
    }
    fn process(
        &self,
        j: &ImageImportJob,
        owner: &str,
        s: &Source,
        item: &ImageImportItem,
        cancel: Arc<AtomicBool>,
    ) -> Result<()> {
        check(&cancel)?;
        let path = resolve_file(Path::new(&s.root), &item.relative_path)?;
        let actual = candidate(&path, j.mode)?;
        let kind = if actual == ImportMedia::Unsupported {
            fallback_kind(&path, j.mode)
        } else {
            actual
        };
        if kind != item.media_kind {
            return Err(E::SourceChanged);
        }
        let c = lock(&self.catalog)?;
        let existing: Option<String> = c
            .db_connection()
            .query_row(
                "SELECT kind FROM asset_registry WHERE source_id=?1 AND relative_path=?2",
                params![s.id, item.relative_path],
                |r| r.get(0),
            )
            .optional()?;
        if kind == ImportMedia::Unsupported && existing.as_deref() == Some("image") {
            return Err(E::DecodeFailed);
        }
        if kind != ImportMedia::Unsupported
            && existing.as_deref().is_some_and(|k| {
                k != if kind == ImportMedia::Audio {
                    "audio"
                } else {
                    "image"
                }
            })
        {
            return Err(E::WrongAssetKind);
        }
        drop(c);
        match kind {
            ImportMedia::Image => self.image(j, owner, s, item, cancel),
            ImportMedia::Audio => self.audio(j, owner, s, item, &path, cancel),
            ImportMedia::Unsupported => {
                lock(&self.catalog)?.image_import_finish_item(
                    &j.id,
                    owner,
                    &item.relative_path,
                    ImportItemState::Unsupported,
                    None,
                    None,
                    Some(E::UnsupportedFormat),
                    false,
                )?;
                Ok(())
            }
        }
    }
    fn image(
        &self,
        j: &ImageImportJob,
        owner: &str,
        s: &Source,
        item: &ImageImportItem,
        cancel: Arc<AtomicBool>,
    ) -> Result<()> {
        let selection =
            ImageSelection::capture(&*lock(&self.catalog)?, &s.id, &item.relative_path)?;
        let digest = selection.hash(&cancel)?;
        self.verify_root(j, owner, s)?;
        check(&cancel)?;
        let image = lock(&self.catalog)?
            .register_image_when(s, &item.relative_path, Some(&digest), |db| {
                fence(db, j, owner)
            })
            .map_err(core_error)?;
        let cached = {
            let c = lock(&self.catalog)?;
            let profile = c
                .cached_image_profile(&digest, image_analysis::ANALYZER)
                .map_err(core_error)?;
            let warnings:Option<String>=c.db_connection().query_row("SELECT warnings FROM image_analysis_warnings WHERE content_hash=?1 AND analyzer_version=?2",params![digest,image_analysis::ANALYZER],|r|r.get(0)).optional()?;
            profile
                .zip(warnings)
                .map(|(p, w)| serde_json::from_str::<Vec<AnalysisWarning>>(&w).map(|w| (p, w)))
                .transpose()?
        };
        if let Some((profile, mut warnings)) = cached {
            if selection.hash(&cancel)? != digest {
                return Err(E::SourceChanged);
            }
            self.verify_root(j, owner, s)?;
            check(&cancel)?;
            add_extension_warning(&mut warnings, &item.relative_path, &profile);
            lock(&self.catalog)?
                .publish_image_when(s, &image.header.id, &digest, &profile, |db| {
                    finish_item_with_warnings_on(
                        db,
                        &j.id,
                        owner,
                        &item.relative_path,
                        ImportItemState::Ready,
                        Some(&image.header.id),
                        Some(&digest),
                        None,
                        true,
                        &warnings,
                    )
                    .map_err(|_| crate::invalid("Import publication rejected"))
                })
                .map_err(core_error)?;
        } else {
            let request = ImageAnalysisRequest::capture(&*lock(&self.catalog)?, &image.header.id)?;
            let analyzed = request.analyze(&self.worker, cancel.clone())?;
            let verified = analyzed.reverify(&cancel)?;
            self.verify_root(j, owner, s)?;
            check(&cancel)?;
            let warnings = verified.analysis().warnings.clone();
            let shared = warnings
                .iter()
                .copied()
                .filter(|w| *w != AnalysisWarning::ExtensionMismatch)
                .collect::<Vec<_>>();
            verified
                .publish_when(&mut *lock(&self.catalog)?, |db| {
                    db.execute(
                        "INSERT OR IGNORE INTO image_analysis_warnings VALUES(?1,?2,?3)",
                        params![
                            digest,
                            image_analysis::ANALYZER,
                            serde_json::to_string(&shared)?
                        ],
                    )?;
                    finish_item_with_warnings_on(
                        db,
                        &j.id,
                        owner,
                        &item.relative_path,
                        ImportItemState::Ready,
                        Some(&image.header.id),
                        Some(&digest),
                        None,
                        false,
                        &warnings,
                    )
                    .map_err(|_| crate::invalid("Import publication rejected"))
                })
                .map_err(E::from)?;
        }
        Ok(())
    }
    fn audio(
        &self,
        j: &ImageImportJob,
        owner: &str,
        s: &Source,
        item: &ImageImportItem,
        path: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<()> {
        let end = Instant::now() + QUANTUM_DEADLINE;
        let digest = hash_audio(path, &cancel, end)?;
        self.verify_root(j, owner, s)?;
        let (id, cached) = {
            let c = lock(&self.catalog)?;
            let tx = Transaction::new_unchecked(c.db_connection(), TransactionBehavior::Immediate)?;
            check_lease(&tx, &j.id, owner, now_secs())?;
            let id = c
                .register(s, &item.relative_path, &digest)
                .map_err(core_error)?;
            let cached = c.cached_profile(&digest).map_err(core_error)?;
            tx.commit()?;
            (id, cached)
        };
        let reused = cached.is_some();
        let profile = if let Some(p) = cached {
            p
        } else {
            crate::media::analyze(
                self.tools.as_ref().ok_or(E::AudioDecodeFailed)?,
                path,
                cancel.clone(),
            )
            .map_err(|_| E::AudioDecodeFailed)?
        };
        let current = resolve_file(Path::new(&s.root), &item.relative_path)?;
        if hash_audio(&current, &cancel, end)? != digest {
            return Err(E::SourceChanged);
        }
        self.verify_root(j, owner, s)?;
        check(&cancel)?;
        lock(&self.catalog)?
            .publish_when(s, &id, &digest, &profile, |db| {
                finish_item_with_warnings_on(
                    db,
                    &j.id,
                    owner,
                    &item.relative_path,
                    ImportItemState::Ready,
                    Some(&id),
                    Some(&digest),
                    None,
                    reused,
                    &[],
                )
                .map_err(|_| crate::invalid("Import publication rejected"))
            })
            .map_err(core_error)?;
        Ok(())
    }
    fn failed_item(
        &self,
        j: &ImageImportJob,
        owner: &str,
        s: &Source,
        item: &ImageImportItem,
        error: E,
    ) -> Result<()> {
        let c = lock(&self.catalog)?;
        let tx = Transaction::new_unchecked(c.db_connection(), TransactionBehavior::Immediate)?;
        check_lease(&tx, &j.id, owner, now_secs())?;
        let found: Option<(String, String)> = tx
            .query_row(
                "SELECT id,kind FROM asset_registry WHERE source_id=?1 AND relative_path=?2",
                params![s.id, item.relative_path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, kind)) = &found {
            if kind == "audio" {
                tx.execute(
                    "UPDATE sounds SET status=?2 WHERE id=?1",
                    params![
                        id,
                        if error == E::FileMissing {
                            "missing"
                        } else {
                            "failed"
                        }
                    ],
                )?;
            } else {
                tx.execute("UPDATE image_assets SET technical_state='failed',analyzer_version=NULL,availability=?2,evidence=(SELECT COALESCE(json_group_array(json_set(value,'$.stale',json('true'))),'[]') FROM json_each(evidence)),enrichment_state='stale' WHERE id=?1",params![id,if error==E::FileMissing{"missing"}else if error==E::Inaccessible{"inaccessible"}else{"available"}])?;
                tx.execute("UPDATE image_assets SET recipes=(SELECT COALESCE(json_group_array(json_set(value,'$.stale',json('true'))),'[]') FROM json_each(recipes)) WHERE id=?1",[id])?;
            }
        }
        let asset = found
            .as_ref()
            .filter(|(_, k)| {
                *k == if item.media_kind == ImportMedia::Audio {
                    "audio"
                } else {
                    "image"
                }
            })
            .map(|(id, _)| id.as_str());
        finish_item_with_warnings_on(
            &tx,
            &j.id,
            owner,
            &item.relative_path,
            ImportItemState::Failed,
            asset,
            None,
            Some(error),
            false,
            &[],
        )?;
        tx.commit()?;
        Ok(())
    }
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(E::Cancelled)
    } else {
        Ok(())
    }
}
fn fallback_kind(path: &Path, mode: image_jobs::ImportMode) -> ImportMedia {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp") {
        ImportMedia::Image
    } else if mode == image_jobs::ImportMode::Mixed && crate::library::supported(path) {
        ImportMedia::Audio
    } else {
        ImportMedia::Unsupported
    }
}
fn dispatch(path: &Path, mode: image_jobs::ImportMode) -> ImportMedia {
    match candidate(path, mode) {
        Ok(ImportMedia::Unsupported) | Err(_) => fallback_kind(path, mode),
        Ok(kind) => kind,
    }
}
fn add_extension_warning(
    warnings: &mut Vec<AnalysisWarning>,
    relative: &str,
    profile: &ImageProfile,
) {
    let ext = Path::new(relative)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let good = match profile.format {
        ImageFormat::Jpeg => matches!(ext.as_str(), "jpg" | "jpeg"),
        ImageFormat::Png => ext == "png",
        ImageFormat::Webp => ext == "webp",
    };
    if !good {
        warnings.push(AnalysisWarning::ExtensionMismatch);
    }
}
fn hash_audio(path: &Path, cancel: &AtomicBool, end: Instant) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            E::FileMissing
        } else {
            E::Inaccessible
        }
    })?;
    if file.metadata().map_err(|_| E::Inaccessible)?.len() > AUDIO_MAX_BYTES {
        return Err(E::LimitExceeded);
    }
    let mut bytes = 0u64;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0u8; 65536];
    loop {
        check(cancel)?;
        if Instant::now() >= end {
            return Err(E::TimedOut);
        }
        let read = file.read(&mut buffer).map_err(|_| E::Inaccessible)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        if bytes > AUDIO_MAX_BYTES {
            return Err(E::LimitExceeded);
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().to_hex().to_string())
}
fn reconcile_missing(db: &rusqlite::Connection, j: &ImageImportJob, owner: &str) -> Result<()> {
    check_lease(db, &j.id, owner, now_secs())?;
    // SQL selection uses equality/substr, never LIKE (file names may contain %/_).
    for prefix in &j.folders {
        db.execute("UPDATE image_assets SET availability='missing' WHERE id IN (SELECT r.id FROM asset_registry r WHERE r.source_id=?1 AND (?2='' OR substr(r.relative_path,1,length(?2)+1)=?2||'/') AND NOT EXISTS(SELECT 1 FROM image_import_items i WHERE i.job_id=?3 AND i.relative_path=r.relative_path AND i.discovered_epoch=?4))",params![j.source_id,prefix,j.id,j.epoch])?;
        if j.mode == image_jobs::ImportMode::Mixed {
            db.execute("UPDATE sounds SET status='missing' WHERE source_id=?1 AND (?2='' OR substr(relative_path,1,length(?2)+1)=?2||'/') AND NOT EXISTS(SELECT 1 FROM image_import_items i WHERE i.job_id=?3 AND i.relative_path=sounds.relative_path AND i.discovered_epoch=?4)",params![j.source_id,prefix,j.id,j.epoch])?;
        }
    }
    Ok(())
}
impl Catalog {
    /// Graceful shutdown checkpoints only this lane's owner. Restart re-discovers
    /// bytes; successfully published assets and user annotations stay durable.
    pub fn checkpoint_image_import_owner(&self, owner: &str) -> Result<()> {
        self.db_connection().execute(
            "UPDATE image_import_jobs SET lease_until=0 WHERE state='running' AND lease_owner=?1",
            [owner],
        )?;
        self.recover_image_imports(now_secs())
    }
}

fn contained_folder(root: &Path, relative: &str) -> Result<PathBuf> {
    crate::image_contracts::validate_relative(relative).map_err(|_| E::PermissionDenied)?;
    let root = root.canonicalize().map_err(|_| E::SourceUnavailable)?;
    let selected = root
        .join(relative)
        .canonicalize()
        .map_err(|_| E::SourceUnavailable)?;
    if !selected.starts_with(&root) || !selected.is_dir() {
        return Err(E::PermissionDenied);
    }
    Ok(selected)
}

fn resolve_file(root: &Path, relative: &str) -> Result<PathBuf> {
    crate::image_contracts::validate_relative(relative).map_err(|_| E::PermissionDenied)?;
    let root = root.canonicalize().map_err(|_| E::SourceUnavailable)?;
    let path = root.join(relative).canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            E::FileMissing
        } else {
            E::Inaccessible
        }
    })?;
    if !path.starts_with(&root) {
        return Err(E::PermissionDenied);
    }
    let metadata = fs::metadata(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            E::FileMissing
        } else {
            E::Inaccessible
        }
    })?;
    if !metadata.is_file() {
        return Err(E::SourceChanged);
    }
    Ok(path)
}
