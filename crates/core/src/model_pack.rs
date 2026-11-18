//! SS-051 optional local model packs: the allowlisted manifest, the host
//! compatibility rules and the lifecycle contract types. Nothing here touches
//! the network or the catalog; `model_pack_store` owns files and state and
//! `model_pack_fetch` owns transfers. The app is complete without any pack.
//!
//! A pack is a flat list of files, each with an exact size and BLAKE3 digest,
//! fetched from an HTTPS origin the manifest names. There is deliberately no
//! archive format: nothing is extracted, so there are no archive paths or
//! links to traverse, and nothing downloaded is ever executed.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const MANIFEST_VERSION: u32 = 1;
/// The trusted allowlist compiled into the app. No pack has passed the SS-050
/// evaluation yet, so it is empty; adding an entry is a reviewed code change.
pub const BUNDLED_MANIFEST: &str = include_str!("model_pack_manifest.json");
pub const MAX_PACKS: usize = 64;
pub const MAX_FILES: usize = 64;
pub const MAX_PACK_BYTES: u64 = 16 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ImageDescription,
    ImageEmbedding,
}
impl Capability {
    pub fn produces_vectors(self) -> bool {
        self == Capability::ImageEmbedding
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackFile {
    /// Flat relative path made of `[A-Za-z0-9._-]` components.
    pub path: String,
    pub bytes: u64,
    /// Lowercase hex BLAKE3-256 of the complete file.
    pub blake3: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    /// HTTPS directory URL that ends with `/`; file paths are joined to it.
    pub base_url: String,
    /// Hosts a download may be served from, including after a redirect.
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct License {
    pub spdx: String,
    pub notice: String,
    pub notice_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    /// Empty means any. Values follow `std::env::consts::{OS, ARCH}`.
    #[serde(default)]
    pub os: Vec<String>,
    #[serde(default)]
    pub arch: Vec<String>,
    #[serde(default)]
    pub cpu_features: Vec<String>,
    pub min_ram_bytes: u64,
    #[serde(default)]
    pub needs_accelerator: bool,
    /// Inference runtime the app must bundle, for example `onnxruntime`.
    pub runtime: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackEntry {
    pub id: String,
    /// Dotted numbers such as `1` or `2.1.0`.
    pub version: String,
    pub display_name: String,
    pub capability: Capability,
    /// Pinned vector-space identity for embedding packs. Packs that share it
    /// produce compatible vectors, so switching between them needs no reindex.
    pub embedding_space: Option<String>,
    pub origin: Origin,
    pub license: License,
    pub requirements: Requirements,
    pub files: Vec<PackFile>,
}
impl PackEntry {
    pub fn key(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }
    pub fn download_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
    pub fn space_id(&self) -> String {
        self.embedding_space.clone().unwrap_or_else(|| format!("pack:{}", self.key()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub manifest_version: u32,
    pub packs: Vec<PackEntry>,
}

/// Where plain HTTP to the local machine is acceptable. Production is strict;
/// only tests open this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub allow_loopback_http: bool,
}
impl Policy {
    pub const STRICT: Policy = Policy { allow_loopback_http: false };
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PackError {
    #[error("The model pack manifest is not valid: {0}")]
    Manifest(String),
    #[error("That model pack is not in the trusted list")]
    UnknownPack,
    #[error("This computer cannot use that pack: {0}")]
    Incompatible(Incompat),
    #[error("Another pack is already being installed")]
    Busy,
    #[error("That pack is not installed")]
    NotInstalled,
    #[error("That pack is not active")]
    NotActive,
    #[error("That pack is being replaced or removed")]
    Draining,
    #[error("The pack is still in use by {0} running task(s); try again when they finish")]
    InUse(usize),
    #[error("Cancelled")]
    Cancelled,
    #[error("Not enough free disk space: {need} bytes are needed and {free} are free")]
    DiskFull { need: u64, free: u64 },
    #[error("The download failed: {0}")]
    Network(String),
    #[error("The download was redirected somewhere that is not trusted: {0}")]
    RedirectRefused(String),
    #[error("The downloaded files do not match the trusted list: {0}")]
    Integrity(String),
    #[error("Local pack storage failed: {0}")]
    Storage(String),
}
impl From<std::io::Error> for PackError {
    fn from(error: std::io::Error) -> Self {
        PackError::Storage(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncompatCode {
    UnsupportedOs,
    UnsupportedArch,
    MissingCpuFeature,
    LowMemory,
    NoAccelerator,
    RuntimeUnavailable,
    RetiredFromManifest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{detail}")]
pub struct Incompat {
    pub code: IncompatCode,
    pub detail: String,
}

// ------------------------------------------------------------- validation

fn bad(message: impl Into<String>) -> PackError {
    PackError::Manifest(message.into())
}
fn is_slug(value: &str) -> bool {
    (1..=48).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.starts_with('-')
}
/// Dotted decimal numbers, one to four parts, each at most nine digits.
pub fn parse_version(value: &str) -> Option<Vec<u64>> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    parts
        .iter()
        .map(|p| (!p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse().ok()).flatten())
        .collect()
}
pub fn version_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}
/// A safe, flat-or-nested relative path: no empty, absolute, drive, backslash,
/// dot-prefixed or `..` components, so it can never leave the pack folder.
pub fn safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 128 || path.starts_with('/') {
        return false;
    }
    path.split('/').all(|part| {
        !part.is_empty()
            && part.len() <= 64
            && !part.starts_with('.')
            && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    })
}
fn origin_host(url: &url::Url, policy: Policy) -> Result<String, PackError> {
    let host = url.host_str().ok_or_else(|| bad("an origin URL has no host"))?.to_ascii_lowercase();
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(bad("an origin URL must not carry credentials, a query or a fragment"));
    }
    let loopback = matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]");
    match url.scheme() {
        "https" if url.port().is_none() || url.port() == Some(443) || (policy.allow_loopback_http && loopback) => {}
        "http" if policy.allow_loopback_http && loopback => {}
        _ => return Err(bad("an origin URL must be HTTPS on the standard port")),
    }
    Ok(host)
}
/// Host check used for every request and redirect target.
pub fn host_allowed(url: &url::Url, entry: &PackEntry, policy: Policy) -> bool {
    origin_host(url, policy).map(|h| entry.origin.hosts.iter().any(|a| a.eq_ignore_ascii_case(&h))).unwrap_or(false)
}

impl Manifest {
    pub fn parse(json: &str, policy: Policy) -> Result<Manifest, PackError> {
        if json.len() > 1 << 20 {
            return Err(bad("larger than 1 MiB"));
        }
        let manifest: Manifest = serde_json::from_str(json).map_err(|e| bad(e.to_string()))?;
        manifest.validate(policy)?;
        Ok(manifest)
    }
    pub fn validate(&self, policy: Policy) -> Result<(), PackError> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(bad(format!("version {} is not supported", self.manifest_version)));
        }
        if self.packs.len() > MAX_PACKS {
            return Err(bad("too many packs"));
        }
        let mut seen = BTreeSet::new();
        for pack in &self.packs {
            pack.validate(policy)?;
            if !seen.insert(pack.key()) {
                return Err(bad(format!("{} is listed twice", pack.key())));
            }
        }
        Ok(())
    }
    pub fn find(&self, id: &str, version: &str) -> Option<&PackEntry> {
        self.packs.iter().find(|p| p.id == id && p.version == version)
    }
}

impl PackEntry {
    pub fn validate(&self, policy: Policy) -> Result<(), PackError> {
        let name = self.key();
        if !is_slug(&self.id) {
            return Err(bad(format!("{name}: the id must be lowercase letters, digits and hyphens")));
        }
        if parse_version(&self.version).is_none() {
            return Err(bad(format!("{name}: the version must be dotted numbers")));
        }
        let text = |s: &str, max: usize| !s.trim().is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control);
        if !text(&self.display_name, 80) || !text(&self.license.spdx, 64) || !text(&self.requirements.runtime, 64) {
            return Err(bad(format!("{name}: a name, license identifier or runtime is missing or too long")));
        }
        if self.license.notice.trim().is_empty() || self.license.notice.len() > 8192 {
            return Err(bad(format!("{name}: the license notice must be present and at most 8 KiB")));
        }
        if let Some(space) = &self.embedding_space {
            if !text(space, 120) || self.capability != Capability::ImageEmbedding {
                return Err(bad(format!("{name}: only embedding packs carry an embedding space")));
            }
        } else if self.capability.produces_vectors() {
            return Err(bad(format!("{name}: an embedding pack must pin its embedding space")));
        }
        if self.files.is_empty() || self.files.len() > MAX_FILES {
            return Err(bad(format!("{name}: a pack needs between 1 and {MAX_FILES} files")));
        }
        let mut total = 0u64;
        let mut paths = BTreeSet::new();
        for file in &self.files {
            if !safe_relative_path(&file.path) {
                return Err(bad(format!("{name}: unsafe file path {:?}", file.path)));
            }
            if !paths.insert(file.path.to_ascii_lowercase()) {
                return Err(bad(format!("{name}: {} is listed twice", file.path)));
            }
            if file.bytes == 0 || file.blake3.len() != 64 || !file.blake3.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return Err(bad(format!("{name}: {} needs a size and a lowercase BLAKE3 digest", file.path)));
            }
            total = total.checked_add(file.bytes).ok_or_else(|| bad("size overflow"))?;
        }
        if total > MAX_PACK_BYTES {
            return Err(bad(format!("{name}: larger than the {MAX_PACK_BYTES} byte limit")));
        }
        let base = url::Url::parse(&self.origin.base_url).map_err(|_| bad(format!("{name}: the origin is not a URL")))?;
        let host = origin_host(&base, policy)?;
        if !base.path().ends_with('/') {
            return Err(bad(format!("{name}: the origin URL must end with /")));
        }
        if self.origin.hosts.is_empty() || self.origin.hosts.len() > 8 || !self.origin.hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
            return Err(bad(format!("{name}: the allowed hosts must include the origin host")));
        }
        if self.origin.hosts.iter().any(|h| h.is_empty() || h.contains(['/', ':', '@', ' '])) {
            return Err(bad(format!("{name}: an allowed host is not a plain host name")));
        }
        if let Some(url) = &self.license.notice_url {
            if !url.starts_with("https://") {
                return Err(bad(format!("{name}: the notice link must be HTTPS")));
            }
        }
        Ok(())
    }
    /// Full URL of one file under the origin.
    pub fn file_url(&self, file: &PackFile) -> Result<url::Url, PackError> {
        let base = url::Url::parse(&self.origin.base_url).map_err(|_| bad("origin is not a URL"))?;
        base.join(&file.path).map_err(|_| bad("file path does not form a URL"))
    }
}

// ------------------------------------------------------------ host rules

/// What the host machine offers. A trait so tests can describe any machine.
pub trait System: Send + Sync {
    fn os(&self) -> String;
    fn arch(&self) -> String;
    /// `None` when the platform does not report it; that is shown, not guessed.
    fn ram_bytes(&self) -> Option<u64>;
    fn cpu_features(&self) -> BTreeSet<String>;
    fn has_accelerator(&self) -> bool;
    /// Inference runtimes bundled with this build.
    fn runtimes(&self) -> BTreeSet<String>;
    /// Free bytes available to this user where `path` (or its nearest parent) lives.
    fn free_disk(&self, path: &Path) -> Option<u64>;
}

pub struct OsSystem;
impl System for OsSystem {
    fn os(&self) -> String {
        std::env::consts::OS.into()
    }
    fn arch(&self) -> String {
        std::env::consts::ARCH.into()
    }
    fn ram_bytes(&self) -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            let text = std::fs::read_to_string("/proc/meminfo").ok()?;
            let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
            let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
            return kib.checked_mul(1024);
        }
        #[cfg(target_os = "macos")]
        {
            let mut value: u64 = 0;
            let mut size = std::mem::size_of::<u64>();
            let name = std::ffi::CString::new("hw.memsize").ok()?;
            let status = unsafe { libc::sysctlbyname(name.as_ptr(), (&mut value as *mut u64).cast(), &mut size, std::ptr::null_mut(), 0) };
            return (status == 0 && value > 0).then_some(value);
        }
        #[allow(unreachable_code)]
        None
    }
    fn cpu_features(&self) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("sse4.2") { found.insert("sse4.2".to_owned()); }
            if is_x86_feature_detected!("avx") { found.insert("avx".to_owned()); }
            if is_x86_feature_detected!("avx2") { found.insert("avx2".to_owned()); }
            if is_x86_feature_detected!("fma") { found.insert("fma".to_owned()); }
            if is_x86_feature_detected!("avx512f") { found.insert("avx512f".to_owned()); }
        }
        #[cfg(target_arch = "aarch64")]
        {
            if std::arch::is_aarch64_feature_detected!("neon") { found.insert("neon".to_owned()); }
            if std::arch::is_aarch64_feature_detected!("dotprod") { found.insert("dotprod".to_owned()); }
        }
        found
    }
    fn has_accelerator(&self) -> bool {
        false
    }
    fn runtimes(&self) -> BTreeSet<String> {
        // No inference runtime ships yet; SS-052 adds the first one.
        BTreeSet::new()
    }
    fn free_disk(&self, path: &Path) -> Option<u64> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let mut probe = Some(path);
            while let Some(p) = probe {
                if p.exists() {
                    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
                    let mut info = unsafe { std::mem::zeroed::<libc::statvfs>() };
                    if unsafe { libc::statvfs(c.as_ptr(), &mut info) } != 0 {
                        return None;
                    }
                    return (info.f_bavail as u64).checked_mul(info.f_frsize as u64);
                }
                probe = p.parent();
            }
            None
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            None
        }
    }
}

/// Result of a successful compatibility check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Compatible {
    /// False when the platform did not report memory, so the RAM guidance
    /// could not be checked.
    pub memory_checked: bool,
}

pub fn check_compatibility(requirements: &Requirements, system: &dyn System) -> Result<Compatible, Incompat> {
    let fail = |code, detail: String| Err(Incompat { code, detail });
    let (os, arch) = (system.os(), system.arch());
    if !requirements.os.is_empty() && !requirements.os.iter().any(|o| *o == os) {
        return fail(IncompatCode::UnsupportedOs, format!("It supports {} but this is {os}", requirements.os.join(", ")));
    }
    if !requirements.arch.is_empty() && !requirements.arch.iter().any(|a| *a == arch) {
        return fail(IncompatCode::UnsupportedArch, format!("It supports {} processors but this is {arch}", requirements.arch.join(", ")));
    }
    let features = system.cpu_features();
    if let Some(missing) = requirements.cpu_features.iter().find(|f| !features.contains(*f)) {
        return fail(IncompatCode::MissingCpuFeature, format!("This processor does not report the {missing} instructions it needs"));
    }
    if requirements.needs_accelerator && !system.has_accelerator() {
        return fail(IncompatCode::NoAccelerator, "It needs a graphics or neural accelerator that was not found".into());
    }
    if !system.runtimes().contains(&requirements.runtime) {
        return fail(IncompatCode::RuntimeUnavailable, format!("This build of CreativeShelf does not include the {} runtime it needs", requirements.runtime));
    }
    let ram = system.ram_bytes();
    if let Some(have) = ram {
        if have < requirements.min_ram_bytes {
            return fail(IncompatCode::LowMemory, format!("It needs about {} MiB of memory and this computer has {} MiB", requirements.min_ram_bytes >> 20, have >> 20));
        }
    }
    Ok(Compatible { memory_checked: ram.is_some() })
}
