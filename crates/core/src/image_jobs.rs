//! Typed, durable image/mixed import metadata. No filesystem work in this module.
use crate::{catalog::Catalog, image_contracts::validate_relative, jobs::now_secs};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub const MAX_ACTIVE: usize = 32;
pub const MAX_HISTORY: usize = 1000;
pub const MAX_ITEMS: usize = 100_000;
pub const MAX_JOB_BYTES: usize = 128 * 1024 * 1024;
pub const LEASE_SECONDS: i64 = 180;
pub const MAX_PATHS: usize = 256;
pub const MAX_SELECTION_BYTES: usize = 1024 * 1024;
const MAX_SEQUENCE: i64 = 9_007_199_254_740_991;
pub type Result<T> = std::result::Result<T, ImportError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImportError {
    #[error("Invalid import request")]
    InvalidArgument,
    #[error("Selection is outside its approved scope")]
    PermissionDenied,
    #[error("Source changed; choose or refresh the selection")]
    SourceChanged,
    #[error("Source is offline or unavailable")]
    SourceUnavailable,
    #[error("Selected file is missing")]
    FileMissing,
    #[error("Another selection for this source is already importing")]
    SourceConflict,
    #[error("Existing folder catalog requires mixed catalog support; use the audio import flow or a disposable development folder")]
    ExistingCatalog,
    #[error("Selection overlaps an existing source scope; use that source or choose exact files")]
    SourceOverlap,
    #[error("Import queue is full; retry after current work finishes")]
    QueueFull,
    #[error("Import exceeds a resource limit")]
    LimitExceeded,
    #[error("Import record was not found")]
    NotFound,
    #[error("Import ownership expired")]
    LeaseLost,
    #[error("Import cancelled")]
    Cancelled,
    #[error("Discovery was incomplete; previous memberships are preserved")]
    DiscoveryIncomplete,
    #[error("Image is corrupt or unsupported")]
    DecodeFailed,
    #[error("A different media kind occupies this path")]
    WrongAssetKind,
    #[error("File access was denied")]
    Inaccessible,
    #[error("Animated images are unsupported")]
    UnsupportedAnimation,
    #[error("Color profile is unsupported")]
    UnsupportedColorProfile,
    #[error("Audio could not be analyzed")]
    AudioDecodeFailed,
    #[error("Image worker is unavailable")]
    WorkerUnavailable,
    #[error("Image worker is busy")]
    Busy,
    #[error("Image analysis timed out")]
    TimedOut,
    #[error("File format is unsupported")]
    UnsupportedFormat,
    #[error("File type is excluded by this selection")]
    FilteredMedia,
    #[error("No eligible files found")]
    NoEligibleFiles,
    #[error("Import operation failed; retry or inspect source access")]
    Internal,
}
impl From<rusqlite::Error> for ImportError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Internal
    }
}
impl From<serde_json::Error> for ImportError {
    fn from(_: serde_json::Error) -> Self {
        Self::InvalidArgument
    }
}
macro_rules! values {($name:ident{$($value:ident),*})=>{#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]#[serde(rename_all="snake_case")]pub enum $name{$($value),*}};}
values!(ImportMode { Images, Mixed });
values!(ImportState {
    Queued,
    Running,
    Complete,
    Failed,
    Cancelled
});
values!(ImportPhase {
    Discovery,
    Analysis,
    Complete
});
values!(ImportMedia {
    Image,
    Audio,
    Unsupported
});
values!(ImportItemState {
    Pending,
    Running,
    Ready,
    Failed,
    Unsupported
});
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageImportJob {
    pub id: String,
    pub source_id: String,
    pub source_generation: String,
    pub mode: ImportMode,
    pub state: ImportState,
    pub phase: ImportPhase,
    pub discovery_complete: bool,
    pub history_truncated: bool,
    pub sequence: String,
    pub total: u32,
    pub completed: u32,
    pub ready: u32,
    pub reused: u32,
    pub failed: u32,
    pub unsupported: u32,
    pub current: String,
    pub error_code: Option<ImportError>,
    #[serde(skip)]
    pub(crate) paths: Option<Vec<String>>,
    #[serde(skip)]
    pub(crate) folders: Vec<String>,
    #[serde(skip)]
    pub(crate) folder_stamps:
        std::collections::BTreeMap<String, crate::image_import_preview::RootStamp>,
    #[serde(skip)]
    pub(crate) lease_owner: Option<String>,
    #[serde(skip)]
    pub(crate) lease_until: Option<i64>,
    #[serde(skip)]
    pub(crate) epoch: i64,
    #[serde(skip)]
    pub(crate) root_stamp: Option<String>,
}
impl ImageImportJob {
    pub fn generation(&self) -> Result<i64> {
        self.source_generation
            .parse()
            .map_err(|_| ImportError::Internal)
    }
    pub fn terminal(&self) -> bool {
        !matches!(self.state, ImportState::Queued | ImportState::Running)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageImportItem {
    pub job_id: String,
    pub relative_path: String,
    pub media_kind: ImportMedia,
    pub state: ImportItemState,
    pub asset_id: Option<String>,
    pub content_hash: Option<String>,
    pub error_code: Option<ImportError>,
    pub attempts: u32,
    pub warnings: Vec<crate::image_analysis::AnalysisWarning>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageImportItemPage {
    pub items: Vec<ImageImportItem>,
    pub next_after: Option<String>,
}
const JOB_COLUMNS:&str="id,source_id,source_generation,mode,state,phase,discovery_complete,sequence,total,completed,ready,reused,failed,unsupported,current,error_code,paths,lease_owner,lease_until,discovery_epoch,root_stamp,history_truncated,folders,folder_stamps";
fn enum_value<T: serde::de::DeserializeOwned>(
    v: String,
) -> std::result::Result<T, rusqlite::Error> {
    serde_json::from_value(serde_json::Value::String(v)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn code(row: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Option<ImportError>> {
    row.get::<_, Option<String>>(i)?.map(enum_value).transpose()
}
fn wire<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or(ImportError::Internal)
}
// Current is a bounded display hint. Exact file names remain in item metadata.
fn display_path(mut path: String) -> String {
    if path.len() > 4096 {
        let mut end = 4093;
        while !path.is_char_boundary(end) {
            end -= 1;
        }
        path.truncate(end);
        path.push('…');
    }
    path
}
fn map_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<ImageImportJob> {
    let paths: Option<String> = r.get(16)?;
    Ok(ImageImportJob {
        id: r.get(0)?,
        source_id: r.get(1)?,
        source_generation: r.get::<_, i64>(2)?.to_string(),
        mode: enum_value(r.get(3)?)?,
        state: enum_value(r.get(4)?)?,
        phase: enum_value(r.get(5)?)?,
        discovery_complete: r.get(6)?,
        sequence: r.get::<_, i64>(7)?.to_string(),
        total: r.get(8)?,
        completed: r.get(9)?,
        ready: r.get(10)?,
        reused: r.get(11)?,
        failed: r.get(12)?,
        unsupported: r.get(13)?,
        current: display_path(r.get(14)?),
        error_code: code(r, 15)?,
        paths: paths
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    16,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        lease_owner: r.get(17)?,
        lease_until: r.get(18)?,
        epoch: r.get(19)?,
        root_stamp: r.get(20)?,
        history_truncated: r.get(21)?,
        folders: serde_json::from_str(&r.get::<_, String>(22)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(22, rusqlite::types::Type::Text, Box::new(e))
        })?,
        folder_stamps: serde_json::from_str(&r.get::<_, String>(23)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(23, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}
pub(crate) fn load_job(db: &Connection, id: &str) -> Result<ImageImportJob> {
    db.query_row(
        &format!("SELECT {JOB_COLUMNS} FROM image_import_jobs WHERE id=?1"),
        [id],
        map_job,
    )
    .optional()?
    .ok_or(ImportError::NotFound)
}
pub(crate) fn check_lease(
    db: &Connection,
    id: &str,
    owner: &str,
    now: i64,
) -> Result<ImageImportJob> {
    let job = load_job(db, id)?;
    if job.state != ImportState::Running
        || job.lease_owner.as_deref() != Some(owner)
        || job.lease_until.unwrap_or(0) <= now
    {
        return Err(ImportError::LeaseLost);
    }
    let generation: Option<i64> = db
        .query_row(
            "SELECT generation FROM sources WHERE id=?1",
            [&job.source_id],
            |r| r.get(0),
        )
        .optional()?;
    if generation != Some(job.generation()?) {
        return Err(ImportError::SourceChanged);
    }
    Ok(job)
}
fn next_order(db: &Connection) -> Result<i64> {
    let old: i64 = db.query_row(
        "SELECT COALESCE(MAX(queue_order),0) FROM image_import_jobs",
        [],
        |r| r.get(0),
    )?;
    old.checked_add(1)
        .filter(|v| *v <= MAX_SEQUENCE)
        .ok_or(ImportError::LimitExceeded)
}
pub(crate) fn enqueue_on(
    db: &Connection,
    source_id: &str,
    generation: i64,
    mode: ImportMode,
    paths: Option<Vec<String>>,
) -> Result<ImageImportJob> {
    let folders = if paths.is_none() {
        vec![String::new()]
    } else {
        vec![]
    };
    enqueue_selection_on(db, source_id, generation, mode, paths, folders)
}
pub(crate) fn enqueue_selection_on(
    db: &Connection,
    source_id: &str,
    generation: i64,
    mode: ImportMode,
    mut paths: Option<Vec<String>>,
    mut folders: Vec<String>,
) -> Result<ImageImportJob> {
    if generation < 0 || source_id.is_empty() || source_id.len() > 256 {
        return Err(ImportError::InvalidArgument);
    }
    let source: Option<(i64, bool, String)> = db
        .query_row(
            "SELECT generation,available,scope FROM sources WHERE id=?1",
            [source_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((current, available, scope)) = source else {
        return Err(ImportError::NotFound);
    };
    if current != generation {
        return Err(ImportError::SourceChanged);
    }
    if !available {
        return Err(ImportError::SourceUnavailable);
    }
    if let Some(paths) = &mut paths {
        paths.sort();
        paths.dedup();
        if paths.is_empty() || paths.len() > MAX_PATHS {
            return Err(ImportError::InvalidArgument);
        }
        for relative in paths.iter() {
            validate_relative(relative).map_err(|_| ImportError::InvalidArgument)?;
            let allowed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sources s WHERE s.id=?1 AND (s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=s.id AND f.relative_path=?2)))",params![source_id,relative],|r|r.get(0))?;
            if !allowed {
                return Err(ImportError::PermissionDenied);
            }
        }
    }
    folders.sort();
    folders.dedup();
    if folders.len() > MAX_PATHS || !folders.is_empty() && scope != "folder" {
        return Err(ImportError::PermissionDenied);
    }
    for folder in &folders {
        if !folder.is_empty() {
            validate_relative(folder).map_err(|_| ImportError::InvalidArgument)?;
        }
    }
    if paths.is_none() && folders.is_empty() {
        return Err(ImportError::InvalidArgument);
    }
    let encoded_folders = serde_json::to_string(&folders)?;
    let encoded = paths.as_ref().map(serde_json::to_string).transpose()?;
    if serde_json::to_vec(&(&paths, &folders))?.len() > MAX_SELECTION_BYTES {
        return Err(ImportError::LimitExceeded);
    }
    let key = format!(
        "{}:{}",
        wire(&mode)?,
        blake3::hash(&serde_json::to_vec(&(&paths, &folders))?).to_hex()
    );
    let active:Option<String>=db.query_row("SELECT id FROM image_import_jobs WHERE source_id=?1 AND source_generation=?2 AND state IN ('queued','running')",params![source_id,generation],|r|r.get(0)).optional()?;
    if let Some(id) = active {
        let existing = load_job(db, &id)?;
        if existing.mode == mode && existing.paths == paths && existing.folders == folders {
            return Ok(existing);
        }
        return Err(ImportError::SourceConflict);
    }
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE source_id=?1 AND state IN ('queued','running'))",
        [source_id],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(ImportError::SourceConflict);
    }
    let count: i64 = db.query_row(
        "SELECT count(*) FROM image_import_jobs WHERE state IN ('queued','running')",
        [],
        |r| r.get(0),
    )?;
    if count >= MAX_ACTIVE as i64 {
        return Err(ImportError::QueueFull);
    }
    let history: i64 = db.query_row("SELECT count(*) FROM image_import_jobs", [], |r| r.get(0))?;
    if history >= MAX_HISTORY as i64 {
        db.execute("DELETE FROM image_import_jobs WHERE id IN (SELECT id FROM image_import_jobs WHERE state NOT IN ('queued','running') AND NOT EXISTS(SELECT 1 FROM image_import_items i WHERE i.job_id=image_import_jobs.id LIMIT 1 OFFSET 128) ORDER BY created_at,id LIMIT ?1)",[history-MAX_HISTORY as i64+1])?;
    }
    if db.query_row("SELECT count(*) FROM image_import_jobs", [], |r| {
        r.get::<_, i64>(0)
    })? >= MAX_HISTORY as i64
    {
        return Err(ImportError::LimitExceeded);
    }
    prune_budget(
        db,
        0,
        40960 + encoded.as_ref().map_or(0, String::len) + encoded_folders.len() + 2,
    )?;
    let id = Uuid::new_v4().to_string();
    let now = now_secs();
    db.execute("INSERT INTO image_import_jobs(id,source_id,source_generation,selection_key,mode,paths,folders,state,phase,queue_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?9,'queued','discovery',?7,?8,?8)",params![id,source_id,generation,key,wire(&mode)?,encoded,next_order(db)?,now,encoded_folders])?;
    load_job(db, &id)
}
/// Evict only bounded transient terminal history. Asset records/annotations remain.
pub(crate) fn prune_budget(db: &Connection, rows: usize, bytes: usize) -> Result<()> {
    for _ in 0..128 {
        let (stored_rows, stored_bytes): (i64, i64) = db.query_row(
            "SELECT rows,bytes FROM image_import_budget WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if stored_rows + rows as i64 <= MAX_ITEMS as i64
            && stored_bytes + bytes as i64 <= MAX_JOB_BYTES as i64
        {
            return Ok(());
        }
        let old:Option<String>=db.query_row("SELECT id FROM image_import_jobs WHERE state IN ('complete','failed','cancelled') ORDER BY created_at,id LIMIT 1",[],|r|r.get(0)).optional()?;
        let Some(old) = old else {
            return Err(ImportError::LimitExceeded);
        };
        db.execute("UPDATE image_import_jobs SET history_truncated=1,sequence=sequence+(history_truncated=0) WHERE id=?1",[&old])?;
        db.execute("DELETE FROM image_import_items WHERE job_id=?1 AND relative_path IN (SELECT relative_path FROM image_import_items WHERE job_id=?1 LIMIT 32)",[&old])?;
        db.execute("DELETE FROM image_import_jobs WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM image_import_items WHERE job_id=?1)",[&old])?;
    }
    Err(ImportError::LimitExceeded)
}
impl Catalog {
    pub fn enqueue_image_import(
        &self,
        source_id: &str,
        generation: i64,
        mode: ImportMode,
        paths: Option<Vec<String>>,
    ) -> Result<ImageImportJob> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let job = enqueue_on(&tx, source_id, generation, mode, paths)?;
        tx.commit()?;
        Ok(job)
    }
    pub fn image_import_job(&self, id: &str) -> Result<ImageImportJob> {
        load_job(&self.db, id)
    }
    pub fn image_import_jobs(
        &self,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ImageImportJob>> {
        if !(1..=50).contains(&limit) {
            return Err(ImportError::InvalidArgument);
        }
        if after.is_some_and(|s| Uuid::parse_str(s).is_err()) {
            return Err(ImportError::InvalidArgument);
        }
        let mut q = self.db.prepare(&format!(
            "SELECT {JOB_COLUMNS} FROM image_import_jobs WHERE id>?1 ORDER BY id LIMIT ?2"
        ))?;
        let jobs = q
            .query_map(params![after.unwrap_or(""), limit], map_job)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if serde_json::to_vec(&jobs)?.len() > 2 * 1024 * 1024 {
            return Err(ImportError::LimitExceeded);
        }
        Ok(jobs)
    }
    pub fn cancel_image_import(&self, id: &str, generation: i64) -> Result<ImageImportJob> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let old = load_job(&tx, id)?;
        if old.generation() != Ok(generation) {
            return Err(ImportError::SourceChanged);
        }
        if !old.terminal() {
            tx.execute("UPDATE image_import_jobs SET state='cancelled',lease_owner=NULL,lease_until=NULL,sequence=sequence+1,error_code='CANCELLED',updated_at=?2 WHERE id=?1",params![id,now_secs()])?;
            tx.execute(
                "UPDATE image_import_items SET state='pending' WHERE job_id=?1 AND state='running'",
                [id],
            )?;
        }
        let result = load_job(&tx, id)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn retry_image_import(&self, id: &str, generation: i64) -> Result<ImageImportJob> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let old = load_job(&tx, id)?;
        if !old.terminal() {
            return Err(ImportError::SourceConflict);
        }
        if old.generation() != Ok(generation) {
            return Err(ImportError::SourceChanged);
        }
        let next = enqueue_selection_on(
            &tx,
            &old.source_id,
            generation,
            old.mode,
            old.paths,
            old.folders,
        )?;
        prune_budget(
            &tx,
            0,
            serde_json::to_vec(&old.folder_stamps)?
                .len()
                .saturating_sub(2),
        )?;
        tx.execute(
            "UPDATE image_import_jobs SET root_stamp=?2,folder_stamps=?3 WHERE id=?1",
            params![
                next.id,
                old.root_stamp,
                serde_json::to_string(&old.folder_stamps)?
            ],
        )?;
        let result = load_job(&tx, &next.id)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn recover_image_imports(&self, now: i64) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        tx.execute("UPDATE image_import_items SET state='pending',error_code=NULL WHERE job_id IN (SELECT id FROM image_import_jobs WHERE state='running' AND lease_until<=?1)",[now])?;
        tx.execute("UPDATE image_import_jobs SET state='queued',phase='discovery',discovery_complete=0,discovery_epoch=discovery_epoch+1,completed=0,ready=0,reused=0,failed=0,unsupported=0,current='',lease_owner=NULL,lease_until=NULL,sequence=sequence+1,updated_at=?1 WHERE state='running' AND lease_until<=?1",[now])?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn claim_next_image_import(
        &self,
        owner: &str,
        now: i64,
    ) -> Result<Option<ImageImportJob>> {
        if owner.is_empty() || owner.len() > 256 {
            return Err(ImportError::InvalidArgument);
        }
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let id:Option<String>=tx.query_row("SELECT id FROM image_import_jobs WHERE state='queued' ORDER BY queue_order,id LIMIT 1",[],|r|r.get(0)).optional()?;
        let Some(id) = id else {
            tx.commit()?;
            return Ok(None);
        };
        tx.execute("UPDATE image_import_jobs SET state='running',lease_owner=?2,lease_until=?3,sequence=sequence+1,updated_at=?4 WHERE id=?1",params![id,owner,now.checked_add(LEASE_SECONDS).ok_or(ImportError::LimitExceeded)?,now])?;
        let result = load_job(&tx, &id)?;
        tx.commit()?;
        Ok(Some(result))
    }
    pub(crate) fn release_image_import(&self, id: &str, owner: &str) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        check_lease(&tx, id, owner, now_secs())?;
        tx.execute("UPDATE image_import_jobs SET state='queued',lease_owner=NULL,lease_until=NULL,queue_order=?3,sequence=sequence+1,updated_at=?4 WHERE id=?1 AND lease_owner=?2",params![id,owner,next_order(&tx)?,now_secs()])?;
        tx.commit()?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredImport {
    pub relative_path: String,
    pub media_kind: ImportMedia,
}
fn map_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<ImageImportItem> {
    Ok(ImageImportItem {
        job_id: r.get(0)?,
        relative_path: r.get(1)?,
        media_kind: enum_value(r.get(2)?)?,
        state: enum_value(r.get(3)?)?,
        asset_id: r.get(4)?,
        content_hash: r.get(5)?,
        error_code: code(r, 6)?,
        attempts: r.get(7)?,
        warnings: serde_json::from_str(&r.get::<_, String>(8)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}
const ITEM_COLUMNS: &str =
    "job_id,relative_path,media_kind,state,asset_id,content_hash,error_code,attempts,warnings";
pub(crate) fn finish_item_on(
    db: &Connection,
    id: &str,
    owner: &str,
    relative: &str,
    state: ImportItemState,
    asset_id: Option<&str>,
    digest: Option<&str>,
    error: Option<ImportError>,
    reused: bool,
) -> Result<()> {
    finish_item_with_warnings_on(
        db,
        id,
        owner,
        relative,
        state,
        asset_id,
        digest,
        error,
        reused,
        &[],
    )
}
pub(crate) fn finish_item_with_warnings_on(
    db: &Connection,
    id: &str,
    owner: &str,
    relative: &str,
    state: ImportItemState,
    asset_id: Option<&str>,
    digest: Option<&str>,
    error: Option<ImportError>,
    reused: bool,
    warnings: &[crate::image_analysis::AnalysisWarning],
) -> Result<()> {
    if warnings.len() > 8 {
        return Err(ImportError::LimitExceeded);
    }
    let job = check_lease(db, id, owner, now_secs())?;
    if job.phase != ImportPhase::Analysis
        || !matches!(
            state,
            ImportItemState::Ready | ImportItemState::Failed | ImportItemState::Unsupported
        )
    {
        return Err(ImportError::InvalidArgument);
    }
    let updated=db.execute("UPDATE image_import_items SET state=?3,asset_id=?4,content_hash=?5,error_code=?6,warnings=?7 WHERE job_id=?1 AND relative_path=?2 AND state='running'",params![id,relative,wire(&state)?,asset_id,digest,error.map(|e|wire(&e)).transpose()?,serde_json::to_string(warnings)?])?;
    if updated != 1 {
        return Err(ImportError::LeaseLost);
    }
    db.execute("UPDATE image_import_jobs SET completed=completed+1,ready=ready+?2,failed=failed+?3,unsupported=unsupported+?4,reused=reused+?5,current='',sequence=sequence+1,lease_until=?6,updated_at=?7 WHERE id=?1",params![id,state==ImportItemState::Ready,state==ImportItemState::Failed,state==ImportItemState::Unsupported,reused,now_secs()+LEASE_SECONDS,now_secs()])?;
    Ok(())
}
impl Catalog {
    pub fn image_import_items(
        &self,
        id: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<ImageImportItemPage> {
        if !(1..=50).contains(&limit) {
            return Err(ImportError::InvalidArgument);
        }
        if let Some(p) = after {
            validate_relative(p).map_err(|_| ImportError::InvalidArgument)?;
        }
        load_job(&self.db, id)?;
        let mut q=self.db.prepare(&format!("SELECT {ITEM_COLUMNS} FROM image_import_items WHERE job_id=?1 AND relative_path>?2 ORDER BY relative_path LIMIT ?3"))?;
        let mut rows = q.query(params![id, after.unwrap_or(""), limit + 1])?;
        let mut items = vec![];
        let mut more = false;
        let mut wire_bytes = 65536 + 1024; // Worst-case escaped full-path cursor plus envelope.
        while let Some(row) = rows.next()? {
            if items.len() == limit as usize {
                more = true;
                break;
            }
            let item = map_item(row)?;
            let size = serde_json::to_vec(&item)?.len() + 1;
            if !items.is_empty() && wire_bytes + size > 2 * 1024 * 1024 {
                more = true;
                break;
            }
            wire_bytes += size;
            items.push(item);
        }
        let next_after = if more {
            items.last().map(|i| i.relative_path.clone())
        } else {
            None
        };
        let result = ImageImportItemPage { items, next_after };
        if serde_json::to_vec(&result)?.len() > 2 * 1024 * 1024 {
            return Err(ImportError::LimitExceeded);
        }
        Ok(result)
    }
    pub(crate) fn image_import_set_root(&self, id: &str, owner: &str, stamp: &str) -> Result<()> {
        if stamp.len() > 4096 || serde_json::from_str::<serde_json::Value>(stamp).is_err() {
            return Err(ImportError::InvalidArgument);
        }
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let job = check_lease(&tx, id, owner, now_secs())?;
        if let Some(previous) = job.root_stamp.as_deref() {
            if !crate::image_import_preview::RootStamp::equivalent(previous, stamp)? {
                return Err(ImportError::SourceChanged);
            }
        }
        let known: Option<(i64, String)> = tx
            .query_row(
                "SELECT source_generation,root_stamp FROM image_source_identity WHERE source_id=?1",
                [&job.source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((generation, previous)) = known {
            if generation == job.generation()?
                && !crate::image_import_preview::RootStamp::equivalent(&previous, stamp)?
            {
                return Err(ImportError::SourceChanged);
            }
        }
        tx.execute("INSERT INTO image_source_identity(source_id,source_generation,root_stamp) VALUES(?1,?2,?3) ON CONFLICT(source_id) DO UPDATE SET source_generation=excluded.source_generation,root_stamp=excluded.root_stamp",params![job.source_id,job.generation()?,stamp])?;
        tx.execute("UPDATE image_import_jobs SET root_stamp=?3,lease_until=?4,updated_at=?5 WHERE id=?1 AND lease_owner=?2",params![id,owner,stamp,now_secs()+LEASE_SECONDS,now_secs()])?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn image_import_discovered(
        &self,
        id: &str,
        owner: &str,
        entries: &[DiscoveredImport],
    ) -> Result<()> {
        if entries.len() > 32
            || entries.iter().map(|e| e.relative_path.len()).sum::<usize>() > MAX_SELECTION_BYTES
        {
            return Err(ImportError::LimitExceeded);
        }
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let job = check_lease(&tx, id, owner, now_secs())?;
        if job.phase != ImportPhase::Discovery {
            return Err(ImportError::InvalidArgument);
        }
        let mut new_paths = std::collections::BTreeSet::new();
        for entry in entries {
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM image_import_items WHERE job_id=?1 AND relative_path=?2)",params![id,entry.relative_path],|r|r.get(0))?;
            if !exists {
                new_paths.insert(entry.relative_path.as_str());
            }
        }
        prune_budget(
            &tx,
            new_paths.len(),
            new_paths.iter().map(|p| 1024 + p.len()).sum(),
        )?;
        for e in entries {
            validate_relative(&e.relative_path).map_err(|_| ImportError::InvalidArgument)?;
            let allowed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sources s WHERE s.id=?1 AND (s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=s.id AND f.relative_path=?2)))",params![job.source_id,e.relative_path],|r|r.get(0))?;
            if !allowed
                || !job
                    .paths
                    .as_ref()
                    .is_some_and(|p| p.contains(&e.relative_path))
                    && !job
                        .folders
                        .iter()
                        .any(|p| p.is_empty() || e.relative_path.starts_with(&format!("{p}/")))
            {
                return Err(ImportError::PermissionDenied);
            }
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM image_import_items WHERE job_id=?1 AND relative_path=?2)",params![id,e.relative_path],|r|r.get(0))?;
            tx.execute("INSERT INTO image_import_items(job_id,relative_path,media_kind,state,discovered_epoch) VALUES(?1,?2,?3,'pending',?4) ON CONFLICT(job_id,relative_path) DO UPDATE SET media_kind=excluded.media_kind,discovered_epoch=excluded.discovered_epoch",params![id,e.relative_path,wire(&e.media_kind)?,job.epoch])?;
            if !exists {
                tx.execute(
                    "UPDATE image_import_jobs SET total=total+1 WHERE id=?1",
                    [id],
                )?;
            }
        }
        tx.execute("UPDATE image_import_jobs SET sequence=sequence+1,lease_until=?2,updated_at=?3 WHERE id=?1",params![id,now_secs()+LEASE_SECONDS,now_secs()])?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn image_import_discovery_finished(
        &self,
        id: &str,
        owner: &str,
        complete: bool,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let job = check_lease(&tx, id, owner, now_secs())?;
        if job.phase != ImportPhase::Discovery {
            return Err(ImportError::InvalidArgument);
        }
        tx.execute("UPDATE image_import_jobs SET phase='analysis',discovery_complete=?2,error_code=?3,sequence=sequence+1,updated_at=?4 WHERE id=?1",params![id,complete,(!complete).then_some("DISCOVERY_INCOMPLETE"),now_secs()])?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn image_import_next_item(
        &self,
        id: &str,
        owner: &str,
    ) -> Result<Option<ImageImportItem>> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let job = check_lease(&tx, id, owner, now_secs())?;
        if job.phase != ImportPhase::Analysis {
            return Err(ImportError::InvalidArgument);
        }
        let relative:Option<String>=tx.query_row("SELECT relative_path FROM image_import_items WHERE job_id=?1 AND state='pending' ORDER BY relative_path LIMIT 1",[id],|r|r.get(0)).optional()?;
        let Some(relative) = relative else {
            tx.commit()?;
            return Ok(None);
        };
        tx.execute("UPDATE image_import_items SET state='running',attempts=attempts+1 WHERE job_id=?1 AND relative_path=?2",params![id,relative])?;
        tx.execute("UPDATE image_import_jobs SET current=?2,sequence=sequence+1,lease_until=?3,updated_at=?4 WHERE id=?1",params![id,relative,now_secs()+LEASE_SECONDS,now_secs()])?;
        let item=tx.query_row(&format!("SELECT {ITEM_COLUMNS} FROM image_import_items WHERE job_id=?1 AND relative_path=?2"),params![id,relative],map_item)?;
        tx.commit()?;
        Ok(Some(item))
    }
    pub(crate) fn image_import_finish_item(
        &self,
        id: &str,
        owner: &str,
        relative: &str,
        state: ImportItemState,
        asset_id: Option<&str>,
        digest: Option<&str>,
        error: Option<ImportError>,
        reused: bool,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        finish_item_on(
            &tx, id, owner, relative, state, asset_id, digest, error, reused,
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn image_import_finish(
        &self,
        id: &str,
        owner: &str,
        error: Option<ImportError>,
    ) -> Result<ImageImportJob> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let job = check_lease(&tx, id, owner, now_secs())?;
        if error.is_none() && job.completed != job.total {
            return Err(ImportError::InvalidArgument);
        }
        let failure = error.or(if !job.discovery_complete {
            Some(ImportError::DiscoveryIncomplete)
        } else {
            None
        });
        let error_code =
            failure.or((job.ready == 0 && job.failed == 0).then_some(ImportError::NoEligibleFiles));
        tx.execute("UPDATE image_import_jobs SET state=?2,phase='complete',lease_owner=NULL,lease_until=NULL,current='',sequence=sequence+1,error_code=?3,updated_at=?4 WHERE id=?1",params![id,if failure.is_some()||job.failed>0{"failed"}else{"complete"},error_code.map(|e|wire(&e)).transpose()?,now_secs()])?;
        let result = load_job(&tx, id)?;
        tx.commit()?;
        Ok(result)
    }
}

/// Restore validation is metadata-only and precedes live/rollback mutation.
pub(crate) fn validate_database(db: &Connection) -> Result<()> {
    let jobs: i64 = db.query_row("SELECT count(*) FROM image_import_jobs", [], |r| r.get(0))?;
    let (rows, bytes): (i64, i64) = db.query_row(
        "SELECT rows,bytes FROM image_import_budget WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (actual_rows,item_bytes):(i64,i64)=db.query_row("SELECT count(*),COALESCE(sum(1024+length(CAST(relative_path AS BLOB))),0) FROM image_import_items",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let job_bytes:i64=db.query_row("SELECT COALESCE(sum(40960+COALESCE(length(CAST(paths AS BLOB)),0)+length(CAST(folders AS BLOB))+length(CAST(folder_stamps AS BLOB))),0) FROM image_import_jobs",[],|r|r.get(0))?;
    let active: i64 = db.query_row(
        "SELECT count(*) FROM image_import_jobs WHERE state IN ('queued','running')",
        [],
        |r| r.get(0),
    )?;
    if active > MAX_ACTIVE as i64 {
        return Err(ImportError::InvalidArgument);
    }
    if jobs > MAX_HISTORY as i64
        || rows != actual_rows
        || bytes != item_bytes + job_bytes
        || rows > MAX_ITEMS as i64
        || bytes > MAX_JOB_BYTES as i64
    {
        return Err(ImportError::InvalidArgument);
    }
    let mut query = db.prepare(&format!("SELECT {JOB_COLUMNS} FROM image_import_jobs"))?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let job = map_job(row)?;
        if Uuid::parse_str(&job.id).is_err()
            || job.source_id.len() > 256
            || job.current.len() > 32768
            || job.sequence.parse::<u64>().is_err()
        {
            return Err(ImportError::InvalidArgument);
        }
        if let Some(paths) = &job.paths {
            if paths.is_empty()
                || paths.len() > MAX_PATHS
                || serde_json::to_vec(paths)?.len() > MAX_SELECTION_BYTES
            {
                return Err(ImportError::InvalidArgument);
            }
            let mut sorted = paths.clone();
            sorted.sort();
            sorted.dedup();
            if &sorted != paths {
                return Err(ImportError::InvalidArgument);
            }
            for p in paths {
                validate_relative(p).map_err(|_| ImportError::InvalidArgument)?;
            }
        }
        let mut sorted = job.folders.clone();
        sorted.sort();
        sorted.dedup();
        if sorted != job.folders || job.paths.is_none() && job.folders.is_empty() {
            return Err(ImportError::InvalidArgument);
        }
        if job.folders.len() > MAX_PATHS {
            return Err(ImportError::InvalidArgument);
        }
        for p in &job.folders {
            if !p.is_empty() {
                validate_relative(p).map_err(|_| ImportError::InvalidArgument)?;
            }
        }
        if serde_json::to_vec(&(&job.paths, &job.folders))?.len() > MAX_SELECTION_BYTES {
            return Err(ImportError::LimitExceeded);
        }
        if let Some(stamp) = &job.root_stamp {
            serde_json::from_str::<crate::image_import_preview::RootStamp>(stamp)?.validate()?;
        }
        for (prefix, stamp) in &job.folder_stamps {
            if !job.folders.contains(prefix) {
                return Err(ImportError::InvalidArgument);
            }
            stamp.validate()?;
        }
        if !job.history_truncated {
            let counts:(u32,u32,u32,u32)=db.query_row("SELECT count(*),COALESCE(sum(state='ready'),0),COALESCE(sum(state='failed'),0),COALESCE(sum(state='unsupported'),0) FROM image_import_items WHERE job_id=?1",[&job.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
            if counts != (job.total, job.ready, job.failed, job.unsupported)
                || job.completed != job.ready + job.failed + job.unsupported
            {
                return Err(ImportError::InvalidArgument);
            }
        }
    }
    let mut identities = db.prepare("SELECT root_stamp FROM image_source_identity")?;
    let mut identity_rows = identities.query([])?;
    while let Some(row) = identity_rows.next()? {
        serde_json::from_str::<crate::image_import_preview::RootStamp>(&row.get::<_, String>(0)?)?
            .validate()?;
    }
    let mut warning_query = db.prepare("SELECT warnings FROM image_analysis_warnings")?;
    let mut warning_rows = warning_query.query([])?;
    while let Some(row) = warning_rows.next()? {
        let warnings: Vec<crate::image_analysis::AnalysisWarning> =
            serde_json::from_str(&row.get::<_, String>(0)?)?;
        if warnings.len() > 8
            || warnings.contains(&crate::image_analysis::AnalysisWarning::ExtensionMismatch)
        {
            return Err(ImportError::InvalidArgument);
        }
    }
    let mut query = db.prepare(&format!("SELECT {ITEM_COLUMNS} FROM image_import_items"))?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let item = map_item(row)?;
        if item.warnings.len() > 8 {
            return Err(ImportError::InvalidArgument);
        }
        validate_relative(&item.relative_path).map_err(|_| ImportError::InvalidArgument)?;
        if let Some(asset) = &item.asset_id {
            let kind: Option<String> = db
                .query_row(
                    "SELECT kind FROM asset_registry WHERE id=?1",
                    [asset],
                    |r| r.get(0),
                )
                .optional()?;
            if kind.as_deref() == Some("audio") && item.media_kind != ImportMedia::Audio
                || kind.as_deref().is_some_and(|k| k != "audio")
                    && item.media_kind != ImportMedia::Image
            {
                return Err(ImportError::InvalidArgument);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn fixture() -> (tempfile::TempDir, Catalog, crate::catalog::Source) {
        let d = tempfile::tempdir().unwrap();
        let c = Catalog::open(&d.path().join("test.sqlite")).unwrap();
        let s = c.add_source(d.path()).unwrap();
        (d, c, s)
    }
    #[test]
    fn image_jobs_deduplicate_generations_and_remain_out_of_audio_jobs() {
        let (_d, c, s) = fixture();
        let j = c
            .enqueue_image_import(&s.id, 0, ImportMode::Mixed, None)
            .unwrap();
        assert_eq!(
            j,
            c.enqueue_image_import(&s.id, 0, ImportMode::Mixed, None)
                .unwrap()
        );
        assert_eq!(
            c.enqueue_image_import(&s.id, 0, ImportMode::Images, None),
            Err(ImportError::SourceConflict)
        );
        assert!(c.active_jobs().unwrap().is_empty());
        assert_eq!(
            c.enqueue_image_import(&s.id, 1, ImportMode::Mixed, None),
            Err(ImportError::SourceChanged)
        );
        assert!(c.require_audio_only_source(&s.id).is_err());
        assert!(c.export_portable().is_err());
        let encoded = serde_json::to_value(j).unwrap();
        assert_eq!(encoded["source_generation"], "0");
        assert!(encoded.get("paths").is_none());
        assert!(encoded.get("lease_owner").is_none());
    }
    #[test]
    fn exact_paths_must_be_authorized_and_cannot_become_a_recursive_job() {
        let (_d, c, s) = fixture();
        c.db.execute("UPDATE sources SET scope='files' WHERE id=?1", [&s.id])
            .unwrap();
        c.db.execute(
            "INSERT INTO source_files(source_id,relative_path) VALUES(?1,'only.png')",
            [&s.id],
        )
        .unwrap();
        assert_eq!(
            c.enqueue_image_import(&s.id, 0, ImportMode::Images, None),
            Err(ImportError::PermissionDenied)
        );
        assert_eq!(
            c.enqueue_image_import(
                &s.id,
                0,
                ImportMode::Images,
                Some(vec!["sibling.png".into()])
            ),
            Err(ImportError::PermissionDenied)
        );
        let j = c
            .enqueue_image_import(
                &s.id,
                0,
                ImportMode::Images,
                Some(vec!["only.png".into(), "only.png".into()]),
            )
            .unwrap();
        assert_eq!(j.paths, Some(vec!["only.png".into()]));
        validate_database(&c.db).unwrap();
    }
    #[test]
    fn cancelled_lease_cannot_commit_even_a_valid_profile() {
        let (_d, mut c, s) = fixture();
        let j = c
            .enqueue_image_import(&s.id, 0, ImportMode::Images, None)
            .unwrap();
        let claimed = c
            .claim_next_image_import("worker", now_secs())
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, j.id);
        let image = c
            .register_image(&s, "one.png", Some(&"a".repeat(64)))
            .unwrap();
        c.cancel_image_import(&j.id, 0).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/contracts/image-v1.json"))
                .unwrap();
        let profile = serde_json::from_value(v["images"][0]["profile"].clone()).unwrap();
        assert!(c
            .publish_image_when(&s, &image.header.id, &"a".repeat(64), &profile, |db| {
                check_lease(db, &j.id, "worker", now_secs())
                    .map(|_| ())
                    .map_err(|_| crate::invalid("Lease lost"))
            })
            .is_err());
        assert_eq!(
            c.image(&image.header.id, None)
                .unwrap()
                .header
                .technical_state,
            crate::image_contracts::TechnicalState::Pending
        );
        assert_eq!(
            c.image_import_job(&j.id).unwrap().state,
            ImportState::Cancelled
        );
    }
    #[test]
    fn recovery_resets_incomplete_work_and_retains_ready_asset_metadata() {
        let (_d, mut c, s) = fixture();
        let image = c
            .register_image(&s, "retained.png", Some(&"a".repeat(64)))
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/contracts/image-v1.json"))
                .unwrap();
        let profile = serde_json::from_value(v["images"][0]["profile"].clone()).unwrap();
        let image = c
            .publish_image(&s, &image.header.id, &"a".repeat(64), &profile)
            .unwrap();
        let mut annotations = image.annotations.clone();
        annotations.comment = "retained work".into();
        let image = c
            .set_image_annotations(&image.header.id, Some(&"a".repeat(64)), &annotations)
            .unwrap();
        let j = c
            .enqueue_image_import(&s.id, 0, ImportMode::Images, None)
            .unwrap();
        c.claim_next_image_import("dead", 100).unwrap().unwrap();
        c.db.execute(
            "UPDATE image_import_jobs SET lease_until=?2 WHERE id=?1",
            params![j.id, 100],
        )
        .unwrap();
        c.recover_image_imports(101).unwrap();
        let next = c.image_import_job(&j.id).unwrap();
        assert_eq!(next.state, ImportState::Queued);
        assert_eq!(next.phase, ImportPhase::Discovery);
        assert_eq!(next.epoch, 2);
        assert_eq!(
            c.claim_next_image_import("overflow", i64::MAX),
            Err(ImportError::LimitExceeded)
        );
        assert_eq!(
            c.image_import_job(&j.id).unwrap().state,
            ImportState::Queued
        );
        assert_eq!(c.image(&image.header.id, None).unwrap(), image);
        validate_database(&c.db).unwrap();
    }
    #[test]
    fn discovery_items_progress_and_history_survive_backup_restore() {
        let (d, mut c, s) = fixture();
        let j = c
            .enqueue_image_import(&s.id, 0, ImportMode::Images, None)
            .unwrap();
        c.claim_next_image_import("worker", now_secs())
            .unwrap()
            .unwrap();
        c.image_import_discovered(
            &j.id,
            "worker",
            &[
                DiscoveredImport {
                    relative_path: "一.png".into(),
                    media_kind: ImportMedia::Image,
                },
                DiscoveredImport {
                    relative_path: "unsupported.txt".into(),
                    media_kind: ImportMedia::Unsupported,
                },
            ],
        )
        .unwrap();
        c.image_import_discovery_finished(&j.id, "worker", true)
            .unwrap();
        let first = c.image_import_next_item(&j.id, "worker").unwrap().unwrap();
        c.image_import_finish_item(
            &j.id,
            "worker",
            &first.relative_path,
            ImportItemState::Failed,
            None,
            None,
            Some(ImportError::DecodeFailed),
            false,
        )
        .unwrap();
        let second = c.image_import_next_item(&j.id, "worker").unwrap().unwrap();
        c.image_import_finish_item(
            &j.id,
            "worker",
            &second.relative_path,
            ImportItemState::Failed,
            None,
            None,
            Some(ImportError::DecodeFailed),
            false,
        )
        .unwrap();
        c.image_import_finish(&j.id, "worker", None).unwrap();
        let before = c.image_import_job(&j.id).unwrap();
        validate_database(&c.db).unwrap();
        let backup = d.path().join("backup.sqlite");
        c.backup_database(&backup).unwrap();
        c.retry_image_import(&j.id, 0).unwrap();
        let retry = c
            .image_import_jobs(None, 50)
            .unwrap()
            .into_iter()
            .find(|j| !j.terminal())
            .unwrap();
        c.cancel_image_import(&retry.id, 0).unwrap();
        c.restore_database(&backup, &d.path().join("rollback.sqlite"))
            .unwrap();
        assert_eq!(c.image_import_job(&j.id).unwrap(), before);
        let page = c.image_import_items(&j.id, None, 1).unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(page.next_after.is_some());
        assert_eq!(
            c.image_import_items(&j.id, page.next_after.as_deref(), 1)
                .unwrap()
                .items
                .len(),
            1
        );
    }
    #[test]
    fn tampered_history_counts_are_rejected_before_live_or_rollback_mutation() {
        let (d, mut c, s) = fixture();
        let j = c
            .enqueue_image_import(&s.id, 0, ImportMode::Images, None)
            .unwrap();
        c.cancel_image_import(&j.id, 0).unwrap();
        let backup = d.path().join("backup.sqlite");
        c.backup_database(&backup).unwrap();
        let db = Connection::open(&backup).unwrap();
        db.execute("UPDATE image_import_jobs SET total=1", [])
            .unwrap();
        drop(db);
        let rollback = d.path().join("unused.sqlite");
        assert!(c.restore_database(&backup, &rollback).is_err());
        assert!(!rollback.exists());
        assert_eq!(c.image_import_job(&j.id).unwrap().total, 0);
    }
    #[test]
    fn fresh_memory_schema_has_consistent_empty_budget() {
        let c = Catalog::open(Path::new(":memory:")).unwrap();
        validate_database(&c.db).unwrap();
    }
    #[test]
    fn item_cap_evicts_only_bounded_terminal_history() {
        let (_d, c, s) = fixture();
        let j = c
            .enqueue_image_import(&s.id, 0, ImportMode::Images, None)
            .unwrap();
        c.db.execute("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<100000) INSERT INTO image_import_items(job_id,relative_path,media_kind,state) SELECT ?1,printf('grid/%06d.png',i),'image','pending' FROM n",[&j.id]).unwrap();
        c.db.execute(
            "UPDATE image_import_jobs SET total=100000 WHERE id=?1",
            [&j.id],
        )
        .unwrap();
        assert!(c.db.execute("INSERT INTO image_import_items(job_id,relative_path,media_kind,state) VALUES(?1,'extra.png','image','pending')",[&j.id]).is_err());
        assert_eq!(
            c.db.query_row("SELECT rows FROM image_import_budget", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            100000
        );
        c.cancel_image_import(&j.id, 0).unwrap();
        let next = c
            .enqueue_image_import(&s.id, 0, ImportMode::Images, None)
            .unwrap();
        c.claim_next_image_import("worker", now_secs())
            .unwrap()
            .unwrap();
        c.image_import_discovered(
            &next.id,
            "worker",
            &[DiscoveredImport {
                relative_path: "new.png".into(),
                media_kind: ImportMedia::Image,
            }],
        )
        .unwrap();
        assert!(c.image_import_job(&j.id).unwrap().history_truncated);
        assert_eq!(
            c.db.query_row("SELECT rows FROM image_import_budget", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            99969
        );
        validate_database(&c.db).unwrap();
    }
    #[test]
    fn source_lanes_are_exclusive_but_other_sources_are_not_blocked() {
        let (_d, c, s) = fixture();
        let audio = c.enqueue_scan(&s.id).unwrap();
        assert_eq!(
            c.enqueue_image_import(&s.id, 0, ImportMode::Mixed, None),
            Err(ImportError::SourceConflict)
        );
        c.cancel_queued().unwrap();
        let images = c
            .enqueue_image_import(&s.id, 0, ImportMode::Mixed, None)
            .unwrap();
        assert!(c.enqueue_scan(&s.id).is_err());
        let second = tempfile::tempdir().unwrap();
        let other = c.add_source(second.path()).unwrap();
        assert!(c.enqueue_scan(&other.id).is_ok());
        assert!(c.job(&audio.id).is_ok());
        c.cancel_image_import(&images.id, 0).unwrap();
        assert!(c.enqueue_scan(&s.id).is_ok());
    }
    #[test]
    fn recovery_can_rediscover_existing_items_at_the_exact_global_cap() {
        let (_d, c, s) = fixture();
        let j = c
            .enqueue_image_import(&s.id, 0, ImportMode::Images, None)
            .unwrap();
        c.db.execute("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<100000) INSERT INTO image_import_items(job_id,relative_path,media_kind,state) SELECT ?1,printf('grid/%06d.png',i),'image','pending' FROM n",[&j.id]).unwrap();
        c.db.execute(
            "UPDATE image_import_jobs SET total=100000 WHERE id=?1",
            [&j.id],
        )
        .unwrap();
        c.claim_next_image_import("worker", now_secs()).unwrap();
        c.image_import_discovered(
            &j.id,
            "worker",
            &[DiscoveredImport {
                relative_path: "grid/000001.png".into(),
                media_kind: ImportMedia::Image,
            }],
        )
        .unwrap();
        assert_eq!(c.image_import_job(&j.id).unwrap().total, 100000);
        validate_database(&c.db).unwrap();
    }
    #[test]
    fn escaped_long_paths_page_under_wire_budget_without_losing_items() {
        let (_d, c, s) = fixture();
        let j = c
            .enqueue_image_import(&s.id, 0, ImportMode::Images, None)
            .unwrap();
        let path = "\"".repeat(30000);
        for i in 0..60 {
            c.db.execute("INSERT INTO image_import_items(job_id,relative_path,media_kind,state) VALUES(?1,?2,'image','pending')",params![j.id,format!("{path}/{i:03}.png")]).unwrap();
        }
        let mut after = None;
        let mut count = 0;
        loop {
            let page = c.image_import_items(&j.id, after.as_deref(), 50).unwrap();
            assert!(serde_json::to_vec(&page).unwrap().len() <= 2 * 1024 * 1024);
            count += page.items.len();
            after = page.next_after;
            if after.is_none() {
                break;
            }
        }
        assert_eq!(count, 60);
    }
    #[test]
    fn v11_mid_migration_failure_rolls_back_all_new_job_tables_and_version() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v10.sqlite");
        let db = Connection::open(&path).unwrap();
        for schema in [
            include_str!("schema.sql"),
            include_str!("source_catalog_schema.sql"),
            include_str!("settings_schema.sql"),
            include_str!("search_profile_schema.sql"),
            include_str!("image_schema.sql"),
        ] {
            db.execute_batch(schema).unwrap();
        }
        db.pragma_update(None, "user_version", 10).unwrap();
        db.execute_batch("CREATE TABLE image_import_items(collision TEXT); INSERT INTO image_import_items VALUES('retained');").unwrap();
        drop(db);
        assert!(Catalog::open(&path).is_err());
        let db = Connection::open(&path).unwrap();
        assert_eq!(
            db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
                .unwrap(),
            10
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='image_import_jobs'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT collision FROM image_import_items", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "retained"
        );
    }
    #[test]
    fn genuine_v10_upgrade_backs_up_and_preserves_image_user_work() {
        use crate::catalog::SCHEMA_VERSION;
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v10.sqlite");
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        for schema in [
            include_str!("schema.sql"),
            include_str!("source_catalog_schema.sql"),
            include_str!("settings_schema.sql"),
            include_str!("search_profile_schema.sql"),
            include_str!("image_schema.sql"),
        ] {
            db.execute_batch(schema).unwrap();
        }
        db.pragma_update(None, "user_version", 10).unwrap();
        let mut old = Catalog { db };
        let source = old.add_source(d.path()).unwrap();
        let image = old
            .register_image(&source, "old.png", Some(&"a".repeat(64)))
            .unwrap();
        let mut notes = image.annotations.clone();
        notes.comment = "before jobs".into();
        let image = old
            .set_image_annotations(&image.header.id, Some(&"a".repeat(64)), &notes)
            .unwrap();
        drop(old);
        let c = Catalog::open(&path).unwrap();
        assert_eq!(c.image(&image.header.id, None).unwrap(), image);
        validate_database(&c.db).unwrap();
        let backup = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.to_string_lossy()
                    .contains(&format!("pre-v{}-", SCHEMA_VERSION))
            })
            .unwrap();
        let db = Connection::open(backup).unwrap();
        assert_eq!(
            db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
                .unwrap(),
            10
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='image_import_jobs'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(Catalog { db }.image(&image.header.id, None).unwrap(), image);
    }
}
