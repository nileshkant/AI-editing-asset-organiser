//! SS-047 source lifecycle: availability probes, refresh/check jobs, the
//! unchanged-file fast path, app-owned path exclusion and explicit move
//! reconciliation. See docs/IMAGE_SOURCES.md. Filesystem work stays outside
//! catalog locks; every mutation is fenced by source generation or revision.
use crate::{
    catalog::{Catalog, Source},
    image_catalog::load,
    image_contracts::{validate_relative, Availability, ImageAsset},
    image_import_preview::RootStamp,
    image_jobs::{
        enqueue_job_on, ImageImportJob, ImportError as E, ImportMode, JobOrigin, JobSpec, Result,
        ScanKind, MAX_PATHS,
    },
    jobs::now_secs,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

/// A stat is trusted only when the file was last written at least this long
/// before it was recorded. FAT/exFAT timestamps have 2 s resolution, so a write
/// in the same tick as hashing could otherwise keep an identical timestamp.
const RACY_NS: i64 = 2_000_000_000;
const MAX_MONITORED: usize = 1000;
const MAX_CANDIDATES: u32 = 50;
const MAX_LISTED: usize = 100_000;

macro_rules! values {($name:ident{$($value:ident),*})=>{#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]#[serde(rename_all="snake_case")]pub enum $name{$($value),*}};}
values!(RootState {
    Unknown,
    Online,
    Offline,
    Inaccessible,
    Changed
});

fn lock(db: &Arc<Mutex<Catalog>>) -> Result<std::sync::MutexGuard<'_, Catalog>> {
    db.lock().map_err(|_| E::Internal)
}
fn nanos(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_nanos()).map_or(i64::MIN, |n| -n),
    }
}

/// Size, timestamps and OS file identity of one member at one moment.
#[derive(Debug, Clone)]
pub(crate) struct FileStamp {
    pub(crate) volume: String,
    pub(crate) file: String,
    size: u64,
    modified: i64,
    changed: Option<i64>,
    observed: i64,
}
impl FileStamp {
    pub(crate) fn read(path: &Path) -> std::io::Result<Self> {
        let observed = nanos(SystemTime::now());
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() {
            return Err(std::io::Error::other("not a regular file"));
        }
        let modified = nanos(metadata.modified()?);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                volume: metadata.dev().to_string(),
                file: metadata.ino().to_string(),
                size: metadata.len(),
                modified,
                changed: metadata
                    .ctime()
                    .checked_mul(1_000_000_000)
                    .and_then(|s| s.checked_add(metadata.ctime_nsec())),
                observed,
            })
        }
        #[cfg(windows)]
        {
            let (volume, file, changed) = windows_identity(path)?;
            Ok(Self {
                volume,
                file,
                size: metadata.len(),
                modified,
                changed,
                observed,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(std::io::Error::other("file identity unsupported"))
        }
    }
    fn same_file(&self, volume: &str, file: &str) -> bool {
        self.volume == volume && self.file == file
    }
    /// Same file and same write evidence; the observation time is not compared.
    pub(crate) fn unchanged_since(&self, other: &Self) -> bool {
        self.same_file(&other.volume, &other.file)
            && self.size == other.size
            && self.modified == other.modified
            && self.changed == other.changed
    }
}
#[cfg(windows)]
fn windows_identity(path: &Path) -> std::io::Result<(String, String, Option<i64>)> {
    use std::{
        mem::MaybeUninit,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    };
    // Attribute-only access: no read handle on the original, shared with writers.
    let file = fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    let mut basic = MaybeUninit::<FILE_BASIC_INFO>::uninit();
    // NTFS ChangeTime moves on every content or metadata write, like Unix ctime.
    let changed = (unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            basic.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    } != 0)
        .then(|| unsafe { basic.assume_init() }.ChangeTime)
        .and_then(|t| t.checked_sub(116_444_736_000_000_000))
        .and_then(|t| t.checked_mul(100));
    Ok((
        info.dwVolumeSerialNumber.to_string(),
        ((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)).to_string(),
        changed,
    ))
}

pub(crate) fn record_stamp(
    db: &Connection,
    asset_id: &str,
    hash: &str,
    stamp: &FileStamp,
) -> crate::Result<()> {
    db.execute("INSERT INTO asset_file_stats(asset_id,content_hash,volume,file_id,size,modified,changed,recorded) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
        ON CONFLICT(asset_id) DO UPDATE SET content_hash=excluded.content_hash,volume=excluded.volume,file_id=excluded.file_id,size=excluded.size,modified=excluded.modified,changed=excluded.changed,recorded=excluded.recorded",
        params![asset_id,hash,stamp.volume,stamp.file,i64::try_from(stamp.size).unwrap_or(i64::MAX),stamp.modified,stamp.changed,stamp.observed])?;
    Ok(())
}
/// True only when the stored stat for exactly this content version matches and
/// was not recorded in the racy window. A false result just means "hash it".
pub(crate) fn trusted_unchanged(
    db: &Connection,
    asset_id: &str,
    hash: &str,
    stamp: &FileStamp,
) -> Result<bool> {
    let stored: Option<(String, String, i64, i64, Option<i64>, i64)> = db
        .query_row(
            "SELECT volume,file_id,size,modified,changed,recorded FROM asset_file_stats WHERE asset_id=?1 AND content_hash=?2",
            params![asset_id, hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let Some((volume, file, size, modified, changed, recorded)) = stored else {
        return Ok(false);
    };
    let written = changed.map_or(modified, |c| c.max(modified));
    Ok(stamp.same_file(&volume, &file)
        && i64::try_from(stamp.size).ok() == Some(size)
        && stamp.modified == modified
        && stamp.changed == changed
        && written.checked_add(RACY_NS).is_some_and(|safe| safe < recorded))
}

/// App-owned sidecars, export staging and editor/download temporaries. Refresh
/// and watching never process these; an explicit exact-file import still may.
pub fn app_owned_or_transient(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(".creativeshelf")
        || lower.starts_with(".soundshelf")
        || lower.starts_with("._")
        || lower.starts_with("~$")
        || lower.starts_with(".#")
        || lower.starts_with(".~")
        || lower.ends_with('~')
        || [".tmp", ".temp", ".part", ".partial", ".crdownload", ".download", ".swp"]
            .iter()
            .any(|s| lower.ends_with(s))
        || matches!(lower.as_str(), ".ds_store" | "thumbs.db" | "desktop.ini")
}
pub fn admitted_relative(relative: &str) -> bool {
    !relative.split('/').any(app_owned_or_transient)
}

/// The exact on-disk entry name exists (case and normalization sensitive), so a
/// case-insensitive lookup cannot mistake a renamed file for the old one.
pub(crate) fn exact_entry_exists(root: &Path, relative: &str) -> bool {
    let path = root.join(relative);
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return true;
    };
    match fs::read_dir(parent) {
        Ok(entries) => entries.flatten().any(|e| e.file_name() == name),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageSourceStatus {
    pub source_id: String,
    pub name: String,
    pub root: String,
    pub scope: String,
    pub source_generation: String,
    pub root_state: RootState,
    pub watch: bool,
    pub mode: ImportMode,
    pub checked_at: u64,
    pub images: u32,
    pub missing: u32,
    pub inaccessible: u32,
    pub failed: u32,
    pub pending: u32,
    pub move_candidates: u32,
    pub active_job: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MoveCandidate {
    pub missing_id: String,
    pub missing_path: String,
    pub missing_revision: u32,
    pub present_id: String,
    pub present_path: String,
    pub present_revision: u32,
    /// Both records last observed the same OS file (a hard link or missed move).
    pub same_file: bool,
}
pub struct Probe {
    pub root_state: RootState,
    /// The root just became reachable again; events may have been missed.
    pub reconnected: bool,
}

pub(crate) fn mark_root(db: &Connection, source: &Source, state: RootState) -> Result<()> {
    db.execute(
        "UPDATE sources SET available=?3 WHERE id=?1 AND generation=?2",
        params![source.id, source.generation, state == RootState::Online],
    )?;
    db.execute("INSERT INTO image_source_monitor(source_id,root_state,checked_at) VALUES(?1,?2,?3)
        ON CONFLICT(source_id) DO UPDATE SET root_state=excluded.root_state,checked_at=excluded.checked_at",
        params![source.id,serde_json::to_value(state)?.as_str(),now_secs()])?;
    Ok(())
}
pub(crate) fn root_state(error: E) -> RootState {
    match error {
        E::Inaccessible | E::PermissionDenied => RootState::Inaccessible,
        E::SourceChanged => RootState::Changed,
        _ => RootState::Offline,
    }
}
fn source_mode(db: &Connection, id: &str) -> Result<ImportMode> {
    let mixed: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM image_source_monitor WHERE source_id=?1 AND mode='mixed') OR EXISTS(SELECT 1 FROM sounds WHERE source_id=?1)",[id],|r|r.get(0))?;
    Ok(if mixed {
        ImportMode::Mixed
    } else {
        ImportMode::Images
    })
}
fn count(v: i64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}
/// Sources whose image metadata this module manages; `s` is the sources row.
pub(crate) const IMAGE_MANAGED: &str = "(EXISTS(SELECT 1 FROM image_source_monitor WHERE source_id=s.id)
 OR EXISTS(SELECT 1 FROM asset_registry WHERE source_id=s.id AND kind!='audio')
 OR EXISTS(SELECT 1 FROM image_source_identity WHERE source_id=s.id))";
const MOVE_MATCH: &str = "FROM asset_registry rm JOIN image_assets m ON m.id=rm.id
 JOIN image_assets p ON p.content_hash=m.content_hash AND p.id!=m.id AND p.availability='available'
 JOIN asset_registry rp ON rp.id=p.id AND rp.source_id=rm.source_id AND rp.kind='image'
 WHERE rm.source_id=?1 AND rm.kind='image' AND m.availability='missing' AND m.content_hash IS NOT NULL";

impl Catalog {
    /// Cheap classification for probes; `status_row` adds the bounded counts.
    fn root_state_of(&self, id: &str) -> Result<RootState> {
        let (available, state): (bool, String) = self.db.query_row(&format!("SELECT s.available,COALESCE(m.root_state,'unknown')
            FROM sources s LEFT JOIN image_source_monitor m ON m.source_id=s.id WHERE s.id=?1 AND {IMAGE_MANAGED}"),[id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(E::NotFound)?;
        let state: RootState = serde_json::from_value(state.into())?;
        // A worker may mark a root offline without a probe having classified it.
        Ok(if !available && matches!(state, RootState::Online | RootState::Unknown) {
            RootState::Offline
        } else {
            state
        })
    }
    fn status_row(&self, id: &str) -> Result<ImageSourceStatus> {
        let row = self.db.query_row(&format!("SELECT s.id,s.name,s.root,s.scope,s.generation,s.available,COALESCE(m.watch,0),COALESCE(m.mode,'images'),
            COALESCE(m.root_state,'unknown'),COALESCE(m.checked_at,0)
            FROM sources s LEFT JOIN image_source_monitor m ON m.source_id=s.id WHERE s.id=?1 AND {IMAGE_MANAGED}"),[id],|r|Ok((
            r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,
            r.get::<_,bool>(5)?,r.get::<_,bool>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?,r.get::<_,i64>(9)?))).optional()?
            .ok_or(E::NotFound)?;
        let (source_id, name, root, scope, generation, _available, watch, mode, _state, checked) =
            row;
        let root_state = self.root_state_of(id)?;
        let (images, missing, inaccessible, failed, pending): (i64, i64, i64, i64, i64) =
            self.db.query_row("SELECT count(*),COALESCE(sum(i.availability='missing'),0),COALESCE(sum(i.availability='inaccessible'),0),
                COALESCE(sum(i.technical_state='failed'),0),COALESCE(sum(i.technical_state='pending'),0)
                FROM asset_registry r JOIN image_assets i ON i.id=r.id WHERE r.source_id=?1 AND r.kind='image'",[id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        let moves: i64 = self.db.query_row(
            &format!("SELECT count(*) FROM (SELECT DISTINCT m.id {MOVE_MATCH} LIMIT 1000)"),
            [id],
            |r| r.get(0),
        )?;
        let active_job = self.db.query_row("SELECT id FROM image_import_jobs WHERE source_id=?1 AND state IN ('queued','running') ORDER BY queue_order LIMIT 1",[id],|r|r.get(0)).optional()?;
        Ok(ImageSourceStatus {
            source_id,
            name,
            root,
            scope,
            source_generation: generation.to_string(),
            root_state,
            watch,
            mode: serde_json::from_value(mode.into())?,
            checked_at: u64::try_from(checked).unwrap_or(0),
            images: count(images),
            missing: count(missing),
            inaccessible: count(inaccessible),
            failed: count(failed),
            pending: count(pending),
            move_candidates: count(moves),
            active_job,
        })
    }
    /// Sources that hold image metadata (or are importing it), with bounded counts.
    pub fn image_source_statuses(&self) -> Result<Vec<ImageSourceStatus>> {
        let ids = self
            .db
            .prepare(&format!("SELECT s.id FROM sources s WHERE {IMAGE_MANAGED} ORDER BY s.name,s.id LIMIT ?1"))?
            .query_map([MAX_MONITORED as i64], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.iter().map(|id| self.status_row(id)).collect()
    }
    pub fn image_source_status(&self, id: &str) -> Result<ImageSourceStatus> {
        self.status_row(id)
    }
    /// Opt-in only; an import never enables watching.
    pub fn set_image_source_watch(&self, id: &str, enabled: bool) -> Result<ImageSourceStatus> {
        self.status_row(id)?;
        self.db.execute("INSERT INTO image_source_monitor(source_id,watch) VALUES(?1,?2) ON CONFLICT(source_id) DO UPDATE SET watch=excluded.watch",params![id,enabled])?;
        self.status_row(id)
    }
    pub fn image_move_candidates(&self, source_id: &str) -> Result<Vec<MoveCandidate>> {
        let sql = format!("SELECT m.id,rm.relative_path,m.annotation_revision,p.id,rp.relative_path,p.annotation_revision,
            EXISTS(SELECT 1 FROM asset_file_stats a JOIN asset_file_stats b ON b.volume=a.volume AND b.file_id=a.file_id WHERE a.asset_id=m.id AND b.asset_id=p.id)
            {MOVE_MATCH} ORDER BY rm.relative_path,rp.relative_path LIMIT ?2");
        let rows = self
            .db
            .prepare(&sql)?
            .query_map(params![source_id, MAX_CANDIDATES], |r| {
                Ok(MoveCandidate {
                    missing_id: r.get(0)?,
                    missing_path: r.get(1)?,
                    missing_revision: r.get(2)?,
                    present_id: r.get(3)?,
                    present_path: r.get(4)?,
                    present_revision: r.get(5)?,
                    same_file: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    /// Explicit reconciliation: the missing record (with the creator's work) takes
    /// over the present copy's path and the unannotated duplicate record is removed.
    /// Content equality alone never triggers this automatically.
    pub fn reconcile_image_move(
        &mut self,
        missing_id: &str,
        present_id: &str,
        missing_revision: u32,
        present_revision: u32,
    ) -> Result<ImageAsset> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let missing = load(&tx, missing_id).map_err(|_| E::NotFound)?;
        let present = load(&tx, present_id).map_err(|_| E::NotFound)?;
        let (m, p) = (&missing.header, &present.header);
        if m.source_id != p.source_id || missing_id == present_id {
            return Err(E::PermissionDenied);
        }
        let busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM image_import_jobs WHERE source_id=?1 AND state IN ('queued','running'))",[&m.source_id],|r|r.get(0))?;
        if busy {
            return Err(E::SourceConflict);
        }
        let a = &present.annotations;
        if missing.annotations.revision != missing_revision
            || a.revision != present_revision
            || m.availability != Availability::Missing
            || p.availability != Availability::Available
            || m.content_hash.is_none()
            || m.content_hash != p.content_hash
            || !a.tags.is_empty()
            || !a.comment.is_empty()
            || a.favorite
            || a.rating.is_some()
            || a.caption.is_some()
        {
            return Err(E::Conflict);
        }
        tx.execute("DELETE FROM asset_file_stats WHERE asset_id=?1", [missing_id])?;
        tx.execute(
            "UPDATE asset_file_stats SET asset_id=?2 WHERE asset_id=?1",
            params![present_id, missing_id],
        )?;
        tx.execute("DELETE FROM asset_registry WHERE id=?1", [present_id])?;
        tx.execute(
            "UPDATE asset_registry SET relative_path=?2 WHERE id=?1",
            params![missing_id, p.relative_path],
        )?;
        // Same bytes: adopt the present copy's verified technical state.
        tx.execute("UPDATE image_assets SET availability='available',
            technical_state=CASE WHEN ?2='ready' THEN 'ready' ELSE technical_state END,
            analyzer_version=CASE WHEN ?2='ready' THEN ?3 ELSE analyzer_version END WHERE id=?1",
            params![missing_id,serde_json::to_value(p.technical_state)?.as_str(),present.profile.as_ref().map(|x|x.analyzer_version.clone())])?;
        let result = load(&tx, missing_id).map_err(|_| E::Internal)?;
        tx.commit()?;
        Ok(result)
    }
}

/// Restore validation: CHECK constraints are not proof for an imported file.
pub(crate) fn validate_tables(db: &Connection) -> Result<()> {
    let bad_stats: i64 = db.query_row("SELECT count(*) FROM asset_file_stats f LEFT JOIN asset_registry r ON r.id=f.asset_id
        WHERE r.id IS NULL OR typeof(f.content_hash)!='text' OR length(f.content_hash)!=64 OR f.content_hash GLOB '*[^0-9a-f]*'
        OR typeof(f.volume)!='text' OR length(CAST(f.volume AS BLOB)) NOT BETWEEN 1 AND 128
        OR typeof(f.file_id)!='text' OR length(f.file_id) NOT BETWEEN 1 AND 20 OR f.file_id GLOB '*[^0-9]*'
        OR typeof(f.size)!='integer' OR f.size<0 OR typeof(f.modified)!='integer' OR typeof(f.recorded)!='integer'
        OR f.changed IS NOT NULL AND typeof(f.changed)!='integer'",[],|r|r.get(0))?;
    let bad_monitor: i64 = db.query_row("SELECT count(*) FROM image_source_monitor m LEFT JOIN sources s ON s.id=m.source_id
        WHERE s.id IS NULL OR m.watch NOT IN (0,1) OR m.mode NOT IN ('images','mixed')
        OR m.root_state NOT IN ('unknown','online','offline','inaccessible','changed') OR typeof(m.checked_at)!='integer' OR m.checked_at<0",[],|r|r.get(0))?;
    let bad_jobs: i64 = db.query_row("SELECT count(*) FROM image_import_jobs WHERE scan_kind NOT IN ('discover','listed') OR verify NOT IN (0,1) OR origin NOT IN ('import','refresh','watch','relink')",[],|r|r.get(0))?;
    if bad_stats + bad_monitor + bad_jobs != 0 {
        return Err(E::InvalidArgument);
    }
    Ok(())
}

/// Classify the root without holding the catalog lock, then publish the result
/// for the generation that was examined.
pub fn probe_image_source(db: &Arc<Mutex<Catalog>>, id: &str) -> Result<Probe> {
    probe(db, id, true)
}
/// Background monitoring: writes only when the classification changes.
pub fn watch_probe(db: &Arc<Mutex<Catalog>>, id: &str) -> Result<Probe> {
    probe(db, id, false)
}
fn probe(db: &Arc<Mutex<Catalog>>, id: &str, record: bool) -> Result<Probe> {
    let (source, identity, before) = {
        let c = lock(db)?;
        let before = c.root_state_of(id)?;
        let source = c.source_header(id).map_err(|_| E::NotFound)?;
        let identity: Option<String> = c.db.query_row("SELECT root_stamp FROM image_source_identity WHERE source_id=?1 AND source_generation=?2",params![id,source.generation],|r|r.get(0)).optional()?;
        (source, identity, before)
    };
    let state = match RootStamp::read(Path::new(&source.root)) {
        Err(error) => root_state(error),
        Ok(stamp) => match identity {
            Some(known) if !RootStamp::equivalent(&known, &stamp.json()?)? => RootState::Changed,
            _ => RootState::Online,
        },
    };
    let c = lock(db)?;
    let changed = state != before || (state == RootState::Online) != source.available;
    if (record || changed) && c.source_generation(id).map_err(|_| E::NotFound)? == source.generation {
        mark_root(&c.db, &source, state)?;
    }
    Ok(Probe {
        root_state: state,
        reconnected: state == RootState::Online
            && matches!(
                before,
                RootState::Offline | RootState::Inaccessible | RootState::Changed
            ),
    })
}

/// Explicit refresh: `Discover` walks the folder for new and removed files;
/// `Listed` re-checks only cataloged members. `verify` rehashes every byte.
pub fn refresh_image_source(
    db: &Arc<Mutex<Catalog>>,
    id: &str,
    scan: ScanKind,
    verify: bool,
) -> Result<ImageImportJob> {
    let probe = probe_image_source(db, id)?;
    match probe.root_state {
        RootState::Online => {}
        RootState::Inaccessible => return Err(E::Inaccessible),
        RootState::Changed => return Err(E::SourceChanged),
        _ => return Err(E::SourceUnavailable),
    }
    let c = lock(db)?;
    let tx = Transaction::new_unchecked(&c.db, TransactionBehavior::Immediate)?;
    let source = c.source_header(id).map_err(|_| E::NotFound)?;
    if scan == ScanKind::Discover && source.scope != "folder" {
        return Err(E::InvalidArgument);
    }
    let job = enqueue_job_on(
        &tx,
        id,
        source.generation,
        JobSpec {
            mode: source_mode(&tx, id)?,
            paths: None,
            folders: if scan == ScanKind::Discover {
                vec![String::new()]
            } else {
                vec![]
            },
            scan,
            verify,
            origin: JobOrigin::Refresh,
        },
    )?;
    tx.commit()?;
    Ok(job)
}

/// What one OS watcher may report for one source generation.
#[derive(Debug, Clone)]
pub struct WatchScope {
    pub source_id: String,
    pub generation: i64,
    pub root: PathBuf,
    pub recursive: bool,
    pub available: bool,
    /// Exact-file sources: only these members, never their siblings.
    listed: Option<HashSet<String>>,
}
impl WatchScope {
    pub fn relative(&self, path: &Path) -> Option<String> {
        let relative = path
            .strip_prefix(&self.root)
            .ok()?
            .to_str()?
            .replace(std::path::MAIN_SEPARATOR, "/");
        validate_relative(&relative).ok()?;
        self.admits(&relative).then_some(relative)
    }
    pub fn admits(&self, relative: &str) -> bool {
        admitted_relative(relative)
            && self
                .listed
                .as_ref()
                .is_none_or(|listed| listed.contains(relative))
    }
}
/// A coalesced set of stable changes, or a request for a complete refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchBatch {
    Changes {
        files: Vec<String>,
        folders: Vec<String>,
        removed: Vec<String>,
    },
    Refresh,
}
impl Catalog {
    pub fn image_watch_scopes(&self) -> Result<Vec<WatchScope>> {
        let rows = self
            .db
            .prepare("SELECT s.id,s.generation,s.root,s.scope,s.available FROM image_source_monitor m JOIN sources s ON s.id=m.source_id WHERE m.watch=1 ORDER BY s.id LIMIT ?1")?
            .query_map([MAX_MONITORED as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, bool>(4)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut scopes = vec![];
        for (source_id, generation, root, scope, available) in rows {
            let listed = if scope == "folder" {
                None
            } else {
                let files = self
                    .db
                    .prepare("SELECT relative_path FROM source_files WHERE source_id=?1 LIMIT ?2")?
                    .query_map(params![source_id, MAX_LISTED as i64 + 1], |r| r.get(0))?
                    .collect::<rusqlite::Result<HashSet<String>>>()?;
                if files.len() > MAX_LISTED {
                    return Err(E::LimitExceeded);
                }
                Some(files)
            };
            scopes.push(WatchScope {
                source_id,
                generation,
                recursive: scope == "folder",
                root: PathBuf::from(root),
                available,
                listed,
            });
        }
        Ok(scopes)
    }
    pub fn image_source_active(&self, id: &str) -> Result<bool> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM image_import_jobs WHERE source_id=?1 AND state IN ('queued','running')) OR EXISTS(SELECT 1 FROM jobs WHERE source_id=?1 AND state IN ('queued','running'))",[id],|r|r.get(0))?)
    }
    /// Turn stable watcher changes into one quick (non-verifying) job for the
    /// watched generation. Deleted folders and oversize batches escalate to a
    /// complete refresh so missing members are only inferred from complete scans.
    pub fn enqueue_watch_batch(
        &self,
        scope: &WatchScope,
        batch: &WatchBatch,
    ) -> Result<Option<ImageImportJob>> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let mut paths = BTreeSet::new();
        let mut folders = BTreeSet::new();
        let mut refresh = *batch == WatchBatch::Refresh;
        if let WatchBatch::Changes {
            files,
            folders: changed,
            removed,
        } = batch
        {
            paths.extend(files.iter().filter(|p| scope.admits(p)).cloned());
            for path in removed.iter().filter(|p| scope.admits(p)) {
                let exact: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_registry WHERE source_id=?1 AND relative_path=?2)",params![scope.source_id,path],|r|r.get(0))?;
                if exact {
                    paths.insert(path.clone());
                } else if scope.recursive {
                    let nested: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_registry WHERE source_id=?1 AND substr(relative_path,1,length(?2)+1)=?2||'/')",params![scope.source_id,path],|r|r.get(0))?;
                    refresh |= nested;
                }
            }
            if scope.recursive {
                for folder in changed.iter().filter(|p| scope.admits(p)) {
                    if !folders
                        .iter()
                        .any(|p: &String| folder.starts_with(&format!("{p}/")))
                    {
                        folders.retain(|p: &String| !p.starts_with(&format!("{folder}/")));
                        folders.insert(folder.clone());
                    }
                }
            }
            refresh |= paths.len() > MAX_PATHS || folders.len() > MAX_PATHS;
        }
        let (paths, folders, scan) = if refresh {
            if scope.recursive {
                (None, vec![String::new()], ScanKind::Discover)
            } else {
                (None, vec![], ScanKind::Listed)
            }
        } else if paths.is_empty() && folders.is_empty() {
            return Ok(None);
        } else {
            (
                (!paths.is_empty()).then(|| paths.into_iter().collect()),
                folders.into_iter().collect(),
                ScanKind::Discover,
            )
        };
        let job = enqueue_job_on(
            &tx,
            &scope.source_id,
            scope.generation,
            JobSpec {
                mode: source_mode(&tx, &scope.source_id)?,
                paths,
                folders,
                scan,
                verify: false,
                origin: JobOrigin::Watch,
            },
        )?;
        tx.commit()?;
        Ok(Some(job))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn racy_mismatched_or_rewritten_stats_are_never_trusted() {
        let d = tempfile::tempdir().unwrap();
        let mut c = Catalog::open(&d.path().join("db.sqlite")).unwrap();
        let s = c.add_source(d.path()).unwrap();
        let hash = "a".repeat(64);
        let id = c.register_image(&s, "a.png", Some(&hash)).unwrap().header.id;
        let path = d.path().join("a.png");
        fs::write(&path, b"one").unwrap();
        let stamp = FileStamp::read(&path).unwrap();
        assert!(stamp.unchanged_since(&FileStamp::read(&path).unwrap()));
        // Recorded in the same instant as the write: a later write could keep it.
        record_stamp(&c.db, &id, &hash, &stamp).unwrap();
        assert!(!trusted_unchanged(&c.db, &id, &hash, &stamp).unwrap());
        let mut later = stamp.clone();
        later.observed += 3 * RACY_NS;
        record_stamp(&c.db, &id, &hash, &later).unwrap();
        assert!(trusted_unchanged(&c.db, &id, &hash, &stamp).unwrap());
        assert!(!trusted_unchanged(&c.db, &id, &"b".repeat(64), &stamp).unwrap());
        // Same size, modification time restored: the change time still moves.
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&path, b"two").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let rewritten = FileStamp::read(&path).unwrap();
        assert_eq!((rewritten.size, rewritten.modified), (stamp.size, stamp.modified));
        assert!(rewritten.changed.is_some() && rewritten.changed != stamp.changed);
        assert!(!trusted_unchanged(&c.db, &id, &hash, &rewritten).unwrap());
        assert!(FileStamp::read(&d.path().join("absent.png")).is_err());
        assert!(FileStamp::read(d.path()).is_err(), "directories are not members");
    }
    #[test]
    fn app_owned_names_cover_sidecars_staging_and_temporaries() {
        for owned in [".creativeshelf", ".soundshelf-export-1", "a.png.tmp", "a.PNG.part", "x.crdownload", "~$x.png", "._x.png", "a.png~", ".DS_Store", "Thumbs.db"] {
            assert!(app_owned_or_transient(owned), "{owned}");
        }
        for user in ["a.png", "tmp.png", "partial take.png", "café.webp", "export/a.png"] {
            assert!(!app_owned_or_transient(user), "{user}");
        }
        assert!(!admitted_relative("album/.creativeshelf/x.png"));
        assert!(admitted_relative("album/x.png"));
    }
}
