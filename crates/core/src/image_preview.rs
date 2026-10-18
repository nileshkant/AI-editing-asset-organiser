//! SS-044 versioned image previews. See docs/IMAGE_PREVIEWS.md.
//!
//! Callers name an asset ID and content version, never a path. Authorization and
//! the current version are checked before reading the original or a cache file and
//! again immediately before serving. The disk cache is disposable derived data keyed
//! by content digest, preprocessing/analyzer version, tier and color policy. It never
//! touches originals, analysis rows or annotations.
use crate::{
    catalog::Catalog,
    image_analysis::{self, AnalysisError, ImageSelection, ANALYZER, WORKER_HEAP},
    image_contracts::{
        AssetKind, Availability, ColorPolicy, TechnicalState, MAX_PREVIEW_BYTES,
        MAX_PREVIEW_DIMENSION,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant, SystemTime},
};

pub const PREVIEW_VERSION: &str = "creativeshelf-preview-v1";
pub const TILE_EDGE: u32 = 512;
/// SS-040 wire ceilings: 2,048 px per edge and 4 MiB per encoded preview.
pub const INSPECT_EDGE: u32 = MAX_PREVIEW_DIMENSION;
const INSPECT_STEPS: [u32; 5] = [INSPECT_EDGE, 1536, 1024, 768, 512];
/// Concurrent preview workers; each child has the SS-042 512 MiB heap budget.
pub const PREVIEW_SLOTS: usize = 2;
/// Registered requests (waiting, decoding or reading cache) per app process.
pub const MAX_OUTSTANDING: usize = 256;
/// Inspect previews hold up to ~16 MiB each in memory; bound them separately.
pub const MAX_INSPECT_OUTSTANDING: usize = 8;
pub const PREVIEW_DEADLINE: Duration = Duration::from_secs(20);
pub const SLOT_WAIT: Duration = Duration::from_secs(60);
pub const DEFAULT_QUOTA: u64 = 512 << 20;
pub const MIN_QUOTA: u64 = 64 << 20;
pub const MAX_QUOTA: u64 = 16 << 30;
const MAX_HEADER: usize = 16 * 1024;
const RECORD_MAGIC: &[u8; 8] = b"CSPREV1\n";
pub(crate) const WORKER_MAGIC: &[u8; 4] = b"CSPW";
pub const ENVELOPE_MAGIC: &[u8; 4] = b"CSPV";
const QUOTA_KEY: &str = "image_preview_cache";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewTier {
    Small,
    Medium,
    Large,
    Inspect,
    Tile,
}
impl PreviewTier {
    /// Maximum long edge in canonical oriented pixels; tiles are 1:1 squares.
    pub fn edge(self) -> u32 {
        match self {
            Self::Small => 128,
            Self::Medium => 256,
            Self::Large => 512,
            Self::Inspect => INSPECT_EDGE,
            Self::Tile => TILE_EDGE,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::Inspect => "inspect",
            Self::Tile => "tile",
        }
    }
    pub(crate) fn quality(self) -> u8 {
        match self {
            Self::Small | Self::Medium | Self::Large => 85,
            Self::Inspect | Self::Tile => 92,
        }
    }
}
/// Fit inside `edge` preserving aspect ratio with nearest rounding; never upscale.
pub fn fit(w: u32, h: u32, edge: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= edge {
        return (w, h);
    }
    let scale = |v: u32| {
        ((u64::from(v) * u64::from(edge) + u64::from(long) / 2) / u64::from(long)).max(1) as u32
    };
    (scale(w), scale(h))
}
/// Size of a 1:1 tile at grid position, or None outside the oriented image.
pub fn tile_size(w: u32, h: u32, col: u32, row: u32) -> Option<(u32, u32)> {
    let x = col.checked_mul(TILE_EDGE)?;
    let y = row.checked_mul(TILE_EDGE)?;
    (x < w && y < h).then(|| ((w - x).min(TILE_EDGE), (h - y).min(TILE_EDGE)))
}
/// Encoded bytes per output never exceed the SS-040 preview ceiling.
pub(crate) const MAX_OUTPUT: u64 = MAX_PREVIEW_BYTES;
/// Inspect sizes, largest first. An encoding over the byte ceiling steps down
/// to the next smaller tier instead of exceeding the wire contract.
pub(crate) fn inspect_steps(w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut steps: Vec<(u32, u32)> = INSPECT_STEPS.iter().map(|e| fit(w, h, *e)).collect();
    steps.dedup();
    steps
}

/// One worker decode. All three thumbnail tiers come from one decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Job {
    Thumbs,
    Inspect,
    Tile(u32, u32),
}
type Expected = (PreviewTier, Option<[u32; 2]>, u32, u32);
impl Job {
    fn for_request(tier: PreviewTier, tile: Option<[u32; 2]>) -> Option<Self> {
        match (tier, tile) {
            (PreviewTier::Small | PreviewTier::Medium | PreviewTier::Large, None) => {
                Some(Self::Thumbs)
            }
            (PreviewTier::Inspect, None) => Some(Self::Inspect),
            (PreviewTier::Tile, Some([c, r])) => Some(Self::Tile(c, r)),
            _ => None,
        }
    }
    fn args(self) -> Vec<String> {
        match self {
            Self::Thumbs => vec!["preview".into(), "thumbs".into()],
            Self::Inspect => vec!["preview".into(), "inspect".into()],
            Self::Tile(c, r) => vec!["preview".into(), "tile".into(), c.to_string(), r.to_string()],
        }
    }
    pub(crate) fn parse(args: &[String]) -> Option<Self> {
        let n = |s: &String| {
            (s.len() <= 2 && s.bytes().all(|b| b.is_ascii_digit()))
                .then(|| s.parse().ok())
                .flatten()
        };
        match args {
            [p, t] if p == "preview" && t == "thumbs" => Some(Self::Thumbs),
            [p, t] if p == "preview" && t == "inspect" => Some(Self::Inspect),
            [p, t, c, r] if p == "preview" && t == "tile" => Some(Self::Tile(n(c)?, n(r)?)),
            _ => None,
        }
    }
    /// Exact outputs, in worker order, for oriented source dimensions.
    pub(crate) fn outputs(self, w: u32, h: u32) -> Option<Vec<Expected>> {
        let sized = |t: PreviewTier| {
            let (a, b) = fit(w, h, t.edge());
            (t, None, a, b)
        };
        Some(match self {
            Self::Thumbs => vec![
                sized(PreviewTier::Large),
                sized(PreviewTier::Medium),
                sized(PreviewTier::Small),
            ],
            Self::Inspect => vec![sized(PreviewTier::Inspect)],
            Self::Tile(c, r) => {
                let (a, b) = tile_size(w, h, c, r)?;
                vec![(PreviewTier::Tile, Some([c, r]), a, b)]
            }
        })
    }
    /// Inspect may legitimately return any smaller step; others are exact.
    fn accepts(self, expected: Expected, o: &PreviewOutput, w: u32, h: u32) -> bool {
        let size = (o.width, o.height);
        (o.tier, o.tile) == (expected.0, expected.1)
            && match self {
                Self::Inspect => inspect_steps(w, h).contains(&size),
                _ => size == (expected.2, expected.3),
            }
    }
    fn reply_cap(self, w: u32, h: u32) -> u64 {
        let n = self.outputs(w, h).map_or(0, |o| o.len()) as u64;
        8 + MAX_HEADER as u64 + n * MAX_OUTPUT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreviewMime {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewOutput {
    pub tier: PreviewTier,
    pub tile: Option<[u32; 2]>,
    pub width: u32,
    pub height: u32,
    pub mime: PreviewMime,
    pub transparency: bool,
    pub bytes: u64,
    pub blake3: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewBatch {
    pub content_hash: String,
    pub analyzer: String,
    pub source_width: u32,
    pub source_height: u32,
    pub outputs: Vec<PreviewOutput>,
    pub peak_heap_bytes: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewReply {
    pub result: std::result::Result<PreviewBatch, AnalysisError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PreviewError {
    #[error("Invalid preview request")]
    InvalidArgument,
    #[error("Image not found in an authorized source")]
    NotFound,
    #[error("Image changed; refresh to preview the current version")]
    StaleVersion,
    #[error("Image is not technically ready for preview")]
    NotReady,
    #[error("Original is unavailable and no cached preview exists")]
    OriginalUnavailable,
    #[error("Image content changed while previewing; retry")]
    SourceChanged,
    #[error("Preview cancelled")]
    Cancelled,
    #[error("Preview queue is full; retry")]
    Busy,
    #[error("Preview timed out")]
    TimedOut,
    #[error("Image exceeds a preview resource limit")]
    LimitExceeded,
    #[error("Image or preview data is malformed")]
    InvalidData,
    #[error("Unsupported image format, animation or color profile")]
    Unsupported,
    #[error("Image is outside the authorized source scope")]
    PermissionDenied,
    #[error("Image previews are unavailable in this build")]
    WorkerUnavailable,
    #[error("Preview worker failed; original media is unchanged")]
    WorkerFailed,
    #[error("Preview service error")]
    Internal,
}
type Result<T> = std::result::Result<T, PreviewError>;
impl From<AnalysisError> for PreviewError {
    fn from(e: AnalysisError) -> Self {
        use AnalysisError as A;
        match e {
            A::Cancelled => Self::Cancelled,
            A::TimedOut => Self::TimedOut,
            A::LimitExceeded => Self::LimitExceeded,
            A::UnsupportedFormat | A::AnimationUnsupported | A::UnsupportedColorProfile => {
                Self::Unsupported
            }
            A::InvalidData => Self::InvalidData,
            A::SourceChanged => Self::SourceChanged,
            A::SourceUnavailable => Self::OriginalUnavailable,
            A::PermissionDenied => Self::PermissionDenied,
            A::WorkerUnavailable => Self::WorkerUnavailable,
            A::WorkerFailed => Self::WorkerFailed,
            A::Busy => Self::Busy,
        }
    }
}

/// Header-only check that payload bytes are the claimed type and size.
fn encoded_dimensions(mime: PreviewMime, b: &[u8]) -> Option<(u32, u32)> {
    let be32 = |p: &[u8]| Some(u32::from_be_bytes(p.get(..4)?.try_into().ok()?));
    match mime {
        PreviewMime::Png => {
            if !b.starts_with(b"\x89PNG\r\n\x1a\n") || b.get(8..16)? != b"\0\0\0\x0dIHDR" {
                return None;
            }
            Some((be32(&b[16..])?, be32(&b[20..])?))
        }
        PreviewMime::Jpeg => {
            if !b.starts_with(b"\xff\xd8") || !b.ends_with(b"\xff\xd9") {
                return None;
            }
            let mut pos = 2;
            while pos + 4 <= b.len().min(1 << 20) {
                if b[pos] != 0xff {
                    return None;
                }
                let marker = b[pos + 1];
                let len = u16::from_be_bytes([b[pos + 2], b[pos + 3]]) as usize;
                if matches!(marker, 0xc0 | 0xc1 | 0xc2) {
                    let p = b.get(pos + 4..pos + 9)?;
                    return Some((
                        u16::from_be_bytes([p[3], p[4]]).into(),
                        u16::from_be_bytes([p[1], p[2]]).into(),
                    ));
                }
                if len < 2 {
                    return None;
                }
                pos += 2 + len;
            }
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreviewKey {
    pub digest: String,
    pub tier: PreviewTier,
    pub tile: Option<[u32; 2]>,
}
impl PreviewKey {
    fn file_name(&self) -> String {
        match self.tile {
            Some([c, r]) => format!("tile-{c}-{r}.cspv"),
            None => format!("{}.cspv", self.tier.name()),
        }
    }
    fn parse_file(digest: &str, name: &str) -> Option<Self> {
        let stem = name.strip_suffix(".cspv")?;
        let (tier, tile) = match stem {
            "small" => (PreviewTier::Small, None),
            "medium" => (PreviewTier::Medium, None),
            "large" => (PreviewTier::Large, None),
            "inspect" => (PreviewTier::Inspect, None),
            _ => {
                let mut p = stem.strip_prefix("tile-")?.split('-');
                let n = |s: &str| {
                    (s.len() <= 2 && s.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| s.parse().ok())
                        .flatten()
                };
                let (c, r) = (n(p.next()?)?, n(p.next()?)?);
                if p.next().is_some() {
                    return None;
                }
                (PreviewTier::Tile, Some([c, r]))
            }
        };
        Some(Self {
            digest: digest.into(),
            tier,
            tile,
        })
    }
}
fn is_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Self-describing cache record header. Payload hash detects corruption/truncation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    preview_version: String,
    analyzer: String,
    color_policy: ColorPolicy,
    digest: String,
    tier: PreviewTier,
    tile: Option<[u32; 2]>,
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
    mime: PreviewMime,
    transparency: bool,
    payload_bytes: u64,
    payload_blake3: String,
}
impl Record {
    fn matches(&self, key: &PreviewKey) -> bool {
        self.preview_version == PREVIEW_VERSION
            && self.analyzer == ANALYZER
            && self.color_policy == ColorPolicy::Srgb
            && self.digest == key.digest
            && self.tier == key.tier
            && self.tile == key.tile
            && (!self.transparency || self.mime == PreviewMime::Png)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CacheCounters {
    pub hits: u64,
    pub misses: u64,
    pub generated: u64,
    pub coalesced: u64,
    pub evicted: u64,
    pub repaired: u64,
    pub write_failures: u64,
    pub cancelled: u64,
    pub stale_rejected: u64,
    pub offline_served: u64,
}
/// Counts only: no source paths, asset IDs or media bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewCacheStatus {
    pub enabled: bool,
    pub entries: u64,
    pub bytes: u64,
    pub quota_bytes: u64,
    pub active: u64,
    pub waiting: u64,
    pub counters: CacheCounters,
}
struct Entry {
    size: u64,
    used: u64,
    pins: u32,
    doomed: bool,
}
struct Store {
    loaded: bool,
    entries: HashMap<PreviewKey, Entry>,
    total: u64,
    quota: u64,
    clock: u64,
    counters: CacheCounters,
}
/// App-owned content-addressed preview store under one versioned namespace.
pub struct PreviewCache {
    root: PathBuf,
    namespace: String,
    store: Mutex<Store>,
}
pub struct CachedPreview {
    record: Record,
    bytes: Arc<Vec<u8>>,
}
impl PreviewCache {
    pub fn new(root: PathBuf, quota: u64) -> Self {
        let tag = blake3::hash(format!("{PREVIEW_VERSION}\0{ANALYZER}\0srgb").as_bytes());
        Self {
            root,
            namespace: format!("v1-{}", &tag.to_hex()[..16]),
            store: Mutex::new(Store {
                loaded: false,
                entries: HashMap::new(),
                total: 0,
                quota: quota.clamp(MIN_QUOTA, MAX_QUOTA),
                clock: 0,
                counters: CacheCounters::default(),
            }),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn path(&self, key: &PreviewKey) -> PathBuf {
        self.root
            .join(&self.namespace)
            .join(&key.digest[..2])
            .join(&key.digest)
            .join(key.file_name())
    }
    fn temporary(&self) -> PathBuf {
        self.root.join("tmp")
    }
    /// Lazy scan: remove only owned leftovers (our temp names, our superseded
    /// namespaces), then index existing records by mtime for LRU order.
    fn ensure(&self, s: &mut Store) {
        if s.loaded {
            return;
        }
        s.loaded = true;
        let _ = fs::create_dir_all(self.temporary());
        if let Ok(dir) = fs::read_dir(self.temporary()) {
            for f in dir.flatten() {
                let name = f.file_name();
                let name = name.to_string_lossy();
                if name
                    .strip_suffix(".part")
                    .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok() && id.len() == 36)
                {
                    let _ = fs::remove_file(f.path());
                }
            }
        }
        if let Ok(dir) = fs::read_dir(&self.root) {
            for f in dir.flatten() {
                let name = f.file_name();
                let name = name.to_string_lossy();
                let owned = name.strip_prefix("v1-").is_some_and(|t| {
                    t.len() == 16 && t.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                });
                if owned && name != self.namespace && f.file_type().is_ok_and(|t| t.is_dir()) {
                    let _ = fs::remove_dir_all(f.path());
                }
            }
        }
        let mut found = vec![];
        for prefix in fs::read_dir(self.root.join(&self.namespace)).into_iter().flatten().flatten() {
            for digest in fs::read_dir(prefix.path()).into_iter().flatten().flatten() {
                let d = digest.file_name().to_string_lossy().to_string();
                if !is_digest(&d) || *prefix.file_name().to_string_lossy() != d[..2] {
                    continue;
                }
                for file in fs::read_dir(digest.path()).into_iter().flatten().flatten() {
                    let Some(key) = PreviewKey::parse_file(&d, &file.file_name().to_string_lossy())
                    else {
                        continue;
                    };
                    if let Some(m) = file.metadata().ok().filter(|m| m.is_file()) {
                        let at = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                        found.push((at, key, m.len()));
                    }
                }
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, key, size) in found {
            s.clock += 1;
            s.total += size;
            let used = s.clock;
            s.entries.insert(
                key,
                Entry {
                    size,
                    used,
                    pins: 0,
                    doomed: false,
                },
            );
        }
        self.evict(s, None);
    }
    fn remove(&self, s: &mut Store, key: &PreviewKey) {
        if let Some(e) = s.entries.remove(key) {
            s.total -= e.size;
            let path = self.path(key);
            let _ = fs::remove_file(&path);
            if let Some(parent) = path.parent() {
                if fs::remove_dir(parent).is_ok() {
                    let _ = parent.parent().map(fs::remove_dir);
                }
            }
        }
    }
    /// LRU eviction never removes a pinned (currently read) or just-written entry.
    fn evict(&self, s: &mut Store, keep: Option<&PreviewKey>) {
        while s.total > s.quota {
            let victim = s
                .entries
                .iter()
                .filter(|(k, e)| e.pins == 0 && Some(*k) != keep)
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone());
            let Some(victim) = victim else { break };
            self.remove(s, &victim);
            s.counters.evicted += 1;
        }
    }
    pub fn get(&self, key: &PreviewKey) -> Option<CachedPreview> {
        self.lookup(key, true)
    }
    fn lookup(&self, key: &PreviewKey, count: bool) -> Option<CachedPreview> {
        {
            let mut s = self.lock();
            self.ensure(&mut s);
            s.clock += 1;
            let clock = s.clock;
            match s.entries.get_mut(key) {
                Some(e) if !e.doomed => {
                    e.pins += 1;
                    e.used = clock;
                }
                _ => {
                    s.counters.misses += u64::from(count);
                    return None;
                }
            }
        }
        let result = read_record(&self.path(key), key);
        let mut s = self.lock();
        let doomed = s.entries.get_mut(key).is_some_and(|e| {
            e.pins -= 1;
            e.doomed && e.pins == 0
        });
        match result {
            Some(hit) => {
                if doomed {
                    self.remove(&mut s, key);
                }
                s.counters.hits += u64::from(count);
                Some(hit)
            }
            None => {
                self.remove(&mut s, key);
                s.counters.repaired += 1;
                s.counters.misses += u64::from(count);
                None
            }
        }
    }
    fn put(&self, key: &PreviewKey, record: &Record, payload: &[u8]) -> bool {
        self.put_with(key, record, payload, |f, b| f.write_all(b))
    }
    /// Stage to an owned temp file, fsync, then rename into place. A failed or
    /// partial write (including disk full) leaves no record and is only counted.
    fn put_with(
        &self,
        key: &PreviewKey,
        record: &Record,
        payload: &[u8],
        write: impl FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
    ) -> bool {
        {
            let mut s = self.lock();
            self.ensure(&mut s);
        }
        let Ok(header) = serde_json::to_vec(record) else {
            return false;
        };
        let mut bytes = Vec::with_capacity(12 + header.len() + payload.len());
        bytes.extend_from_slice(RECORD_MAGIC);
        bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(payload);
        let size = bytes.len() as u64;
        let tmp = self
            .temporary()
            .join(format!("{}.part", uuid::Uuid::new_v4()));
        let target = self.path(key);
        let staged = (|| {
            fs::create_dir_all(self.temporary())?;
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            write(&mut f, &bytes)?;
            f.sync_all()?;
            drop(f);
            fs::create_dir_all(target.parent().ok_or(std::io::ErrorKind::InvalidInput)?)?;
            fs::rename(&tmp, &target)
        })();
        let mut s = self.lock();
        if staged.is_err() {
            let _ = fs::remove_file(&tmp);
            s.counters.write_failures += 1;
            return false;
        }
        s.clock += 1;
        let used = s.clock;
        let previous = s.entries.insert(
            key.clone(),
            Entry {
                size,
                used,
                pins: 0,
                doomed: false,
            },
        );
        if let Some(old) = previous {
            s.total -= old.size;
            if let Some(e) = s.entries.get_mut(key) {
                e.pins = old.pins;
            }
        }
        s.total += size;
        self.evict(&mut s, Some(key));
        true
    }
    pub fn set_quota(&self, quota: u64) -> std::result::Result<(), PreviewError> {
        if !(MIN_QUOTA..=MAX_QUOTA).contains(&quota) {
            return Err(PreviewError::InvalidArgument);
        }
        let mut s = self.lock();
        self.ensure(&mut s);
        s.quota = quota;
        self.evict(&mut s, None);
        Ok(())
    }
    /// Remove every preview record. In-use records go when their reader finishes.
    /// Analysis rows, annotations and originals are never touched.
    pub fn clear(&self) -> u64 {
        self.retain(|_| false)
    }
    /// Drop records for digests no current image references (replaced content).
    pub fn prune(&self, referenced: &HashSet<String>) -> u64 {
        self.retain(|k| referenced.contains(&k.digest))
    }
    fn retain(&self, keep: impl Fn(&PreviewKey) -> bool) -> u64 {
        let mut s = self.lock();
        self.ensure(&mut s);
        let keys: Vec<_> = s.entries.keys().filter(|k| !keep(k)).cloned().collect();
        let mut removed = 0;
        for k in keys {
            if let Some(e) = s.entries.get_mut(&k) {
                if e.pins > 0 {
                    e.doomed = true;
                    continue;
                }
            }
            self.remove(&mut s, &k);
            removed += 1;
        }
        removed
    }
    pub fn status(&self) -> PreviewCacheStatus {
        let mut s = self.lock();
        self.ensure(&mut s);
        PreviewCacheStatus {
            enabled: false,
            entries: s.entries.len() as u64,
            bytes: s.total,
            quota_bytes: s.quota,
            active: 0,
            waiting: 0,
            counters: s.counters,
        }
    }
    fn count(&self, f: impl FnOnce(&mut CacheCounters)) {
        f(&mut self.lock().counters);
    }
}
fn read_record(path: &Path, key: &PreviewKey) -> Option<CachedPreview> {
    let bound = 12 + MAX_HEADER as u64 + MAX_OUTPUT;
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(bound + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() < 12 || &bytes[..8] != RECORD_MAGIC {
        return None;
    }
    let n = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
    if n > MAX_HEADER || bytes.len() < 12 + n {
        return None;
    }
    let record: Record = serde_json::from_slice(&bytes[12..12 + n]).ok()?;
    let payload = &bytes[12 + n..];
    if !record.matches(key)
        || payload.len() as u64 != record.payload_bytes
        || payload.len() as u64 > MAX_OUTPUT
        || record.width.max(record.height) > INSPECT_EDGE
        || blake3::hash(payload).to_hex().as_str() != record.payload_blake3
        || encoded_dimensions(record.mime, payload) != Some((record.width, record.height))
    {
        return None;
    }
    let payload = payload.to_vec();
    Some(CachedPreview {
        record,
        bytes: Arc::new(payload),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub request_id: String,
    pub asset_id: String,
    pub version: String,
    pub tier: PreviewTier,
    pub tile: Option<[u32; 2]>,
}
impl PreviewRequest {
    fn validate(&self) -> Result<Job> {
        let id_ok = !self.request_id.is_empty()
            && self.request_id.len() <= 64
            && self
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        let asset_ok = uuid::Uuid::parse_str(&self.asset_id)
            .is_ok_and(|u| u.hyphenated().to_string() == self.asset_id);
        if !id_ok || !asset_ok || !is_digest(&self.version) {
            return Err(PreviewError::InvalidArgument);
        }
        if self.tile.is_some_and(|[c, r]| c > 64 || r > 64) {
            return Err(PreviewError::InvalidArgument);
        }
        Job::for_request(self.tier, self.tile).ok_or(PreviewError::InvalidArgument)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    Hit,
    Generated,
    /// Served from a fresh decode; the cache write failed (for example disk full).
    Uncached,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreviewMeta {
    pub asset_id: String,
    pub version: String,
    pub tier: PreviewTier,
    pub tile: Option<[u32; 2]>,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub mime: PreviewMime,
    pub transparency: bool,
    pub cache: CacheState,
    /// Catalog state of the original; a cached preview never proves availability.
    pub original_available: bool,
    /// Served from cache while the original is unavailable; not exportable.
    pub offline_reference: bool,
    pub preview_version: String,
}
pub struct Preview {
    pub meta: PreviewMeta,
    pub bytes: Arc<Vec<u8>>,
}
impl Preview {
    /// `CSPV` + u32 LE metadata length + metadata JSON + encoded image bytes.
    pub fn envelope(&self) -> Vec<u8> {
        let meta = serde_json::to_vec(&self.meta).unwrap_or_default();
        let mut out = Vec::with_capacity(8 + meta.len() + self.bytes.len());
        out.extend_from_slice(ENVELOPE_MAGIC);
        out.extend_from_slice(&(meta.len() as u32).to_le_bytes());
        out.extend_from_slice(&meta);
        out.extend_from_slice(&self.bytes);
        out
    }
}

struct Authorized {
    asset_id: String,
    source_id: String,
    digest: String,
    width: u32,
    height: u32,
    original_available: bool,
    selection: Option<ImageSelection>,
}
struct Generated {
    key: PreviewKey,
    record: Record,
    bytes: Arc<Vec<u8>>,
    state: CacheState,
}
type Outcome = Result<Arc<Vec<Generated>>>;
struct Flight {
    cancel: Arc<AtomicBool>,
    waiters: Mutex<usize>,
    done: Mutex<Option<Outcome>>,
    ready: Condvar,
}
impl Flight {
    fn leave(&self) {
        let mut w = self.waiters.lock().unwrap_or_else(|e| e.into_inner());
        *w = w.saturating_sub(1);
        if *w == 0 {
            self.cancel.store(true, Ordering::Release);
        }
    }
}
struct Ticket {
    cancelled: AtomicBool,
    inspect: bool,
    flight: Mutex<Option<Arc<Flight>>>,
}
impl Ticket {
    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            if let Some(f) = self.flight.lock().unwrap_or_else(|e| e.into_inner()).take() {
                f.leave();
            }
        }
    }
}
/// Authorized, coalesced, cancellable preview generation over a shared cache.
pub struct PreviewService {
    catalog: Arc<Mutex<Catalog>>,
    cache: Arc<PreviewCache>,
    worker: PathBuf,
    slots: Mutex<usize>,
    slot_free: Condvar,
    flights: Mutex<HashMap<(String, Job), Arc<Flight>>>,
    tickets: Mutex<HashMap<String, Arc<Ticket>>>,
    active: AtomicUsize,
}
struct Registration<'a> {
    service: &'a PreviewService,
    id: String,
    ticket: Arc<Ticket>,
}
impl Drop for Registration<'_> {
    fn drop(&mut self) {
        // A completed request leaves its flight; others may still wait on it.
        if let Some(f) = self.ticket.flight.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let mut w = f.waiters.lock().unwrap_or_else(|e| e.into_inner());
            *w = w.saturating_sub(1);
        }
        lock(&self.service.tickets).remove(&self.id);
    }
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
impl PreviewService {
    pub fn new(catalog: Arc<Mutex<Catalog>>, cache: Arc<PreviewCache>, worker: PathBuf) -> Self {
        Self {
            catalog,
            cache,
            worker,
            slots: Mutex::new(0),
            slot_free: Condvar::new(),
            flights: Mutex::new(HashMap::new()),
            tickets: Mutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
        }
    }
    pub fn cache(&self) -> &Arc<PreviewCache> {
        &self.cache
    }
    /// Cancel a request when its view disappears. Shared work stops only when
    /// every coalesced requester has cancelled.
    pub fn cancel(&self, request_id: &str) -> bool {
        let ticket = lock(&self.tickets).get(request_id).cloned();
        ticket.map(|t| t.cancel()).is_some()
    }
    pub fn status(&self) -> PreviewCacheStatus {
        let mut s = self.cache.status();
        s.enabled = true;
        s.active = self.active.load(Ordering::Relaxed) as u64;
        s.waiting = lock(&self.tickets).len() as u64;
        s
    }
    pub fn preview(&self, request: &PreviewRequest) -> Result<Preview> {
        let job = request.validate()?;
        let registration = self.register(&request.request_id, job == Job::Inspect)?;
        let ticket = &registration.ticket;
        let auth = self.authorize(request)?;
        let key = PreviewKey {
            digest: auth.digest.clone(),
            tier: request.tier,
            tile: request.tile,
        };
        if let Some(hit) = self.cache.get(&key) {
            self.fence(&auth)?;
            return Ok(self.respond(&auth, hit.record, hit.bytes, CacheState::Hit));
        }
        if ticket.cancelled.load(Ordering::Acquire) {
            self.cache.count(|c| c.cancelled += 1);
            return Err(PreviewError::Cancelled);
        }
        if !auth.original_available || auth.selection.is_none() {
            return Err(PreviewError::OriginalUnavailable);
        }
        let outputs = self.generate(ticket, &auth, job)?;
        if ticket.cancelled.load(Ordering::Acquire) {
            self.cache.count(|c| c.cancelled += 1);
            return Err(PreviewError::Cancelled);
        }
        let g = outputs
            .iter()
            .find(|g| g.key == key)
            .ok_or(PreviewError::Internal)?;
        self.fence(&auth)?;
        Ok(self.respond(&auth, g.record.clone(), g.bytes.clone(), g.state))
    }
    fn register(&self, id: &str, inspect: bool) -> Result<Registration<'_>> {
        let mut tickets = lock(&self.tickets);
        if tickets.contains_key(id) {
            return Err(PreviewError::InvalidArgument);
        }
        if tickets.len() >= MAX_OUTSTANDING
            || inspect && tickets.values().filter(|t| t.inspect).count() >= MAX_INSPECT_OUTSTANDING
        {
            return Err(PreviewError::Busy);
        }
        let ticket = Arc::new(Ticket {
            cancelled: AtomicBool::new(false),
            inspect,
            flight: Mutex::new(None),
        });
        tickets.insert(id.into(), ticket.clone());
        Ok(Registration {
            service: self,
            id: id.into(),
            ticket,
        })
    }
    fn authorize(&self, r: &PreviewRequest) -> Result<Authorized> {
        let c = self.catalog.lock().map_err(|_| PreviewError::Internal)?;
        let asset = c
            .image(&r.asset_id, None)
            .map_err(|_| PreviewError::NotFound)?;
        if asset.header.kind != AssetKind::Image {
            return Err(PreviewError::NotFound);
        }
        if asset.header.content_hash.as_deref() != Some(r.version.as_str()) {
            drop(c);
            self.cache.count(|s| s.stale_rejected += 1);
            return Err(PreviewError::StaleVersion);
        }
        let profile = match (asset.header.technical_state, asset.profile) {
            (TechnicalState::Ready, Some(p)) => p,
            _ => return Err(PreviewError::NotReady),
        };
        if let Some([col, row]) = r.tile {
            tile_size(profile.width, profile.height, col, row)
                .ok_or(PreviewError::InvalidArgument)?;
        }
        let source = c
            .source_header(&asset.header.source_id)
            .map_err(|_| PreviewError::NotFound)?;
        let original_available =
            source.available && asset.header.availability == Availability::Available;
        let selection = if original_available {
            ImageSelection::capture(&c, &source.id, &asset.header.relative_path).ok()
        } else {
            None
        };
        Ok(Authorized {
            asset_id: r.asset_id.clone(),
            source_id: source.id,
            digest: r.version.clone(),
            width: profile.width,
            height: profile.height,
            original_available,
            selection,
        })
    }
    /// Recheck identity, version and source registration immediately before serving.
    fn fence(&self, a: &Authorized) -> Result<()> {
        let c = self.catalog.lock().map_err(|_| PreviewError::Internal)?;
        let asset = c
            .image(&a.asset_id, None)
            .map_err(|_| PreviewError::NotFound)?;
        let stale = asset.header.source_id != a.source_id
            || asset.header.content_hash.as_deref() != Some(a.digest.as_str())
            || asset.header.technical_state != TechnicalState::Ready;
        c.source_header(&a.source_id)
            .map_err(|_| PreviewError::NotFound)?;
        drop(c);
        if stale {
            self.cache.count(|s| s.stale_rejected += 1);
            return Err(PreviewError::StaleVersion);
        }
        Ok(())
    }
    fn respond(&self, a: &Authorized, r: Record, bytes: Arc<Vec<u8>>, cache: CacheState) -> Preview {
        if !a.original_available {
            self.cache.count(|s| s.offline_served += 1);
        }
        Preview {
            meta: PreviewMeta {
                asset_id: a.asset_id.clone(),
                version: a.digest.clone(),
                tier: r.tier,
                tile: r.tile,
                width: r.width,
                height: r.height,
                source_width: r.source_width,
                source_height: r.source_height,
                mime: r.mime,
                transparency: r.transparency,
                cache,
                original_available: a.original_available,
                offline_reference: !a.original_available,
                preview_version: PREVIEW_VERSION.into(),
            },
            bytes,
        }
    }
    fn generate(&self, ticket: &Arc<Ticket>, auth: &Authorized, job: Job) -> Outcome {
        let fk = (auth.digest.clone(), job);
        let (flight, leader) = {
            let mut flights = lock(&self.flights);
            match flights.get(&fk) {
                Some(f) if !f.cancel.load(Ordering::Acquire) => (f.clone(), false),
                _ => {
                    let f = Arc::new(Flight {
                        cancel: Arc::new(AtomicBool::new(false)),
                        waiters: Mutex::new(0),
                        done: Mutex::new(None),
                        ready: Condvar::new(),
                    });
                    flights.insert(fk.clone(), f.clone());
                    (f, true)
                }
            }
        };
        *lock(&flight.waiters) += 1;
        *lock(&ticket.flight) = Some(flight.clone());
        // A cancel that raced ahead of joining found no flight to leave.
        if ticket.cancelled.load(Ordering::Acquire) {
            if let Some(f) = lock(&ticket.flight).take() {
                f.leave();
            }
        }
        if !leader {
            self.cache.count(|s| s.coalesced += 1);
        } else {
            let outcome = self.run(&flight, auth, job);
            if matches!(outcome, Err(PreviewError::Cancelled)) {
                self.cache.count(|s| s.cancelled += 1);
            }
            *lock(&flight.done) = Some(outcome);
            flight.ready.notify_all();
            let mut flights = lock(&self.flights);
            if flights.get(&fk).is_some_and(|f| Arc::ptr_eq(f, &flight)) {
                flights.remove(&fk);
            }
        }
        let mut done = lock(&flight.done);
        loop {
            if let Some(outcome) = done.as_ref() {
                return outcome.clone();
            }
            if ticket.cancelled.load(Ordering::Acquire) {
                return Err(PreviewError::Cancelled);
            }
            done = flight
                .ready
                .wait_timeout(done, Duration::from_millis(25))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
    fn acquire(&self, cancel: &AtomicBool) -> Result<SlotGuard<'_>> {
        let start = Instant::now();
        let mut used = lock(&self.slots);
        while *used >= PREVIEW_SLOTS {
            if cancel.load(Ordering::Acquire) {
                return Err(PreviewError::Cancelled);
            }
            if start.elapsed() >= SLOT_WAIT {
                return Err(PreviewError::Busy);
            }
            used = self
                .slot_free
                .wait_timeout(used, Duration::from_millis(25))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *used += 1;
        self.active.fetch_add(1, Ordering::Relaxed);
        Ok(SlotGuard(self))
    }
    /// A new leader may have missed the cache just before a previous flight
    /// published it; reuse those records instead of decoding again.
    fn cached_job(&self, auth: &Authorized, job: Job) -> Option<Vec<Generated>> {
        let outputs = job.outputs(auth.width, auth.height)?;
        outputs
            .into_iter()
            .map(|(tier, tile, ..)| {
                let key = PreviewKey {
                    digest: auth.digest.clone(),
                    tier,
                    tile,
                };
                let hit = self.cache.lookup(&key, false)?;
                Some(Generated {
                    key,
                    record: hit.record,
                    bytes: hit.bytes,
                    state: CacheState::Hit,
                })
            })
            .collect()
    }
    fn run(&self, flight: &Flight, auth: &Authorized, job: Job) -> Outcome {
        if let Some(cached) = self.cached_job(auth, job) {
            return Ok(Arc::new(cached));
        }
        let _slot = self.acquire(&flight.cancel)?;
        if flight.cancel.load(Ordering::Acquire) {
            return Err(PreviewError::Cancelled);
        }
        let selection = auth
            .selection
            .as_ref()
            .ok_or(PreviewError::OriginalUnavailable)?;
        // Containment and symlink resolution happen at this open, after the lock.
        let input = selection.open()?;
        let args = job.args();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let raw = image_analysis::run_worker(
            &self.worker,
            input,
            flight.cancel.clone(),
            Instant::now() + PREVIEW_DEADLINE,
            &args,
            job.reply_cap(auth.width, auth.height),
        )?;
        let outputs = parse_batch(&raw, auth, job)?;
        let mut generated = Vec::with_capacity(outputs.len());
        for (key, record, bytes) in outputs {
            let state = if self.cache.put(&key, &record, &bytes) {
                CacheState::Generated
            } else {
                CacheState::Uncached
            };
            generated.push(Generated {
                key,
                record,
                bytes: Arc::new(bytes),
                state,
            });
        }
        self.cache.count(|s| s.generated += 1);
        Ok(Arc::new(generated))
    }
}
struct SlotGuard<'a>(&'a PreviewService);
impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        *lock(&self.0.slots) -= 1;
        self.0.active.fetch_sub(1, Ordering::Relaxed);
        self.0.slot_free.notify_one();
    }
}
/// Validate the worker's framed batch against the authorized version and job.
fn parse_batch(raw: &[u8], auth: &Authorized, job: Job) -> Result<Vec<(PreviewKey, Record, Vec<u8>)>> {
    let bad = PreviewError::InvalidData;
    if raw.len() < 8 || &raw[..4] != WORKER_MAGIC {
        return Err(PreviewError::WorkerFailed);
    }
    let n = u32::from_le_bytes(raw[4..8].try_into().map_err(|_| bad)?) as usize;
    if n > MAX_HEADER || raw.len() < 8 + n {
        return Err(bad);
    }
    let reply: PreviewReply = serde_json::from_slice(&raw[8..8 + n]).map_err(|_| bad)?;
    let batch = reply.result?;
    if batch.content_hash != auth.digest {
        return Err(PreviewError::SourceChanged);
    }
    let expected = job.outputs(auth.width, auth.height).ok_or(bad)?;
    if batch.analyzer != ANALYZER
        || (batch.source_width, batch.source_height) != (auth.width, auth.height)
        || batch.peak_heap_bytes > WORKER_HEAP as u64
        || batch.outputs.len() != expected.len()
    {
        return Err(bad);
    }
    let mut payload = &raw[8 + n..];
    let mut out = Vec::with_capacity(expected.len());
    for (o, e) in batch.outputs.into_iter().zip(expected) {
        let (tier, tile, w, h) = (e.0, e.1, o.width, o.height);
        if !job.accepts(e, &o, auth.width, auth.height)
            || o.bytes > MAX_OUTPUT
            || o.bytes as usize > payload.len()
            || o.transparency && o.mime != PreviewMime::Png
        {
            return Err(bad);
        }
        let (bytes, rest) = payload.split_at(o.bytes as usize);
        payload = rest;
        if blake3::hash(bytes).to_hex().as_str() != o.blake3
            || encoded_dimensions(o.mime, bytes) != Some((w, h))
        {
            return Err(bad);
        }
        let key = PreviewKey {
            digest: auth.digest.clone(),
            tier,
            tile,
        };
        let record = Record {
            preview_version: PREVIEW_VERSION.into(),
            analyzer: ANALYZER.into(),
            color_policy: ColorPolicy::Srgb,
            digest: auth.digest.clone(),
            tier,
            tile,
            width: w,
            height: h,
            source_width: auth.width,
            source_height: auth.height,
            mime: o.mime,
            transparency: o.transparency,
            payload_bytes: o.bytes,
            payload_blake3: o.blake3,
        };
        out.push((key, record, bytes.to_vec()));
    }
    if !payload.is_empty() {
        return Err(bad);
    }
    Ok(out)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuotaSetting {
    quota_bytes: u64,
}
impl Catalog {
    /// Separate app_settings key: older binaries ignore it (Preferences rejects
    /// unknown fields). An invalid stored value falls back to the default.
    pub fn image_preview_quota(&self) -> crate::Result<u64> {
        use rusqlite::OptionalExtension;
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [QUOTA_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw
            .and_then(|v| serde_json::from_str::<QuotaSetting>(&v).ok())
            .map(|s| s.quota_bytes)
            .filter(|q| (MIN_QUOTA..=MAX_QUOTA).contains(q))
            .unwrap_or(DEFAULT_QUOTA))
    }
    pub fn set_image_preview_quota(&self, quota_bytes: u64) -> crate::Result<()> {
        if !(MIN_QUOTA..=MAX_QUOTA).contains(&quota_bytes) {
            return Err(crate::invalid("Preview cache size is outside the supported range"));
        }
        self.db.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [QUOTA_KEY, &serde_json::to_string(&QuotaSetting { quota_bytes })?],
        )?;
        Ok(())
    }
    /// Current content digests of image records, used to prune replaced previews.
    pub fn image_content_digests(&self) -> crate::Result<HashSet<String>> {
        let mut q = self
            .db
            .prepare("SELECT content_hash FROM image_assets WHERE content_hash IS NOT NULL")?;
        let rows = q.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    /// Incompressible synthetic pixels so record sizes are predictable.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut x = 0x9e37_79b9u32 ^ w;
        let pixels: Vec<u8> = (0..w * h * 4)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        let mut out = vec![];
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&pixels, w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }
    fn key(n: u8, tier: PreviewTier) -> PreviewKey {
        PreviewKey {
            digest: format!("{n:02x}").repeat(32),
            tier,
            tile: None,
        }
    }
    fn record(k: &PreviewKey, payload: &[u8], w: u32, h: u32) -> Record {
        Record {
            preview_version: PREVIEW_VERSION.into(),
            analyzer: ANALYZER.into(),
            color_policy: ColorPolicy::Srgb,
            digest: k.digest.clone(),
            tier: k.tier,
            tile: k.tile,
            width: w,
            height: h,
            source_width: w,
            source_height: h,
            mime: PreviewMime::Png,
            transparency: true,
            payload_bytes: payload.len() as u64,
            payload_blake3: blake3::hash(payload).to_hex().to_string(),
        }
    }
    fn put(c: &PreviewCache, k: &PreviewKey, w: u32) -> u64 {
        let p = png(w, w);
        assert!(c.put(k, &record(k, &p, w, w), &p));
        c.status().bytes
    }

    #[test]
    fn records_round_trip_and_corruption_or_truncation_is_repaired_as_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        let c = PreviewCache::new(dir.path().into(), DEFAULT_QUOTA);
        let k = key(1, PreviewTier::Small);
        put(&c, &k, 16);
        let hit = c.get(&k).unwrap();
        assert_eq!(hit.bytes.as_slice(), png(16, 16).as_slice());
        // Flip one payload byte: hash mismatch removes the record.
        let path = c.path(&k);
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, &bytes).unwrap();
        assert!(c.get(&k).is_none());
        assert!(!path.exists());
        // A truncated (partially written by an external tool) record is also repaired.
        put(&c, &k, 16);
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(c.get(&k).is_none());
        let s = c.status();
        assert_eq!((s.counters.repaired, s.entries, s.bytes), (2, 0, 0));
        // A record for a different key placed at this path is never served.
        let other = key(2, PreviewTier::Small);
        put(&c, &other, 8);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(c.path(&other), &path).unwrap();
        let reopened = PreviewCache::new(dir.path().into(), DEFAULT_QUOTA);
        assert!(reopened.get(&k).is_none());
        assert!(reopened.get(&other).is_some());
    }

    #[test]
    fn disk_full_and_partial_writes_leave_no_record_or_owned_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let c = PreviewCache::new(dir.path().into(), DEFAULT_QUOTA);
        let k = key(3, PreviewTier::Medium);
        let p = png(32, 32);
        let ok = c.put_with(&k, &record(&k, &p, 32, 32), &p, |f, b| {
            f.write_all(&b[..b.len() / 2])?;
            Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
        });
        assert!(!ok);
        assert!(c.get(&k).is_none());
        assert!(!c.path(&k).exists());
        assert_eq!(fs::read_dir(c.temporary()).unwrap().count(), 0);
        assert_eq!(c.status().counters.write_failures, 1);
        // The next successful write is unaffected.
        put(&c, &k, 32);
        assert!(c.get(&k).is_some());
    }

    #[test]
    fn lru_eviction_respects_quota_and_never_evicts_a_record_being_read() {
        let dir = tempfile::tempdir().unwrap();
        let c = PreviewCache::new(dir.path().into(), MIN_QUOTA);
        let big = 64;
        let one = put(&c, &key(1, PreviewTier::Inspect), big);
        // Room for two records; the public minimum is enforced separately below.
        c.lock().quota = one * 2 + one / 2;
        put(&c, &key(2, PreviewTier::Inspect), big);
        // Touch #1 so #2 is least recently used, then pin #1 as an active read.
        assert!(c.get(&key(1, PreviewTier::Inspect)).is_some());
        c.lock()
            .entries
            .get_mut(&key(1, PreviewTier::Inspect))
            .unwrap()
            .pins += 1;
        put(&c, &key(3, PreviewTier::Inspect), big);
        put(&c, &key(4, PreviewTier::Inspect), big);
        let s = c.status();
        assert!(s.bytes <= one * 2 + one / 2, "{s:?}");
        assert!(c.lock().entries.contains_key(&key(1, PreviewTier::Inspect)));
        assert!(!c.path(&key(2, PreviewTier::Inspect)).exists());
        assert!(s.counters.evicted >= 1);
        // Clearing while pinned defers that record's deletion until the read ends.
        c.clear();
        assert!(c.path(&key(1, PreviewTier::Inspect)).exists());
        c.lock()
            .entries
            .get_mut(&key(1, PreviewTier::Inspect))
            .unwrap()
            .pins -= 1;
        assert!(c.get(&key(1, PreviewTier::Inspect)).is_none());
        assert!(c.set_quota(MIN_QUOTA - 1).is_err());
        assert!(c.set_quota(MAX_QUOTA + 1).is_err());
    }

    #[test]
    fn restart_removes_only_owned_temporaries_and_superseded_namespaces() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let c = PreviewCache::new(root.clone(), DEFAULT_QUOTA);
        let k = key(5, PreviewTier::Large);
        put(&c, &k, 8);
        drop(c);
        let owned = root.join("tmp").join(format!("{}.part", uuid::Uuid::new_v4()));
        let foreign = root.join("tmp").join("notes.part");
        fs::write(&owned, b"partial").unwrap();
        fs::write(&foreign, b"user").unwrap();
        let old = root.join("v1-0123456789abcdef");
        fs::create_dir_all(old.join("aa")).unwrap();
        let unrelated = root.join("v1-not-ours");
        fs::create_dir_all(&unrelated).unwrap();
        let c = PreviewCache::new(root.clone(), DEFAULT_QUOTA);
        // The restarted index finds the existing record without regeneration.
        assert!(c.get(&k).is_some());
        assert!(!owned.exists() && foreign.exists());
        assert!(!old.exists() && unrelated.exists());
        // Pruning removes replaced content only.
        put(&c, &key(6, PreviewTier::Large), 8);
        let keep: HashSet<String> = [key(6, PreviewTier::Large).digest].into();
        assert_eq!(c.prune(&keep), 1);
        assert!(c.get(&k).is_none() && c.get(&key(6, PreviewTier::Large)).is_some());
    }

    #[test]
    fn outstanding_requests_are_bounded_and_request_ids_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Arc::new(Mutex::new(Catalog::open(&dir.path().join("db.sqlite")).unwrap()));
        let cache = Arc::new(PreviewCache::new(dir.path().join("p"), DEFAULT_QUOTA));
        let s = PreviewService::new(catalog, cache, PathBuf::from("/none"));
        let inspect: Vec<_> = (0..MAX_INSPECT_OUTSTANDING)
            .map(|i| s.register(&format!("i{i}"), true).ok().unwrap())
            .collect();
        assert!(matches!(s.register("i-extra", true), Err(PreviewError::Busy)));
        let held: Vec<_> = (MAX_INSPECT_OUTSTANDING..MAX_OUTSTANDING)
            .map(|i| s.register(&format!("r{i}"), false).ok().unwrap())
            .collect();
        assert!(matches!(s.register("extra", false), Err(PreviewError::Busy)));
        assert_eq!(s.status().waiting, MAX_OUTSTANDING as u64);
        drop((held, inspect));
        let a = s.register("dup", false).ok().unwrap();
        assert!(matches!(s.register("dup", false), Err(PreviewError::InvalidArgument)));
        assert!(s.cancel("dup") && a.ticket.cancelled.load(Ordering::Acquire));
        drop(a);
        assert!(!s.cancel("dup"));
    }
    #[test]
    fn quota_setting_is_separate_from_preferences_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        let c = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
        assert_eq!(c.image_preview_quota().unwrap(), DEFAULT_QUOTA);
        c.set_image_preview_quota(MIN_QUOTA).unwrap();
        assert!(c.set_image_preview_quota(MAX_QUOTA + 1).is_err());
        assert_eq!(c.image_preview_quota().unwrap(), MIN_QUOTA);
        // Existing deny_unknown_fields preferences remain readable.
        assert_eq!(c.preferences().unwrap(), Default::default());
        c.db.execute("UPDATE app_settings SET value='{\"quota_bytes\":1}' WHERE key=?1", [QUOTA_KEY])
            .unwrap();
        assert_eq!(c.image_preview_quota().unwrap(), DEFAULT_QUOTA);
    }
    #[test]
    fn request_and_worker_argument_contracts_reject_unbounded_or_mixed_inputs() {
        let base = PreviewRequest {
            request_id: "r-1".into(),
            asset_id: uuid::Uuid::new_v4().to_string(),
            version: "a".repeat(64),
            tier: PreviewTier::Small,
            tile: None,
        };
        assert_eq!(base.validate(), Ok(Job::Thumbs));
        for bad in [
            PreviewRequest { asset_id: "../../etc/passwd".into(), ..base.clone() },
            PreviewRequest { asset_id: base.asset_id.to_uppercase(), ..base.clone() },
            PreviewRequest { version: "A".repeat(64), ..base.clone() },
            PreviewRequest { request_id: "x".repeat(65), ..base.clone() },
            PreviewRequest { request_id: "a/b".into(), ..base.clone() },
            PreviewRequest { tile: Some([0, 0]), ..base.clone() },
            PreviewRequest { tier: PreviewTier::Tile, ..base.clone() },
            PreviewRequest { tier: PreviewTier::Tile, tile: Some([65, 0]), ..base.clone() },
        ] {
            assert_eq!(bad.validate(), Err(PreviewError::InvalidArgument), "{bad:?}");
        }
        for job in [Job::Thumbs, Job::Inspect, Job::Tile(3, 64)] {
            assert_eq!(Job::parse(&job.args()), Some(job));
        }
        assert_eq!(Job::parse(&["preview".into(), "tile".into(), "999".into(), "0".into()]), None);
        assert_eq!(Job::parse(&["/tmp/x".into()]), None);
        assert_eq!(fit(4096, 1024, 2048), (2048, 512));
        assert_eq!(inspect_steps(1100, 1100), vec![(1100, 1100), (1024, 1024), (768, 768), (512, 512)]);
        assert_eq!(inspect_steps(300, 200), vec![(300, 200)]);
        assert_eq!(fit(3, 2, 128), (3, 2));
        assert_eq!(fit(32768, 1, 128), (128, 1));
        assert_eq!(tile_size(1000, 513, 1, 1), Some((488, 1)));
        assert_eq!(tile_size(1000, 513, 2, 0), None);
    }
}
