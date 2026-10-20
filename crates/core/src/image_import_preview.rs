//! Native-grant previews and atomic membership/job creation. Filesystem work is
//! outside catalog locks; previewing alone grants no persistent source access.
use crate::{
    catalog::{contained, path_text, Catalog, Source},
    image_contracts::validate_relative,
    image_jobs::{
        self, enqueue_selection_on, ImageImportJob, ImportError as E, ImportMedia, ImportMode,
        Result, MAX_PATHS, MAX_SELECTION_BYTES,
    },
};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
use walkdir::WalkDir;
pub const PREVIEW_TTL: Duration = Duration::from_secs(300);
pub const DISCOVERY_DEADLINE: Duration = Duration::from_secs(30);
pub const MAX_DEPTH: usize = 64;
pub const MAX_FILES: usize = 100_000;
pub const MAX_PREVIEWS: usize = 8;
const MAX_SOURCES: usize = 10_000;
const MAX_CONTEXT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootStamp {
    platform: String,
    volume: String,
    file: String,
    created: Option<String>,
}
impl RootStamp {
    pub(crate) fn read(root: &Path) -> Result<Self> {
        let metadata = fs::metadata(root).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                E::Inaccessible
            } else {
                E::SourceUnavailable
            }
        })?;
        if !metadata.is_dir() {
            return Err(E::SourceUnavailable);
        }
        let created = metadata
            .created()
            .ok()
            .map(|v| match v.duration_since(UNIX_EPOCH) {
                Ok(d) => d.as_nanos().to_string(),
                Err(e) => format!("-{}", e.duration().as_nanos()),
            });
        #[cfg(unix)]
        {
            use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
            let path = std::ffi::CString::new(root.as_os_str().as_bytes())
                .map_err(|_| E::InvalidArgument)?;
            let mut info = unsafe { std::mem::zeroed::<libc::statvfs>() };
            if unsafe { libc::statvfs(path.as_ptr(), &mut info) } != 0 {
                return Err(
                    if std::io::Error::last_os_error().kind()
                        == std::io::ErrorKind::PermissionDenied
                    {
                        E::Inaccessible
                    } else {
                        E::SourceUnavailable
                    },
                );
            }
            let fallback = if info.f_fsid != 0 {
                format!("fs:{}", info.f_fsid)
            } else {
                format!("runtime:{}", metadata.dev())
            };
            #[cfg(target_os = "macos")]
            let volume = {
                let mut attributes = unsafe { std::mem::zeroed::<libc::attrlist>() };
                attributes.bitmapcount = 5;
                attributes.volattr = libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID;
                let mut data = [0u8; 20];
                let success = unsafe {
                    libc::getattrlist(
                        path.as_ptr(),
                        (&mut attributes as *mut libc::attrlist).cast(),
                        data.as_mut_ptr().cast(),
                        data.len(),
                        0,
                    )
                } == 0;
                if success
                    && u32::from_ne_bytes(data[..4].try_into().map_err(|_| E::Internal)?) >= 20
                    && data[4..].iter().any(|b| *b != 0)
                {
                    format!(
                        "uuid:{}",
                        data[4..]
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                    )
                } else {
                    fallback
                }
            };
            #[cfg(not(target_os = "macos"))]
            let volume = fallback;
            Ok(Self {
                platform: "unix".into(),
                volume,
                file: metadata.ino().to_string(),
                created,
            })
        }
        #[cfg(windows)]
        {
            use std::{
                mem::MaybeUninit,
                os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
            };
            use windows_sys::Win32::Storage::FileSystem::{
                GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
            };
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(root)
                .map_err(|_| E::Inaccessible)?;
            let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
                return Err(E::Inaccessible);
            }
            let info = unsafe { info.assume_init() };
            Ok(Self {
                platform: "windows".into(),
                volume: info.dwVolumeSerialNumber.to_string(),
                file: ((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
                    .to_string(),
                created,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(E::SourceUnavailable)
        }
    }
    pub(crate) fn json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
    pub(crate) fn equivalent(a: &str, b: &str) -> Result<bool> {
        let a = serde_json::from_str::<Self>(a)?;
        let b = serde_json::from_str::<Self>(b)?;
        a.validate()?;
        b.validate()?;
        Ok(a == b)
    }
    pub(crate) fn same_volume(&self, other: &Self) -> bool {
        self.platform == other.platform && self.volume == other.volume
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if !matches!(self.platform.as_str(), "unix" | "windows")
            || self.volume.is_empty()
            || self.volume.len() > 128
            || !self
                .volume
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b':')
            || self.file.parse::<u64>().is_err()
            || self.file.len() > 20
            || self.created.as_ref().is_some_and(|v| {
                v.len() > 40 || v.strip_prefix('-').unwrap_or(v).parse::<u128>().is_err()
            })
        {
            return Err(E::InvalidArgument);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPreviewScope {
    pub root: String,
    pub recursive: bool,
    pub files: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPreviewIssue {
    pub path: String,
    pub code: E,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaImportPreview {
    pub id: String,
    pub request_id: String,
    pub mode: ImportMode,
    pub scopes: Vec<ImportPreviewScope>,
    pub images: u32,
    pub audio: u32,
    pub unsupported: u32,
    pub inaccessible: u32,
    pub complete: bool,
    pub issues: Vec<ImportPreviewIssue>,
    pub reference_in_place: bool,
    pub expires_at: u64,
}
#[derive(Clone)]
struct Group {
    root: PathBuf,
    existing: Option<Source>,
    recursive: bool,
    files: Vec<String>,
    folders: Vec<String>,
    stamp: Option<RootStamp>,
    folder_stamps: BTreeMap<String, RootStamp>,
}
#[derive(Clone)]
struct Plan {
    groups: Vec<Group>,
    fingerprint: String,
    mode: ImportMode,
    raw: Vec<String>,
    eligible: bool,
}
struct Reserved {
    expires: Instant,
    busy: bool,
    plan: Plan,
}
#[derive(Clone, Copy)]
pub enum NativeSelectionKind {
    Files,
    Folders,
    Drop,
}
#[derive(Clone, Copy)]
struct NativeGrant {
    expires: Instant,
    folder: bool,
}
#[derive(Default)]
pub struct MediaImportPreviews {
    native: Mutex<HashMap<String, NativeGrant>>,
    plans: Mutex<HashMap<String, Reserved>>,
    requests: Mutex<HashMap<String, Arc<AtomicBool>>>,
    active: AtomicUsize,
}
fn lock<T>(v: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    v.lock().map_err(|_| E::Internal)
}
fn checkpoint(cancel: &AtomicBool, end: Instant) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(E::Cancelled);
    }
    if Instant::now() >= end {
        return Err(E::TimedOut);
    }
    Ok(())
}
fn bounded_headers(c: &Catalog) -> Result<Vec<Source>> {
    let (count,bytes):(i64,i64)=c.db_connection().query_row("SELECT count(*),COALESCE(sum(length(CAST(root AS BLOB))+length(CAST(name AS BLOB))+length(CAST(id AS BLOB))+128),0) FROM sources",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if count > MAX_SOURCES as i64 || bytes > MAX_CONTEXT_BYTES as i64 {
        return Err(E::LimitExceeded);
    }
    c.source_headers().map_err(|_| E::Internal)
}
fn source_fingerprint(headers: &[Source]) -> Result<String> {
    let mut rows = headers
        .iter()
        .map(|s| (&s.id, &s.root, s.generation, &s.scope))
        .collect::<Vec<_>>();
    rows.sort();
    let bytes = serde_json::to_vec(&rows)?;
    if headers.len() > MAX_SOURCES || bytes.len() > MAX_CONTEXT_BYTES {
        return Err(E::LimitExceeded);
    }
    Ok(blake3::hash(&bytes).to_hex().to_string())
}
/// Header admission is a dispatch hint; full native decode still decides readiness.
pub(crate) fn candidate(path: &Path, mode: ImportMode) -> Result<ImportMedia> {
    let metadata = fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            E::FileMissing
        } else {
            E::Inaccessible
        }
    })?;
    if !metadata.is_file() {
        return Err(E::InvalidArgument);
    }
    let file = fs::File::open(path).map_err(|_| E::Inaccessible)?;
    let mut head = Vec::with_capacity(64);
    file.take(64)
        .read_to_end(&mut head)
        .map_err(|_| E::Inaccessible)?;
    let p = &head[..];
    let image = p.starts_with(b"\x89PNG\r\n\x1a\n")
        || p.starts_with(b"\xff\xd8")
        || (p.starts_with(b"RIFF") && p.get(8..12) == Some(b"WEBP"));
    if image {
        return Ok(ImportMedia::Image);
    }
    if mode == ImportMode::Mixed && crate::library::supported(path) {
        return Ok(ImportMedia::Audio);
    }
    Ok(ImportMedia::Unsupported)
}
fn inspect(
    preview: &mut MediaImportPreview,
    path: &Path,
    mode: ImportMode,
    cancel: &AtomicBool,
    end: Instant,
) -> Result<()> {
    checkpoint(cancel, end)?;
    if u64::from(preview.images)
        + u64::from(preview.audio)
        + u64::from(preview.unsupported)
        + u64::from(preview.inaccessible)
        >= MAX_FILES as u64
    {
        return Err(E::LimitExceeded);
    }
    match candidate(path, mode) {
        Ok(ImportMedia::Image) => preview.images += 1,
        Ok(ImportMedia::Audio) => preview.audio += 1,
        Ok(ImportMedia::Unsupported) => preview.unsupported += 1,
        Err(code) => {
            preview.inaccessible += 1;
            preview.complete = false;
            if preview.issues.len() < 20 {
                preview.issues.push(ImportPreviewIssue {
                    path: path.to_string_lossy().into_owned(),
                    code,
                });
            }
        }
    }
    Ok(())
}
impl MediaImportPreviews {
    /// Native adapters only: call with paths from an OS picker/drop event, never
    /// from an unvalidated renderer/MCP request. The exposed prepare call checks
    /// exact native strings and expiry before any filesystem access.
    pub fn approve_native(
        &self,
        paths: &[PathBuf],
        kind: NativeSelectionKind,
    ) -> Result<Vec<String>> {
        if paths.is_empty() || paths.len() > MAX_PATHS {
            return Err(E::InvalidArgument);
        }
        let raw = paths
            .iter()
            .map(|p| {
                if !p.is_absolute() {
                    return Err(E::InvalidArgument);
                }
                path_text(p).map_err(|_| E::InvalidArgument)
            })
            .collect::<Result<Vec<_>>>()?;
        if raw.iter().any(|p| p.len() > 32768)
            || serde_json::to_vec(&raw)?.len() > MAX_SELECTION_BYTES
        {
            return Err(E::LimitExceeded);
        }
        let mut grants = vec![];
        for p in &raw {
            let folder = match kind {
                NativeSelectionKind::Files => false,
                NativeSelectionKind::Folders => true,
                NativeSelectionKind::Drop => {
                    fs::symlink_metadata(p).map(|m| m.is_dir()).unwrap_or(false)
                }
            };
            grants.push((
                p.clone(),
                NativeGrant {
                    expires: Instant::now() + PREVIEW_TTL,
                    folder,
                },
            ));
        }
        let mut approved = lock(&self.native)?;
        approved.clear();
        approved.extend(grants);
        Ok(raw)
    }
    pub fn invalidate(&self) -> Result<()> {
        for flag in lock(&self.requests)?.values() {
            flag.store(true, Ordering::Relaxed);
        }
        lock(&self.plans)?.clear();
        lock(&self.native)?.clear();
        Ok(())
    }
    pub fn cancel_prepare(&self, id: &str) -> Result<()> {
        if let Some(flag) = lock(&self.requests)?.get(id) {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
    pub fn discard(&self, id: &str) -> Result<()> {
        let mut plans = lock(&self.plans)?;
        if plans.get(id).is_some_and(|r| r.busy) {
            return Err(E::Busy);
        }
        plans.remove(id);
        Ok(())
    }
    pub fn prepare(
        &self,
        db: &Arc<Mutex<Catalog>>,
        request_id: &str,
        paths: Vec<String>,
        mode: ImportMode,
    ) -> Result<MediaImportPreview> {
        if Uuid::parse_str(request_id).is_err() || paths.is_empty() || paths.len() > MAX_PATHS {
            return Err(E::InvalidArgument);
        }
        if serde_json::to_vec(&paths)?.len() > MAX_SELECTION_BYTES {
            return Err(E::LimitExceeded);
        }
        let folders = {
            let now = Instant::now();
            let approved = lock(&self.native)?;
            let mut folders = vec![];
            for p in &paths {
                let grant = approved
                    .get(p)
                    .filter(|g| g.expires > now)
                    .ok_or(E::PermissionDenied)?;
                folders.push(grant.folder);
            }
            folders
        };
        if self
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |n| {
                (n < 4).then_some(n + 1)
            })
            .is_err()
        {
            return Err(E::Busy);
        }
        struct Slot<'a>(&'a AtomicUsize);
        impl Drop for Slot<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::Release);
            }
        }
        let _slot = Slot(&self.active);
        let flag = Arc::new(AtomicBool::new(false));
        {
            let mut pending = lock(&self.requests)?;
            if pending.contains_key(request_id) {
                return Err(E::Busy);
            }
            pending.insert(request_id.into(), flag.clone());
        }
        let result = self.prepare_inner(db, request_id, &paths, &folders, mode, &flag);
        lock(&self.requests)?.remove(request_id);
        result
    }
    fn prepare_inner(
        &self,
        db: &Arc<Mutex<Catalog>>,
        request_id: &str,
        raw: &[String],
        native_folders: &[bool],
        mode: ImportMode,
        cancel: &AtomicBool,
    ) -> Result<MediaImportPreview> {
        let end = Instant::now() + DISCOVERY_DEADLINE;
        let headers = bounded_headers(&*lock(db)?)?;
        let fingerprint = source_fingerprint(&headers)?;
        // Observe existing roots off-lock to detect alias/case collisions. No
        // root is silently moved or reconciled by selecting a similar path.
        let mut observed = HashMap::new();
        for s in &headers {
            checkpoint(cancel, end)?;
            observed.insert(s.id.clone(), Path::new(&s.root).canonicalize().ok());
        }
        let mut folders = vec![];
        let mut files = vec![];
        let mut issues = vec![];
        let mut input_errors = 0u32;
        for (path, folder) in raw.iter().zip(native_folders) {
            checkpoint(cancel, end)?;
            let path = PathBuf::from(path);
            let result = (|| -> Result<()> {
                if path.is_dir() != *folder {
                    return Err(E::SourceChanged);
                }
                if *folder {
                    folders.push(path.canonicalize().map_err(|_| E::SourceUnavailable)?);
                } else {
                    let parent = path
                        .parent()
                        .ok_or(E::InvalidArgument)?
                        .canonicalize()
                        .map_err(|_| E::SourceUnavailable)?;
                    let name = path
                        .file_name()
                        .and_then(|p| p.to_str())
                        .ok_or(E::InvalidArgument)?;
                    let actual = contained(&parent, name).map_err(|_| E::PermissionDenied)?;
                    files.push(actual);
                }
                Ok(())
            })();
            if let Err(code) = result {
                input_errors += 1;
                if issues.len() < 20 {
                    issues.push(ImportPreviewIssue {
                        path: path.to_string_lossy().into_owned(),
                        code,
                    });
                }
            }
        }
        folders.sort_by_key(|p| p.components().count());
        folders.dedup();
        let mut roots: Vec<PathBuf> = vec![];
        for folder in folders {
            if !roots.iter().any(|r| folder.starts_with(r)) {
                roots.push(folder);
            }
        }
        files.sort();
        files.dedup();
        files.retain(|p| !roots.iter().any(|r| p.starts_with(r)));
        let mut groups: BTreeMap<PathBuf, Group> = BTreeMap::new();
        {
            let c = lock(db)?;
            if source_fingerprint(&bounded_headers(&c)?)? != fingerprint {
                return Err(E::SourceChanged);
            }
            for selected_root in roots {
                let mut ancestors = vec![];
                for source in &headers {
                    let other = observed
                        .get(&source.id)
                        .and_then(Option::as_deref)
                        .unwrap_or(Path::new(&source.root));
                    if selected_root.starts_with(other) && other != Path::new(&source.root) {
                        return Err(E::SourceChanged);
                    }
                    if source.scope == "folder" && selected_root.starts_with(other) {
                        ancestors.push(source);
                    }
                }
                if ancestors.len() > 1 {
                    return Err(E::SourceOverlap);
                }
                let existing = ancestors.first().copied();
                for source in &headers {
                    if existing.is_some_and(|s| s.id == source.id) {
                        continue;
                    }
                    let other = observed
                        .get(&source.id)
                        .and_then(Option::as_deref)
                        .unwrap_or(Path::new(&source.root));
                    if selected_root.starts_with(other) || other.starts_with(&selected_root) {
                        return Err(E::SourceOverlap);
                    }
                }
                let base = existing
                    .map(|s| PathBuf::from(&s.root))
                    .unwrap_or_else(|| selected_root.clone());
                let prefix = path_text(
                    selected_root
                        .strip_prefix(&base)
                        .map_err(|_| E::PermissionDenied)?,
                )
                .map_err(|_| E::InvalidArgument)?
                .replace(std::path::MAIN_SEPARATOR, "/");
                if !prefix.is_empty() {
                    validate_relative(&prefix).map_err(|_| E::InvalidArgument)?;
                }
                let group = groups.entry(base.clone()).or_insert_with(|| Group {
                    root: base,
                    existing: existing.cloned(),
                    recursive: true,
                    files: vec![],
                    folders: vec![],
                    stamp: None,
                    folder_stamps: BTreeMap::new(),
                });
                group.recursive = true;
                group.folders.push(prefix);
            }
            for file in files {
                let candidates = headers
                    .iter()
                    .filter_map(|s| {
                        let root = observed
                            .get(&s.id)
                            .and_then(Option::as_deref)
                            .unwrap_or(Path::new(&s.root));
                        file.strip_prefix(root)
                            .ok()
                            .map(|relative| (s, root, relative))
                    })
                    .collect::<Vec<_>>();
                if candidates
                    .iter()
                    .any(|(s, root, _)| *root != Path::new(&s.root))
                {
                    return Err(E::SourceChanged);
                }
                let members = candidates
                    .iter()
                    .filter(|(s, _, p)| {
                        path_text(p).is_ok_and(|p| {
                            c.validate_source_entry(
                                &s.id,
                                &p.replace(std::path::MAIN_SEPARATOR, "/"),
                            )
                            .is_ok()
                        })
                    })
                    .collect::<Vec<_>>();
                if members.len() > 1 {
                    return Err(E::SourceOverlap);
                }
                let selected = members.first().map(|(s, _, _)| *s).or_else(|| {
                    candidates
                        .iter()
                        .max_by_key(|(_, r, _)| r.components().count())
                        .map(|(s, _, _)| *s)
                });
                let root = selected
                    .map(|s| PathBuf::from(&s.root))
                    .unwrap_or_else(|| file.parent().unwrap_or(Path::new("")).to_path_buf());
                let relative =
                    path_text(file.strip_prefix(&root).map_err(|_| E::PermissionDenied)?)
                        .map_err(|_| E::InvalidArgument)?
                        .replace(std::path::MAIN_SEPARATOR, "/");
                validate_relative(&relative).map_err(|_| E::InvalidArgument)?;
                let group = groups.entry(root.clone()).or_insert_with(|| Group {
                    root,
                    existing: selected.cloned(),
                    recursive: false,
                    files: vec![],
                    folders: vec![],
                    stamp: None,
                    folder_stamps: BTreeMap::new(),
                });
                group.files.push(relative);
            }
        }
        if groups.len() > image_jobs::MAX_ACTIVE {
            return Err(E::LimitExceeded);
        }
        let mut preview = MediaImportPreview {
            id: Uuid::new_v4().to_string(),
            request_id: request_id.into(),
            mode,
            scopes: vec![],
            images: 0,
            audio: 0,
            unsupported: 0,
            inaccessible: input_errors,
            complete: input_errors == 0,
            issues,
            reference_in_place: true,
            expires_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| E::Internal)?
                .as_secs()
                + PREVIEW_TTL.as_secs(),
        };
        for group in groups.values_mut() {
            checkpoint(cancel, end)?;
            // A new image source never adopts an existing folder catalog: Add
            // folder restores it (with its identities) and later imports reuse it.
            if group.existing.is_none()
                && group.recursive
                && group.root.join(".creativeshelf/catalog.json").exists()
            {
                return Err(E::ExistingCatalog);
            }
            group.files.sort();
            group.files.dedup();
            group.folders.sort();
            group.folders.dedup();
            group.stamp = Some(RootStamp::read(&group.root)?);
            for prefix in &group.folders {
                let selected_root = group.root.join(prefix);
                let stamp = RootStamp::read(&selected_root)?;
                group.folder_stamps.insert(prefix.clone(), stamp.clone());
                preview.scopes.push(ImportPreviewScope {
                    root: path_text(&selected_root).map_err(|_| E::InvalidArgument)?,
                    recursive: true,
                    files: vec![],
                });
                let mut walk = WalkDir::new(&selected_root)
                    .follow_links(false)
                    .max_open(8)
                    .max_depth(MAX_DEPTH + 1)
                    .into_iter();
                while let Some(entry) = walk.next() {
                    checkpoint(cancel, end)?;
                    match entry {
                        Ok(e)
                            if e.depth() > 0
                                && e.file_type().is_dir()
                                && !stamp.same_volume(&RootStamp::read(e.path())?) =>
                        {
                            walk.skip_current_dir();
                            preview.complete = false;
                            preview.inaccessible += 1;
                        }
                        Ok(e) if e.depth() > MAX_DEPTH => return Err(E::LimitExceeded),
                        Ok(e) if e.file_type().is_file() => {
                            inspect(&mut preview, e.path(), mode, cancel, end)?
                        }
                        Ok(e) if e.file_type().is_symlink() => preview.unsupported += 1,
                        Ok(_) => {}
                        Err(_) => {
                            preview.complete = false;
                            preview.inaccessible += 1;
                        }
                    }
                }
                if stamp != RootStamp::read(&selected_root)? {
                    return Err(E::SourceChanged);
                }
            }
            if !group.files.is_empty() {
                preview.scopes.push(ImportPreviewScope {
                    root: path_text(&group.root).map_err(|_| E::InvalidArgument)?,
                    recursive: false,
                    files: group.files.clone(),
                });
                for p in &group.files {
                    let path = contained(&group.root, p).map_err(|_| E::PermissionDenied)?;
                    inspect(&mut preview, &path, mode, cancel, end)?;
                }
            }
            if group.stamp.as_ref() != Some(&RootStamp::read(&group.root)?) {
                return Err(E::SourceChanged);
            }
        }
        if preview.scopes.is_empty() {
            return Err(preview
                .issues
                .first()
                .map(|i| i.code)
                .unwrap_or(E::SourceUnavailable));
        }
        if serde_json::to_vec(&preview)?.len() > 2 * 1024 * 1024 {
            return Err(E::LimitExceeded);
        }
        checkpoint(cancel, end)?;
        let mut plans = lock(&self.plans)?;
        plans.retain(|_, r| r.expires > Instant::now() || r.busy);
        if plans.len() >= MAX_PREVIEWS {
            return Err(E::Busy);
        }
        plans.insert(
            preview.id.clone(),
            Reserved {
                expires: Instant::now() + PREVIEW_TTL,
                busy: false,
                plan: Plan {
                    groups: groups.into_values().collect(),
                    fingerprint,
                    mode,
                    raw: raw.to_vec(),
                    eligible: preview.images + preview.audio > 0,
                },
            },
        );
        Ok(preview)
    }
    pub fn confirm(&self, db: &Arc<Mutex<Catalog>>, id: &str) -> Result<Vec<ImageImportJob>> {
        let plan = {
            let mut plans = lock(&self.plans)?;
            let r = plans.get_mut(id).ok_or(E::NotFound)?;
            if r.expires <= Instant::now() {
                return Err(E::PermissionDenied);
            }
            if !r.plan.eligible {
                return Err(E::NoEligibleFiles);
            }
            if r.busy {
                return Err(E::Busy);
            }
            r.busy = true;
            r.plan.clone()
        };
        let result = (|| -> Result<Vec<ImageImportJob>> {
            let c = lock(db)?;
            let tx = Transaction::new_unchecked(c.db_connection(), TransactionBehavior::Immediate)?;
            if source_fingerprint(&bounded_headers(&c)?)? != plan.fingerprint {
                return Err(E::SourceChanged);
            }
            let mut jobs = vec![];
            for group in &plan.groups {
                let (source_id, generation) = if let Some(s) = &group.existing {
                    tx.execute("UPDATE sources SET available=1 WHERE id=?1", [&s.id])?;
                    (s.id.clone(), s.generation)
                } else {
                    let id = Uuid::new_v4().to_string();
                    let root = path_text(&group.root).map_err(|_| E::InvalidArgument)?;
                    let name = group
                        .root
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.clone());
                    tx.execute(
                        "INSERT INTO sources(id,name,root,scope) VALUES(?1,?2,?3,?4)",
                        params![
                            id,
                            name,
                            root,
                            if group.recursive { "folder" } else { "files" }
                        ],
                    )?;
                    (id, 0)
                };
                for file in &group.files {
                    tx.execute("INSERT INTO source_files(source_id,relative_path) VALUES(?1,?2) ON CONFLICT DO NOTHING",params![source_id,file])?;
                }
                let job = enqueue_selection_on(
                    &tx,
                    &source_id,
                    generation,
                    plan.mode,
                    (!group.files.is_empty()).then_some(group.files.clone()),
                    group.folders.clone(),
                )?;
                let stamp = group.stamp.as_ref().ok_or(E::Internal)?.json()?;
                if let Some(previous) = job.root_stamp.as_deref() {
                    if !RootStamp::equivalent(previous, &stamp)? {
                        return Err(E::SourceChanged);
                    }
                }
                let known:Option<(i64,String)>=tx.query_row("SELECT source_generation,root_stamp FROM image_source_identity WHERE source_id=?1",[&source_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                if let Some((g, old)) = known {
                    if g == generation && !RootStamp::equivalent(&old, &stamp)? {
                        return Err(E::SourceChanged);
                    }
                }
                tx.execute("INSERT INTO image_source_identity(source_id,source_generation,root_stamp) VALUES(?1,?2,?3) ON CONFLICT(source_id) DO UPDATE SET source_generation=excluded.source_generation,root_stamp=excluded.root_stamp",params![source_id,generation,stamp])?;
                if !job.folder_stamps.is_empty() && job.folder_stamps != group.folder_stamps {
                    return Err(E::SourceChanged);
                }
                let folder_stamps = serde_json::to_string(&group.folder_stamps)?;
                if folder_stamps.len() > 131072 {
                    return Err(E::LimitExceeded);
                }
                image_jobs::prune_budget(
                    &tx,
                    0,
                    folder_stamps
                        .len()
                        .saturating_sub(serde_json::to_vec(&job.folder_stamps)?.len()),
                )?;
                tx.execute(
                    "UPDATE image_import_jobs SET root_stamp=?2,folder_stamps=?3 WHERE id=?1",
                    params![job.id, stamp, folder_stamps],
                )?;
                jobs.push(image_jobs::load_job(&tx, &job.id)?);
            }
            tx.commit()?;
            Ok(jobs)
        })();
        let mut plans = lock(&self.plans)?;
        if result.is_ok() {
            plans.remove(id);
            let mut approvals = lock(&self.native)?;
            for p in plan.raw {
                approvals.remove(&p);
            }
        } else if let Some(r) = plans.get_mut(id) {
            r.busy = false;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    fn fixture() -> (
        tempfile::TempDir,
        Arc<Mutex<Catalog>>,
        PathBuf,
        MediaImportPreviews,
    ) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("media");
        fs::create_dir(&root).unwrap();
        let c = Arc::new(Mutex::new(
            Catalog::open(&d.path().join("db.sqlite")).unwrap(),
        ));
        (d, c, root, MediaImportPreviews::default())
    }
    fn picture(path: &Path) {
        let mut b = vec![];
        image::codecs::png::PngEncoder::new(&mut b)
            .write_image(&[32, 64, 128, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        fs::write(path, b).unwrap();
    }
    fn prepare(
        p: &MediaImportPreviews,
        db: &Arc<Mutex<Catalog>>,
        paths: &[PathBuf],
        kind: NativeSelectionKind,
        mode: ImportMode,
    ) -> MediaImportPreview {
        let raw = p.approve_native(paths, kind).unwrap();
        p.prepare(db, &Uuid::new_v4().to_string(), raw, mode)
            .unwrap()
    }
    #[test]
    fn preview_needs_native_approval_and_grants_only_selected_files_after_confirm() {
        let (_d, db, root, p) = fixture();
        let file = root.join("图像 one.png");
        picture(&file);
        picture(&root.join("sibling.png"));
        let original = fs::read(&file).unwrap();
        assert!(matches!(
            p.prepare(
                &db,
                &Uuid::new_v4().to_string(),
                vec![path_text(&file).unwrap()],
                ImportMode::Images
            ),
            Err(E::PermissionDenied)
        ));
        let preview = prepare(
            &p,
            &db,
            &[file.clone()],
            NativeSelectionKind::Files,
            ImportMode::Images,
        );
        assert_eq!(preview.images, 1);
        assert!(!preview.scopes[0].recursive);
        assert!(lock(&db).unwrap().sources().unwrap().is_empty());
        let jobs = p.confirm(&db, &preview.id).unwrap();
        let c = lock(&db).unwrap();
        let source = c.source(&jobs[0].source_id).unwrap();
        assert_eq!(source.scope, "files");
        assert_eq!(source.files.len(), 1);
        assert_eq!(source.files[0].relative_path, "图像 one.png");
        assert!(jobs[0].folders.is_empty());
        assert_eq!(jobs[0].paths, Some(vec!["图像 one.png".into()]));
        assert!(matches!(p.confirm(&db, &preview.id), Err(E::NotFound)));
        assert_eq!(fs::read(file).unwrap(), original);
        assert!(!root.join(".creativeshelf").exists());
    }
    #[test]
    fn recursive_preview_collapses_duplicate_paths_and_discloses_mixed_counts() {
        let (_d, db, root, p) = fixture();
        let child = root.join("child");
        fs::create_dir(&child).unwrap();
        picture(&root.join("a.png"));
        picture(&child.join("b.png"));
        fs::write(
            root.join("voice.wav"),
            b"RIFF0000WAVE synthetic preview header",
        )
        .unwrap();
        fs::write(root.join("notes.txt"), b"not media").unwrap();
        let preview = prepare(
            &p,
            &db,
            &[root.clone(), child, root.join("a.png")],
            NativeSelectionKind::Drop,
            ImportMode::Mixed,
        );
        assert_eq!(
            (preview.images, preview.audio, preview.unsupported),
            (2, 1, 1)
        );
        assert_eq!(preview.scopes.len(), 1);
        assert!(preview.scopes[0].recursive);
        assert!(preview.reference_in_place);
        let jobs = p.confirm(&db, &preview.id).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].folders, vec![String::new()]);
        assert_eq!(
            lock(&db).unwrap().source(&jobs[0].source_id).unwrap().scope,
            "folder"
        );
    }
    #[test]
    fn nested_folder_reuses_recursive_source_and_cannot_admit_siblings() {
        let (_d, db, root, p) = fixture();
        let source = lock(&db).unwrap().add_source(&root).unwrap();
        let child = root.join("chosen");
        fs::create_dir(&child).unwrap();
        picture(&child.join("a.png"));
        picture(&root.join("sibling.png"));
        let preview = prepare(
            &p,
            &db,
            &[child.clone()],
            NativeSelectionKind::Folders,
            ImportMode::Images,
        );
        assert_eq!(preview.images, 1);
        assert_eq!(
            preview.scopes[0].root,
            path_text(&child.canonicalize().unwrap()).unwrap()
        );
        let jobs = p.confirm(&db, &preview.id).unwrap();
        assert_eq!(jobs[0].source_id, source.id);
        assert_eq!(jobs[0].folders, vec!["chosen"]);
        let c = lock(&db).unwrap();
        c.claim_next_image_import("worker", crate::jobs::now_secs())
            .unwrap()
            .unwrap();
        assert_eq!(
            c.image_import_discovered(
                &jobs[0].id,
                "worker",
                &[image_jobs::DiscoveredImport {
                    relative_path: "sibling.png".into(),
                    media_kind: ImportMedia::Image
                }]
            ),
            Err(E::PermissionDenied)
        );
        c.image_import_discovered(
            &jobs[0].id,
            "worker",
            &[image_jobs::DiscoveredImport {
                relative_path: "chosen/a.png".into(),
                media_kind: ImportMedia::Image,
            }],
        )
        .unwrap();
        assert_eq!(c.image_import_job(&jobs[0].id).unwrap().total, 1);
    }
    #[test]
    fn file_only_source_cannot_become_recursive_through_preview() {
        let (_d, db, root, p) = fixture();
        picture(&root.join("one.png"));
        let source = lock(&db).unwrap().add_source(&root).unwrap();
        lock(&db)
            .unwrap()
            .db
            .execute("UPDATE sources SET scope='files' WHERE id=?1", [&source.id])
            .unwrap();
        lock(&db)
            .unwrap()
            .db
            .execute(
                "INSERT INTO source_files(source_id,relative_path) VALUES(?1,'one.png')",
                [&source.id],
            )
            .unwrap();
        let raw = p
            .approve_native(&[root.clone()], NativeSelectionKind::Folders)
            .unwrap();
        assert!(matches!(
            p.prepare(&db, &Uuid::new_v4().to_string(), raw, ImportMode::Images),
            Err(E::SourceOverlap)
        ));
        assert_eq!(
            lock(&db).unwrap().source(&source.id).unwrap().scope,
            "files"
        );
    }
    #[test]
    fn file_picker_grant_cannot_turn_into_a_folder_grant_after_replacement() {
        let (_d, db, root, p) = fixture();
        let path = root.join("one.png");
        picture(&path);
        let raw = p
            .approve_native(&[path.clone()], NativeSelectionKind::Files)
            .unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        picture(&path.join("private.png"));
        assert!(matches!(
            p.prepare(&db, &Uuid::new_v4().to_string(), raw, ImportMode::Images),
            Err(E::SourceChanged)
        ));
        assert!(lock(&db).unwrap().sources().unwrap().is_empty());
    }
    #[test]
    fn unsupported_empty_and_stale_previews_never_create_sources() {
        let (d, db, root, p) = fixture();
        let text = root.join("notes.txt");
        fs::write(&text, b"text").unwrap();
        let empty = prepare(
            &p,
            &db,
            &[text],
            NativeSelectionKind::Files,
            ImportMode::Images,
        );
        assert!(matches!(p.confirm(&db, &empty.id), Err(E::NoEligibleFiles)));
        let image = root.join("one.png");
        picture(&image);
        let preview = prepare(
            &p,
            &db,
            &[image],
            NativeSelectionKind::Files,
            ImportMode::Images,
        );
        let other = d.path().join("other");
        fs::create_dir(&other).unwrap();
        lock(&db).unwrap().add_source(&other).unwrap();
        assert!(matches!(p.confirm(&db, &preview.id), Err(E::SourceChanged)));
        assert_eq!(lock(&db).unwrap().sources().unwrap().len(), 1);
        assert!(lock(&db)
            .unwrap()
            .image_import_jobs(None, 50)
            .unwrap()
            .is_empty());
        p.discard(&preview.id).unwrap();
        assert!(matches!(p.confirm(&db, &preview.id), Err(E::NotFound)));
    }
    #[cfg(unix)]
    #[test]
    fn selected_symlink_escape_cannot_grant_target_or_siblings() {
        use std::os::unix::fs::symlink;
        let (d, db, root, p) = fixture();
        let outside = d.path().join("outside");
        fs::create_dir(&outside).unwrap();
        picture(&outside.join("secret.png"));
        let link = root.join("link.png");
        symlink(outside.join("secret.png"), &link).unwrap();
        let raw = p
            .approve_native(&[link], NativeSelectionKind::Files)
            .unwrap();
        assert!(matches!(
            p.prepare(&db, &Uuid::new_v4().to_string(), raw, ImportMode::Images),
            Err(E::PermissionDenied)
        ));
        assert!(lock(&db).unwrap().sources().unwrap().is_empty());
    }
    #[test]
    fn expired_grants_and_plans_cannot_create_memberships() {
        let (_d, db, root, p) = fixture();
        let file = root.join("one.png");
        picture(&file);
        let raw = p
            .approve_native(&[file.clone()], NativeSelectionKind::Files)
            .unwrap();
        for grant in lock(&p.native).unwrap().values_mut() {
            grant.expires = Instant::now() - Duration::from_secs(1);
        }
        assert!(matches!(
            p.prepare(&db, &Uuid::new_v4().to_string(), raw, ImportMode::Images),
            Err(E::PermissionDenied)
        ));
        let preview = prepare(
            &p,
            &db,
            &[file],
            NativeSelectionKind::Files,
            ImportMode::Images,
        );
        lock(&p.plans)
            .unwrap()
            .get_mut(&preview.id)
            .unwrap()
            .expires = Instant::now() - Duration::from_secs(1);
        assert!(matches!(
            p.confirm(&db, &preview.id),
            Err(E::PermissionDenied)
        ));
        assert!(lock(&db).unwrap().sources().unwrap().is_empty());
    }
    #[test]
    fn unknown_sidecars_and_evicted_cancelled_history_cannot_enable_audio_only_overwrite() {
        let (_d, db, root, p) = fixture();
        picture(&root.join("one.png"));
        let sidecar = root.join(".creativeshelf");
        fs::create_dir(&sidecar).unwrap();
        let catalog = sidecar.join("catalog.json");
        fs::write(&catalog, b"existing creator metadata").unwrap();
        let raw = p
            .approve_native(&[root.clone()], NativeSelectionKind::Folders)
            .unwrap();
        assert!(matches!(
            p.prepare(&db, &Uuid::new_v4().to_string(), raw, ImportMode::Mixed),
            Err(E::ExistingCatalog)
        ));
        assert_eq!(fs::read(&catalog).unwrap(), b"existing creator metadata");
        assert!(lock(&db).unwrap().sources().unwrap().is_empty());
        fs::remove_file(&catalog).unwrap();
        let preview = prepare(
            &p,
            &db,
            &[root],
            NativeSelectionKind::Folders,
            ImportMode::Images,
        );
        let jobs = p.confirm(&db, &preview.id).unwrap();
        let c = lock(&db).unwrap();
        c.cancel_image_import(&jobs[0].id, 0).unwrap();
        c.db_connection()
            .execute("DELETE FROM image_import_jobs WHERE id=?1", [&jobs[0].id])
            .unwrap();
        assert!(c.require_audio_only_source(&jobs[0].source_id).is_err());
        assert!(c.export_portable().is_ok());
    }
}
