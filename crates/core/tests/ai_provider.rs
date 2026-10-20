//! SS-049: provider configuration, endpoint policy, secret lifecycle and the
//! four wire contracts, exercised through real HTTP on loopback sockets.
use serde_json::{json, Value};
use soundshelf_core::{
    ai_provider::*,
    ai_provider_wire::{self as wire, HttpAdapter, ProviderAdapter},
    catalog::Catalog,
};
use std::{
    collections::HashMap,
    io::Write,
    net::{SocketAddr, TcpListener},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const KEY: &str = "sk-test-0123456789abcdefSECRET";

mod common;
use common::http::Server;

fn profile(kind: ProviderKind, base: &str, model: &str) -> ProviderProfile {
    ProviderProfile {
        id: "p1".into(),
        kind,
        label: "Test".into(),
        base_url: base.into(),
        model: model.into(),
        allow_local: true,
        key_storage: KeyStorage::Keychain,
        key_origin: None,
        revision: 1,
        capabilities: Capabilities::default(),
    }
}
fn key() -> Option<Secret> {
    Some(Secret::new(KEY).unwrap())
}
fn red_pixel_seen(data: &str) -> bool {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
    let image = image::load_from_memory(&bytes).unwrap().to_rgb8();
    image.dimensions() == (16, 16) && image.get_pixel(8, 8).0 == [220, 20, 20]
}

/// A well-behaved multimodal provider for each wire contract.
fn provider(kind: ProviderKind) -> Server {
    Server::json(move |s| {
        let b = &s.body;
        let structured = match kind {
            ProviderKind::Gemini => b["generationConfig"]["responseJsonSchema"].is_object(),
            ProviderKind::Anthropic => b["system"].as_str().is_some_and(|t| t.contains("JSON Schema")),
            _ => b["response_format"]["type"] == "json_schema",
        };
        let image = match kind {
            ProviderKind::Gemini => b["contents"][0]["parts"][0]["inline_data"]["data"].as_str().map(str::to_owned),
            ProviderKind::Anthropic => b["messages"][0]["content"][0]["source"]["data"].as_str().map(str::to_owned),
            _ => b["messages"][1]["content"][1]["image_url"]["url"].as_str().map(|u| u.trim_start_matches("data:image/png;base64,").to_owned()),
        };
        let answer = if structured { r#"{"status":"ok"}"#.to_owned() } else if image.as_deref().is_some_and(red_pixel_seen) { "red".into() } else { "blue".into() };
        match (kind, s.method.as_str(), s.path.as_str()) {
            (ProviderKind::OpenaiCompatible, "GET", "/v1/models") => (200, json!({"data": [{"id": "org/vision-model"}]})),
            (_, "GET", p) if p.starts_with("/v1/models/") => (200, json!({"id": "vision-model"})),
            (_, "GET", p) if p.starts_with("/v1beta/models/") => (200, json!({"name": "models/gemini-test", "version": "001"})),
            (_, "POST", "/v1/chat/completions") => (200, json!({"model": "vision-model-2026-01-01", "choices": [{"message": {"content": answer}}], "usage": {"prompt_tokens": 10, "completion_tokens": 2}})),
            (_, "POST", "/v1/messages") => (200, json!({"model": "claude-test-20260101", "content": [{"type": "text", "text": answer}], "usage": {"input_tokens": 7, "output_tokens": 3}})),
            (_, "POST", p) if p.ends_with(":generateContent") => (200, json!({"modelVersion": "gemini-test-001", "candidates": [{"content": {"parts": [{"text": answer}]}}], "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 1}})),
            (_, "POST", "/v1/embeddings") => (200, json!({"model": "embed", "data": [{"embedding": [0.1, 0.2, 0.3]}], "usage": {"prompt_tokens": 3}})),
            (_, "POST", p) if p.ends_with(":batchEmbedContents") => (200, json!({"embeddings": [{"values": [0.5, 0.25]}]})),
            _ => (404, json!({"error": {"message": "no route"}})),
        }
    })
}

#[test]
fn endpoint_policy_requires_https_and_explicit_local_opt_in() {
    let ok = |url: &str, local: bool| validate_endpoint(url, local).map(|e| e.locality);
    assert_eq!(ok("https://api.example.com/v1/", false), Ok(Locality::Remote));
    assert_eq!(validate_endpoint("https://api.example.com/v1/", false).unwrap().url.as_str(), "https://api.example.com/v1");
    assert_eq!(ok("http://api.example.com/v1", true), Err(ProviderError::InsecureEndpoint));
    assert_eq!(ok("http://127.0.0.1:11434/v1", false), Err(ProviderError::LocalNotPermitted));
    assert_eq!(ok("http://127.0.0.1:11434/v1", true), Ok(Locality::Loopback));
    assert_eq!(ok("http://localhost:1234", true), Ok(Locality::Loopback));
    assert_eq!(ok("http://[::1]:8080", true), Ok(Locality::Loopback));
    for private in ["http://10.0.0.5", "http://192.168.1.20:8000", "http://172.20.0.1", "http://100.64.0.1", "http://[fd00::1]", "http://gpu-box", "http://studio.local"] {
        assert_eq!(ok(private, false), Err(ProviderError::LocalNotPermitted), "{private}");
        assert_eq!(ok(private, true), Ok(Locality::Private), "{private}");
    }
    // Cloud metadata and other never-contacted ranges stay blocked even with opt-in.
    for blocked in ["http://169.254.169.254/latest", "http://[fe80::1]", "http://0.0.0.0", "http://224.0.0.1", "http://255.255.255.255"] {
        assert_eq!(ok(blocked, true), Err(ProviderError::AddressNotAllowed), "{blocked}");
    }
    for invalid in ["ftp://api.example.com", "https://user:pass@api.example.com", "https://api.example.com/v1?key=abc", "https://api.example.com/#x", "not a url", ""] {
        assert!(matches!(validate_endpoint(invalid, true), Err(ProviderError::InvalidSettings(_))), "{invalid}");
    }
    // The same policy runs on every address a hostname resolves to.
    let addr = |s: &str| s.parse::<SocketAddr>().unwrap();
    assert_eq!(wire::check_addresses([addr("93.184.216.34:443")], false), Ok(()));
    assert_eq!(wire::check_addresses([addr("93.184.216.34:443"), addr("10.0.0.1:443")], false), Err(ProviderError::LocalNotPermitted));
    assert_eq!(wire::check_addresses([addr("169.254.169.254:80")], true), Err(ProviderError::AddressNotAllowed));
}

#[test]
fn four_wire_contracts_verify_capabilities_from_observed_answers() {
    for (kind, base, model) in [
        (ProviderKind::Openai, "/v1", "vision-model"),
        (ProviderKind::Anthropic, "/v1", "claude-test"),
        (ProviderKind::Gemini, "/v1beta", "gemini-test"),
        (ProviderKind::OpenaiCompatible, "/v1", "org/vision-model"),
    ] {
        let server = provider(kind);
        let adapter = HttpAdapter::new(&profile(kind, &server.url(base), model), key()).unwrap();
        let (steps, caps, usage) = wire::connection_test(&adapter, kind, true);
        assert!(steps.iter().take(3).all(|s| s.passed), "{kind:?}: {steps:?}");
        assert_eq!(caps.image_input, CapabilityState::Supported, "{kind:?} saw the generated red image");
        assert_eq!(caps.structured_output, CapabilityState::Supported, "{kind:?}");
        let embeddings = if kind == ProviderKind::Anthropic { CapabilityState::Unsupported } else { CapabilityState::Supported };
        assert_eq!(caps.embeddings, embeddings, "{kind:?}");
        assert!(caps.model_version.is_some(), "{kind:?} records the served model version");
        assert!(usage.input_tokens.unwrap_or(0) > 0, "{kind:?} reports usage");
        for request in server.requests() {
            assert!(!request.path.contains(KEY) && !request.body.to_string().contains(KEY), "key only travels in a header");
            let auth = match kind {
                ProviderKind::Anthropic => request.headers.get("x-api-key").cloned(),
                ProviderKind::Gemini => request.headers.get("x-goog-api-key").cloned(),
                _ => request.headers.get("authorization").map(|v| v.trim_start_matches("Bearer ").to_owned()),
            };
            assert_eq!(auth.as_deref(), Some(KEY), "{kind:?} {}", request.path);
            if kind == ProviderKind::Anthropic {
                assert_eq!(request.headers.get("anthropic-version").map(String::as_str), Some("2023-06-01"));
            }
        }
        if kind == ProviderKind::Anthropic {
            assert!(server.requests().iter().all(|r| !r.path.contains("embed")), "no embeddings request to a provider without an API");
        }
    }
}

#[test]
fn text_only_models_are_unsupported_for_images_not_authentication_failures() {
    let server = Server::json(|s| match (s.method.as_str(), s.path.as_str()) {
        ("GET", _) => (200, json!({"id": "text-model"})),
        ("POST", _) if s.body["messages"][1]["content"].is_array() => {
            (400, json!({"error": {"message": "Invalid content type. image_url is only supported by certain models.", "type": "invalid_request_error"}}))
        }
        ("POST", _) if s.body["response_format"].is_object() => (400, json!({"error": {"message": "response_format json_schema is not supported with this model"}})),
        _ => (200, json!({"choices": [{"message": {"content": "hello"}}]})),
    });
    let adapter = HttpAdapter::new(&profile(ProviderKind::Openai, &server.url("/v1"), "text-model"), key()).unwrap();
    let (steps, caps, _) = wire::connection_test(&adapter, ProviderKind::Openai, false);
    assert!(steps[0].passed);
    assert_eq!(steps[1].error.as_ref().unwrap().code, "IMAGE_UNSUPPORTED");
    assert_eq!(caps.image_input, CapabilityState::Unsupported);
    assert_eq!(caps.structured_output, CapabilityState::Unsupported);
    assert_eq!(caps.embeddings, CapabilityState::Untested, "not requested");

    // A model that accepts the request but cannot see the image is not counted as image-capable.
    let blind = provider_answering("blue");
    let adapter = HttpAdapter::new(&profile(ProviderKind::Openai, &blind.url("/v1"), "m"), key()).unwrap();
    assert_eq!(wire::connection_test(&adapter, ProviderKind::Openai, false).1.image_input, CapabilityState::Inconclusive);

    let unauthorized = Server::json(|_| (401, json!({"error": {"message": "Incorrect API key provided: sk-test-0123456789abcdefSECRET"}})));
    let adapter = HttpAdapter::new(&profile(ProviderKind::Openai, &unauthorized.url("/v1"), "m"), key()).unwrap();
    let (steps, caps, _) = wire::connection_test(&adapter, ProviderKind::Openai, true);
    assert_eq!((steps.len(), steps[0].error.as_ref().unwrap().code), (1, "AUTHENTICATION"));
    assert_eq!(caps.image_input, CapabilityState::Untested, "an auth failure says nothing about images");
}
fn provider_answering(answer: &'static str) -> Server {
    Server::json(move |s| match s.method.as_str() {
        "GET" => (200, json!({"id": "m"})),
        _ => (200, json!({"choices": [{"message": {"content": answer}}]})),
    })
}

#[test]
fn wire_errors_map_to_distinct_sanitized_codes() {
    let cases: Vec<(u16, Value, ProviderError)> = vec![
        (401, json!({"error": {"message": "bad key"}}), ProviderError::Authentication),
        (403, json!({"error": {"message": "You do not have access to model x"}}), ProviderError::ModelNotFound),
        (404, json!({"error": {"message": "The model does not exist"}}), ProviderError::ModelNotFound),
        (400, json!({"error": {"code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT"}}), ProviderError::Authentication),
        (429, json!({"error": {"code": "insufficient_quota", "message": "You exceeded your current quota"}}), ProviderError::QuotaExceeded),
        (429, json!({"error": {"type": "rate_limit_error", "message": "Too many requests"}}), ProviderError::RateLimited),
        (402, json!({}), ProviderError::QuotaExceeded),
        (529, json!({"type": "error", "error": {"type": "overloaded_error"}}), ProviderError::ProviderUnavailable(529)),
        (503, json!({}), ProviderError::ProviderUnavailable(503)),
    ];
    for (status, body, expected) in cases {
        let server = Server::json(move |_| (status, body.clone()));
        let adapter = HttpAdapter::new(&profile(ProviderKind::Openai, &server.url("/v1"), "m"), key()).unwrap();
        assert_eq!(adapter.model().unwrap_err(), expected, "HTTP {status}");
    }
    // Unknown rejections keep a short provider message with the key removed.
    let echo = Server::json(|_| (409, json!({"error": {"message": format!("conflict for {KEY} and AIzaSyAbCdEfGhIjKlMn\n\u{7}")}})));
    let adapter = HttpAdapter::new(&profile(ProviderKind::Openai, &echo.url("/v1"), "m"), key()).unwrap();
    let error = adapter.model().unwrap_err();
    assert_eq!(error.code(), "REJECTED");
    let text = error.to_string();
    assert!(!text.contains(KEY) && !text.contains("AIzaSy") && !text.contains('\u{7}'), "{text}");
    assert!(!format!("{adapter:?}").contains(KEY));
}

#[test]
fn redirects_are_refused_and_never_followed() {
    let target = Server::json(|_| (200, json!({"id": "m"})));
    let location = target.url("/v1/models/m");
    let redirect = Server::start(Arc::new(move |_| (302, vec![("location".into(), location.clone())], vec![])));
    let adapter = HttpAdapter::new(&profile(ProviderKind::Openai, &redirect.url("/v1"), "m"), key()).unwrap();
    assert_eq!(adapter.model().unwrap_err(), ProviderError::Redirected);
    assert!(target.requests().is_empty(), "the key is never sent to a redirect target");
}

#[test]
fn transport_failures_are_bounded_and_distinct() {
    // Timeout: the server accepts and never answers.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = silent.local_addr().unwrap();
    let hold = thread::spawn(move || {
        let held: Vec<_> = silent.incoming().take(1).collect();
        thread::sleep(Duration::from_secs(3));
        drop(held);
    });
    let p = profile(ProviderKind::Openai, &format!("http://{address}/v1"), "m");
    let started = Instant::now();
    let error = HttpAdapter::with_timeout(&p, key(), Duration::from_millis(800)).unwrap().model().unwrap_err();
    assert_eq!(error, ProviderError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(3));
    hold.join().unwrap();

    // TLS: HTTPS to a plain HTTP server fails the handshake on its non-TLS answer.
    let plain = TcpListener::bind("127.0.0.1:0").unwrap();
    let plain_address = plain.local_addr().unwrap();
    thread::spawn(move || {
        for mut stream in plain.incoming().flatten() {
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n");
            thread::sleep(Duration::from_millis(200));
        }
    });
    let tls = profile(ProviderKind::Openai, &format!("https://{plain_address}/v1"), "m");
    assert_eq!(HttpAdapter::new(&tls, key()).unwrap().model().unwrap_err(), ProviderError::Tls);

    // A real TLS server whose certificate is not trusted (self-signed test
    // fixture, generated for these tests only) is refused as a TLS failure.
    let untrusted = self_signed_tls_server();
    let cert = profile(ProviderKind::Openai, &format!("https://{untrusted}/v1"), "m");
    assert_eq!(HttpAdapter::new(&cert, key()).unwrap().model().unwrap_err(), ProviderError::Tls);

    // Nothing listening.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let gone = profile(ProviderKind::Openai, &format!("http://{closed}/v1"), "m");
    assert_eq!(HttpAdapter::new(&gone, key()).unwrap().model().unwrap_err(), ProviderError::Unreachable);

    // Oversized responses stop at the limit.
    let huge = Server::start(Arc::new(|_| (200, vec![], vec![b' '; 3 * 1024 * 1024])));
    let big = profile(ProviderKind::Openai, &huge.url("/v1"), "m");
    assert_eq!(HttpAdapter::new(&big, key()).unwrap().model().unwrap_err(), ProviderError::ResponseTooLarge);

    // Oversized requests are refused before sending.
    let server = provider(ProviderKind::Openai);
    let adapter = HttpAdapter::new(&profile(ProviderKind::Openai, &server.url("/v1"), "m"), key()).unwrap();
    let request = |images: usize, bytes: usize| wire::GenerateRequest {
        instructions: String::new(),
        text: "x".into(),
        images: (0..images).map(|_| wire::ImagePart { media_type: "image/png", bytes: vec![0; bytes] }).collect(),
        json_schema: None,
        max_output_tokens: 10,
    };
    assert_eq!(adapter.generate(&request(9, 10)).unwrap_err(), ProviderError::RequestTooLarge);
    assert_eq!(adapter.generate(&request(1, wire::MAX_IMAGE_BYTES + 1)).unwrap_err(), ProviderError::RequestTooLarge);
    assert!(server.requests().is_empty());
}

fn self_signed_tls_server() -> SocketAddr {
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
    let certs = vec![CertificateDer::from_pem_slice(include_bytes!("fixtures/selfsigned-cert.pem")).unwrap()];
    let key = PrivateKeyDer::from_pem_slice(include_bytes!("fixtures/selfsigned-key.pem")).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions().unwrap().with_no_client_auth().with_single_cert(certs, key).unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut tls = rustls::StreamOwned::new(rustls::ServerConnection::new(config.clone()).unwrap(), stream);
            let _ = tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
        }
    });
    address
}

// ------------------------------------------------------------------ service

#[derive(Default)]
struct FakeKeychain {
    entries: Mutex<HashMap<String, Secret>>,
    fail_set: Mutex<Option<ProviderError>>,
    fail_delete: Mutex<Option<ProviderError>>,
}
impl SecretStore for FakeKeychain {
    fn get(&self, account: &str) -> Result<Option<Secret>, ProviderError> {
        Ok(self.entries.lock().unwrap().get(account).cloned())
    }
    fn set(&self, account: &str, secret: &Secret) -> Result<(), ProviderError> {
        if let Some(e) = self.fail_set.lock().unwrap().clone() {
            return Err(e);
        }
        self.entries.lock().unwrap().insert(account.into(), secret.clone());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), ProviderError> {
        if let Some(e) = self.fail_delete.lock().unwrap().clone() {
            return Err(e);
        }
        self.entries.lock().unwrap().remove(account);
        Ok(())
    }
}
struct Rig {
    _dir: tempfile::TempDir,
    db: Arc<Mutex<Catalog>>,
    keychain: Arc<FakeKeychain>,
    connects: Arc<AtomicUsize>,
    providers: Providers,
}
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Mutex::new(Catalog::open(&dir.path().join("c.sqlite")).unwrap()));
    let keychain = Arc::new(FakeKeychain::default());
    let connects = Arc::new(AtomicUsize::new(0));
    let providers = counting(&db, &keychain, &connects);
    Rig { _dir: dir, db, keychain, connects, providers }
}
fn counting(db: &Arc<Mutex<Catalog>>, keychain: &Arc<FakeKeychain>, connects: &Arc<AtomicUsize>) -> Providers {
    let count = connects.clone();
    Providers::with_connector(db.clone(), keychain.clone(), Arc::new(move |p, k| {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(HttpAdapter::with_timeout(p, k, Duration::from_secs(5))?) as Box<dyn ProviderAdapter>)
    }))
}
fn draft(kind: ProviderKind, base: Option<&str>, local: bool) -> ProviderDraft {
    ProviderDraft { kind, label: "Studio".into(), base_url: base.map(str::to_owned), model: "vision-model".into(), allow_local: local }
}

#[test]
fn configuring_providers_and_keys_makes_no_requests_and_never_stores_keys_in_the_catalog() {
    let r = rig();
    let created = r.providers.create(&draft(ProviderKind::Openai, None, false)).unwrap();
    assert_eq!(created.profile.base_url, "https://api.openai.com/v1");
    assert_eq!((created.locality, created.has_key, created.profile.revision), (Some(Locality::Remote), false, 1));
    let keyed = r.providers.set_key(&created.profile.id, 1, &format!("  {KEY} "), StoreChoice::Keychain).unwrap();
    assert!(keyed.has_key && keyed.profile.key_storage == KeyStorage::Keychain);
    assert_eq!(keyed.profile.key_origin.as_deref(), Some("https://api.openai.com"));
    assert_eq!(r.keychain.get(&created.profile.id).unwrap().unwrap().expose(), KEY, "trimmed key in the keychain");
    r.providers.list().unwrap();
    // Restart: a new service lists profiles without touching keychain or network.
    let restarted = counting(&r.db, &r.keychain, &r.connects);
    assert_eq!(restarted.list().unwrap().len(), 1);
    assert_eq!(r.connects.load(Ordering::SeqCst), 0, "no provider request without an explicit test");

    // The key is absent from SQLite, diagnostics, portable export, serialized views and Debug output.
    let c = r.db.lock().unwrap();
    let mut values = vec![];
    let mut statement = c.db_connection().prepare("SELECT value FROM app_settings").unwrap();
    for value in statement.query_map([], |row| row.get::<_, String>(0)).unwrap() {
        values.push(value.unwrap());
    }
    drop(statement);
    assert!(values.iter().any(|v| v.contains("api.openai.com")) && values.iter().all(|v| !v.contains(KEY)));
    assert!(!c.support_report(true).unwrap().contains(KEY));
    assert!(!c.export_portable().unwrap().json().unwrap().contains("api.openai.com"), "provider settings are machine-local");
    drop(c);
    let file = std::fs::read(r._dir.path().join("c.sqlite")).unwrap();
    assert!(!file.windows(KEY.len()).any(|w| w == KEY.as_bytes()));
    assert!(!serde_json::to_string(&keyed).unwrap().contains(KEY));
    assert!(!format!("{keyed:?} {:?}", Secret::new(KEY).unwrap()).contains(KEY));
}

#[test]
fn rotation_session_keys_and_endpoint_moves_never_send_a_key_elsewhere() {
    let r = rig();
    let p = r.providers.create(&draft(ProviderKind::Openai, None, false)).unwrap().profile;
    let p = r.providers.set_key(&p.id, 1, KEY, StoreChoice::Keychain).unwrap().profile;
    let p = r.providers.set_key(&p.id, 2, "sk-rotated-key-000000", StoreChoice::Keychain).unwrap().profile;
    assert_eq!((p.revision, r.keychain.get(&p.id).unwrap().unwrap().expose()), (3, "sk-rotated-key-000000"));
    // Switching to session-only removes the keychain copy.
    let p = r.providers.set_key(&p.id, 3, KEY, StoreChoice::Session).unwrap().profile;
    assert!(r.keychain.get(&p.id).unwrap().is_none());
    assert!(r.providers.list().unwrap()[0].has_key);
    let restarted = counting(&r.db, &r.keychain, &r.connects);
    assert!(!restarted.list().unwrap()[0].has_key, "session keys end with the app");
    assert_eq!(restarted.adapter(&p.id, p.revision).unwrap_err(), ProviderError::KeyMissing);

    // A label change keeps the key; an endpoint origin change deletes it.
    let mut d = draft(ProviderKind::Openai, None, false);
    d.label = "Renamed".into();
    let p = r.providers.update(&p.id, p.revision, &d).unwrap();
    assert!(p.has_key && p.profile.revision == 5);
    let p = r.providers.update(&p.profile.id, 5, &draft(ProviderKind::Openai, Some("https://gateway.example.net/v1"), false)).unwrap();
    assert!(!p.has_key && p.profile.key_storage == KeyStorage::None);
    // Changing kind on the same origin also deletes the key.
    let p = r.providers.set_key(&p.profile.id, p.profile.revision, KEY, StoreChoice::Keychain).unwrap();
    let mut other = draft(ProviderKind::OpenaiCompatible, Some("https://gateway.example.net/v1"), false);
    other.model = "org/model".into();
    let p = r.providers.update(&p.profile.id, p.profile.revision, &other).unwrap();
    assert!(!p.has_key && r.keychain.get(&p.profile.id).unwrap().is_none());

    // Stale revisions conflict instead of overwriting.
    assert_eq!(r.providers.clear_key(&p.profile.id, 1).unwrap_err(), ProviderError::Conflict);
    r.providers.delete(&p.profile.id, p.profile.revision).unwrap();
    assert!(r.providers.list().unwrap().is_empty());
}

#[test]
fn keychain_failures_leave_settings_unchanged_and_restored_backups_need_the_key_again() {
    let r = rig();
    let p = r.providers.create(&draft(ProviderKind::Anthropic, None, false)).unwrap().profile;
    *r.keychain.fail_set.lock().unwrap() = Some(ProviderError::KeychainUnavailable("no secret service".into()));
    assert_eq!(r.providers.set_key(&p.id, 1, KEY, StoreChoice::Keychain).unwrap_err().code(), "KEYCHAIN_UNAVAILABLE");
    let listed = &r.providers.list().unwrap()[0];
    assert_eq!((listed.has_key, listed.profile.revision), (false, 1));
    // Session-only remains available when the keychain is not.
    let p = r.providers.set_key(&p.id, 1, KEY, StoreChoice::Session).unwrap().profile;
    *r.keychain.fail_set.lock().unwrap() = None;
    let p = r.providers.set_key(&p.id, p.revision, KEY, StoreChoice::Keychain).unwrap().profile;
    *r.keychain.fail_delete.lock().unwrap() = Some(ProviderError::KeychainDenied);
    assert_eq!(r.providers.delete(&p.id, p.revision).unwrap_err(), ProviderError::KeychainDenied);
    assert_eq!(r.providers.list().unwrap().len(), 1, "profile kept so the key is not orphaned");
    *r.keychain.fail_delete.lock().unwrap() = None;
    // A restored settings row whose keychain entry is absent (new machine) asks for the key.
    r.keychain.entries.lock().unwrap().clear();
    assert_eq!(r.providers.adapter(&p.id, p.revision).unwrap_err(), ProviderError::KeyMissing);
    assert_eq!(r.connects.load(Ordering::SeqCst), 0);
}

#[test]
fn connection_test_records_capabilities_only_for_the_tested_revision() {
    let r = rig();
    let server = provider(ProviderKind::OpenaiCompatible);
    // Local servers need the opt-in; with it they may run without a key.
    let mut d = draft(ProviderKind::OpenaiCompatible, Some(&server.url("/v1")), false);
    d.model = "org/vision-model".into();
    assert_eq!(r.providers.create(&d).unwrap_err(), ProviderError::LocalNotPermitted);
    d.allow_local = true;
    let p = r.providers.create(&d).unwrap().profile;
    let report = r.providers.test(&p.id, p.revision, true).unwrap();
    let caps = &report.provider.profile.capabilities;
    assert_eq!((caps.image_input, caps.structured_output, caps.embeddings), (CapabilityState::Supported, CapabilityState::Supported, CapabilityState::Supported));
    assert_eq!(caps.tested_revision, Some(p.revision));
    assert_eq!(report.provider.locality, Some(Locality::Loopback));
    assert!(server.requests().iter().all(|s| !s.headers.contains_key("authorization")), "no key, no auth header");
    // Any configuration change clears tested capabilities.
    let p = r.providers.set_key(&p.id, p.revision, KEY, StoreChoice::Session).unwrap().profile;
    assert_eq!(p.capabilities, Capabilities::default());

    // A change while a test runs fences its result.
    let db = r.db.clone();
    let id = p.id.clone();
    let racing = Server::json(move |s| {
        if s.method == "GET" {
            db.lock().unwrap().db_connection().execute("UPDATE app_settings SET value=json_set(value,'$.profiles[0].revision',json_extract(value,'$.profiles[0].revision')+1) WHERE key='ai_providers'", []).unwrap();
            (200, json!({"data": [{"id": "org/vision-model"}]}))
        } else {
            (200, json!({"choices": [{"message": {"content": "red"}}]}))
        }
    });
    let p = r.providers.update(&id, p.revision, &ProviderDraft { base_url: Some(racing.url("/v1")), ..d.clone() }).unwrap().profile;
    assert_eq!(r.providers.test(&p.id, p.revision, false).unwrap_err(), ProviderError::Conflict);
    assert_eq!(r.providers.list().unwrap()[0].profile.capabilities.tested_revision, None);
}

#[test]
fn settings_are_bounded_and_newer_formats_fail_clearly() {
    let r = rig();
    for _ in 0..MAX_PROFILES {
        r.providers.create(&draft(ProviderKind::Gemini, None, false)).unwrap();
    }
    assert!(matches!(r.providers.create(&draft(ProviderKind::Gemini, None, false)), Err(ProviderError::InvalidSettings(_))));
    let mut bad = draft(ProviderKind::Gemini, None, false);
    bad.model = "models/gemini-test".into();
    assert_eq!(r.providers.update(&r.providers.list().unwrap()[0].profile.id, 1, &bad).unwrap().profile.model, "gemini-test");
    for model in ["", "a b", "../x", "x/y"] {
        bad.model = model.into();
        assert!(r.providers.create(&bad).is_err(), "{model}");
    }
    assert!(Secret::new("short").is_err() && Secret::new("has space in key").is_err());
    r.db.lock().unwrap().db_connection().execute("UPDATE app_settings SET value='{\"version\":2,\"profiles\":[]}' WHERE key='ai_providers'", []).unwrap();
    assert!(r.providers.list().unwrap_err().to_string().contains("newer CreativeShelf"));
}
