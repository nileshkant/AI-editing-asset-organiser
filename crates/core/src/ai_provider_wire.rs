//! SS-049 provider wire contracts: OpenAI chat completions, Anthropic messages,
//! Gemini generateContent and OpenAI-compatible servers, behind one bounded,
//! blocking adapter. Requests never follow redirects, never use a proxy, check
//! every resolved address against the profile's local-network policy, and
//! return sanitized errors. SS-050 qualifies a production image adapter; this
//! module only defines the contract and the explicit connection test.
use crate::ai_provider::{
    address_locality, CapabilityState, Capabilities, Check, Endpoint, Locality, ProviderError, ProviderKind,
    ProviderProfile, Secret, TestStep,
};
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use std::{fmt, time::Duration};
use ureq::unversioned::{
    resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver},
    transport::{DefaultConnector, NextTimeout},
};

pub const MAX_IMAGES: usize = 8;
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_OUTPUT_TOKENS: u32 = 8192;
pub const MAX_EMBED_INPUTS: usize = 64;
pub const MAX_EMBED_DIMENSIONS: usize = 8192;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
impl Usage {
    fn add(&mut self, other: &Usage) {
        let sum = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
        self.input_tokens = sum(self.input_tokens, other.input_tokens);
        self.output_tokens = sum(self.output_tokens, other.output_tokens);
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct ImagePart {
    pub media_type: &'static str,
    pub bytes: Vec<u8>,
}
impl fmt::Debug for ImagePart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImagePart({}, {} bytes)", self.media_type, self.bytes.len())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateRequest {
    pub instructions: String,
    pub text: String,
    pub images: Vec<ImagePart>,
    /// Requested JSON object shape. The answer is validated against its
    /// `required` properties and property types before it is returned.
    pub json_schema: Option<Value>,
    pub max_output_tokens: u32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateResponse {
    pub text: String,
    pub json: Option<Value>,
    pub model_version: Option<String>,
    pub usage: Usage,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Embeddings {
    pub vectors: Vec<Vec<f32>>,
    pub model_version: Option<String>,
    pub usage: Usage,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub version: Option<String>,
}

/// One configured provider/model. Implementations are bounded, time out and
/// never retry; callers decide about retries and billing uncertainty.
pub trait ProviderAdapter: Send + Sync + fmt::Debug {
    fn kind(&self) -> ProviderKind;
    fn model(&self) -> Result<ModelInfo, ProviderError>;
    fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ProviderError>;
    fn embed(&self, inputs: &[String]) -> Result<Embeddings, ProviderError>;
}

// ------------------------------------------------------------- networking

#[derive(Debug)]
struct Blocked(ProviderError);
impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Blocked {}

/// Resolves normally, then refuses the connection if any address is never
/// allowed or is local/private without the profile's opt-in. This also covers
/// public hostnames that resolve to private addresses.
#[derive(Debug)]
struct GuardResolver {
    allow_local: bool,
}
impl Resolver for GuardResolver {
    fn resolve(&self, uri: &ureq::http::Uri, config: &ureq::config::Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addresses = DefaultResolver::default().resolve(uri, config, timeout)?;
        check_addresses(addresses.iter().copied(), self.allow_local).map_err(|e| ureq::Error::Other(Box::new(Blocked(e))))?;
        Ok(addresses)
    }
}
/// Connection-time policy for every resolved address of an endpoint.
pub fn check_addresses(addresses: impl IntoIterator<Item = std::net::SocketAddr>, allow_local: bool) -> Result<(), ProviderError> {
    for address in addresses {
        match address_locality(address.ip()) {
            None => return Err(ProviderError::AddressNotAllowed),
            Some(Locality::Remote) => {}
            Some(_) if !allow_local => return Err(ProviderError::LocalNotPermitted),
            Some(_) => {}
        }
    }
    Ok(())
}
fn agent(allow_local: bool, timeout: Duration) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    let config = ureq::config::Config::builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .proxy(None)
        .timeout_connect(Some(CONNECT_TIMEOUT.min(timeout)))
        .timeout_global(Some(timeout))
        .user_agent(format!("CreativeShelf/{}", env!("CARGO_PKG_VERSION")))
        .tls_config(tls)
        .build();
    ureq::Agent::with_parts(config, DefaultConnector::new(), GuardResolver { allow_local })
}

/// Removes the key and anything key-shaped, control characters and length.
pub fn sanitize(message: &str, key: Option<&Secret>) -> String {
    let mut text = message.to_owned();
    if let Some(key) = key {
        text = text.replace(key.expose(), "[redacted]");
    }
    let pattern = regex::Regex::new(r"(sk|pk|rk|AIza|xai|gsk|key)[-_A-Za-z0-9]{8,}").expect("static pattern");
    let text = pattern.replace_all(&text, "[redacted]");
    let mut clean: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if clean.chars().count() > 200 {
        clean = clean.chars().take(200).collect::<String>() + "…";
    }
    clean.trim().to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Model,
    Image,
    Structured,
    Generate,
    Embed,
}
fn classify(status: u16, body: &[u8], ctx: Ctx, key: Option<&Secret>) -> ProviderError {
    let value: Option<Value> = serde_json::from_slice(body).ok();
    let error = value.as_ref().and_then(|v| v.get("error"));
    let field = |name: &str| error.and_then(|e| e.get(name)).map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).unwrap_or_default();
    let message = field("message");
    let text = format!("{} {} {} {message}", field("code"), field("type"), field("status")).to_lowercase();
    let mentions = |words: &[&str]| words.iter().any(|w| text.contains(w));
    use ProviderError as E;
    match status {
        300..=399 => E::Redirected,
        401 => E::Authentication,
        402 => E::QuotaExceeded,
        403 if mentions(&["model"]) => E::ModelNotFound,
        403 => E::Authentication,
        404 if ctx == Ctx::Embed => E::Unsupported,
        404 => E::ModelNotFound,
        408 => E::Timeout,
        413 => E::RequestTooLarge,
        429 if mentions(&["quota", "billing", "insufficient", "credit"]) => E::QuotaExceeded,
        429 => E::RateLimited,
        400 if mentions(&["api key not valid", "api_key_invalid", "invalid api key", "invalid x-api-key", "incorrect api key"]) => E::Authentication,
        400 | 415 | 422 if ctx == Ctx::Image && mentions(&["image", "vision", "multimodal", "modalit"]) => E::ImageUnsupported,
        400 | 422 if ctx == Ctx::Structured && mentions(&["response_format", "json_schema", "schema", "responsemimetype", "structured"]) => E::SchemaIncompatible,
        400 | 422 if ctx == Ctx::Embed && mentions(&["embed", "not supported", "does not support"]) => E::Unsupported,
        400 | 422 if mentions(&["model"]) && mentions(&["not found", "does not exist", "unknown model", "invalid model"]) => E::ModelNotFound,
        500..=599 => E::ProviderUnavailable(status),
        _ => E::Rejected { status, message: sanitize(&message, key) },
    }
}
fn transport_error(error: ureq::Error) -> ProviderError {
    match error {
        ureq::Error::Timeout(_) => ProviderError::Timeout,
        ureq::Error::Other(inner) => inner.downcast::<Blocked>().map_or(ProviderError::Unreachable, |b| b.0),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::TlsRequired => ProviderError::Tls,
        ureq::Error::BodyExceedsLimit(_) => ProviderError::ResponseTooLarge,
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => ProviderError::Redirected,
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => ProviderError::Timeout,
        // rustls reports handshake and certificate failures as invalid data.
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidData => ProviderError::Tls,
        _ => ProviderError::Unreachable,
    }
}

/// Checks a JSON answer against the requested object schema: required
/// properties present and declared property types respected.
pub fn conforms(schema: &Value, value: &Value) -> bool {
    let type_ok = |kind: Option<&str>, v: &Value| match kind {
        Some("object") => v.is_object(),
        Some("array") => v.is_array(),
        Some("string") => v.is_string(),
        Some("number") => v.is_number(),
        Some("integer") => v.is_i64() || v.is_u64(),
        Some("boolean") => v.is_boolean(),
        _ => true,
    };
    if !type_ok(schema.get("type").and_then(Value::as_str), value) {
        return false;
    }
    let Some(object) = value.as_object() else { return true };
    let required = schema.get("required").and_then(Value::as_array).into_iter().flatten();
    if required.filter_map(Value::as_str).any(|name| !object.contains_key(name)) {
        return false;
    }
    let properties = schema.get("properties").and_then(Value::as_object);
    object.iter().all(|(name, v)| {
        properties.and_then(|p| p.get(name)).map_or(schema.get("additionalProperties") != Some(&Value::Bool(false)), |s| conforms(s, v))
    })
}

// --------------------------------------------------------------- adapter

pub struct HttpAdapter {
    kind: ProviderKind,
    endpoint: Endpoint,
    model: String,
    key: Option<Secret>,
    agent: ureq::Agent,
}
impl fmt::Debug for HttpAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HttpAdapter({:?}, {}, {})", self.kind, self.endpoint.origin(), self.model)
    }
}
fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
fn text_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(value, |v, k| v.get(*k))?.as_str()
}
fn count_at(value: &Value, path: &[&str]) -> Option<u64> {
    path.iter().try_fold(value, |v, k| v.get(*k))?.as_u64()
}
impl HttpAdapter {
    pub fn new(profile: &ProviderProfile, key: Option<Secret>) -> Result<Self, ProviderError> {
        Self::with_timeout(profile, key, REQUEST_TIMEOUT)
    }
    pub fn with_timeout(profile: &ProviderProfile, key: Option<Secret>, timeout: Duration) -> Result<Self, ProviderError> {
        let endpoint = profile.endpoint()?;
        Ok(Self { kind: profile.kind, model: profile.model.clone(), agent: agent(profile.allow_local, timeout), endpoint, key })
    }
    fn call(&self, segments: &[&str], body: Option<&Value>, ctx: Ctx) -> Result<Value, ProviderError> {
        let mut url = self.endpoint.url.clone();
        url.path_segments_mut().map_err(|_| ProviderError::InvalidSettings("Endpoint cannot have a path".into()))?.pop_if_empty().extend(segments);
        let key = self.key.as_ref().map(Secret::expose);
        let mut headers: Vec<(&str, String)> = vec![("accept", "application/json".into())];
        match (self.kind, key) {
            (ProviderKind::Anthropic, Some(k)) => {
                headers.push(("x-api-key", k.into()));
                headers.push(("anthropic-version", ANTHROPIC_VERSION.into()));
            }
            (ProviderKind::Gemini, Some(k)) => headers.push(("x-goog-api-key", k.into())),
            (_, Some(k)) => headers.push(("authorization", format!("Bearer {k}"))),
            (_, None) => {}
        }
        let result = match body {
            Some(body) => {
                let bytes = serde_json::to_vec(body).map_err(|_| ProviderError::RequestTooLarge)?;
                if bytes.len() > MAX_REQUEST_BYTES {
                    return Err(ProviderError::RequestTooLarge);
                }
                let mut request = self.agent.post(url.as_str());
                for (name, value) in &headers {
                    request = request.header(*name, value.as_str());
                }
                request.content_type("application/json").send(&bytes[..])
            }
            None => {
                let mut request = self.agent.get(url.as_str());
                for (name, value) in &headers {
                    request = request.header(*name, value.as_str());
                }
                request.call()
            }
        };
        let mut response = result.map_err(transport_error)?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(ProviderError::Redirected);
        }
        let bytes = response.body_mut().with_config().limit(MAX_RESPONSE_BYTES).read_to_vec().map_err(transport_error)?;
        if !(200..300).contains(&status) {
            return Err(classify(status, &bytes, ctx, self.key.as_ref()));
        }
        serde_json::from_slice(&bytes).map_err(|_| ProviderError::MalformedResponse)
    }
    fn generate_body(&self, r: &GenerateRequest) -> Value {
        let schema_text = r.json_schema.as_ref().map(|s| format!("\n\nRespond with only a JSON object that matches this JSON Schema:\n{s}"));
        match self.kind {
            ProviderKind::Openai | ProviderKind::OpenaiCompatible => {
                let content = if r.images.is_empty() {
                    json!(r.text)
                } else {
                    let mut parts = vec![json!({"type": "text", "text": r.text})];
                    parts.extend(r.images.iter().map(|i| json!({"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", i.media_type, b64(&i.bytes))}})));
                    Value::Array(parts)
                };
                let mut body = json!({"model": self.model, "messages": [{"role": "system", "content": r.instructions}, {"role": "user", "content": content}]});
                let limit = if self.kind == ProviderKind::Openai { "max_completion_tokens" } else { "max_tokens" };
                body[limit] = json!(r.max_output_tokens);
                if let Some(schema) = &r.json_schema {
                    body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "creativeshelf_result", "strict": true, "schema": schema}});
                }
                body
            }
            ProviderKind::Anthropic => {
                // Structured output is requested in the instructions and the
                // answer is validated; no beta API is assumed.
                let mut content: Vec<Value> = r.images.iter().map(|i| json!({"type": "image", "source": {"type": "base64", "media_type": i.media_type, "data": b64(&i.bytes)}})).collect();
                content.push(json!({"type": "text", "text": r.text}));
                json!({"model": self.model, "max_tokens": r.max_output_tokens, "system": format!("{}{}", r.instructions, schema_text.unwrap_or_default()), "messages": [{"role": "user", "content": content}]})
            }
            ProviderKind::Gemini => {
                let mut parts: Vec<Value> = r.images.iter().map(|i| json!({"inline_data": {"mime_type": i.media_type, "data": b64(&i.bytes)}})).collect();
                parts.push(json!({"text": r.text}));
                let mut config = json!({"maxOutputTokens": r.max_output_tokens});
                if let Some(schema) = &r.json_schema {
                    config["responseMimeType"] = json!("application/json");
                    config["responseJsonSchema"] = schema.clone();
                }
                json!({"systemInstruction": {"parts": [{"text": r.instructions}]}, "contents": [{"role": "user", "parts": parts}], "generationConfig": config})
            }
        }
    }
    fn parse_generate(&self, v: &Value) -> Result<(String, Option<String>, Usage), ProviderError> {
        let text = match self.kind {
            ProviderKind::Openai | ProviderKind::OpenaiCompatible => {
                v.get("choices").and_then(|c| c.get(0)?.get("message")?.get("content")?.as_str()).map(str::to_owned)
            }
            ProviderKind::Anthropic => v.get("content").and_then(Value::as_array).map(|parts| {
                parts.iter().filter(|p| p.get("type").and_then(Value::as_str) == Some("text")).filter_map(|p| p.get("text")?.as_str()).collect::<String>()
            }),
            ProviderKind::Gemini => {
                if let Some(reason) = text_at(v, &["promptFeedback", "blockReason"]) {
                    return Err(ProviderError::Rejected { status: 200, message: format!("blocked: {}", sanitize(reason, None)) });
                }
                v.get("candidates").and_then(|c| c.get(0)).and_then(|c| c.get("content")?.get("parts")?.as_array()).map(|parts| {
                    parts.iter().filter_map(|p| p.get("text")?.as_str()).collect::<String>()
                })
            }
        };
        let text = text.ok_or(ProviderError::MalformedResponse)?;
        let (version, usage) = match self.kind {
            ProviderKind::Openai | ProviderKind::OpenaiCompatible => (
                text_at(v, &["model"]),
                Usage { input_tokens: count_at(v, &["usage", "prompt_tokens"]), output_tokens: count_at(v, &["usage", "completion_tokens"]) },
            ),
            ProviderKind::Anthropic => (
                text_at(v, &["model"]),
                Usage { input_tokens: count_at(v, &["usage", "input_tokens"]), output_tokens: count_at(v, &["usage", "output_tokens"]) },
            ),
            ProviderKind::Gemini => (
                text_at(v, &["modelVersion"]),
                Usage { input_tokens: count_at(v, &["usageMetadata", "promptTokenCount"]), output_tokens: count_at(v, &["usageMetadata", "candidatesTokenCount"]) },
            ),
        };
        Ok((text, version.map(|s| sanitize(s, None)), usage))
    }
}
fn validate_request(r: &GenerateRequest) -> Result<(), ProviderError> {
    let images: usize = r.images.iter().map(|i| i.bytes.len()).sum();
    if r.images.len() > MAX_IMAGES
        || r.images.iter().any(|i| i.bytes.len() > MAX_IMAGE_BYTES || !matches!(i.media_type, "image/png" | "image/jpeg" | "image/webp"))
        || images > 4 * MAX_IMAGE_BYTES
        || r.text.len() + r.instructions.len() > MAX_TEXT_BYTES
        || !(1..=MAX_OUTPUT_TOKENS).contains(&r.max_output_tokens)
    {
        return Err(ProviderError::RequestTooLarge);
    }
    Ok(())
}
impl ProviderAdapter for HttpAdapter {
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    fn model(&self) -> Result<ModelInfo, ProviderError> {
        match self.kind {
            ProviderKind::OpenaiCompatible => {
                // Model IDs on these servers may contain '/', so list instead.
                let v = self.call(&["models"], None, Ctx::Model)?;
                let found = v.get("data").and_then(Value::as_array).into_iter().flatten().any(|m| text_at(m, &["id"]) == Some(self.model.as_str()));
                if !found {
                    return Err(ProviderError::ModelNotFound);
                }
                Ok(ModelInfo { id: self.model.clone(), version: None })
            }
            _ => {
                let v = self.call(&["models", &self.model], None, Ctx::Model)?;
                let id = text_at(&v, &["id"]).or_else(|| text_at(&v, &["name"])).ok_or(ProviderError::MalformedResponse)?;
                Ok(ModelInfo { id: sanitize(id, None), version: text_at(&v, &["version"]).map(|s| sanitize(s, None)) })
            }
        }
    }
    fn generate(&self, r: &GenerateRequest) -> Result<GenerateResponse, ProviderError> {
        validate_request(r)?;
        let ctx = match (r.images.is_empty(), r.json_schema.is_some()) {
            (false, _) => Ctx::Image,
            (true, true) => Ctx::Structured,
            (true, false) => Ctx::Generate,
        };
        let body = self.generate_body(r);
        let v = match self.kind {
            ProviderKind::Gemini => self.call(&["models", &format!("{}:generateContent", self.model)], Some(&body), ctx)?,
            ProviderKind::Anthropic => self.call(&["messages"], Some(&body), ctx)?,
            _ => self.call(&["chat", "completions"], Some(&body), ctx)?,
        };
        let (text, model_version, usage) = self.parse_generate(&v)?;
        let json = match &r.json_schema {
            None => None,
            Some(schema) => {
                let trimmed = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
                let value: Value = serde_json::from_str(trimmed).map_err(|_| ProviderError::SchemaIncompatible)?;
                if !conforms(schema, &value) {
                    return Err(ProviderError::SchemaIncompatible);
                }
                Some(value)
            }
        };
        Ok(GenerateResponse { text, json, model_version, usage })
    }
    fn embed(&self, inputs: &[String]) -> Result<Embeddings, ProviderError> {
        if inputs.is_empty() || inputs.len() > MAX_EMBED_INPUTS || inputs.iter().map(String::len).sum::<usize>() > MAX_TEXT_BYTES {
            return Err(ProviderError::RequestTooLarge);
        }
        let (vectors, version, usage): (Vec<Vec<f64>>, Option<&str>, Usage);
        let v;
        match self.kind {
            ProviderKind::Anthropic => return Err(ProviderError::Unsupported),
            ProviderKind::Gemini => {
                let requests: Vec<Value> = inputs.iter().map(|t| json!({"model": format!("models/{}", self.model), "content": {"parts": [{"text": t}]}})).collect();
                v = self.call(&["models", &format!("{}:batchEmbedContents", self.model)], Some(&json!({"requests": requests})), Ctx::Embed)?;
                vectors = v.get("embeddings").and_then(Value::as_array).into_iter().flatten().map(|e| floats(e.get("values"))).collect::<Option<_>>().ok_or(ProviderError::MalformedResponse)?;
                (version, usage) = (None, Usage::default());
            }
            _ => {
                v = self.call(&["embeddings"], Some(&json!({"model": self.model, "input": inputs})), Ctx::Embed)?;
                vectors = v.get("data").and_then(Value::as_array).into_iter().flatten().map(|e| floats(e.get("embedding"))).collect::<Option<_>>().ok_or(ProviderError::MalformedResponse)?;
                (version, usage) = (text_at(&v, &["model"]), Usage { input_tokens: count_at(&v, &["usage", "prompt_tokens"]), output_tokens: None });
            }
        }
        let dims = vectors.first().map_or(0, Vec::len);
        if vectors.len() != inputs.len() || dims == 0 || dims > MAX_EMBED_DIMENSIONS || vectors.iter().any(|x| x.len() != dims || x.iter().any(|f| !f.is_finite())) {
            return Err(ProviderError::MalformedResponse);
        }
        Ok(Embeddings { vectors: vectors.into_iter().map(|x| x.into_iter().map(|f| f as f32).collect()).collect(), model_version: version.map(|s| sanitize(s, None)), usage })
    }
}
fn floats(v: Option<&Value>) -> Option<Vec<f64>> {
    v?.as_array()?.iter().map(Value::as_f64).collect()
}

// --------------------------------------------------------- connection test

/// A generated 16x16 solid red PNG: never user content.
pub fn probe_image() -> ImagePart {
    let image = image::RgbImage::from_pixel(16, 16, image::Rgb([220, 20, 20]));
    let mut bytes = std::io::Cursor::new(vec![]);
    image.write_to(&mut bytes, image::ImageFormat::Png).expect("in-memory PNG");
    ImagePart { media_type: "image/png", bytes: bytes.into_inner() }
}
const PROBE_INSTRUCTIONS: &str = "You are a connection test for a desktop app. Follow the user's request exactly and briefly.";
pub fn probe_schema() -> Value {
    json!({"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"], "additionalProperties": false})
}

/// Model access, then image input, structured output and (optionally)
/// embeddings. Each capability is `Supported` only from observed behavior;
/// a provider refusing image input is `Unsupported`, which stays distinct
/// from authentication, quota and transport failures.
pub fn connection_test(adapter: &dyn ProviderAdapter, kind: ProviderKind, embeddings: bool) -> (Vec<TestStep>, Capabilities, Usage) {
    let mut steps = vec![];
    let mut caps = Capabilities::default();
    let mut usage = Usage::default();
    let mut step = |check: Check, result: Result<(), ProviderError>| {
        let passed = result.is_ok();
        steps.push(TestStep { check, passed, error: result.err().map(|e| e.view()) });
        passed
    };
    match adapter.model() {
        Ok(info) => {
            caps.model_version = info.version;
            step(Check::ModelAccess, Ok(()));
        }
        Err(e) => {
            step(Check::ModelAccess, Err(e));
            return (steps, caps, usage);
        }
    }
    let image = adapter.generate(&GenerateRequest {
        instructions: PROBE_INSTRUCTIONS.into(),
        text: "What single color fills this image? Answer with one lowercase English word.".into(),
        images: vec![probe_image()],
        json_schema: None,
        max_output_tokens: 16,
    });
    match image {
        Ok(r) => {
            usage.add(&r.usage);
            caps.model_version = r.model_version.or(caps.model_version);
            caps.image_input = if r.text.to_lowercase().contains("red") { CapabilityState::Supported } else { CapabilityState::Inconclusive };
            step(Check::ImageInput, Ok(()));
        }
        Err(e) => {
            if e == ProviderError::ImageUnsupported {
                caps.image_input = CapabilityState::Unsupported;
            }
            step(Check::ImageInput, Err(e));
        }
    }
    let schema = probe_schema();
    let structured = adapter.generate(&GenerateRequest {
        instructions: PROBE_INSTRUCTIONS.into(),
        text: "Set the status field to the word ok.".into(),
        images: vec![],
        json_schema: Some(schema),
        max_output_tokens: 32,
    });
    match structured {
        Ok(r) => {
            usage.add(&r.usage);
            caps.structured_output = CapabilityState::Supported;
            step(Check::StructuredOutput, Ok(()));
        }
        Err(e) => {
            if e == ProviderError::SchemaIncompatible {
                caps.structured_output = CapabilityState::Unsupported;
            }
            step(Check::StructuredOutput, Err(e));
        }
    }
    if embeddings {
        if kind == ProviderKind::Anthropic {
            caps.embeddings = CapabilityState::Unsupported;
            step(Check::Embeddings, Err(ProviderError::Unsupported));
        } else {
            match adapter.embed(&["CreativeShelf connection test".into()]) {
                Ok(r) => {
                    usage.add(&r.usage);
                    caps.embeddings = CapabilityState::Supported;
                    step(Check::Embeddings, Ok(()));
                }
                Err(e) => {
                    if matches!(e, ProviderError::Unsupported | ProviderError::ModelNotFound) {
                        caps.embeddings = CapabilityState::Unsupported;
                    }
                    step(Check::Embeddings, Err(e));
                }
            }
        }
    }
    (steps, caps, usage)
}
