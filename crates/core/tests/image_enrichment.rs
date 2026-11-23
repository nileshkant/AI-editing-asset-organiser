//! SS-052: consented image enrichment over real catalogs and the real image
//! worker. The provider is a scripted dispatcher that counts and records every
//! request, so "nothing was sent" is observed, not assumed. One test drives the
//! real provider adapter against a loopback server to see the wire.
mod common;
use image::{codecs::png, ImageEncoder};
use soundshelf_core::{
    ai_provider_wire::Usage,
    catalog::Catalog,
    image_ai::{prepare_pixels, DescribeError, Described, Insight, InsightError, PreparedImage},
    image_enrichment::*,
    image_import::ImageImportEngine,
    image_import_preview::{MediaImportPreviews, NativeSelectionKind},
    image_jobs::ImportMode,
    ai_provider::ProviderError,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, atomic::{AtomicUsize, Ordering}, Arc, Mutex},
};
static SERIAL: Mutex<()> = Mutex::new(());

fn bytes(size: u32, shift: u8) -> Vec<u8> {
    let pixels: Vec<u8> = (0..size * size)
        .flat_map(|i| {
            let (x, y) = ((i % size) as u8, (i / size) as u8);
            [(x * 30).wrapping_add(shift), y * 30, if x == y { 255 } else { 0 }, 255]
        })
        .collect();
    let mut out = vec![];
    png::PngEncoder::new(&mut out).write_image(&pixels, size, size, image::ExtendedColorType::Rgba8).unwrap();
    out
}
fn put(dir: &Path, name: &str, shift: u8) {
    fs::write(dir.join(name), bytes(8, shift)).unwrap();
}

const PROVIDER: &str = "prov-1";
fn profile_json(id: &str, revision: u64, model: &str, ready: bool) -> serde_json::Value {
    serde_json::json!({
        "id": id, "kind": "openai_compatible", "label": "Local", "base_url": "http://127.0.0.1:1/v1", "model": model,
        "allow_local": true, "key_storage": "none", "key_origin": null, "revision": revision,
        "capabilities": {
            "image_input": if ready { "supported" } else { "untested" }, "structured_output": if ready { "supported" } else { "untested" },
            "embeddings": "untested", "tested_revision": if ready { serde_json::json!(revision) } else { serde_json::Value::Null },
            "tested_at": 1, "model_version": null
        }
    })
}
/// Writes provider settings directly: the settings store is what the real
/// service reads, and the connection test itself is covered in ai_provider.rs.
fn set_profiles(db: &rusqlite::Connection, profiles: Vec<serde_json::Value>) {
    let value = serde_json::json!({"version": 1, "profiles": profiles}).to_string();
    db.execute("INSERT INTO app_settings(key,value) VALUES('ai_providers',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [value]).unwrap();
}

struct Fixture {
    temp: tempfile::TempDir,
    db: Arc<Mutex<Catalog>>,
    previews: MediaImportPreviews,
    engine: ImageImportEngine,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(Mutex::new(Catalog::open(&temp.path().join("db.sqlite")).unwrap()));
        let engine = ImageImportEngine { catalog: db.clone(), worker: PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker")), tools: None };
        set_profiles(db.lock().unwrap().db_connection(), vec![profile_json(PROVIDER, 1, "vision-1", true)]);
        Self { temp, db, previews: MediaImportPreviews::default(), engine }
    }
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.temp.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    }
    fn import(&self, root: &Path) -> String {
        let raw = self.previews.approve_native(&[root.to_owned()], NativeSelectionKind::Folders).unwrap();
        let preview = self.previews.prepare(&self.db, &uuid::Uuid::new_v4().to_string(), raw, ImportMode::Images).unwrap();
        let job = self.previews.confirm(&self.db, &preview.id).unwrap()[0].clone();
        for _ in 0..400 {
            if self.db.lock().unwrap().image_import_job(&job.id).unwrap().terminal() {
                return job.source_id;
            }
            self.engine.tick("test", &AtomicBool::new(false)).unwrap();
        }
        panic!("import did not finish")
    }
    fn ids(&self, source: &str) -> Vec<String> {
        let c = self.db.lock().unwrap();
        let mut q = c.db_connection().prepare("SELECT id FROM asset_registry WHERE source_id=?1 ORDER BY relative_path").unwrap();
        let rows = q.query_map([source], |r| r.get(0)).unwrap();
        rows.collect::<std::result::Result<_, _>>().unwrap()
    }
    fn sql<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.db.lock().unwrap().db_connection().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn exec(&self, sql: &str) {
        // Tests rewrite columns that other tables reference (content hashes).
        self.db.lock().unwrap().db_connection().execute_batch(&format!("PRAGMA foreign_keys=OFF; {sql}; PRAGMA foreign_keys=ON;")).unwrap();
    }
    fn consent(&self, source: &str, analyze_new: bool) -> std::result::Result<Consent, EnrichError> {
        self.db.lock().unwrap().grant_enrichment_consent(&ConsentRequest { source_id: source.into(), provider_id: PROVIDER.into(), provider_revision: 1, analyze_new })
    }
    fn start(&self, source: &str, requests: i64, units: i64) -> std::result::Result<JobView, EnrichError> {
        self.db.lock().unwrap().start_enrichment(&StartRequest {
            source_id: source.into(), provider_id: PROVIDER.into(), provider_revision: 1, kind: Kind::Backfill, asset_ids: None, max_requests: requests, max_units: units,
        })
    }
    fn job(&self, id: &str) -> JobView {
        self.db.lock().unwrap().enrichment_job(id).unwrap()
    }
    fn outputs(&self) -> i64 {
        self.sql("SELECT count(*) FROM image_enrichment_outputs")
    }
}

type Script = Box<dyn Fn(usize) -> std::result::Result<Described, DescribeError> + Send + Sync>;
struct Fake {
    calls: AtomicUsize,
    seen: Mutex<Vec<(String, u64)>>,
    script: Script,
}
impl Fake {
    fn new(script: impl Fn(usize) -> std::result::Result<Described, DescribeError> + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { calls: AtomicUsize::new(0), seen: Mutex::new(vec![]), script: Box::new(script) })
    }
    fn ok() -> Arc<Self> {
        Self::new(|_| Ok(described(Some((100, 40)))))
    }
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Dispatcher for Fake {
    fn describe(&self, provider_id: &str, revision: u64, _: &PreparedImage) -> std::result::Result<Described, DescribeError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push((provider_id.into(), revision));
        (self.script)(n)
    }
}
fn described(tokens: Option<(u64, u64)>) -> Described {
    Described {
        insight: Insight { caption: Some("A red fox".into()), tags: vec!["fox".into()], text_in_image: None, uncertain: false, visual_tags: vec![
            soundshelf_core::image_contracts::VisualTag { category: soundshelf_core::image_contracts::VisualCategory::Subject, label: "fox".into() },
        ] },
        model_version: Some("vision-1-2026".into()),
        usage: Usage { input_tokens: tokens.map(|t| t.0), output_tokens: tokens.map(|t| t.1) },
    }
}
struct Pixels;
impl PixelSource for Pixels {
    fn prepare(&self, _: &str, _: &str) -> std::result::Result<PreparedImage, PixelError> {
        prepare_pixels(&image::RgbaImage::from_pixel(16, 16, image::Rgba([200, 60, 20, 255]))).map_err(|_| PixelError::Failed)
    }
}
fn runner(f: &Fixture, fake: &Arc<Fake>) -> Runner {
    Runner { catalog: f.db.clone(), pixels: Arc::new(Pixels), dispatcher: fake.clone(), busy: Arc::new(|| false), owner: "test".into() }
}
fn drain(r: &Runner) -> usize {
    let mut worked = 0;
    for _ in 0..200 {
        match r.step().unwrap() {
            Step::Worked => worked += 1,
            Step::Idle | Step::Yielded => break,
        }
    }
    worked
}

#[test]
fn obsolete_description_jobs_never_dispatch_with_the_new_prompt() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let root = f.folder("schema-change");
    put(&root, "image.png", 1);
    let source = f.import(&root);
    f.consent(&source, false).unwrap();
    let old = f.start(&source, 1, 100_000).unwrap();
    f.exec("UPDATE image_enrichment_jobs SET schema_id='creativeshelf.image-insight/v2'");
    let fake = Fake::ok();
    let run = runner(&f, &fake);
    drain(&run);
    assert_eq!(fake.count(), 0);
    assert_eq!(f.job(&old.id).error_code.as_deref(), Some("ANALYSIS_SCHEMA_CHANGED"));
    let current = f.start(&source, 1, 100_000).unwrap();
    drain(&run);
    assert_eq!(fake.count(), 1);
    assert_eq!(f.job(&current.id).counts.done, 1);
}

// ------------------------------------------------------------------ consent

/// T1/R1/R2: nothing is sent before consent, for an unconsented source, or by
/// estimating; the disclosure says exactly what leaves the computer.
#[test]
fn nothing_is_sent_before_consent_and_only_for_consented_sources() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let (a, b) = (f.folder("a"), f.folder("b"));
    put(&a, "one.png", 1);
    put(&a, "two.png", 2);
    put(&b, "three.png", 3);
    let (sa, sb) = (f.import(&a), f.import(&b));
    let fake = Fake::ok();
    let r = runner(&f, &fake);
    // Estimating needs no consent and sends nothing.
    let estimate = f.db.lock().unwrap().estimate_enrichment(&EstimateRequest { source_id: sa.clone(), provider_id: PROVIDER.into(), provider_revision: 1, kind: Kind::Backfill, asset_ids: None }).unwrap();
    assert_eq!((estimate.to_send, estimate.reusable, estimate.estimated_units), (2, 0, 2 * EST_UNITS));
    assert!(estimate.cost.starts_with("unknown") && estimate.unit.contains("estimate"), "the estimate is labeled and no price is invented");
    assert_eq!(f.start(&sa, 10, 100_000).unwrap_err(), EnrichError::ConsentRequired);
    assert_eq!(drain(&r), 0);
    assert_eq!(fake.count(), 0, "zero requests before authorization");
    let d = disclosure();
    assert!(d.sends.contains("512 pixels") && d.never_sends.iter().any(|n| n.contains("file names")) && d.never_sends.iter().any(|n| n.contains("EXIF")));
    // Consent for one source covers that source only.
    f.consent(&sa, false).unwrap();
    assert_eq!(f.start(&sb, 10, 100_000).unwrap_err(), EnrichError::ConsentRequired);
    let job = f.start(&sa, 10, 100_000).unwrap();
    assert_eq!(drain(&r), 2);
    assert_eq!(fake.count(), 2);
    assert_eq!(f.job(&job.id).counts.done, 2);
    let sent_for_b: i64 = f.sql(&format!("SELECT count(*) FROM image_enrichment_items i JOIN asset_registry r ON r.id=i.asset_id WHERE r.source_id='{sb}'"));
    assert_eq!(sent_for_b, 0, "the unconsented source was never queued");
}

#[test]
fn consent_needs_a_tested_provider_at_the_exact_revision() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    let sa = f.import(&a);
    set_profiles(f.db.lock().unwrap().db_connection(), vec![profile_json(PROVIDER, 1, "vision-1", false)]);
    assert_eq!(f.consent(&sa, false).unwrap_err(), EnrichError::ProviderNotReady, "an untested provider cannot be consented to");
    set_profiles(f.db.lock().unwrap().db_connection(), vec![profile_json(PROVIDER, 2, "vision-1", true)]);
    assert_eq!(f.consent(&sa, false).unwrap_err(), EnrichError::ProviderChanged, "a stale revision is refused");
    assert_eq!(
        f.db.lock().unwrap().grant_enrichment_consent(&ConsentRequest { source_id: "nope".into(), provider_id: PROVIDER.into(), provider_revision: 2, analyze_new: false }).unwrap_err(),
        EnrichError::SourceNotFound
    );
    let consent = f.db.lock().unwrap().grant_enrichment_consent(&ConsentRequest { source_id: sa, provider_id: PROVIDER.into(), provider_revision: 2, analyze_new: true }).unwrap();
    assert_eq!((consent.provider_revision, consent.model.as_str(), consent.analyze_new), (2, "vision-1", true));
}

/// R7/R8: revoking before dispatch sends nothing; revoking while a request is in
/// flight discards the late answer, keeps the spent units counted and cannot
/// resurrect the batch.
#[test]
fn revocation_blocks_dispatch_and_discards_a_late_answer() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    for (i, name) in ["1.png", "2.png", "3.png"].iter().enumerate() {
        put(&a, name, i as u8 + 1);
    }
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    let job = f.start(&sa, 10, 100_000).unwrap();
    // Revoked between claim and reserve: no request leaves.
    let claim = match f.db.lock().unwrap().claim_enrichment("w").unwrap() {
        Claimed::Item(c) => c,
        other => panic!("{other:?}"),
    };
    f.db.lock().unwrap().revoke_enrichment_consent(&sa).unwrap();
    assert_eq!(f.db.lock().unwrap().reserve_enrichment(&claim).unwrap_err(), EnrichError::Cancelled);
    let fake = Fake::ok();
    assert_eq!(drain(&runner(&f, &fake)), 0);
    assert_eq!(fake.count(), 0);
    let view = f.job(&job.id);
    assert_eq!((view.state.as_str(), view.error_code.as_deref()), ("cancelled", Some("CONSENT_REVOKED")));
    assert_eq!(view.counts.revoked, 3);
    assert_eq!(f.sql::<i64>("SELECT count(*) FROM image_enrichment_usage"), 0, "nothing was reserved");
    assert_eq!(f.start(&sa, 10, 100_000).unwrap_err(), EnrichError::ConsentRequired);
    // Revoked while the request is in flight.
    f.consent(&sa, false).unwrap();
    let job2 = f.start(&sa, 10, 100_000).unwrap();
    let path = f.temp.path().join("db.sqlite");
    let racer = Arc::new(Mutex::new(Catalog::open(&path).unwrap()));
    let source = sa.clone();
    let fake = Fake::new(move |_| {
        racer.lock().unwrap().revoke_enrichment_consent(&source).unwrap();
        Ok(described(Some((300, 50))))
    });
    let r = runner(&f, &fake);
    assert_eq!(r.step().unwrap(), Step::Worked);
    assert_eq!(fake.count(), 1);
    assert_eq!(f.outputs(), 0, "the late answer is not published");
    assert_eq!(f.sql::<i64>("SELECT count(*) FROM image_enrichment_results"), 0, "nor stored");
    let view = f.job(&job2.id);
    assert_eq!(view.state, "cancelled");
    assert_eq!(view.usage.used_units, 350, "what was spent is still counted");
    assert_eq!(drain(&r), 0);
    assert_eq!(fake.count(), 1, "the cancelled batch is never resumed by a late response");
}

// ------------------------------------------------------------------ reuse

/// R5: identical bytes are sent once; each asset gets its own attachment and
/// no annotation is touched; a source or model without consent gets no reuse.
#[test]
fn identical_content_is_sent_once_and_reuse_respects_consent_and_model() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let (a, b) = (f.folder("a"), f.folder("b"));
    put(&a, "fox.png", 7);
    fs::copy(a.join("fox.png"), a.join("fox-copy.png")).unwrap();
    fs::copy(a.join("fox.png"), b.join("fox-elsewhere.png")).unwrap();
    put(&a, "other.png", 8);
    let (sa, sb) = (f.import(&a), f.import(&b));
    let annotations_before: String = f.sql("SELECT group_concat(annotations,'|') FROM (SELECT annotations FROM image_assets ORDER BY id)");
    f.consent(&sa, false).unwrap();
    let fake = Fake::ok();
    let r = runner(&f, &fake);
    let job = f.start(&sa, 10, 100_000).unwrap();
    drain(&r);
    let view = f.job(&job.id);
    assert_eq!(fake.count(), 2, "fox once, other once");
    assert_eq!((view.counts.done, view.counts.reused), (2, 1));
    assert_eq!(f.outputs(), 3, "every asset has its own attachment");
    assert_eq!(f.sql::<i64>("SELECT count(*) FROM image_enrichment_results"), 2);
    assert_eq!(f.sql::<String>("SELECT group_concat(annotations,'|') FROM (SELECT annotations FROM image_assets ORDER BY id)"), annotations_before, "annotations are untouched");
    // Source B never agreed: it cannot start, so it cannot receive A's result.
    assert_eq!(f.start(&sb, 10, 100_000).unwrap_err(), EnrichError::ConsentRequired);
    assert_eq!(f.sql::<i64>(&format!("SELECT count(*) FROM image_enrichment_outputs o JOIN asset_registry r ON r.id=o.asset_id WHERE r.source_id='{sb}'")), 0);
    // A different model is a different result: no reuse, a real request is made.
    set_profiles(f.db.lock().unwrap().db_connection(), vec![profile_json(PROVIDER, 1, "vision-1", true), profile_json("prov-2", 1, "other-model", true)]);
    f.db.lock().unwrap().grant_enrichment_consent(&ConsentRequest { source_id: sb.clone(), provider_id: "prov-2".into(), provider_revision: 1, analyze_new: false }).unwrap();
    let job_b = f.db.lock().unwrap().start_enrichment(&StartRequest { source_id: sb.clone(), provider_id: "prov-2".into(), provider_revision: 1, kind: Kind::Backfill, asset_ids: None, max_requests: 5, max_units: 100_000 }).unwrap();
    drain(&r);
    assert_eq!(fake.count(), 3, "same bytes, other model: sent");
    assert_eq!(f.job(&job_b.id).counts.done, 1);
    // The same model with consent reuses at no cost.
    let c = f.folder("c");
    fs::copy(a.join("other.png"), c.join("other-again.png")).unwrap();
    let sc = f.import(&c);
    f.consent(&sc, false).unwrap();
    let job_c = f.start(&sc, 5, 100_000).unwrap();
    drain(&r);
    assert_eq!(fake.count(), 3);
    assert_eq!((f.job(&job_c.id).counts.reused, f.job(&job_c.id).usage.requests), (1, 0));
    // Already-analyzed images are not queued twice.
    assert_eq!(f.start(&sa, 5, 100_000).unwrap_err(), EnrichError::NothingToDo);
}

// ------------------------------------------------------------------ budget

/// R3/R4: ceilings stop dispatch before the next request, atomically.
#[test]
fn batch_ceilings_pause_the_batch_without_sending_more() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    for i in 0..4u8 {
        put(&a, &format!("{i}.png"), i + 1);
    }
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    assert_eq!(f.start(&sa, 0, 100_000).unwrap_err(), EnrichError::InvalidArgument);
    assert_eq!(f.start(&sa, 5, EST_UNITS - 1).unwrap_err(), EnrichError::InvalidArgument, "less than one request");
    let job = f.start(&sa, 2, 100_000).unwrap();
    let fake = Fake::ok();
    let r = runner(&f, &fake);
    drain(&r);
    assert_eq!(fake.count(), 2, "the request ceiling holds");
    let view = f.job(&job.id);
    assert_eq!((view.state.as_str(), view.error_code.as_deref()), ("paused", Some("REQUEST_CEILING")));
    assert_eq!((view.counts.done, view.counts.pending), (2, 2));
    assert_eq!(view.usage.used_units, 280, "reported tokens are used when the provider reports them");
    assert_eq!(view.usage.estimated_entries, 0);
    // Unit ceiling, with a provider that reports nothing: the estimate is used and labeled.
    let g = Fixture::new();
    let a2 = g.folder("a");
    for i in 0..3u8 {
        put(&a2, &format!("{i}.png"), i + 1);
    }
    let s2 = g.import(&a2);
    g.consent(&s2, false).unwrap();
    let job2 = g.start(&s2, 10, EST_UNITS * 2 + 1).unwrap();
    let fake2 = Fake::new(|_| Ok(described(None)));
    drain(&runner(&g, &fake2));
    assert_eq!(fake2.count(), 2);
    let view = g.job(&job2.id);
    assert_eq!(view.error_code.as_deref(), Some("UNIT_CEILING"));
    assert_eq!((view.usage.used_units, view.usage.estimated_entries), (EST_UNITS * 2, 2), "unreported usage is an estimate and says so");
}

/// T2: two connections racing for the last slot of a ceiling.
#[test]
fn concurrent_reservations_cannot_overspend_a_ceiling() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    for i in 0..2u8 {
        put(&a, &format!("{i}.png"), i + 1);
    }
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    f.start(&sa, 1, 100_000).unwrap();
    let path = f.temp.path().join("db.sqlite");
    let mut one = Catalog::open(&path).unwrap();
    let mut two = Catalog::open(&path).unwrap();
    let claim = |c: &mut Catalog, owner: &str| match c.claim_enrichment(owner).unwrap() {
        Claimed::Item(c) => c,
        other => panic!("{other:?}"),
    };
    let (c1, c2) = (claim(&mut one, "w1"), claim(&mut two, "w2"));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let (b1, b2) = (barrier.clone(), barrier);
    let t1 = std::thread::spawn(move || { b1.wait(); one.reserve_enrichment(&c1) });
    let t2 = std::thread::spawn(move || { b2.wait(); two.reserve_enrichment(&c2) });
    let results = [t1.join().unwrap(), t2.join().unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "{results:?}");
    assert_eq!(results.iter().filter(|r| **r == Err(EnrichError::BudgetExceeded)).count(), 1);
    assert_eq!(f.sql::<i64>("SELECT count(*) FROM image_enrichment_usage WHERE status='reserved'"), 1);
}

// ------------------------------------------------------------ uncertain billing

/// R4/T2: a timeout after sending is an unknown outcome: counted, never retried
/// automatically, retried only knowingly with the same operation identity.
#[test]
fn an_unknown_outcome_is_counted_never_auto_retried_and_keeps_its_identity() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    let job = f.start(&sa, 1, 100_000).unwrap();
    let fake = Fake::new(|n| if n == 0 { Err(DescribeError::Provider(ProviderError::Timeout)) } else { Ok(described(Some((10, 5)))) });
    let r = runner(&f, &fake);
    drain(&r);
    drain(&r);
    assert_eq!(fake.count(), 1, "not retried automatically");
    let view = f.job(&job.id);
    assert_eq!((view.counts.unknown_outcome, view.usage.unknown_units, view.usage.used_units), (1, EST_UNITS, 0));
    assert_eq!(view.state, "failed");
    let item = f.db.lock().unwrap().enrichment_items(&job.id, 0, 10).unwrap().remove(0);
    assert_eq!((item.state.as_str(), item.error_code.as_deref()), ("unknown_outcome", Some("OUTCOME_UNKNOWN")));
    let operation = item.operation_id.clone();
    assert_eq!(f.db.lock().unwrap().retry_enrichment_item(&job.id, &item.asset_id, false).unwrap_err(), EnrichError::InvalidArgument, "the possible charge must be accepted");
    // The unknown request still counts against the request ceiling.
    f.db.lock().unwrap().retry_enrichment_item(&job.id, &item.asset_id, true).unwrap();
    drain(&r);
    assert_eq!(fake.count(), 1, "the earlier possible charge uses the whole one-request ceiling");
    assert_eq!(f.job(&job.id).error_code.as_deref(), Some("REQUEST_CEILING"));
    f.exec(&format!("UPDATE image_enrichment_jobs SET max_requests=5,state='queued',error_code=NULL WHERE id='{}'", job.id));
    drain(&r);
    assert_eq!(fake.count(), 2);
    let item = f.db.lock().unwrap().enrichment_items(&job.id, 0, 10).unwrap().remove(0);
    assert_eq!((item.state.as_str(), item.operation_id.as_str(), item.attempts), ("done", operation.as_str(), 2), "same identity, second attempt");
    let attempts: i64 = f.sql(&format!("SELECT count(*) FROM image_enrichment_usage WHERE operation_id='{operation}'"));
    assert_eq!(attempts, 2);
}

/// T2: a crash after the request was sent must not be forgotten or repeated.
#[test]
fn restart_turns_in_flight_requests_into_unknown_outcomes_and_requeues_claims() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    for i in 0..3u8 {
        put(&a, &format!("{i}.png"), i + 1);
    }
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    let job = f.start(&sa, 10, 100_000).unwrap();
    let take = |owner: &str| match f.db.lock().unwrap().claim_enrichment(owner).unwrap() {
        Claimed::Item(c) => c,
        other => panic!("{other:?}"),
    };
    let sent = take("app-before-crash");
    f.db.lock().unwrap().reserve_enrichment(&sent).unwrap();
    let claimed_only = take("app-before-crash");
    // The process dies here. Reopen the same file as a fresh process would.
    let path = f.temp.path().join("db.sqlite");
    let mut reopened = Catalog::open(&path).unwrap();
    assert_eq!(reopened.recover_enrichment(false).unwrap(), 0, "live leases are respected while they last");
    assert_eq!(reopened.recover_enrichment(true).unwrap(), 2);
    let view = reopened.enrichment_job(&job.id).unwrap();
    assert_eq!((view.counts.unknown_outcome, view.counts.pending, view.counts.running), (1, 2, 0));
    assert_eq!((view.usage.unknown_units, view.usage.reserved_units), (EST_UNITS, 0), "the request that may have been sent stays counted");
    let items = reopened.enrichment_items(&job.id, 0, 10).unwrap();
    assert_eq!(items.iter().find(|i| i.asset_id == sent.asset_id).unwrap().state, "unknown_outcome");
    assert_eq!(items.iter().find(|i| i.asset_id == claimed_only.asset_id).unwrap().state, "pending");
    drop(reopened);
    let fake = Fake::ok();
    drain(&runner(&f, &fake));
    assert_eq!(fake.count(), 2, "only the images that were never sent are sent");
}

/// R7: expired leases (a stuck worker) are recovered without waiting for a restart.
#[test]
fn expired_leases_are_recovered_but_live_ones_are_not() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    f.start(&sa, 10, 100_000).unwrap();
    assert!(matches!(f.db.lock().unwrap().claim_enrichment("slow").unwrap(), Claimed::Item(_)));
    assert_eq!(f.db.lock().unwrap().recover_enrichment(false).unwrap(), 0);
    f.exec("UPDATE image_enrichment_items SET lease_until=1 WHERE state='running'");
    assert_eq!(f.db.lock().unwrap().recover_enrichment(false).unwrap(), 1);
    assert!(f.db.lock().unwrap().enrichment_has_work().unwrap());
}

// ------------------------------------------------------------------ errors

#[test]
fn provider_failures_are_sorted_into_retry_pause_and_failure() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    // Rate limiting backs off, releases the reservation and gives up after three tries.
    let job = f.start(&sa, 10, 100_000).unwrap();
    let fake = Fake::new(|_| Err(DescribeError::Provider(ProviderError::RateLimited)));
    let r = runner(&f, &fake);
    drain(&r);
    assert_eq!(fake.count(), 1);
    let view = f.job(&job.id);
    assert_eq!((view.counts.pending, view.usage.requests), (1, 0), "a rate-limited request is not billed or counted");
    drain(&r);
    assert_eq!(fake.count(), 1, "backoff holds the image back");
    for _ in 0..2 {
        f.exec("UPDATE image_enrichment_items SET not_before=0");
        drain(&r);
    }
    assert_eq!(fake.count(), 3);
    let view = f.job(&job.id);
    assert_eq!((view.counts.failed, view.state.as_str()), (1, "failed"));
    assert_eq!(f.db.lock().unwrap().enrichment_items(&job.id, 0, 5).unwrap()[0].error_code.as_deref(), Some("RATE_LIMITED"));
    // Quota exhaustion pauses the batch with its images still queued, then resumes.
    f.exec("DELETE FROM image_enrichment_jobs");
    let job = f.start(&sa, 10, 100_000).unwrap();
    let fake = Fake::new(|n| if n == 0 { Err(DescribeError::Provider(ProviderError::QuotaExceeded)) } else { Ok(described(Some((5, 5)))) });
    let r = runner(&f, &fake);
    drain(&r);
    let view = f.job(&job.id);
    assert_eq!((view.state.as_str(), view.error_code.as_deref(), view.counts.pending), ("paused", Some("QUOTA_EXCEEDED"), 1));
    drain(&r);
    assert_eq!(fake.count(), 1, "a paused batch sends nothing");
    f.db.lock().unwrap().resume_enrichment_job(&job.id).unwrap();
    drain(&r);
    assert_eq!(f.job(&job.id).counts.done, 1);
    // An unusable answer was billed and fails that image only; it is not retried.
    f.exec("DELETE FROM image_enrichment_jobs; DELETE FROM image_enrichment_results; DELETE FROM image_enrichment_outputs");
    let job = f.start(&sa, 10, 100_000).unwrap();
    let fake = Fake::new(|_| Err(DescribeError::Output(InsightError::TooManyTags)));
    drain(&runner(&f, &fake));
    let view = f.job(&job.id);
    assert_eq!((view.counts.failed, view.usage.used_units, view.usage.estimated_entries), (1, EST_UNITS, 1));
    assert_eq!(f.db.lock().unwrap().enrichment_items(&job.id, 0, 5).unwrap()[0].error_code.as_deref(), Some("MODEL_OUTPUT_INVALID"));
}

// ------------------------------------------------------------------ fences

#[test]
fn a_changed_provider_an_offline_source_and_changed_content_stop_dispatch() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    put(&a, "two.png", 2);
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    let fake = Fake::ok();
    let r = runner(&f, &fake);
    // Provider reconfigured after consent: nothing is sent until the user agrees again.
    let job = f.start(&sa, 10, 100_000).unwrap();
    set_profiles(f.db.lock().unwrap().db_connection(), vec![profile_json(PROVIDER, 2, "vision-1", true)]);
    drain(&r);
    assert_eq!(fake.count(), 0);
    assert_eq!(f.job(&job.id).error_code.as_deref(), Some("PROVIDER_CHANGED"), "the consent named revision 1");
    assert_eq!(f.job(&job.id).state, "paused");
    // Model removed entirely.
    f.consent(&sa, false).unwrap_err();
    set_profiles(f.db.lock().unwrap().db_connection(), vec![profile_json(PROVIDER, 1, "vision-1", true)]);
    f.db.lock().unwrap().revoke_enrichment_consent(&sa).unwrap();
    f.consent(&sa, false).unwrap();
    f.exec("DELETE FROM image_enrichment_jobs");
    let job = f.start(&sa, 10, 100_000).unwrap();
    set_profiles(f.db.lock().unwrap().db_connection(), vec![]);
    drain(&r);
    assert_eq!((f.job(&job.id).state.as_str(), f.job(&job.id).error_code.as_deref()), ("paused", Some("PROVIDER_REMOVED")));
    assert_eq!(fake.count(), 0);
    set_profiles(f.db.lock().unwrap().db_connection(), vec![profile_json(PROVIDER, 1, "vision-1", true)]);
    // An offline source pauses and resumes.
    f.exec(&format!("UPDATE sources SET available=0 WHERE id='{sa}'"));
    f.db.lock().unwrap().resume_enrichment_job(&job.id).unwrap();
    drain(&r);
    assert_eq!((f.job(&job.id).state.as_str(), f.job(&job.id).error_code.as_deref()), ("paused", Some("SOURCE_OFFLINE")));
    assert_eq!(f.start(&sa, 5, 100_000).unwrap_err(), EnrichError::SourceOffline);
    f.exec(&format!("UPDATE sources SET available=1 WHERE id='{sa}'"));
    f.db.lock().unwrap().resume_enrichment_job(&job.id).unwrap();
    // One image changed after it was queued: it is stale, the other is analyzed.
    let ids = f.ids(&sa);
    f.exec(&format!("UPDATE image_assets SET content_hash='{}' WHERE id='{}'", "e".repeat(64), ids[0]));
    drain(&r);
    let view = f.job(&job.id);
    assert_eq!((view.counts.stale, view.counts.done, view.state.as_str()), (1, 1, "complete"));
    assert_eq!(fake.count(), 1);
    assert_eq!(f.sql::<i64>(&format!("SELECT count(*) FROM image_enrichment_outputs WHERE asset_id='{}'", ids[0])), 0);
}

/// R7: a source whose generation moved on (relink, replacement) ends the batch.
#[test]
fn a_new_source_generation_ends_the_batch_as_stale() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    let job = f.start(&sa, 10, 100_000).unwrap();
    f.exec(&format!("UPDATE sources SET generation=generation+1 WHERE id='{sa}'"));
    let fake = Fake::ok();
    drain(&runner(&f, &fake));
    assert_eq!(fake.count(), 0);
    let view = f.job(&job.id);
    assert_eq!((view.state.as_str(), view.error_code.as_deref(), view.counts.stale), ("cancelled", Some("SOURCE_CHANGED"), 1));
}

/// R7: a published answer needs the content it was requested for. A late
/// answer for changed content is discarded (and paid for).
#[test]
fn an_answer_for_content_that_changed_in_flight_is_discarded() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    let job = f.start(&sa, 10, 100_000).unwrap();
    let racer = Arc::new(Mutex::new(Catalog::open(&f.temp.path().join("db.sqlite")).unwrap()));
    let fake = Fake::new(move |_| {
        let c = racer.lock().unwrap();
        c.db_connection().execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        c.db_connection().execute("UPDATE image_assets SET content_hash=?1", [&"f".repeat(64)]).unwrap();
        Ok(described(Some((10, 10))))
    });
    drain(&runner(&f, &fake));
    assert_eq!(f.outputs(), 0);
    let view = f.job(&job.id);
    assert_eq!((view.counts.stale, view.usage.used_units), (1, 20));
}

// ------------------------------------------------------------ cancel / yield

#[test]
fn cancel_stops_pending_work_and_interactive_work_has_priority() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    for i in 0..3u8 {
        put(&a, &format!("{i}.png"), i + 1);
    }
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    let job = f.start(&sa, 10, 100_000).unwrap();
    let fake = Fake::ok();
    let busy = Arc::new(AtomicBool::new(true));
    let flag = busy.clone();
    let r = Runner { busy: Arc::new(move || flag.load(Ordering::SeqCst)), ..runner(&f, &fake) };
    assert_eq!(r.step().unwrap(), Step::Yielded);
    assert_eq!(fake.count(), 0, "interactive work holds enrichment back");
    busy.store(false, Ordering::SeqCst);
    assert_eq!(r.step().unwrap(), Step::Worked);
    f.db.lock().unwrap().cancel_enrichment_job(&job.id).unwrap();
    assert_eq!(drain(&r), 0);
    assert_eq!(fake.count(), 1);
    let view = f.job(&job.id);
    assert_eq!((view.state.as_str(), view.counts.done, view.counts.revoked), ("cancelled", 1, 2));
    assert_eq!(f.db.lock().unwrap().cancel_enrichment_job(&job.id).unwrap_err(), EnrichError::Conflict, "already finished");
    assert_eq!(f.db.lock().unwrap().cancel_enrichment_job("missing").unwrap_err(), EnrichError::NotFound);
}

/// R3: analyze new imports needs the source's explicit option, is bounded, and
/// sends nothing without it.
#[test]
fn new_imports_are_queued_only_with_the_opt_in_and_a_bounded_ceiling() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    put(&a, "two.png", 2);
    let sa = f.import(&a);
    let ids = f.ids(&sa);
    assert!(f.db.lock().unwrap().enqueue_new_imports(&sa, &ids).unwrap().is_none(), "no consent, no job");
    f.consent(&sa, false).unwrap();
    assert!(f.db.lock().unwrap().enqueue_new_imports(&sa, &ids).unwrap().is_none(), "consent without the new-imports option queues nothing");
    f.consent(&sa, true).unwrap();
    let job = f.db.lock().unwrap().enqueue_new_imports(&sa, &ids).unwrap().expect("queued");
    assert_eq!((job.kind, job.max_requests, job.max_units, job.counts.pending), (Kind::NewImports, 2, 2 * EST_UNITS, 2));
    assert!(f.db.lock().unwrap().enqueue_new_imports(&sa, &ids).unwrap().is_none(), "images already waiting are not queued twice");
    let fake = Fake::ok();
    drain(&runner(&f, &fake));
    assert_eq!(fake.count(), 2);
}

/// The import hook: a finished import queues exactly the images it made ready,
/// and only for a source that opted in.
#[test]
fn a_finished_import_queues_its_images_only_for_an_opted_in_source() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    put(&a, "two.png", 2);
    let sa = f.import(&a);
    let import: String = f.sql(&format!("SELECT id FROM image_import_jobs WHERE source_id='{sa}'"));
    assert!(f.db.lock().unwrap().enqueue_new_from_import(&import).unwrap().is_none(), "no consent, no job");
    assert!(f.db.lock().unwrap().enqueue_new_from_import("missing-job").unwrap().is_none());
    f.consent(&sa, true).unwrap();
    let job = f.db.lock().unwrap().enqueue_new_from_import(&import).unwrap().expect("queued");
    assert_eq!((job.kind, job.counts.pending), (Kind::NewImports, 2));
    assert!(!f.db.lock().unwrap().image_imports_active().unwrap());
    f.exec("UPDATE image_import_jobs SET state='running',lease_owner='t',lease_until=9999999999");
    assert!(f.db.lock().unwrap().image_imports_active().unwrap(), "analysis yields to a running import");
}

/// The production pixel path: the isolated worker's 512 px preview of the exact
/// content version, with coverage judged against the original's real size.
#[test]
fn the_preview_pixel_source_sends_a_bounded_copy_of_the_exact_version() {
    use soundshelf_core::image_preview::{PreviewCache, PreviewService, DEFAULT_QUOTA};
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    let mut big = vec![];
    png::PngEncoder::new(&mut big).write_image(&image::RgbaImage::from_pixel(1200, 800, image::Rgba([10, 120, 200, 255])), 1200, 800, image::ExtendedColorType::Rgba8).unwrap();
    fs::write(a.join("big.png"), big).unwrap();
    put(&a, "small.png", 3);
    let sa = f.import(&a);
    let service = Arc::new(PreviewService::new(
        f.db.clone(),
        Arc::new(PreviewCache::new(f.temp.path().join("previews"), DEFAULT_QUOTA)),
        PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker")),
    ));
    let pixels = PreviewPixels { previews: service };
    let rows: Vec<(String, String, String)> = {
        let c = f.db.lock().unwrap();
        let mut q = c.db_connection().prepare("SELECT r.id,r.relative_path,a.content_hash FROM asset_registry r JOIN image_assets a ON a.id=r.id WHERE r.source_id=?1 ORDER BY r.relative_path").unwrap();
        let rows = q.query_map([&sa], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        rows.collect::<std::result::Result<_, _>>().unwrap()
    };
    let (big_id, _, big_hash) = rows.iter().find(|r| r.1 == "big.png").unwrap().clone();
    let (small_id, _, small_hash) = rows.iter().find(|r| r.1 == "small.png").unwrap().clone();
    let prepared = pixels.prepare(&big_id, &big_hash).unwrap();
    assert_eq!((prepared.width, prepared.height, prepared.source_width, prepared.source_height), (512, 341, 1200, 800));
    assert_eq!(prepared.coverage(), soundshelf_core::image_contracts::Coverage::Preview);
    assert!(prepared.part.bytes.starts_with(b"\x89PNG"));
    let small = pixels.prepare(&small_id, &small_hash).unwrap();
    assert_eq!((small.width, small.source_width), (8, 8));
    assert_eq!(small.coverage(), soundshelf_core::image_contracts::Coverage::FullImage);
    assert_eq!(pixels.prepare(&big_id, &"0".repeat(64)).unwrap_err(), PixelError::Stale, "a different content version is never prepared");
    f.exec(&format!("UPDATE sources SET available=0 WHERE id='{sa}'"));
}

#[test]
fn a_same_stat_replacement_during_dispatch_discards_the_answer() {
    use soundshelf_core::image_preview::{PreviewCache, PreviewService, DEFAULT_QUOTA};
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let root = f.folder("changing");
    let path = root.join("image.png");
    let make_bytes = |color| {
        let mut out = vec![];
        let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([color, 80, 120, 255]));
        png::PngEncoder::new_with_quality(&mut out, png::CompressionType::Uncompressed, png::FilterType::NoFilter)
            .write_image(&image, 8, 8, image::ExtendedColorType::Rgba8).unwrap();
        out
    };
    let first = make_bytes(1);
    let second = make_bytes(2);
    assert_ne!(first, second);
    assert_eq!(first.len(), second.len());
    fs::write(&path, first).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let source = f.import(&root);
    let id = f.ids(&source)[0].clone();
    f.consent(&source, false).unwrap();
    let job = f.start(&source, 1, 100_000).unwrap();
    let fake = Fake::new(move |_| {
        fs::write(&path, &second).unwrap();
        fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();
        Ok(described(None))
    });
    let pixels = PreviewPixels { previews: Arc::new(PreviewService::new(
        f.db.clone(), Arc::new(PreviewCache::new(f.temp.path().join("previews"), DEFAULT_QUOTA)),
        PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker")),
    )) };
    let run = Runner { pixels: Arc::new(pixels), ..runner(&f, &fake) };
    drain(&run);
    assert_eq!(fake.count(), 1);
    assert_eq!(f.job(&job.id).counts.stale, 1);
    assert!(f.db.lock().unwrap().image(&id, None).unwrap().evidence.is_empty());
}

#[test]
fn structured_evidence_survives_portable_catalogs_and_source_snapshots() {
    use soundshelf_core::{portable, source_catalog};
    use std::collections::BTreeMap;
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let root = f.folder("source");
    put(&root, "image.png", 1);
    let source_id = f.import(&root);
    let id = f.ids(&source_id)[0].clone();
    let source = f.db.lock().unwrap().source(&source_id).unwrap();
    let annotations = f.db.lock().unwrap().edit_image_annotations(
        &id, 1, &soundshelf_core::image_annotations::AnnotationPatch {
            caption: Some(Some("Creator caption".into())),
            add_tags: vec!["human-reviewed".into()],
            ..Default::default()
        },
    ).unwrap().annotations;
    source_catalog::save(&f.db, &source).unwrap();
    f.consent(&source_id, false).unwrap();
    f.start(&source_id, 1, 100_000).unwrap();
    drain(&runner(&f, &Fake::ok()));
    let (expected, document) = {
        use soundshelf_core::image_search::{ImageQuery, TagState, TextField};
        let c = f.db.lock().unwrap();
        assert!(c.snapshot_states().unwrap()[0].dirty);
        let asset = c.image(&id, None).unwrap();
        assert_eq!(asset.evidence[0].tags, ["fox"]);
        assert_eq!(asset.annotations, annotations);
        let query = ImageQuery {
            text: "red fox".into(), fields: vec![TextField::SuggestedTags], tag_state: TagState::Suggested,
            tags: vec!["fox".into()], ..Default::default()
        };
        assert_eq!(c.search_images(None, &query, 0, 10, false).unwrap().total, 1);
        (asset.evidence, c.export_portable().unwrap().json().unwrap())
    };
    let mut imported = Catalog::open(&f.temp.path().join("portable.sqlite")).unwrap();
    let mappings = BTreeMap::new();
    let portable = portable::prepare_import(&document, &mappings, false).unwrap();
    imported.import_portable(&portable, &mappings, f.temp.path()).unwrap();
    let portable_image = imported.image(&id, None).unwrap();
    assert_eq!(portable_image.evidence, expected);
    assert_eq!(portable_image.annotations, annotations);
    source_catalog::save(&f.db, &source).unwrap();
    let warm = Arc::new(Mutex::new(Catalog::open(&f.temp.path().join("warm.sqlite")).unwrap()));
    source_catalog::import_folder(&warm, &root).unwrap();
    assert_eq!(warm.lock().unwrap().image(&id, None).unwrap().evidence, expected);
    fs::write(root.join("image.png"), b"changed size for stat check").unwrap();
    let changed = Arc::new(Mutex::new(Catalog::open(&f.temp.path().join("changed.sqlite")).unwrap()));
    source_catalog::import_folder(&changed, &root).unwrap();
    let asset = changed.lock().unwrap().image(&id, None).unwrap();
    assert!(asset.evidence[0].stale);
}

/// R7: a restored backup never re-grants consent or revives queued work.
#[test]
fn restoring_a_backup_clears_consent_and_cancels_queued_work_but_keeps_results() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    put(&a, "two.png", 2);
    let sa = f.import(&a);
    f.consent(&sa, true).unwrap();
    let job = f.start(&sa, 10, 100_000).unwrap();
    let fake = Fake::ok();
    assert_eq!(runner(&f, &fake).step().unwrap(), Step::Worked);
    let published = f.ids(&sa).into_iter().find_map(|id| {
        let evidence = f.db.lock().unwrap().image(&id, None).unwrap().evidence;
        (!evidence.is_empty()).then_some((id, evidence))
    }).unwrap();
    let backup = f.temp.path().join("backup.sqlite");
    f.db.lock().unwrap().backup_database(&backup).unwrap();
    let rollback = f.temp.path().join("rollback.sqlite");
    f.db.lock().unwrap().restore_database(&backup, &rollback).unwrap();
    let c = f.db.lock().unwrap();
    assert!(c.enrichment_consents().unwrap().is_empty(), "consent is not restored");
    let view = c.enrichment_job(&job.id).unwrap();
    assert_eq!((view.state.as_str(), view.counts.done, view.counts.revoked), ("cancelled", 1, 1));
    assert_eq!(c.db_connection().query_row::<i64, _, _>("SELECT count(*) FROM image_enrichment_results", [], |r| r.get(0)).unwrap(), 1, "paid results stay");
    assert_eq!(c.image(&published.0, None).unwrap().evidence, published.1);
}

/// Deleting a source removes its consent, queue, usage and attachments.
#[test]
fn removing_a_source_removes_its_enrichment_records() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    f.start(&sa, 10, 100_000).unwrap();
    drain(&runner(&f, &Fake::ok()));
    assert_eq!(f.outputs(), 1);
    f.db.lock().unwrap().remove_source(&sa, true).unwrap();
    for table in ["image_enrichment_consent", "image_enrichment_jobs", "image_enrichment_items", "image_enrichment_usage", "image_enrichment_outputs"] {
        assert_eq!(f.sql::<i64>(&format!("SELECT count(*) FROM {table}")), 0, "{table}");
    }
}

/// Originals are never touched by enrichment.
#[test]
fn originals_are_unchanged_and_the_real_pixel_path_strips_metadata() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    put(&a, "one.png", 1);
    let before = blake3::hash(&fs::read(a.join("one.png")).unwrap());
    let sa = f.import(&a);
    f.consent(&sa, false).unwrap();
    f.start(&sa, 10, 100_000).unwrap();
    drain(&runner(&f, &Fake::ok()));
    assert_eq!(blake3::hash(&fs::read(a.join("one.png")).unwrap()), before);
    // The prepared payload is a fresh PNG: no text, EXIF or color-profile chunks.
    let prepared = prepare_pixels(&image::RgbaImage::from_pixel(40, 30, image::Rgba([1, 2, 3, 255]))).unwrap();
    let body = &prepared.part.bytes;
    for chunk in [&b"tEXt"[..], b"iTXt", b"zTXt", b"eXIf", b"iCCP"] {
        assert!(!body.windows(4).any(|w| w == chunk), "{:?}", std::str::from_utf8(chunk));
    }
}

/// T1 on the wire: through the real provider adapter, a loopback server sees no
/// request until a consented batch runs, and exactly one per image after.
#[test]
fn the_real_adapter_sends_one_bounded_request_per_image_and_nothing_earlier() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let server = common::http::Server::json(|_| {
        (200, serde_json::json!({
            "model": "vision-1-2026",
            "choices": [{"message": {"content": "{\"caption\":\"A red square\",\"tags\":[{\"category\":\"palette\",\"label\":\"red\"}],\"uncertain\":false}"}}],
            "usage": {"prompt_tokens": 120, "completion_tokens": 30}
        }))
    });
    let mut profile = profile_json(PROVIDER, 1, "vision-1", true);
    profile["base_url"] = serde_json::json!(format!("http://{}/v1", server.address));
    set_profiles(f.db.lock().unwrap().db_connection(), vec![profile]);
    let a = f.folder("a");
    put(&a, "one.png", 1);
    put(&a, "two.png", 2);
    let sa = f.import(&a);
    let providers = Arc::new(soundshelf_core::ai_provider::Providers::new(f.db.clone(), Arc::new(soundshelf_core::ai_provider::MemorySecrets::default())));
    let dispatcher = Arc::new(ProviderDispatcher { providers });
    let r = Runner { dispatcher, ..runner(&f, &Fake::ok()) };
    // Nothing leaves before consent, while estimating, or while idle.
    let _ = f.start(&sa, 5, 100_000);
    f.db.lock().unwrap().estimate_enrichment(&EstimateRequest { source_id: sa.clone(), provider_id: PROVIDER.into(), provider_revision: 1, kind: Kind::Backfill, asset_ids: None }).unwrap();
    assert_eq!(drain(&r), 0);
    assert!(server.requests().is_empty(), "no user image reached the network before consent");
    f.consent(&sa, false).unwrap();
    let job = f.start(&sa, 5, 100_000).unwrap();
    drain(&r);
    let seen = server.requests();
    assert_eq!(seen.len(), 2, "one request per image");
    let sent = seen[0].body.to_string();
    assert!(sent.contains("image/png") || sent.contains("data:image"), "the image goes as an inline PNG");
    for secret in ["one.png", "two.png", sa.as_str()] {
        assert!(!sent.contains(secret), "no file name or id is sent: {secret}");
    }
    let view = f.job(&job.id);
    assert_eq!((view.counts.done, view.usage.used_units, view.usage.estimated_entries), (2, 300, 0), "provider-reported usage is recorded");
    let first = f.ids(&sa)[0].clone();
    let asset = f.db.lock().unwrap().image(&first, None).unwrap();
    let evidence = &asset.evidence[0];
    assert_eq!(evidence.tags, ["red"]);
    assert_eq!(evidence.model.as_ref().unwrap().revision, "vision-1-2026");
    assert_eq!(evidence.visual.as_ref().unwrap().provider_revision, 1);
}
