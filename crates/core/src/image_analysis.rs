//! Version-bound still-image analysis. Call on background threads, without a catalog lock.
//! The decoder runs only in the separately supervised, heap-budgeted image worker.
use crate::{
    catalog::{contained, Catalog, Source},
    image_contracts::{self, ImageProfile},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const ANALYZER: &str = "creativeshelf-still-v1-image02510-mox091";
pub const MAX_ENCODED: u64 = 64 * 1024 * 1024;
pub const MAX_METADATA: usize = 1024 * 1024;
pub const MAX_PIXELS: u64 = 16_777_216;
pub const MAX_DECODED: u64 = 128 * 1024 * 1024;
pub const WORKER_HEAP: usize = 512 * 1024 * 1024;
pub const DEADLINE: Duration = Duration::from_secs(30);
const MAX_REPLY: u64 = 16 * 1024;
static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Advisory scheduling signal; analyze still atomically rejects overload.
pub fn worker_busy() -> bool {
    WORKER_ACTIVE.load(Ordering::Acquire)
}

type Result<T> = std::result::Result<T, AnalysisError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalysisError {
    #[error("Image analysis cancelled")]
    Cancelled,
    #[error("Image analysis timed out")]
    TimedOut,
    #[error("Image exceeds a resource limit")]
    LimitExceeded,
    #[error("Unsupported still-image format")]
    UnsupportedFormat,
    #[error("Animated images are unsupported")]
    AnimationUnsupported,
    #[error("Image data or metadata is malformed")]
    InvalidData,
    #[error("Unsupported color profile; sRGB conversion unavailable")]
    UnsupportedColorProfile,
    #[error("Image content or source changed; retry analysis")]
    SourceChanged,
    #[error("Image source is unavailable")]
    SourceUnavailable,
    #[error("Image is outside the authorized source scope")]
    PermissionDenied,
    #[error("Image worker is unavailable")]
    WorkerUnavailable,
    #[error("Image worker failed; original media is unchanged")]
    WorkerFailed,
    #[error("Image worker is busy")]
    Busy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisWarning {
    ExtensionMismatch,
    AssumedSrgb,
    HighBitDepthDisplay8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageAnalysis {
    pub content_hash: String,
    pub profile: ImageProfile,
    /// Exact ratio numerator/denominator, avoiding rounded persisted facts.
    pub aspect_ratio: [u32; 2],
    pub transparent_pixels: u64,
    pub partial_alpha_pixels: u64,
    pub source_color_space: String,
    pub warnings: Vec<AnalysisWarning>,
    /// Hash of canonical oriented, straight-alpha, display-sRGB RGBA8 pixels.
    pub display_rgba_hash: String,
    pub display_corners: [[u8; 4]; 4],
    /// Worker allocator high-water mark, excluding mappings/stack/allocator overhead.
    pub peak_heap_bytes: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerReply {
    pub result: Result<ImageAnalysis>,
}

/// Source/file authorization captured from the catalog, without filesystem work.
#[derive(Clone)]
pub struct ImageSelection {
    source: Source,
    relative: String,
}
impl ImageSelection {
    pub fn capture(c: &Catalog, source_id: &str, relative: &str) -> Result<Self> {
        image_contracts::validate_relative(relative)
            .map_err(|_| AnalysisError::PermissionDenied)?;
        c.validate_source_entry(source_id, relative)
            .map_err(|_| AnalysisError::PermissionDenied)?;
        let source = c
            .source_header(source_id)
            .map_err(|_| AnalysisError::SourceUnavailable)?;
        if !source.available {
            return Err(AnalysisError::SourceUnavailable);
        }
        Ok(Self {
            source,
            relative: relative.into(),
        })
    }
    fn path(&self) -> Result<PathBuf> {
        contained(Path::new(&self.source.root), &self.relative)
            .map_err(|_| AnalysisError::SourceUnavailable)
    }
    /// Resolve containment now and open the exact member read-only for a worker's stdin.
    pub(crate) fn open(&self) -> Result<File> {
        let input = File::open(self.path()?).map_err(|_| AnalysisError::SourceUnavailable)?;
        let meta = input
            .metadata()
            .map_err(|_| AnalysisError::SourceUnavailable)?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_ENCODED {
            return Err(AnalysisError::LimitExceeded);
        }
        Ok(input)
    }
    /// Bounded streaming hash for discovery before metadata registration.
    pub fn hash(&self, cancel: &AtomicBool) -> Result<String> {
        hash_path(&self.path()?, cancel, Instant::now() + DEADLINE)
    }
}
/// A known image version must be registered before decoding (SS-043 owns discovery).
pub struct ImageAnalysisRequest {
    selection: ImageSelection,
    asset_id: String,
    expected: String,
}
impl ImageAnalysisRequest {
    pub fn capture(c: &Catalog, id: &str) -> Result<Self> {
        let asset = c
            .image(id, None)
            .map_err(|_| AnalysisError::PermissionDenied)?;
        let selection =
            ImageSelection::capture(c, &asset.header.source_id, &asset.header.relative_path)?;
        let expected = asset
            .header
            .content_hash
            .ok_or(AnalysisError::SourceChanged)?;
        Ok(Self {
            selection,
            asset_id: id.into(),
            expected,
        })
    }
    pub fn analyze(self, worker: &Path, cancel: Arc<AtomicBool>) -> Result<AnalyzedImage> {
        self.analyze_with_deadline(worker, cancel, DEADLINE)
    }
    /// A shorter caller deadline is useful for interactive work; never exceed the hard cap.
    pub fn analyze_with_deadline(
        self,
        worker: &Path,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<AnalyzedImage> {
        let timeout = timeout.min(DEADLINE);
        let end = Instant::now() + timeout;
        checkpoint(&cancel, end)?;
        if WORKER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return Err(AnalysisError::Busy);
        }
        struct Slot;
        impl Drop for Slot {
            fn drop(&mut self) {
                WORKER_ACTIVE.store(false, Ordering::Release);
            }
        }
        let _slot = Slot;
        let input = self.selection.open()?;
        let mut analysis = supervise(worker, input, cancel.clone(), end)?;
        if analysis.content_hash != self.expected {
            return Err(AnalysisError::SourceChanged);
        }
        if hash_path(&self.selection.path()?, &cancel, end)? != analysis.content_hash {
            return Err(AnalysisError::SourceChanged);
        }
        // The filename is a hint, never a decoder selection or instruction.
        let extension = Path::new(&self.selection.relative)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let matches = match analysis.profile.format {
            image_contracts::ImageFormat::Jpeg => matches!(extension.as_str(), "jpg" | "jpeg"),
            image_contracts::ImageFormat::Png => extension == "png",
            image_contracts::ImageFormat::Webp => extension == "webp",
        };
        if !matches {
            analysis.warnings.push(AnalysisWarning::ExtensionMismatch);
        }
        Ok(AnalyzedImage {
            request: self,
            analysis,
        })
    }
}
pub struct AnalyzedImage {
    request: ImageAnalysisRequest,
    analysis: ImageAnalysis,
}
impl AnalyzedImage {
    pub fn analysis(&self) -> &ImageAnalysis {
        &self.analysis
    }
    /// Rehash immediately before publication, outside the catalog lock. Source/version
    /// fencing is then enforced by publish(). File edits after this fence require refresh.
    pub fn reverify(self, cancel: &AtomicBool) -> Result<VerifiedImageAnalysis> {
        if self.request.selection.hash(cancel)? != self.analysis.content_hash {
            return Err(AnalysisError::SourceChanged);
        }
        Ok(VerifiedImageAnalysis {
            request: self.request,
            analysis: self.analysis,
        })
    }
}
pub struct VerifiedImageAnalysis {
    request: ImageAnalysisRequest,
    analysis: ImageAnalysis,
}
impl VerifiedImageAnalysis {
    pub fn analysis(&self) -> &ImageAnalysis {
        &self.analysis
    }
    pub fn publish(self, c: &mut Catalog) -> Result<crate::image_contracts::ImageAsset> {
        self.publish_when(c,|_|Ok(()))
    }
    pub(crate) fn publish_when(self,c:&mut Catalog,fence:impl FnOnce(&rusqlite::Connection)->crate::Result<()>)->Result<crate::image_contracts::ImageAsset>{
        let current = c
            .image(&self.request.asset_id, Some(&self.request.expected))
            .map_err(|_| AnalysisError::SourceChanged)?;
        if current.header.source_id != self.request.selection.source.id
            || current.header.relative_path != self.request.selection.relative
        {
            return Err(AnalysisError::SourceChanged);
        }
        c.publish_image_when(
            &self.request.selection.source,
            &self.request.asset_id,
            &self.request.expected,
            &self.analysis.profile,
            fence,
        )
        .map_err(|_| AnalysisError::SourceChanged)
    }
}
fn checkpoint(cancel: &AtomicBool, end: Instant) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(AnalysisError::Cancelled);
    }
    if Instant::now() >= end {
        return Err(AnalysisError::TimedOut);
    }
    Ok(())
}
pub(crate) fn hash_path(path: &Path, cancel: &AtomicBool, end: Instant) -> Result<String> {
    let mut file = File::open(path).map_err(|_| AnalysisError::SourceUnavailable)?;
    let size = file
        .metadata()
        .map_err(|_| AnalysisError::SourceUnavailable)?
        .len();
    if size == 0 || size > MAX_ENCODED {
        return Err(AnalysisError::LimitExceeded);
    }
    let mut h = blake3::Hasher::new();
    let mut buffer = [0; 65536];
    let mut read = 0;
    loop {
        checkpoint(cancel, end)?;
        let n = file
            .read(&mut buffer)
            .map_err(|_| AnalysisError::SourceUnavailable)?;
        if n == 0 {
            break;
        }
        read += n as u64;
        if read > MAX_ENCODED {
            return Err(AnalysisError::LimitExceeded);
        }
        h.update(&buffer[..n]);
    }
    if read != size {
        return Err(AnalysisError::SourceChanged);
    }
    Ok(h.finalize().to_hex().to_string())
}
fn supervise(
    worker: &Path,
    input: File,
    cancel: Arc<AtomicBool>,
    end: Instant,
) -> Result<ImageAnalysis> {
    let bytes = run_worker(worker, input, cancel, end, &[], MAX_REPLY)?;
    let reply: WorkerReply =
        serde_json::from_slice(&bytes).map_err(|_| AnalysisError::InvalidData)?;
    let analysis = reply.result?;
    if analysis.profile.analyzer_version != ANALYZER
        || analysis.profile.encoded_bytes > MAX_ENCODED
        || analysis.warnings.len() > 8
        || analysis.content_hash.len() != 64
        || analysis.display_rgba_hash.len() != 64
        || analysis.peak_heap_bytes > WORKER_HEAP as u64
    {
        return Err(AnalysisError::InvalidData);
    }
    Ok(analysis)
}
/// Spawn the trusted worker with fixed non-path arguments, supervise it and
/// return at most `cap` stdout bytes. Cancellation/deadline kill and reap it.
pub(crate) fn run_worker(
    worker: &Path,
    input: File,
    cancel: Arc<AtomicBool>,
    end: Instant,
    args: &[&str],
    cap: u64,
) -> Result<Vec<u8>> {
    if !worker.is_absolute() || !worker.is_file() {
        return Err(AnalysisError::WorkerUnavailable);
    }
    let mut command = Command::new(worker);
    crate::media::configure_background_process(&mut command);
    let mut child = command
        .args(args)
        .stdin(Stdio::from(input))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| AnalysisError::WorkerUnavailable)?;
    let mut stdout = child.stdout.take().ok_or(AnalysisError::WorkerFailed)?;
    let process = Arc::new(Mutex::new(child));
    let monitor = process.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let done = finished.clone();
    let aborted = Arc::new(AtomicU8::new(0));
    let reason = aborted.clone();
    let watchdog = thread::Builder::new()
        .name("image-watchdog".into())
        .spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let why = if cancel.load(Ordering::Relaxed) {
                    1
                } else if Instant::now() >= end {
                    2
                } else {
                    0
                };
                if why != 0 {
                    reason.store(why, Ordering::Relaxed);
                    if let Ok(mut p) = monitor.lock() {
                        let _ = p.kill();
                    }
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        })
        .map_err(|_| {
            if let Ok(mut p) = process.lock() {
                let _ = p.kill();
                let _ = p.wait();
            }
            AnalysisError::WorkerFailed
        })?;
    let mut bytes = Vec::new();
    let read = stdout.by_ref().take(cap + 1).read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > cap {
        if let Ok(mut p) = process.lock() {
            let _ = p.kill();
        }
    }
    let status = loop {
        match process
            .lock()
            .map_err(|_| AnalysisError::WorkerFailed)?
            .try_wait()
        {
            Ok(Some(s)) => break Ok(s),
            Err(_) => break Err(AnalysisError::WorkerFailed),
            Ok(None) => thread::sleep(Duration::from_millis(5)),
        }
    };
    finished.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    match aborted.load(Ordering::Relaxed) {
        1 => return Err(AnalysisError::Cancelled),
        2 => return Err(AnalysisError::TimedOut),
        _ => {}
    }
    let status = status?;
    if status.code() == Some(78) {
        return Err(AnalysisError::LimitExceeded);
    }
    if bytes.len() as u64 > cap {
        return Err(AnalysisError::LimitExceeded);
    }
    if !status.success() || read.is_err() {
        return Err(AnalysisError::WorkerFailed);
    }
    Ok(bytes)
}

/// Internal entry point for the dedicated heap-budgeted binary. Never call from
/// the desktop process; abort/OOM isolation relies on the separate executable.
#[doc(hidden)]
pub fn worker_main(peak: impl Fn() -> usize + std::panic::RefUnwindSafe) {
    let args: Vec<String> = std::env::args().skip(1).take(5).collect();
    if !args.is_empty() {
        return crate::image_preview_render::worker_main(&args, peak);
    }
    let result = std::panic::catch_unwind(|| {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(MAX_ENCODED + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| AnalysisError::InvalidData)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_ENCODED {
            return Err(AnalysisError::LimitExceeded);
        }
        let mut result = super::image_decode::decode(&bytes)?;
        result.peak_heap_bytes = peak() as u64;
        Ok(result)
    })
    .unwrap_or(Err(AnalysisError::WorkerFailed));
    let reply = serde_json::to_vec(&WorkerReply { result }).unwrap_or_default();
    if reply.len() as u64 <= MAX_REPLY {
        let _ = std::io::stdout().write_all(&reply);
    }
}
