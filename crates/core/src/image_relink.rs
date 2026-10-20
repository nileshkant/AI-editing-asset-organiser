//! SS-047 folder relink for image and mixed sources. A preview stats every
//! cataloged member at the new root and hashes a bounded sample outside the
//! catalog lock; confirmation re-checks the root identity, moves the source to
//! a new generation and queues an exact-byte listed check. Asset IDs never change.
use crate::{
    catalog::{contained, path_text, Catalog},
    image_analysis::{hash_path, MAX_ENCODED},
    image_import_preview::{RootStamp, DISCOVERY_DEADLINE, MAX_FILES},
    image_jobs::{
        enqueue_job_on, ImageImportJob, ImportError as E, JobOrigin, JobSpec, Result, ScanKind,
    },
    image_sources::{mark_root, RootState},
    jobs::now_secs,
};
use rusqlite::{params, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const RELINK_TTL: Duration = Duration::from_secs(300);
const MAX_PLANS: usize = 8;
const SAMPLE_FILES: usize = 32;
const SAMPLE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RelinkPreview {
    pub id: String,
    pub source_id: String,
    pub source_generation: String,
    pub root: String,
    pub members: u32,
    pub present: u32,
    pub absent: u32,
    pub sampled: u32,
    pub matched: u32,
    pub changed: u32,
    pub unreadable: u32,
    /// False when the member check hit its deadline; counts are then partial.
    pub complete: bool,
    /// Absent, changed, unreadable or unchecked members need explicit consent.
    pub acknowledgement_required: bool,
    pub expires_at: u64,
}
struct Plan {
    preview: RelinkPreview,
    root: PathBuf,
    stamp: RootStamp,
    generation: i64,
    expires: Instant,
}
#[derive(Default)]
pub struct ImageRelinks {
    plans: Mutex<HashMap<String, Plan>>,
}
fn lock<T>(v: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    v.lock().map_err(|_| E::Internal)
}
fn overlaps(c: &Catalog, id: &str, root: &Path) -> Result<bool> {
    Ok(c.source_headers()
        .map_err(|_| E::Internal)?
        .iter()
        .any(|s| s.id != id && (root.starts_with(&s.root) || Path::new(&s.root).starts_with(root))))
}
impl ImageRelinks {
    pub fn prepare(
        &self,
        db: &Arc<Mutex<Catalog>>,
        source_id: &str,
        root: &Path,
        cancel: &AtomicBool,
    ) -> Result<RelinkPreview> {
        let root = root.canonicalize().map_err(|_| E::SourceUnavailable)?;
        if !root.is_dir() {
            return Err(E::SourceUnavailable);
        }
        let (generation, members) = {
            let c = lock(db)?;
            let monitored: bool = c.db_connection().query_row(&format!("SELECT EXISTS(SELECT 1 FROM sources s WHERE s.id=?1 AND {})",crate::image_sources::IMAGE_MANAGED),[source_id],|r|r.get(0))?;
            if !monitored {
                return Err(E::NotFound);
            }
            if overlaps(&c, source_id, &root)? {
                return Err(E::SourceOverlap);
            }
            let generation = c.source_generation(source_id).map_err(|_| E::NotFound)?;
            let members = c.db_connection().prepare("SELECT r.relative_path,r.kind,i.content_hash FROM asset_registry r LEFT JOIN image_assets i ON i.id=r.id WHERE r.source_id=?1 ORDER BY r.relative_path LIMIT ?2")?
                .query_map(params![source_id,MAX_FILES as i64+1],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if members.len() > MAX_FILES {
                return Err(E::LimitExceeded);
            }
            (generation, members)
        };
        let stamp = RootStamp::read(&root)?;
        let end = Instant::now() + DISCOVERY_DEADLINE;
        let mut complete = true;
        let mut present = vec![];
        let mut absent = 0u32;
        for (relative, kind, hash) in &members {
            if Instant::now() >= end || cancel.load(std::sync::atomic::Ordering::Relaxed) {
                complete = false;
                break;
            }
            match contained(&root, relative) {
                Ok(path) if path.is_file() => present.push((path, kind.as_str(), hash.as_deref())),
                _ => absent += 1,
            }
        }
        // Evenly spaced image sample: catches a wholly different library cheaply.
        let candidates: Vec<_> = present
            .iter()
            .filter(|(_, kind, hash)| *kind == "image" && hash.is_some())
            .collect();
        let step = candidates.len().div_ceil(SAMPLE_FILES).max(1);
        let (mut sampled, mut matched, mut changed, mut unreadable, mut bytes) = (0, 0, 0, 0, 0u64);
        for (path, _, hash) in candidates.into_iter().step_by(step).take(SAMPLE_FILES) {
            let size = path.metadata().map(|m| m.len()).unwrap_or(MAX_ENCODED);
            if bytes + size > SAMPLE_BYTES || Instant::now() >= end {
                break;
            }
            bytes += size;
            sampled += 1;
            match hash_path(path, cancel, end) {
                Ok(digest) if Some(digest.as_str()) == *hash => matched += 1,
                Ok(_) => changed += 1,
                Err(_) => unreadable += 1,
            }
        }
        if present.is_empty() {
            return Err(E::NoMatchingFiles);
        }
        let id = Uuid::new_v4().to_string();
        let expires_at = now_secs().max(0) as u64 + RELINK_TTL.as_secs();
        let preview = RelinkPreview {
            id: id.clone(),
            source_id: source_id.into(),
            source_generation: generation.to_string(),
            root: path_text(&root).map_err(|_| E::InvalidArgument)?,
            members: members.len() as u32,
            present: present.len() as u32,
            absent,
            sampled,
            matched,
            changed,
            unreadable,
            complete,
            acknowledgement_required: absent > 0 || changed > 0 || unreadable > 0 || !complete,
            expires_at,
        };
        let mut plans = lock(&self.plans)?;
        let now = Instant::now();
        plans.retain(|_, p| p.expires > now);
        if plans.len() >= MAX_PLANS {
            if let Some(oldest) = plans.iter().min_by_key(|(_, p)| p.expires).map(|(k, _)| k.clone()) {
                plans.remove(&oldest);
            }
        }
        plans.insert(
            id,
            Plan {
                preview: preview.clone(),
                root,
                stamp,
                generation,
                expires: now + RELINK_TTL,
            },
        );
        Ok(preview)
    }
    pub fn discard(&self, id: &str) -> Result<()> {
        lock(&self.plans)?.remove(id);
        Ok(())
    }
    pub fn confirm(
        &self,
        db: &Arc<Mutex<Catalog>>,
        id: &str,
        acknowledged: bool,
    ) -> Result<ImageImportJob> {
        let plan = {
            let mut plans = lock(&self.plans)?;
            let plan = plans.get(id).ok_or(E::NotFound)?;
            if plan.preview.acknowledgement_required && !acknowledged {
                return Err(E::InvalidArgument);
            }
            plans.remove(id).ok_or(E::NotFound)?
        };
        if plan.expires <= Instant::now() {
            return Err(E::SourceChanged);
        }
        // The same folder that was previewed, not a drive swapped in since.
        if RootStamp::read(&plan.root)? != plan.stamp {
            return Err(E::SourceChanged);
        }
        let c = lock(db)?;
        let tx = Transaction::new_unchecked(c.db_connection(), TransactionBehavior::Immediate)?;
        let source_id = plan.preview.source_id.as_str();
        if c.source_generation(source_id).map_err(|_| E::NotFound)? != plan.generation {
            return Err(E::SourceChanged);
        }
        if overlaps(&c, source_id, &plan.root)? {
            return Err(E::SourceOverlap);
        }
        let audio_busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE source_id=?1 AND state IN ('queued','running'))",[source_id],|r|r.get(0))?;
        if audio_busy {
            return Err(E::SourceConflict);
        }
        let next = plan.generation + 1;
        tx.execute(
            "UPDATE sources SET root=?2,generation=?3,available=1 WHERE id=?1 AND generation=?4",
            params![source_id, plan.preview.root, next, plan.generation],
        )?;
        // Older-generation work is fenced by its lease check; stop it explicitly too.
        tx.execute("UPDATE image_import_items SET state='pending' WHERE state='running' AND job_id IN (SELECT id FROM image_import_jobs WHERE source_id=?1 AND state IN ('queued','running'))",[source_id])?;
        tx.execute("UPDATE image_import_jobs SET state='cancelled',lease_owner=NULL,lease_until=NULL,sequence=sequence+1,error_code='SOURCE_CHANGED',updated_at=?2 WHERE source_id=?1 AND state IN ('queued','running')",params![source_id,now_secs()])?;
        tx.execute("INSERT INTO image_source_identity(source_id,source_generation,root_stamp) VALUES(?1,?2,?3) ON CONFLICT(source_id) DO UPDATE SET source_generation=excluded.source_generation,root_stamp=excluded.root_stamp",params![source_id,next,plan.stamp.json()?])?;
        let source = c.source_header(source_id).map_err(|_| E::Internal)?;
        mark_root(&tx, &source, RootState::Online)?;
        let mode = tx.query_row("SELECT EXISTS(SELECT 1 FROM image_source_monitor WHERE source_id=?1 AND mode='mixed') OR EXISTS(SELECT 1 FROM sounds WHERE source_id=?1)",[source_id],|r|r.get::<_,bool>(0))?;
        let job = enqueue_job_on(
            &tx,
            source_id,
            next,
            JobSpec {
                mode: if mode {
                    crate::image_jobs::ImportMode::Mixed
                } else {
                    crate::image_jobs::ImportMode::Images
                },
                paths: None,
                folders: vec![],
                scan: ScanKind::Listed,
                verify: true,
                origin: JobOrigin::Relink,
            },
        )?;
        tx.commit()?;
        Ok(job)
    }
}
