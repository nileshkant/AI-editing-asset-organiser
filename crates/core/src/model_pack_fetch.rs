//! SS-051 transfers: a small `Fetcher` seam, the production HTTP fetcher, and
//! the resumable, verified single-file download used by the pack store.
//!
//! Rules enforced here for every request: HTTPS to an allowlisted host only,
//! redirects followed by hand and only to another allowlisted HTTPS host (at
//! most three hops), the connection address policy of the provider adapter
//! (no private or local addresses), bounded timeouts, and a hard cap on the
//! bytes accepted. A partial file is resumed only when the server proves the
//! same identity (matching `Content-Range` total and an unchanged ETag).
use crate::model_pack::{host_allowed, PackEntry, PackError, PackFile, Policy};
use std::{
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

pub const SLICE_BYTES: u64 = 8 << 20;
const CHUNK: usize = 64 * 1024;
const MAX_HOPS: usize = 3;
/// Slowest transfer rate tolerated for a whole-body stream before it is abandoned.
const MIN_RATE: u64 = 128 * 1024;

#[derive(Debug, Default, Clone)]
pub struct Headers {
    pub content_length: Option<u64>,
    /// `(first, last, total)` from `Content-Range: bytes first-last/total`.
    pub content_range: Option<(u64, u64, u64)>,
    pub etag: Option<String>,
    pub location: Option<String>,
}
pub struct Response {
    pub status: u16,
    pub headers: Headers,
    pub body: Box<dyn Read + Send>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub first: u64,
    pub last: u64,
}
pub trait Fetcher: Send + Sync {
    /// One request, no redirect handling. `timeout` bounds the whole exchange.
    fn get(&self, url: &url::Url, range: Option<Range>, if_range: Option<&str>, timeout: Duration) -> Result<Response, PackError>;
}

/// Production fetcher over the shared guarded ureq agent.
pub struct HttpFetcher {
    agent: ureq::Agent,
}
impl HttpFetcher {
    pub fn new(allow_local: bool) -> Self {
        Self { agent: crate::ai_provider_wire::download_agent(allow_local) }
    }
}
fn header(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}
fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let rest = value.strip_prefix("bytes ")?;
    let (span, total) = rest.split_once('/')?;
    let (first, last) = span.split_once('-')?;
    Some((first.parse().ok()?, last.parse().ok()?, total.parse().ok()?))
}
impl Fetcher for HttpFetcher {
    fn get(&self, url: &url::Url, range: Option<Range>, if_range: Option<&str>, timeout: Duration) -> Result<Response, PackError> {
        let mut request = self.agent.get(url.as_str()).header("Accept-Encoding", "identity");
        if let Some(r) = range {
            request = request.header("Range", format!("bytes={}-{}", r.first, r.last));
            if let Some(tag) = if_range {
                request = request.header("If-Range", tag);
            }
        }
        let response = request.config().timeout_global(Some(timeout)).build().call().map_err(|e| PackError::Network(match e {
            ureq::Error::Timeout(_) => "the server did not answer in time".to_owned(),
            ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::TlsRequired => "the secure connection failed".to_owned(),
            ureq::Error::Other(_) => "the address is not allowed".to_owned(),
            _ => "the server could not be reached".to_owned(),
        }))?;
        let headers = Headers {
            content_length: header(&response, "content-length").and_then(|v| v.parse().ok()),
            content_range: header(&response, "content-range").and_then(|v| parse_content_range(&v)),
            etag: header(&response, "etag"),
            location: header(&response, "location"),
        };
        let status = response.status().as_u16();
        Ok(Response { status, headers, body: Box::new(response.into_body().into_reader()) })
    }
}

// --------------------------------------------------------------- progress

/// Shared with the UI: bytes received so far across the whole pack, a cancel
/// flag, and the stage being run.
pub struct Progress {
    pub received: AtomicU64,
    pub total: u64,
    pub cancel: AtomicBool,
}
impl Progress {
    pub fn new(total: u64) -> Self {
        Self { received: AtomicU64::new(0), total, cancel: AtomicBool::new(false) }
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

// --------------------------------------------------------------- download

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct PartMeta {
    url: String,
    bytes: u64,
    blake3: String,
    etag: Option<String>,
}

fn io_error(error: std::io::Error) -> PackError {
    if error.kind() == std::io::ErrorKind::StorageFull {
        PackError::Storage("the disk is full".into())
    } else {
        PackError::Storage(error.to_string())
    }
}
fn net(message: impl Into<String>) -> PackError {
    PackError::Network(message.into())
}

pub struct Downloader<'a> {
    pub fetcher: &'a dyn Fetcher,
    pub entry: &'a PackEntry,
    pub policy: Policy,
    pub progress: &'a Progress,
}

impl Downloader<'_> {
    /// One request with manual, allowlisted redirects.
    fn request(&self, url: &url::Url, range: Option<Range>, if_range: Option<&str>, timeout: Duration) -> Result<Response, PackError> {
        let mut current = url.clone();
        for _ in 0..=MAX_HOPS {
            if !host_allowed(&current, self.entry, self.policy) {
                return Err(PackError::RedirectRefused(current.host_str().unwrap_or("unknown host").to_owned()));
            }
            let response = self.fetcher.get(&current, range, if_range, timeout)?;
            if (300..400).contains(&response.status) {
                let location = response.headers.location.as_deref().ok_or_else(|| net("a redirect had no destination"))?;
                current = current.join(location).map_err(|_| net("a redirect destination was not a URL"))?;
                continue;
            }
            return Ok(response);
        }
        Err(PackError::RedirectRefused("too many redirects".into()))
    }

    /// Downloads one file to `dest`, resuming a validated `.part`, and leaves
    /// only a complete, digest-verified file at `dest`.
    pub fn file(&self, file: &PackFile, dest: &Path, supports_ranges: &mut Option<bool>) -> Result<(), PackError> {
        let url = self.entry.file_url(file)?;
        let part = dest.with_file_name(format!("{}.part", dest.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
        let meta_path = dest.with_file_name(format!("{}.part.json", dest.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(io_error)?;
        }
        let mut restarted = false;
        let base = self.progress.received.load(Ordering::Relaxed);
        loop {
            match self.attempt(file, &url, &part, &meta_path, supports_ranges) {
                Err(Restart) if !restarted => {
                    restarted = true;
                    discard(&part, &meta_path);
                    // Progress counts this file's bytes again from zero.
                    self.progress.received.store(base, Ordering::Relaxed);
                }
                Err(Restart) => return Err(net("the server's file identity kept changing")),
                Ok(result) => {
                    return match result {
                        Ok(()) => {
                            std::fs::rename(&part, dest).map_err(io_error)?;
                            let _ = std::fs::remove_file(&meta_path);
                            Ok(())
                        }
                        Err(e) => {
                            // Wrong bytes are never kept; an interruption keeps the prefix.
                            if matches!(e, PackError::Integrity(_)) {
                                discard(&part, &meta_path);
                            }
                            Err(e)
                        }
                    };
                }
            }
        }
    }

    fn attempt(&self, file: &PackFile, url: &url::Url, part: &Path, meta_path: &Path, ranges: &mut Option<bool>) -> Result<Result<(), PackError>, Restart> {
        let outcome = (|| -> Result<(), Step> {
            let expected = file.bytes;
            let mut hasher = blake3::Hasher::new();
            let mut etag: Option<String> = None;
            let mut offset = 0u64;
            // A link planted where the partial file goes is removed, never followed.
            for path in [part, meta_path] {
                if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
                    let _ = std::fs::remove_file(path);
                }
            }
            // Resume only a part whose recorded identity is exactly this file.
            if let Ok(raw) = std::fs::read(meta_path) {
                let ok = serde_json::from_slice::<PartMeta>(&raw)
                    .map(|m| m.url == url.as_str() && m.bytes == expected && m.blake3 == file.blake3)
                    .unwrap_or(false);
                let len = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
                if ok && len > 0 && len < expected {
                    etag = serde_json::from_slice::<PartMeta>(&raw).ok().and_then(|m| m.etag);
                    hash_prefix(part, len, &mut hasher).map_err(Step::Fail)?;
                    offset = len;
                } else {
                    discard(part, meta_path);
                }
            } else {
                discard(part, meta_path);
            }
            self.progress.received.fetch_add(offset, Ordering::Relaxed);
            let write_meta = |etag: &Option<String>| {
                let meta = PartMeta { url: url.as_str().to_owned(), bytes: expected, blake3: file.blake3.clone(), etag: etag.clone() };
                std::fs::write(meta_path, serde_json::to_vec(&meta).unwrap_or_default())
            };
            let mut out = std::fs::OpenOptions::new().create(true).append(true).open(part).map_err(|e| Step::Fail(io_error(e)))?;
            if std::fs::metadata(part).map(|m| m.len()).unwrap_or(0) != offset {
                return Err(Step::Fail(PackError::Storage("the partial file changed while resuming".into())));
            }
            if offset == 0 {
                write_meta(&etag).map_err(|e| Step::Fail(io_error(e)))?;
            }
            while offset < expected {
                if self.progress.cancelled() {
                    return Err(Step::Fail(PackError::Cancelled));
                }
                // Probe range support once per install for files larger than a slice.
                if offset == 0 && ranges.is_none() && expected > SLICE_BYTES {
                    let response = self.request(url, Some(Range { first: 0, last: 0 }), None, Duration::from_secs(60)).map_err(Step::Fail)?;
                    *ranges = Some(response.status == 206 && response.headers.content_range.map(|(_, _, t)| t) == Some(expected));
                }
                if offset > 0 && *ranges == Some(false) {
                    // This server cannot resume: start again from the beginning.
                    return Err(Step::Restart);
                }
                let use_range = offset > 0 || (expected > SLICE_BYTES && *ranges != Some(false));
                let (range, timeout) = if use_range {
                    (Some(Range { first: offset, last: (offset + SLICE_BYTES - 1).min(expected - 1) }), Duration::from_secs(90))
                } else {
                    (None, Duration::from_secs(60 + expected / MIN_RATE))
                };
                let mut response = self.request(url, range, etag.as_deref(), timeout).map_err(Step::Fail)?;
                let remaining = match (response.status, range) {
                    (206, Some(r)) => {
                        let (first, last, total) = response.headers.content_range.ok_or_else(|| Step::Fail(net("a partial answer had no range")))?;
                        if first != r.first || last < first || last > r.last || total != expected {
                            return Err(Step::Fail(net("the server answered a different range than asked")));
                        }
                        if let (Some(old), Some(new)) = (&etag, &response.headers.etag) {
                            if old != new {
                                return Err(Step::Restart);
                            }
                        }
                        if etag.is_none() && response.headers.etag.is_some() {
                            etag = response.headers.etag.clone();
                            write_meta(&etag).map_err(|e| Step::Fail(io_error(e)))?;
                        }
                        last - first + 1
                    }
                    (200, _) => {
                        // Whole file; only trustworthy from the very start.
                        *ranges = Some(false);
                        if offset > 0 {
                            return Err(Step::Restart);
                        }
                        if response.headers.content_length.is_some_and(|l| l != expected) {
                            return Err(Step::Fail(PackError::Integrity(format!("{} is not the size the trusted list expects", file.path))));
                        }
                        if etag.is_none() && response.headers.etag.is_some() {
                            etag = response.headers.etag.clone();
                            write_meta(&etag).map_err(|e| Step::Fail(io_error(e)))?;
                        }
                        expected
                    }
                    (416, _) if offset > 0 => return Err(Step::Restart),
                    (status, _) => return Err(Step::Fail(net(format!("the server answered {status}")))),
                };
                let mut taken = 0u64;
                let mut buffer = vec![0u8; CHUNK];
                loop {
                    if self.progress.cancelled() {
                        return Err(Step::Fail(PackError::Cancelled));
                    }
                    let want = (remaining + 1 - taken).min(CHUNK as u64) as usize; // one extra byte detects overlong bodies
                    let n = match response.body.read(&mut buffer[..want]) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) => return Err(Step::Fail(net(format!("the connection was interrupted ({})", e.kind())))),
                    };
                    taken += n as u64;
                    if taken > remaining || offset + n as u64 > expected {
                        return Err(Step::Fail(PackError::Integrity(format!("{} is larger than the trusted list says", file.path))));
                    }
                    out.write_all(&buffer[..n]).map_err(|e| Step::Fail(io_error(e)))?;
                    hasher.update(&buffer[..n]);
                    offset += n as u64;
                    self.progress.received.fetch_add(n as u64, Ordering::Relaxed);
                }
                out.flush().map_err(|e| Step::Fail(io_error(e)))?;
                if taken != remaining {
                    return Err(Step::Fail(net("the download ended early")));
                }
            }
            if hasher.finalize().to_hex().as_str() != file.blake3 {
                return Err(Step::Fail(PackError::Integrity(format!("{} does not match its trusted digest", file.path))));
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => Ok(Ok(())),
            Err(Step::Fail(e)) => Ok(Err(e)),
            Err(Step::Restart) => Err(Restart),
        }
    }
}

struct Restart;
enum Step {
    Fail(PackError),
    Restart,
}

fn discard(part: &Path, meta: &Path) {
    let _ = std::fs::remove_file(part);
    let _ = std::fs::remove_file(meta);
}
fn hash_prefix(path: &Path, len: u64, hasher: &mut blake3::Hasher) -> Result<(), PackError> {
    let mut file = std::fs::File::open(path).map_err(io_error)?;
    let mut remaining = len;
    let mut buffer = vec![0u8; CHUNK];
    while remaining > 0 {
        let take = remaining.min(CHUNK as u64) as usize;
        file.read_exact(&mut buffer[..take]).map_err(io_error)?;
        hasher.update(&buffer[..take]);
        remaining -= take as u64;
    }
    Ok(())
}

/// Streams a file through BLAKE3 and checks its length, never reading past
/// `expected + 1` bytes. Used to re-verify installed files.
pub fn verify_file(path: &Path, file: &PackFile, progress: Option<&Progress>) -> Result<(), PackError> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| PackError::Integrity(format!("{} is missing", file.path)))?;
    if !meta.is_file() {
        return Err(PackError::Integrity(format!("{} is not a regular file", file.path)));
    }
    if meta.len() != file.bytes {
        return Err(PackError::Integrity(format!("{} has the wrong size", file.path)));
    }
    let mut reader = std::fs::File::open(path).map_err(io_error)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if progress.is_some_and(Progress::cancelled) {
            return Err(PackError::Cancelled);
        }
        let n = reader.read(&mut buffer).map_err(io_error)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    if hasher.finalize().to_hex().as_str() == file.blake3 {
        Ok(())
    } else {
        Err(PackError::Integrity(format!("{} does not match its trusted digest", file.path)))
    }
}
