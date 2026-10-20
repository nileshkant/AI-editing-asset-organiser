//! SS-050: the qualified image-AI path. Untrusted model output, pixel
//! preparation, embedding spaces, feature gating, the generated evaluation
//! corpus and the bounded runner.
use serde_json::{json, Value};
use soundshelf_core::{
    ai_provider::*,
    ai_provider_wire::{self as wire, GenerateRequest, GenerateResponse, HttpAdapter, ModelInfo, ProviderAdapter, Usage},
    image_ai::*,
    image_contracts::{ConfidenceKind, Coverage, Origin, ReviewState},
    image_eval::{self, *},
};
use std::{
    collections::HashSet,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

mod common;
use common::http::Server;

fn answer(caption: &str, tags: &[&str]) -> Value {
    json!({"caption": caption, "tags": tags, "text_in_image": "", "uncertain": false})
}

#[test]
fn model_output_is_validated_bounded_and_never_truncated() {
    let ok = parse_insight(&json!({"caption": " A red\ncircle\ton white. ", "tags": ["Red", "red", "  circle ", "", "flat_design"], "text_in_image": "", "uncertain": false})).unwrap();
    assert_eq!(ok.caption.as_deref(), Some("A red circle on white."));
    assert_eq!(ok.tags, ["Red", "circle", "flat design"], "normalized like user tags, duplicates by identity removed");
    assert_eq!(ok.text_in_image, None);

    let bad: &[(Value, &str)] = &[
        (json!("just text"), "not an object"),
        (json!([1, 2]), "array"),
        (json!({"tags": [], "text_in_image": "", "uncertain": false}), "missing caption"),
        (json!({"caption": "x", "text_in_image": "", "uncertain": false}), "missing tags"),
        (json!({"caption": "x", "tags": [], "uncertain": false}), "missing text"),
        (json!({"caption": "x", "tags": [], "text_in_image": ""}), "missing uncertain"),
        (json!({"caption": 3, "tags": [], "text_in_image": "", "uncertain": false}), "wrong type"),
        (json!({"caption": "x", "tags": [1], "text_in_image": "", "uncertain": false}), "non-string tag"),
        (json!({"caption": "x", "tags": [], "text_in_image": "", "uncertain": "no"}), "string bool"),
        (json!({"caption": "x\u{0007}", "tags": [], "text_in_image": "", "uncertain": false}), "control character"),
        (json!({"caption": "x", "tags": [], "text_in_image": "", "uncertain": false, "url": "https://evil"}), "unknown field"),
    ];
    for (v, why) in bad {
        assert!(matches!(parse_insight(v), Err(InsightError::Malformed(_))), "{why}");
    }
    let many: Vec<String> = (0..=MAX_INSIGHT_TAGS).map(|i| format!("t{i}")).collect();
    assert_eq!(parse_insight(&json!({"caption": "x", "tags": many, "text_in_image": "", "uncertain": false})), Err(InsightError::TooManyTags));
    let long = "a".repeat(MAX_CAPTION_CHARS + 1);
    assert_eq!(parse_insight(&json!({"caption": long, "tags": [], "text_in_image": "", "uncertain": false})), Err(InsightError::TooLong("caption")));
    let long_tag = "b".repeat(65);
    assert_eq!(parse_insight(&json!({"caption": "x", "tags": [long_tag], "text_in_image": "", "uncertain": false})), Err(InsightError::TooLong("tags")));
    assert_eq!(InsightError::TooManyTags.code(), "MODEL_OUTPUT_INVALID");
}

#[test]
fn prepared_pixels_are_bounded_oriented_and_carry_no_metadata() {
    let big = image::RgbaImage::from_fn(3000, 1500, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, if x < 100 { 0 } else { 255 }]));
    let p = prepare_pixels(&big).unwrap();
    assert_eq!((p.width, p.height, p.source_width, p.source_height), (1024, 512, 3000, 1500));
    assert_eq!(p.coverage(), Coverage::Preview);
    assert_eq!(p.part.media_type, "image/png");
    let decoded = image::load_from_memory(&p.part.bytes).unwrap().to_rgba8();
    assert_eq!(decoded.dimensions(), (1024, 512));
    assert_eq!(decoded.get_pixel(0, 0).0[3], 0, "alpha is preserved");
    // Only structural PNG chunks are written: no text, EXIF, time or color profile.
    let mut chunks = vec![];
    let mut at = 8;
    while at + 8 <= p.part.bytes.len() {
        let len = u32::from_be_bytes(p.part.bytes[at..at + 4].try_into().unwrap()) as usize;
        chunks.push(String::from_utf8_lossy(&p.part.bytes[at + 4..at + 8]).into_owned());
        at += 12 + len;
    }
    assert!(chunks.iter().all(|c| ["IHDR", "IDAT", "IEND"].contains(&c.as_str())), "{chunks:?}");

    let small = image::RgbaImage::from_pixel(300, 200, image::Rgba([1, 2, 3, 255]));
    let p = prepare_pixels(&small).unwrap();
    assert_eq!((p.width, p.height), (300, 200), "never upscaled");
    assert_eq!(p.coverage(), Coverage::FullImage);
    assert!(format!("{p:?}").len() < 400, "Debug does not dump bytes");
    assert_eq!(prepare_pixels(&image::RgbaImage::new(0, 0)).unwrap_err(), PrepareError::Empty);
}

/// In-process adapter with scripted answers and a request counter.
type Answer = Box<dyn Fn(&GenerateRequest) -> Result<GenerateResponse, ProviderError> + Send + Sync>;
type Embed = Box<dyn Fn(&[String]) -> Result<wire::Embeddings, ProviderError> + Send + Sync>;
struct Scripted {
    answer: Answer,
    embed: Embed,
    calls: AtomicUsize,
}
impl std::fmt::Debug for Scripted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Scripted")
    }
}
impl ProviderAdapter for Scripted {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Openai
    }
    fn model(&self) -> Result<ModelInfo, ProviderError> {
        Ok(ModelInfo { id: "scripted".into(), version: None })
    }
    fn generate(&self, r: &GenerateRequest) -> Result<GenerateResponse, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.answer)(r)
    }
    fn embed(&self, inputs: &[String]) -> Result<wire::Embeddings, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.embed)(inputs)
    }
}
fn scripted(answer: impl Fn(&GenerateRequest) -> Result<GenerateResponse, ProviderError> + Send + Sync + 'static) -> Scripted {
    Scripted { answer: Box::new(answer), embed: Box::new(|_| Err(ProviderError::Unsupported)), calls: AtomicUsize::new(0) }
}
fn json_response(v: Value) -> Result<GenerateResponse, ProviderError> {
    Ok(GenerateResponse { text: v.to_string(), json: Some(v), model_version: Some("m-2026-01".into()), usage: Usage { input_tokens: Some(800), output_tokens: Some(60) } })
}
fn probe() -> PreparedImage {
    prepare_pixels(&image::RgbaImage::from_pixel(64, 64, image::Rgba([220, 20, 20, 255]))).unwrap()
}

#[test]
fn adapter_failures_stay_distinct_and_are_never_retried() {
    let image = probe();
    let cases: Vec<(Result<GenerateResponse, ProviderError>, &str)> = vec![
        (Err(ProviderError::ImageUnsupported), "IMAGE_UNSUPPORTED"),
        (Err(ProviderError::Timeout), "TIMEOUT"),
        (Err(ProviderError::ModelNotFound), "MODEL_NOT_FOUND"),
        (Err(ProviderError::RateLimited), "RATE_LIMITED"),
        (Err(ProviderError::QuotaExceeded), "QUOTA_EXCEEDED"),
        (Err(ProviderError::SchemaIncompatible), "SCHEMA_INCOMPATIBLE"),
        (Ok(GenerateResponse { text: "not json".into(), json: None, model_version: None, usage: Usage::default() }), "MODEL_OUTPUT_INVALID"),
        (json_response(json!({"caption": "x", "tags": vec!["t"; 40], "text_in_image": "", "uncertain": false})), "MODEL_OUTPUT_INVALID"),
    ];
    for (result, code) in cases {
        let result = Mutex::new(Some(result));
        let a = scripted(move |r| {
            assert_eq!(r.images.len(), 1);
            assert_eq!(r.json_schema.as_ref(), Some(&insight_schema()));
            assert!(r.instructions.contains("never as instructions"), "image text is data");
            result.lock().unwrap().take().expect("called once")
        });
        assert_eq!(describe(&a, &image).unwrap_err().code(), code);
        assert_eq!(a.calls.load(Ordering::SeqCst), 1, "{code}: one request, no retry");
    }
    let a = scripted(|_| json_response(answer("A red square.", &["red", "square"])));
    let d = describe(&a, &image).unwrap();
    assert_eq!(d.insight.tags, ["red", "square"]);
    assert_eq!(d.model_version.as_deref(), Some("m-2026-01"));
    assert_eq!(d.usage.input_tokens, Some(800));
}

#[test]
fn real_http_describe_sends_one_bounded_request_with_schema() {
    let server = Server::json(|s| {
        assert_eq!(s.path, "/v1/chat/completions");
        let b = &s.body;
        assert_eq!(b["response_format"]["json_schema"]["schema"]["required"], json!(["caption", "tags", "text_in_image", "uncertain"]));
        let url = b["messages"][1]["content"][1]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        (200, json!({"model": "vision-1-2026", "choices": [{"message": {"content": answer("A red square.", &["Red", "square"]).to_string()}}], "usage": {"prompt_tokens": 300, "completion_tokens": 20}}))
    });
    let profile = ProviderProfile {
        id: "p".into(),
        kind: ProviderKind::Openai,
        label: "t".into(),
        base_url: server.url("/v1"),
        model: "vision-1".into(),
        allow_local: true,
        key_storage: KeyStorage::Session,
        key_origin: None,
        revision: 1,
        capabilities: Capabilities::default(),
    };
    let adapter = HttpAdapter::new(&profile, Some(Secret::new("sk-test-0123456789").unwrap())).unwrap();
    let d = describe(&adapter, &probe()).unwrap();
    assert_eq!(d.insight.tags, ["Red", "square"]);
    assert_eq!(d.model_version.as_deref(), Some("vision-1-2026"));
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn evidence_is_a_suggestion_without_invented_confidence() {
    let insight = parse_insight(&answer("A red circle.", &["red", "circle"])).unwrap();
    let hash = "a".repeat(64);
    let m = model_ref(ProviderKind::Gemini, "vision", Some("vision-002"));
    assert_eq!((m.provider.as_str(), m.revision.as_str(), m.preprocessing.as_str(), m.output_schema.as_str()), ("gemini", "vision-002", PREPROCESSING_ID, INSIGHT_SCHEMA_ID));
    let e = evidence(&insight, &hash, Coverage::Preview, m, 1);
    assert_eq!((e.origin, e.review_state, e.confidence_kind, e.confidence), (Origin::Model, ReviewState::Suggested, ConfidenceKind::Unavailable, None));
    assert_eq!(e.tags, ["red", "circle"]);
    let unsure = Insight { uncertain: true, ..insight };
    assert!(evidence(&unsure, &hash, Coverage::Preview, model_ref(ProviderKind::Openai, "x", None), 1).tags.is_empty(), "uncertain answers suggest no tags");
}

#[test]
fn embedding_spaces_never_mix_and_unrelated_queries_return_nothing() {
    let space = EmbeddingSpace::new(ProviderKind::Openai, "embed-3", Some("embed-3-2026"), 3);
    assert_eq!(space.id(), "openai/embed-3@embed-3-2026#3/l2/insight-text/v1");
    let mut index = EmbeddingIndex::new(space.clone());
    index.add("red", &space, &[1.0, 0.0, 0.0]).unwrap();
    index.add("blue", &space, &[0.0, 2.0, 0.0]).unwrap();
    index.add("purple", &space, &[0.7, 0.7, 0.0]).unwrap();
    let hits = index.search(&space, &[0.9, 0.1, 0.0], 10, 0.0).unwrap();
    assert_eq!(hits.iter().map(|h| h.0.as_str()).collect::<Vec<_>>(), ["red", "purple", "blue"]);
    assert!(index.search(&space, &[0.0, 0.0, 1.0], 10, 0.3).unwrap().is_empty(), "out of domain");
    let other_version = EmbeddingSpace::new(ProviderKind::Openai, "embed-3", Some("embed-3-2027"), 3);
    assert_eq!(index.add("x", &other_version, &[1.0, 0.0, 0.0]), Err(EmbeddingError::SpaceMismatch));
    assert_eq!(index.search(&other_version, &[1.0, 0.0, 0.0], 1, 0.0), Err(EmbeddingError::SpaceMismatch));
    assert_eq!(index.add("x", &space, &[1.0, 0.0]), Err(EmbeddingError::SpaceMismatch), "dimension");
    assert_eq!(index.add("x", &space, &[0.0, 0.0, 0.0]), Err(EmbeddingError::InvalidVector));
    assert_eq!(index.add("x", &space, &[f32::NAN, 0.0, 0.0]), Err(EmbeddingError::InvalidVector));
    let v = l2_normalize(&[3.0, 4.0]).unwrap();
    assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
}

fn view(id: &str, kind: ProviderKind, has_key: bool, image: CapabilityState, structured: CapabilityState, embeddings: CapabilityState) -> ProviderView {
    ProviderView {
        profile: ProviderProfile {
            id: id.into(),
            kind,
            label: id.to_uppercase(),
            base_url: "https://example.test/v1".into(),
            model: "m".into(),
            allow_local: false,
            key_storage: if has_key { KeyStorage::Keychain } else { KeyStorage::None },
            key_origin: None,
            revision: 1,
            capabilities: Capabilities { image_input: image, structured_output: structured, embeddings, ..Capabilities::default() },
        },
        locality: None,
        has_key,
    }
}

#[test]
fn features_stay_visible_and_say_what_they_need() {
    use CapabilityState::*;
    let state = |views: &[ProviderView]| feature_status(views).into_iter().map(|s| (s.feature, s.state)).collect::<Vec<_>>();
    let none = state(&[]);
    assert_eq!(none[0], (AiFeature::ImageDescriptions, FeatureState::NeedsProvider));
    assert_eq!(none[1], (AiFeature::SemanticSearch, FeatureState::NeedsProvider));
    assert_eq!(none[2], (AiFeature::OfflineTagging, FeatureState::NotOffered), "offline tagging is never advertised");

    let keyless = state(&[view("a", ProviderKind::Openai, false, Untested, Untested, Untested)]);
    assert_eq!(keyless[0].1, FeatureState::NeedsKey { provider_id: "a".into(), provider_label: "A".into() });

    let untested = state(&[view("a", ProviderKind::Openai, true, Untested, Untested, Untested)]);
    assert_eq!(untested[0].1, FeatureState::NeedsTest { provider_id: "a".into(), provider_label: "A".into() });

    // Text-only models never unlock image features.
    let text_only = state(&[view("a", ProviderKind::Openai, true, Unsupported, Supported, Supported)]);
    assert_eq!(text_only[0].1, FeatureState::Unsupported);
    let inconclusive = state(&[view("a", ProviderKind::Openai, true, Inconclusive, Supported, Untested)]);
    assert_eq!(inconclusive[0].1, FeatureState::NeedsTest { provider_id: "a".into(), provider_label: "A".into() });

    // Anthropic describes; an OpenAI profile adds embeddings for search.
    let mixed = state(&[view("claude", ProviderKind::Anthropic, true, Supported, Supported, Unsupported), view("oa", ProviderKind::Openai, true, Unsupported, Supported, Supported)]);
    assert_eq!(mixed[0].1, FeatureState::Ready { provider_id: "claude".into(), provider_label: "CLAUDE".into() });
    assert_eq!(mixed[1].1, FeatureState::Ready { provider_id: "oa".into(), provider_label: "OA".into() });
    let describe_only = state(&[view("claude", ProviderKind::Anthropic, true, Supported, Supported, Unsupported)]);
    assert_eq!(describe_only[1].1, FeatureState::Unsupported);

    // A local server without a key is usable.
    let local = state(&[view("local", ProviderKind::OpenaiCompatible, false, Supported, Supported, Untested)]);
    assert!(matches!(local[0].1, FeatureState::Ready { .. }));
    assert!(feature_status(&[]).iter().all(|s| s.quality == QualityState::PendingEvaluation));
    let wire = serde_json::to_value(feature_status(&[])).unwrap();
    assert_eq!(wire[0], json!({"feature": "image_descriptions", "state": "needs_provider", "quality": {"state": "pending_evaluation"}}));
}

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn generated_corpus_is_reproducible_and_matches_the_committed_manifest() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let m = generate_corpus(a.path()).unwrap();
    generate_corpus(b.path()).unwrap();
    let committed = std::fs::read_to_string(repository_eval_dir(repo()).join("corpus-v1.json")).unwrap();
    assert_eq!(std::fs::read_to_string(a.path().join("manifest.json")).unwrap(), committed, "regenerate eval/image-ai/corpus-v1.json only with a new corpus version");
    for item in &m.items {
        assert_eq!(std::fs::read(a.path().join(&item.file)).unwrap(), std::fs::read(b.path().join(&item.file)).unwrap());
        read_verified(a.path(), item).unwrap();
    }
    let categories: HashSet<&str> = m.items.iter().map(|i| i.category.as_str()).collect();
    for c in ["watercolor", "collage", "3d", "screenshot", "text", "transparent", "multiple", "crop", "ambiguous"] {
        assert!(m.items.iter().filter(|i| i.category == c).count() >= 4, "{c}");
        assert!(m.items.iter().any(|i| i.category == c && i.split == Split::Heldout), "{c} has held-out items");
    }
    assert!(categories.len() >= 10);
    assert_eq!(m.categories_pending, ["photo"]);
    assert!(m.queries.iter().any(|q| q.relevant.is_empty()), "out-of-domain queries");
    assert_eq!(m.languages, ["en"]);
    // Rendered text is legible in the glyph set used.
    assert!(render_item("ui-dialog").is_some());
}

#[test]
fn approved_inputs_only_tampered_or_unsafe_manifests_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let m = generate_corpus(dir.path()).unwrap();
    let first = &m.items[0];
    std::fs::write(dir.path().join(&first.file), b"changed").unwrap();
    assert!(matches!(read_verified(dir.path(), first), Err(EvalError::Digest(_))));
    for bad in ["../outside.png", "/etc/passwd", "a/../../b.png", "a\\b.png", ""] {
        let mut m2 = m.clone();
        m2.items[0].file = bad.into();
        assert!(m2.validate().is_err(), "{bad}");
    }
    let mut m3 = m.clone();
    m3.queries[0].relevant.push("missing".into());
    assert!(m3.validate().is_err());
    let mut m4 = m.clone();
    m4.schema = "creativeshelf-image-eval-manifest/v9".into();
    assert!(m4.validate().is_err());
}

#[test]
fn scoring_rules_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let m = generate_corpus(dir.path()).unwrap();
    let vocab = vocabulary(&m);
    let red = m.items.iter().find(|i| i.id == "flat-red-circle").unwrap();
    let s = score_tags(red, &["Red".into(), "circle".into(), "blue".into(), "minimalist".into()], &vocab);
    assert_eq!((s.judged, s.correct, s.unjudged, s.gold_facts, s.gold_found), (3, 2, 1, 2, 2));
    assert_eq!(s.precision(), Some(2.0 / 3.0));
    let c = score_caption(red, "A bright red disc centred on a white background, not a square.");
    assert_eq!((c.required, c.covered), (2, 2));
    assert_eq!(c.forbidden_found, ["square"], "the rubric is lexical; negations still count, which is conservative");
    assert!(mentions(&words("Three yellow stars"), "star"), "plural folded");
    assert!(!mentions(&words("redwood"), "red"), "word boundaries");
    let ranked = lexical_rank("red ball", &[("a".into(), "red-ball.png".into()), ("b".into(), "ball.png".into()), ("c".into(), "x.png".into())]);
    assert_eq!(ranked, ["a", "b"]);
}

#[test]
fn filename_baseline_is_measured_without_any_model() {
    let dir = tempfile::tempdir().unwrap();
    let m = generate_corpus(dir.path()).unwrap();
    let ranks = filename_baseline(&m);
    let pool = m.items.iter().filter(|i| i.split == Split::Heldout).map(|i| i.id.clone()).collect();
    let s = score_retrieval(&m.queries, &ranks, &pool);
    println!("held-out filename baseline: {}", serde_json::to_string(&s).unwrap());
    assert!(s.queries >= 15);
    assert!(s.mrr < 0.4, "file names alone are a weak baseline: {}", s.mrr);
    assert_eq!(s.out_of_domain_with_results, 0);
}

fn thresholds() -> Thresholds {
    serde_json::from_slice(&std::fs::read(repository_eval_dir(repo()).join("thresholds.json")).unwrap()).unwrap()
}
fn input<'a>(dir: &'a Path, m: &'a Manifest, t: &'a Thresholds, allowance: u32) -> RunInput<'a> {
    RunInput { dir, manifest: m, manifest_blake3: "m".into(), thresholds: t, thresholds_blake3: "t".into(), request_allowance: allowance }
}

/// Answers from the gold labels for each image, identified by its prepared
/// pixels. Used to show the runner and judge work end to end; it says nothing
/// about any real model.
fn oracle(m: &Manifest) -> Scripted {
    let by_pixels: Vec<(Vec<u8>, ManifestItem)> = m
        .items
        .iter()
        .map(|i| {
            let img = render_item(&i.id).unwrap();
            (prepare_pixels(&img).unwrap().part.bytes, i.clone())
        })
        .collect();
    let mut s = scripted(move |r| {
        let item = &by_pixels.iter().find(|(b, _)| *b == r.images[0].bytes).expect("only corpus images are sent").1;
        let tags: Vec<&str> = item.tags.iter().map(|t| t[0].as_str()).collect();
        let caption = item.caption_required.iter().map(|f| f[0].clone()).collect::<Vec<_>>().join(" ");
        json_response(json!({"caption": format!("Image showing {caption}."), "tags": tags, "text_in_image": "", "uncertain": false}))
    });
    // Bag-of-words vectors over a fixed vocabulary: a deterministic stand-in embedding.
    let vocab: Vec<String> = vocabulary(m).into_iter().flat_map(|v| words(&v)).collect::<HashSet<_>>().into_iter().collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    s.embed = Box::new(move |inputs| {
        Ok(wire::Embeddings {
            vectors: inputs
                .iter()
                .map(|t| {
                    let w = words(t);
                    let mut v: Vec<f32> = vocab.iter().map(|k| w.iter().filter(|x| *x == k).count() as f32).collect();
                    v.push(0.01);
                    v
                })
                .collect(),
            model_version: Some("bow-1".into()),
            usage: Usage { input_tokens: Some(10), output_tokens: None },
        })
    });
    s
}

#[test]
fn runner_scores_judges_and_respects_the_request_allowance() {
    let dir = tempfile::tempdir().unwrap();
    let m = generate_corpus(dir.path()).unwrap();
    let t = thresholds();

    // No provider: no requests, no verdict.
    let r = image_eval::run(&input(dir.path(), &m, &t, 100), None, None).unwrap();
    assert_eq!((r.requests, r.verdict.clone()), (0, None));

    let a = oracle(&m);
    let e = oracle(&m);
    let d = Describer { adapter: &a, kind: ProviderKind::Openai, model: "oracle".into() };
    let em = Embedder { adapter: &e, kind: ProviderKind::Openai, model: "bow".into() };
    let r = image_eval::run(&input(dir.path(), &m, &t, 100), Some(&d), Some(&em)).unwrap();
    assert!(r.complete);
    assert_eq!(a.calls.load(Ordering::SeqCst), m.items.len(), "one request per image");
    assert_eq!(r.requests as usize, m.items.len() + 1, "plus one batched embedding call");
    assert_eq!(r.heldout.tag_precision, Some(1.0));
    assert_eq!(r.heldout.tag_recall, Some(1.0));
    assert!(r.embedding_space.as_deref().unwrap().starts_with("openai/bow@bow-1#"));
    assert!(r.similarity_cutoff.is_some());
    assert!(r.heldout.insight_lexical_retrieval.mrr > r.heldout.filename_retrieval.mrr);
    println!("oracle held-out: {}", serde_json::to_string(&r.heldout).unwrap());
    assert_eq!(r.verdict.as_ref().map(|v| v.failures.clone()), Some(vec![]), "the thresholds are reachable");
    let report = serde_json::to_string(&r).unwrap();
    assert!(!report.contains("sk-"), "no secrets in reports");

    // A model that names the wrong colors fails the pre-registered thresholds.
    let wrong = scripted(|_| json_response(answer("A blue square on green.", &["blue", "square", "green"])));
    let d = Describer { adapter: &wrong, kind: ProviderKind::Openai, model: "wrong".into() };
    let r = image_eval::run(&input(dir.path(), &m, &t, 100), Some(&d), None).unwrap();
    let v = r.verdict.unwrap();
    assert!(!v.passed);
    assert!(v.failures.iter().any(|f| f.starts_with("tag precision")), "{:?}", v.failures);

    // The allowance is a hard stop and an incomplete run never passes.
    let a = oracle(&m);
    let d = Describer { adapter: &a, kind: ProviderKind::Openai, model: "oracle".into() };
    let r = image_eval::run(&input(dir.path(), &m, &t, 5), Some(&d), None).unwrap();
    assert_eq!((r.requests, a.calls.load(Ordering::SeqCst), r.complete), (5, 5, false));
    assert!(r.items.iter().filter(|o| o.error.as_deref() == Some("REQUEST_ALLOWANCE_EXHAUSTED")).count() == m.items.len() - 5);
    assert!(r.verdict.unwrap().failures.iter().any(|f| f.contains("allowance")));

    // Provider failures are recorded per item, not retried.
    let limited = scripted(|_| Err(ProviderError::RateLimited));
    let d = Describer { adapter: &limited, kind: ProviderKind::Openai, model: "x".into() };
    let r = image_eval::run(&input(dir.path(), &m, &t, 100), Some(&d), None).unwrap();
    assert_eq!(limited.calls.load(Ordering::SeqCst), m.items.len());
    assert!(r.items.iter().all(|o| o.error.as_deref() == Some("RATE_LIMITED")));
    assert!(!r.verdict.unwrap().passed);
}

/// Release-mode measurement for the review: preparing a 24-megapixel image.
/// `cargo test --release -p soundshelf-core --test image_ai -- --ignored --nocapture`
#[test]
#[ignore]
fn measure_preparation_cost() {
    let big = image::RgbaImage::from_fn(6000, 4000, |x, y| image::Rgba([(x * 7 % 256) as u8, (y * 3 % 256) as u8, ((x ^ y) % 256) as u8, 255]));
    let mut times = vec![];
    let mut bytes = 0;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let p = prepare_pixels(&big).unwrap();
        times.push(t.elapsed().as_millis());
        bytes = p.part.bytes.len();
    }
    times.sort();
    println!("prepare 6000x4000 -> 1024x683: median {} ms, max {} ms, {} bytes", times[2], times[4], bytes);
    let dir = tempfile::tempdir().unwrap();
    let t = std::time::Instant::now();
    generate_corpus(dir.path()).unwrap();
    println!("generate corpus: {} ms", t.elapsed().as_millis());
}
