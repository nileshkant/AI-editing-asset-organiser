//! SS-049: shared AI provider profiles, endpoint policy and secret handling.
//!
//! Profiles (kind, endpoint, model, local opt-in, tested capabilities) are
//! machine-local settings in `app_settings`. API keys never enter SQLite,
//! portable catalogs, diagnostics or the renderer: they live in the OS keychain
//! or, for session-only use, in process memory. Nothing here contacts a
//! provider except an explicit [`Providers::test`]; configuring a key is not
//! consent to analyze images.
use crate::{ai_provider_wire as wire, catalog::Catalog};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_PROFILES: usize = 16;
pub const KEYCHAIN_SERVICE: &str = "app.creativeshelf.providers";
const SETTINGS_KEY: &str = "ai_providers";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Openai,
    Anthropic,
    Gemini,
    /// A server speaking the OpenAI chat-completions wire format (for example a
    /// local model server). It is not the OpenAI service.
    OpenaiCompatible,
}
impl ProviderKind {
    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            Self::Openai => Some("https://api.openai.com/v1"),
            Self::Anthropic => Some("https://api.anthropic.com/v1"),
            Self::Gemini => Some("https://generativelanguage.googleapis.com/v1beta"),
            Self::OpenaiCompatible => None,
        }
    }
}

/// Public, stable failure codes. Messages never contain keys or response bodies
/// beyond a short sanitized provider message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("Invalid provider settings: {0}")]
    InvalidSettings(String),
    #[error("Remote endpoints must use HTTPS")]
    InsecureEndpoint,
    #[error("This is a local or private network endpoint; turn on local endpoints for this provider to use it")]
    LocalNotPermitted,
    #[error("This endpoint address is not allowed (link-local, multicast or unspecified)")]
    AddressNotAllowed,
    #[error("No API key is stored for this provider; enter it again")]
    KeyMissing,
    #[error("The system keychain is unavailable: {0}")]
    KeychainUnavailable(String),
    #[error("Access to the system keychain was denied")]
    KeychainDenied,
    #[error("Provider settings changed; refresh and try again")]
    Conflict,
    #[error("Provider not found")]
    NotFound,
    #[error("The provider rejected the API key")]
    Authentication,
    #[error("The model was not found or this key cannot use it")]
    ModelNotFound,
    #[error("This model does not accept image input")]
    ImageUnsupported,
    #[error("This model or endpoint does not support the structured output CreativeShelf requires")]
    SchemaIncompatible,
    #[error("This provider or model does not offer that capability")]
    Unsupported,
    #[error("The request timed out")]
    Timeout,
    #[error("The provider reports the account quota or billing limit is exhausted")]
    QuotaExceeded,
    #[error("The provider is rate limiting requests; try again later")]
    RateLimited,
    #[error("The provider is unavailable (HTTP {0}); try again later")]
    ProviderUnavailable(u16),
    #[error("The endpoint answered with a redirect; redirects are never followed")]
    Redirected,
    #[error("Secure connection failed (certificate or TLS error)")]
    Tls,
    #[error("Could not connect to the endpoint")]
    Unreachable,
    #[error("The provider response was larger than allowed")]
    ResponseTooLarge,
    #[error("The request is larger than allowed")]
    RequestTooLarge,
    #[error("The provider response could not be understood")]
    MalformedResponse,
    #[error("The provider rejected the request (HTTP {status}): {message}")]
    Rejected { status: u16, message: String },
}
impl ProviderError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidSettings(_) => "INVALID_SETTINGS",
            Self::InsecureEndpoint => "INSECURE_ENDPOINT",
            Self::LocalNotPermitted => "LOCAL_NOT_PERMITTED",
            Self::AddressNotAllowed => "ADDRESS_NOT_ALLOWED",
            Self::KeyMissing => "KEY_MISSING",
            Self::KeychainUnavailable(_) => "KEYCHAIN_UNAVAILABLE",
            Self::KeychainDenied => "KEYCHAIN_DENIED",
            Self::Conflict => "CONFLICT",
            Self::NotFound => "NOT_FOUND",
            Self::Authentication => "AUTHENTICATION",
            Self::ModelNotFound => "MODEL_NOT_FOUND",
            Self::ImageUnsupported => "IMAGE_UNSUPPORTED",
            Self::SchemaIncompatible => "SCHEMA_INCOMPATIBLE",
            Self::Unsupported => "UNSUPPORTED",
            Self::Timeout => "TIMEOUT",
            Self::QuotaExceeded => "QUOTA_EXCEEDED",
            Self::RateLimited => "RATE_LIMITED",
            Self::ProviderUnavailable(_) => "PROVIDER_UNAVAILABLE",
            Self::Redirected => "REDIRECTED",
            Self::Tls => "TLS",
            Self::Unreachable => "UNREACHABLE",
            Self::ResponseTooLarge => "RESPONSE_TOO_LARGE",
            Self::RequestTooLarge => "REQUEST_TOO_LARGE",
            Self::MalformedResponse => "MALFORMED_RESPONSE",
            Self::Rejected { .. } => "REJECTED",
        }
    }
    pub fn view(&self) -> ErrorView {
        ErrorView { code: self.code(), message: self.to_string() }
    }
}
fn settings(message: &str) -> ProviderError {
    ProviderError::InvalidSettings(message.into())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorView {
    pub code: &'static str,
    pub message: String,
}

// ---------------------------------------------------------------- endpoints

/// Where an endpoint lives. Loopback and private endpoints need explicit opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    Remote,
    Private,
    Loopback,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub url: url::Url,
    pub locality: Locality,
}
impl Endpoint {
    /// `scheme://host:port`; a stored key is bound to it.
    pub fn origin(&self) -> String {
        self.url.origin().ascii_serialization()
    }
}

/// Locality of a concrete address, or `None` for addresses never contacted
/// (link-local such as cloud metadata services, multicast, unspecified,
/// broadcast and reserved ranges).
pub fn address_locality(ip: IpAddr) -> Option<Locality> {
    match ip {
        IpAddr::V4(v4) => v4_locality(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4_locality(v4);
            }
            let first = v6.segments()[0];
            if v6.is_loopback() {
                Some(Locality::Loopback)
            } else if v6.is_unspecified() || v6.is_multicast() || first & 0xffc0 == 0xfe80 {
                None
            } else if first & 0xfe00 == 0xfc00 {
                Some(Locality::Private)
            } else {
                Some(Locality::Remote)
            }
        }
    }
}
fn v4_locality(ip: Ipv4Addr) -> Option<Locality> {
    let [a, b, ..] = ip.octets();
    if ip.is_loopback() {
        Some(Locality::Loopback)
    } else if ip.is_unspecified() || ip.is_link_local() || ip.is_multicast() || ip.is_broadcast() || a == 0 || a >= 240 {
        None
    } else if ip.is_private() || (a == 100 && (64..128).contains(&b)) {
        Some(Locality::Private)
    } else {
        Some(Locality::Remote)
    }
}
fn host_locality(host: &url::Host<&str>) -> Option<Locality> {
    match host {
        url::Host::Ipv4(ip) => address_locality(IpAddr::V4(*ip)),
        url::Host::Ipv6(ip) => address_locality(IpAddr::V6(*ip)),
        url::Host::Domain(name) => {
            let name = name.trim_end_matches('.');
            Some(if name == "localhost" || name.ends_with(".localhost") {
                Locality::Loopback
            } else if !name.contains('.')
                || [".local", ".lan", ".internal", ".home.arpa"].iter().any(|s| name.ends_with(s))
            {
                Locality::Private
            } else {
                // Resolved addresses are checked again when connecting.
                Locality::Remote
            })
        }
    }
}

/// Validates a user-entered base URL. Remote endpoints require HTTPS; loopback
/// and private endpoints require `allow_local` and may use plain HTTP. Embedded
/// credentials, queries and fragments are rejected so keys never travel in URLs.
pub fn validate_endpoint(raw: &str, allow_local: bool) -> Result<Endpoint, ProviderError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 2048 || raw.chars().any(char::is_control) {
        return Err(settings("Enter an endpoint URL up to 2048 characters"));
    }
    let mut url = url::Url::parse(raw).map_err(|_| settings("Endpoint is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(settings("Endpoint must start with https://"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(settings("Endpoint must not contain a user name or password; enter the key separately"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(settings("Endpoint must not contain a query or fragment"));
    }
    let host = url.host().ok_or_else(|| settings("Endpoint needs a host"))?;
    let locality = host_locality(&host).ok_or(ProviderError::AddressNotAllowed)?;
    match locality {
        Locality::Remote if url.scheme() != "https" => return Err(ProviderError::InsecureEndpoint),
        Locality::Private | Locality::Loopback if !allow_local => return Err(ProviderError::LocalNotPermitted),
        _ => {}
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(Endpoint { url, locality })
}

// ------------------------------------------------------------------ secrets

/// An API key. It is never serialized, and `Debug` never prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);
impl Secret {
    pub fn new(raw: &str) -> Result<Self, ProviderError> {
        let raw = raw.trim();
        if !(8..=512).contains(&raw.len()) || !raw.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(settings("API keys are 8 to 512 visible ASCII characters with no spaces"));
        }
        Ok(Self(raw.to_owned()))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        // Zero bytes are valid UTF-8, so the String stays well-formed.
        unsafe { self.0.as_mut_vec() }.fill(0);
    }
}

/// Keychain-like storage keyed by profile ID. `get` of an absent entry is `None`.
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<Secret>, ProviderError>;
    fn set(&self, account: &str, secret: &Secret) -> Result<(), ProviderError>;
    fn delete(&self, account: &str) -> Result<(), ProviderError>;
}
/// Session-only keys: forgotten when the app quits.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, Secret>>);
impl SecretStore for MemorySecrets {
    fn get(&self, account: &str) -> Result<Option<Secret>, ProviderError> {
        Ok(self.0.lock().map_err(|_| ProviderError::KeychainUnavailable("session store".into()))?.get(account).cloned())
    }
    fn set(&self, account: &str, secret: &Secret) -> Result<(), ProviderError> {
        self.0.lock().map_err(|_| ProviderError::KeychainUnavailable("session store".into()))?.insert(account.into(), secret.clone());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), ProviderError> {
        self.0.lock().map_err(|_| ProviderError::KeychainUnavailable("session store".into()))?.remove(account);
        Ok(())
    }
}

// ----------------------------------------------------------------- profiles

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStorage {
    None,
    Keychain,
    Session,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Untested,
    Supported,
    Unsupported,
    /// The request was accepted but the answer did not prove the capability.
    Inconclusive,
}
/// Capabilities are only ever set by an explicit connection test of this exact
/// configuration (`tested_revision`). Text support never implies image support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub image_input: CapabilityState,
    pub structured_output: CapabilityState,
    pub embeddings: CapabilityState,
    pub tested_revision: Option<u64>,
    pub tested_at: Option<i64>,
    pub model_version: Option<String>,
}
impl Default for Capabilities {
    fn default() -> Self {
        Self {
            image_input: CapabilityState::Untested,
            structured_output: CapabilityState::Untested,
            embeddings: CapabilityState::Untested,
            tested_revision: None,
            tested_at: None,
            model_version: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    pub model: String,
    pub allow_local: bool,
    pub key_storage: KeyStorage,
    /// Origin the stored key was entered for. A key never follows an endpoint
    /// to a different origin.
    pub key_origin: Option<String>,
    pub revision: u64,
    pub capabilities: Capabilities,
}
impl ProviderProfile {
    pub fn endpoint(&self) -> Result<Endpoint, ProviderError> {
        validate_endpoint(&self.base_url, self.allow_local)
    }
}
/// What the renderer may set. Keys are set separately and never read back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDraft {
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: Option<String>,
    pub model: String,
    pub allow_local: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderView {
    #[serde(flatten)]
    pub profile: ProviderProfile,
    pub locality: Option<Locality>,
    /// For keychain storage this reflects the recorded setting; the keychain
    /// itself is read only by a connection test, so listing never prompts.
    pub has_key: bool,
}
fn normalize_model(kind: ProviderKind, raw: &str) -> Result<String, ProviderError> {
    let mut model = raw.trim();
    if kind == ProviderKind::Gemini {
        model = model.strip_prefix("models/").unwrap_or(model);
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || "._:@-".contains(c) || (c == '/' && kind == ProviderKind::OpenaiCompatible);
    if model.is_empty() || model.len() > 200 || !model.chars().all(allowed) || model.starts_with(['.', '-', '/']) {
        return Err(settings("Model IDs are up to 200 letters, digits and . _ : @ - characters"));
    }
    Ok(model.to_owned())
}
fn validate_draft(draft: &ProviderDraft) -> Result<(String, String, String), ProviderError> {
    let label = draft.label.trim();
    if label.is_empty() || label.chars().count() > 80 || label.chars().any(char::is_control) {
        return Err(settings("Name the provider in 1 to 80 characters"));
    }
    let base = match (&draft.base_url, draft.kind.default_base_url()) {
        (Some(url), _) if !url.trim().is_empty() => url.trim().to_owned(),
        (_, Some(default)) => default.to_owned(),
        _ => return Err(settings("Enter the endpoint URL of the OpenAI-compatible server")),
    };
    let endpoint = validate_endpoint(&base, draft.allow_local)?;
    Ok((label.to_owned(), endpoint.url.to_string().trim_end_matches('/').to_owned(), normalize_model(draft.kind, &draft.model)?))
}
fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSettings {
    version: u32,
    profiles: Vec<ProviderProfile>,
}
impl Catalog {
    /// Provider profiles (machine-local; not in portable or folder catalogs).
    pub fn ai_provider_profiles(&self) -> crate::Result<Vec<ProviderProfile>> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT value FROM app_settings WHERE key=?1", [SETTINGS_KEY], |r| r.get(0))
            .optional()?;
        let Some(raw) = raw else { return Ok(vec![]) };
        let stored: StoredSettings = serde_json::from_str(&raw)?;
        if stored.version != 1 {
            return Err(crate::invalid("Provider settings were written by a newer CreativeShelf"));
        }
        if stored.profiles.len() > MAX_PROFILES {
            return Err(crate::invalid("Provider settings are malformed"));
        }
        Ok(stored.profiles)
    }
    fn put_ai_provider_profiles(&self, profiles: &[ProviderProfile]) -> crate::Result<()> {
        let value = serde_json::to_string(&StoredSettings { version: 1, profiles: profiles.to_vec() })?;
        self.db.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![SETTINGS_KEY, value],
        )?;
        Ok(())
    }
}

// ------------------------------------------------------------------ service

/// What a connection test checks, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    ModelAccess,
    ImageInput,
    StructuredOutput,
    Embeddings,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TestStep {
    pub check: Check,
    pub passed: bool,
    pub error: Option<ErrorView>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TestReport {
    pub provider: ProviderView,
    pub steps: Vec<TestStep>,
    pub usage: wire::Usage,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreChoice {
    Keychain,
    Session,
}

/// Provider configuration service. Mutations are serialized and revision
/// checked; a key is written to the chosen store before the profile records it.
pub struct Providers {
    db: Arc<Mutex<Catalog>>,
    keychain: Arc<dyn SecretStore>,
    session: MemorySecrets,
    ops: Mutex<()>,
    connect: Arc<dyn Fn(&ProviderProfile, Option<Secret>) -> Result<Box<dyn wire::ProviderAdapter>, ProviderError> + Send + Sync>,
}
fn db_error(e: impl fmt::Display) -> ProviderError {
    ProviderError::InvalidSettings(format!("settings storage failed: {e}"))
}
impl Providers {
    /// Creating the service performs no network or keychain access.
    pub fn new(db: Arc<Mutex<Catalog>>, keychain: Arc<dyn SecretStore>) -> Self {
        Self::with_connector(db, keychain, Arc::new(|p, k| Ok(Box::new(wire::HttpAdapter::new(p, k)?) as Box<dyn wire::ProviderAdapter>)))
    }
    pub fn with_connector(
        db: Arc<Mutex<Catalog>>,
        keychain: Arc<dyn SecretStore>,
        connect: Arc<dyn Fn(&ProviderProfile, Option<Secret>) -> Result<Box<dyn wire::ProviderAdapter>, ProviderError> + Send + Sync>,
    ) -> Self {
        Self { db, keychain, session: MemorySecrets::default(), ops: Mutex::new(()), connect }
    }
    fn load(&self) -> Result<Vec<ProviderProfile>, ProviderError> {
        self.db.lock().map_err(db_error)?.ai_provider_profiles().map_err(db_error)
    }
    fn save(&self, profiles: &[ProviderProfile]) -> Result<(), ProviderError> {
        self.db.lock().map_err(db_error)?.put_ai_provider_profiles(profiles).map_err(db_error)
    }
    fn view(&self, profile: ProviderProfile) -> ProviderView {
        let has_key = match profile.key_storage {
            KeyStorage::None => false,
            KeyStorage::Keychain => true,
            KeyStorage::Session => self.session.get(&profile.id).ok().flatten().is_some(),
        };
        ProviderView { locality: profile.endpoint().ok().map(|e| e.locality), has_key, profile }
    }
    pub fn list(&self) -> Result<Vec<ProviderView>, ProviderError> {
        Ok(self.load()?.into_iter().map(|p| self.view(p)).collect())
    }
    fn find(profiles: &mut [ProviderProfile], id: &str, revision: u64) -> Result<usize, ProviderError> {
        let index = profiles.iter().position(|p| p.id == id).ok_or(ProviderError::NotFound)?;
        if profiles[index].revision != revision {
            return Err(ProviderError::Conflict);
        }
        Ok(index)
    }
    fn forget_key(&self, id: &str) -> Result<(), ProviderError> {
        self.session.delete(id)?;
        self.keychain.delete(id)
    }
    pub fn create(&self, draft: &ProviderDraft) -> Result<ProviderView, ProviderError> {
        let _guard = self.ops.lock().map_err(db_error)?;
        let (label, base_url, model) = validate_draft(draft)?;
        let mut profiles = self.load()?;
        if profiles.len() >= MAX_PROFILES {
            return Err(settings("Remove a provider before adding another (limit 16)"));
        }
        let profile = ProviderProfile {
            id: uuid::Uuid::new_v4().to_string(),
            kind: draft.kind,
            label,
            base_url,
            model,
            allow_local: draft.allow_local,
            key_storage: KeyStorage::None,
            key_origin: None,
            revision: 1,
            capabilities: Capabilities::default(),
        };
        profiles.push(profile.clone());
        self.save(&profiles)?;
        Ok(self.view(profile))
    }
    /// Any change resets tested capabilities. Changing the provider kind or the
    /// endpoint origin also deletes the stored key so it is never sent elsewhere.
    pub fn update(&self, id: &str, revision: u64, draft: &ProviderDraft) -> Result<ProviderView, ProviderError> {
        let _guard = self.ops.lock().map_err(db_error)?;
        let (label, base_url, model) = validate_draft(draft)?;
        let mut profiles = self.load()?;
        let index = Self::find(&mut profiles, id, revision)?;
        let origin = validate_endpoint(&base_url, draft.allow_local)?.origin();
        let p = &mut profiles[index];
        let moved = p.kind != draft.kind || p.key_origin.as_deref().is_some_and(|o| o != origin);
        if moved && p.key_storage != KeyStorage::None {
            self.forget_key(id)?;
            p.key_storage = KeyStorage::None;
            p.key_origin = None;
        }
        let changed = (p.kind, &p.label, &p.base_url, &p.model, p.allow_local) != (draft.kind, &label, &base_url, &model, draft.allow_local);
        if changed {
            *p = ProviderProfile { kind: draft.kind, label, base_url, model, allow_local: draft.allow_local, revision: p.revision + 1, capabilities: Capabilities::default(), ..p.clone() };
        }
        let profile = p.clone();
        self.save(&profiles)?;
        Ok(self.view(profile))
    }
    pub fn delete(&self, id: &str, revision: u64) -> Result<(), ProviderError> {
        let _guard = self.ops.lock().map_err(db_error)?;
        let mut profiles = self.load()?;
        let index = Self::find(&mut profiles, id, revision)?;
        // Delete the key first: if the keychain refuses, the profile stays so
        // the user can retry instead of leaving an orphaned secret behind.
        self.forget_key(id)?;
        profiles.remove(index);
        self.save(&profiles)
    }
    /// Stores (or rotates) the key. It is written to exactly one store.
    pub fn set_key(&self, id: &str, revision: u64, key: &str, store: StoreChoice) -> Result<ProviderView, ProviderError> {
        let _guard = self.ops.lock().map_err(db_error)?;
        let secret = Secret::new(key)?;
        let mut profiles = self.load()?;
        let index = Self::find(&mut profiles, id, revision)?;
        let origin = profiles[index].endpoint()?.origin();
        match store {
            StoreChoice::Keychain => {
                self.keychain.set(id, &secret)?;
                self.session.delete(id)?;
            }
            StoreChoice::Session => {
                self.session.set(id, &secret)?;
                self.keychain.delete(id)?;
            }
        }
        let p = &mut profiles[index];
        p.key_storage = match store {
            StoreChoice::Keychain => KeyStorage::Keychain,
            StoreChoice::Session => KeyStorage::Session,
        };
        p.key_origin = Some(origin);
        p.revision += 1;
        p.capabilities = Capabilities::default();
        let profile = p.clone();
        self.save(&profiles)?;
        Ok(self.view(profile))
    }
    pub fn clear_key(&self, id: &str, revision: u64) -> Result<ProviderView, ProviderError> {
        let _guard = self.ops.lock().map_err(db_error)?;
        let mut profiles = self.load()?;
        let index = Self::find(&mut profiles, id, revision)?;
        self.forget_key(id)?;
        let p = &mut profiles[index];
        p.key_storage = KeyStorage::None;
        p.key_origin = None;
        p.revision += 1;
        p.capabilities = Capabilities::default();
        let profile = p.clone();
        self.save(&profiles)?;
        Ok(self.view(profile))
    }
    fn key_for(&self, p: &ProviderProfile) -> Result<Option<Secret>, ProviderError> {
        let key = match p.key_storage {
            KeyStorage::None => return Ok(None),
            KeyStorage::Keychain => self.keychain.get(&p.id)?,
            KeyStorage::Session => self.session.get(&p.id)?,
        };
        let origin = p.endpoint()?.origin();
        if key.is_none() || p.key_origin.as_deref() != Some(origin.as_str()) {
            return Err(ProviderError::KeyMissing);
        }
        Ok(key)
    }
    /// The adapter for one exact configuration revision, for later analysis
    /// tickets. It fails with `Conflict` rather than falling back to another
    /// provider or revision.
    pub fn adapter(&self, id: &str, revision: u64) -> Result<Box<dyn wire::ProviderAdapter>, ProviderError> {
        let mut profiles = self.load()?;
        let index = Self::find(&mut profiles, id, revision)?;
        let profile = profiles.swap_remove(index);
        let key = self.key_for(&profile)?;
        if key.is_none() && profile.kind != ProviderKind::OpenaiCompatible {
            return Err(ProviderError::KeyMissing);
        }
        (self.connect)(&profile, key)
    }
    /// Explicit connection test. It sends only a generated 16x16 test image and
    /// fixed text; it never sends user images or catalog metadata. Results are
    /// recorded only if the configuration did not change while it ran.
    pub fn test(&self, id: &str, revision: u64, embeddings: bool) -> Result<TestReport, ProviderError> {
        let adapter = self.adapter(id, revision)?;
        let kind = self.load()?.into_iter().find(|p| p.id == id).ok_or(ProviderError::NotFound)?.kind;
        let (steps, mut capabilities, usage) = wire::connection_test(adapter.as_ref(), kind, embeddings);
        let _guard = self.ops.lock().map_err(db_error)?;
        let mut profiles = self.load()?;
        let index = Self::find(&mut profiles, id, revision)?;
        capabilities.tested_revision = Some(revision);
        capabilities.tested_at = Some(now());
        profiles[index].capabilities = capabilities;
        let profile = profiles[index].clone();
        self.save(&profiles)?;
        Ok(TestReport { provider: self.view(profile), steps, usage })
    }
}
