//! SS-062: exact-duplicate stacks over real files, the real image worker and
//! real catalogs. Originals are hashed before and after: grouping writes nothing.
use image::{codecs::png, ImageEncoder};
use soundshelf_core::{
    catalog::Catalog,
    image_duplicates::{DuplicateAction as A, DuplicateError, DuplicateGroup, GroupKind, PreferredState},
    image_import::ImageImportEngine,
    image_import_preview::{MediaImportPreviews, NativeSelectionKind},
    image_jobs::{ImportMode, ScanKind},
    image_relink::ImageRelinks,
    image_sources::{probe_image_source, refresh_image_source},
    portable::{self, PortableCatalog},
    source_catalog,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};
static SERIAL: Mutex<()> = Mutex::new(());

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
        let engine = ImageImportEngine {
            catalog: db.clone(),
            worker: PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker")),
            tools: None,
        };
        Self { temp, db, previews: MediaImportPreviews::default(), engine }
    }
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.temp.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    }
    fn import(&self, root: &Path) -> String {
        let raw = self.previews.approve_native(&[root.to_owned()], NativeSelectionKind::Folders).unwrap();
        let preview = self
            .previews
            .prepare(&self.db, &uuid::Uuid::new_v4().to_string(), raw, ImportMode::Images)
            .unwrap();
        let job = self.previews.confirm(&self.db, &preview.id).unwrap()[0].clone();
        self.finish(&job.id);
        job.source_id
    }
    fn finish(&self, id: &str) {
        for _ in 0..400 {
            if self.db.lock().unwrap().image_import_job(id).unwrap().terminal() {
                return;
            }
            self.engine.tick("test", &AtomicBool::new(false)).unwrap();
        }
        panic!("job did not finish")
    }
    fn refresh(&self, source: &str) {
        let job = refresh_image_source(&self.db, source, ScanKind::Discover, true).unwrap();
        self.finish(&job.id);
    }
    fn id(&self, source: &str, path: &str) -> String {
        self.db
            .lock()
            .unwrap()
            .db_connection()
            .query_row(
                "SELECT id FROM asset_registry WHERE source_id=?1 AND relative_path=?2",
                [source, path],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn groups(&self, scope: Option<&[String]>, hidden: bool) -> Vec<DuplicateGroup> {
        self.db.lock().unwrap().image_duplicate_groups(scope, hidden, 0, 25).unwrap().groups
    }
    fn act(&self, group: &DuplicateGroup, action: A) -> Result<DuplicateGroup, DuplicateError> {
        self.db.lock().unwrap().update_image_duplicate_group(None, &group.content_hash, group.revision, &action)
    }
}

/// 8×8 RGBA with a diagonal so crops differ from the whole frame.
fn pixels(size: u32, shift: u8) -> Vec<u8> {
    (0..size * size)
        .flat_map(|i| {
            let (x, y) = ((i % size) as u8, (i / size) as u8);
            [(x * 30).wrapping_add(shift), y * 30, if x == y { 255 } else { 0 }, 255]
        })
        .collect()
}
fn png(path: &Path, size: u32, shift: u8, compression: png::CompressionType) {
    let mut bytes = vec![];
    png::PngEncoder::new_with_quality(&mut bytes, compression, png::FilterType::Adaptive)
        .write_image(&pixels(size, shift), size, size, image::ExtendedColorType::Rgba8)
        .unwrap();
    fs::write(path, bytes).unwrap();
}
fn hashes(roots: &[&Path]) -> BTreeMap<PathBuf, String> {
    roots
        .iter()
        .flat_map(|root| walkdir::WalkDir::new(root).into_iter().flatten())
        .filter(|e| e.file_type().is_file() && !e.path().components().any(|c| c.as_os_str() == ".creativeshelf"))
        .map(|e| (e.path().to_owned(), blake3::hash(&fs::read(e.path()).unwrap()).to_hex().to_string()))
        .collect()
}
fn paths(g: &DuplicateGroup) -> Vec<(String, bool)> {
    g.members.iter().map(|m| (m.image.header.relative_path.clone(), m.separated)).collect()
}

/// T1, R1, R6, R7: identical bytes group across folders; re-encoded, cropped and
/// near-identical images never do; scoped callers learn nothing outside scope.
#[test]
fn identical_bytes_form_one_labeled_stack_and_lookalikes_stay_apart() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("shoot");
    let b = f.folder("archive");
    fs::create_dir(a.join("copies")).unwrap();
    png(&a.join("fox.png"), 8, 0, png::CompressionType::Default);
    fs::copy(a.join("fox.png"), a.join("copies/fox (1).png")).unwrap();
    fs::copy(a.join("fox.png"), b.join("fox-backup.png")).unwrap();
    // Same pixels, different encoder settings: different bytes, not an exact copy.
    png(&a.join("fox-reencoded.png"), 8, 0, png::CompressionType::Fast);
    // Cropped and slightly shifted variants are SS-060's to suggest, not this stack's.
    png(&a.join("fox-crop.png"), 4, 0, png::CompressionType::Default);
    png(&a.join("fox-brighter.png"), 8, 1, png::CompressionType::Default);
    // An identical pair whose only other copy is in a folder outside a scope.
    png(&b.join("lonely.png"), 8, 9, png::CompressionType::Default);
    fs::copy(b.join("lonely.png"), a.join("lonely-twin.png")).unwrap();
    assert_ne!(fs::read(a.join("fox.png")).unwrap(), fs::read(a.join("fox-reencoded.png")).unwrap());
    let before = hashes(&[&a, &b]);
    let sa = f.import(&a);
    let sb = f.import(&b);
    let hero = f.id(&sa, "fox.png");
    f.db.lock().unwrap().edit_image_annotations(
        &hero,
        1,
        &soundshelf_core::image_annotations::AnnotationPatch { comment: Some("hero shot".into()), ..Default::default() },
    ).unwrap();

    let page = f.db.lock().unwrap().image_duplicate_groups(None, false, 0, 25).unwrap();
    assert_eq!((page.total, page.open_groups, page.extra_copies, page.hidden_groups), (2, 2, 3, 0));
    let fox = &page.groups[0];
    assert_eq!(fox.kind, GroupKind::Exact);
    assert_eq!((fox.member_count, fox.revision, fox.preferred_state), (3, 0, PreferredState::None));
    let mut names: Vec<_> = fox.members.iter().map(|m| (m.source_name.clone(), m.image.header.relative_path.clone())).collect();
    names.sort();
    assert_eq!(names, [("archive".into(), "fox-backup.png".into()), ("shoot".into(), "copies/fox (1).png".into()), ("shoot".into(), "fox.png".into())]);
    for lookalike in ["fox-reencoded.png", "fox-crop.png", "fox-brighter.png"] {
        assert!(page.groups.iter().all(|g| g.members.iter().all(|m| m.image.header.relative_path != lookalike)), "{lookalike}");
    }
    // Each copy keeps its own identity and annotations.
    let c = f.db.lock().unwrap();
    let notes: Vec<String> = fox.members.iter().map(|m| c.image(&m.image.header.id, None).unwrap().annotations.comment).collect();
    assert_eq!(notes.iter().filter(|n| n.as_str() == "hero shot").count(), 1);
    drop(c);

    // R7: a scope sees only stacks with two copies inside it, and never their outside copies.
    let only_b = f.groups(Some(&[sb.clone()]), true);
    assert!(only_b.is_empty(), "archive alone holds one copy of each");
    let only_a = f.groups(Some(&[sa.clone()]), true);
    assert_eq!(only_a.len(), 1);
    assert_eq!(only_a[0].member_count, 2);
    assert!(only_a[0].members.iter().all(|m| m.image.header.source_id == sa));
    assert_eq!(f.db.lock().unwrap().image_duplicate_groups(Some(&[]), true, 0, 25).unwrap().total, 0);
    let lonely = page.groups.iter().find(|g| g.members.iter().any(|m| m.image.header.relative_path == "lonely.png")).unwrap();
    let (backup, hero) = (f.id(&sb, "fox-backup.png"), f.id(&sa, "fox.png"));
    let scope = [sa.clone()];
    assert_eq!(
        f.db.lock().unwrap().update_image_duplicate_group(Some(&scope), &lonely.content_hash, 0, &A::Dismiss).unwrap_err(),
        DuplicateError::NotFound,
        "a scoped caller cannot act on a stack it cannot see"
    );
    // A preference set by the owner is invisible to, and not overridable by, a scoped caller.
    let fox = f.act(fox, A::Prefer { asset_id: backup.clone() }).unwrap();
    let mut c = f.db.lock().unwrap();
    let scoped = c.image_duplicate_group(Some(&scope), &fox.content_hash).unwrap();
    assert_eq!((scoped.preferred_id, scoped.preferred_state), (None, PreferredState::None));
    assert_eq!(
        c.update_image_duplicate_group(Some(&scope), &fox.content_hash, fox.revision, &A::Prefer { asset_id: hero }).unwrap_err(),
        DuplicateError::PermissionDenied
    );
    assert_eq!(
        c.update_image_duplicate_group(Some(&scope), &fox.content_hash, fox.revision, &A::Prefer { asset_id: backup }).unwrap_err(),
        DuplicateError::NotFound
    );
    drop(c);
    assert_eq!(before, hashes(&[&a, &b]), "originals are never written");
}

/// R3, R4, R5, T2: decisions are revision-fenced, never delete or merge, and
/// survive recomputation; changed, missing and offline copies report their state.
#[test]
fn decisions_survive_file_changes_and_report_unavailable_members() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("set");
    for name in ["one.png", "two.png", "three.png"] {
        png(&a.join(name), 8, 0, png::CompressionType::Default);
    }
    let sa = f.import(&a);
    let (one, two, three) = (f.id(&sa, "one.png"), f.id(&sa, "two.png"), f.id(&sa, "three.png"));
    let g = f.groups(None, false).remove(0);
    let before_assets: i64 = f.db.lock().unwrap().db_connection().query_row("SELECT count(*) FROM image_assets", [], |r| r.get(0)).unwrap();

    let g = f.act(&g, A::Prefer { asset_id: two.clone() }).unwrap();
    assert_eq!((g.revision, g.preferred_id.as_deref(), g.preferred_state), (1, Some(two.as_str()), PreferredState::Available));
    assert_eq!(g.representative_id.as_deref(), Some(two.as_str()));
    // A stale editor is refused, not merged.
    let stale = DuplicateGroup { revision: 0, ..g.clone() };
    assert_eq!(f.act(&stale, A::Dismiss).unwrap_err(), DuplicateError::Conflict);
    assert_eq!(f.act(&g, A::Prefer { asset_id: "00000000-0000-4000-8000-000000000000".into() }).unwrap_err(), DuplicateError::NotFound);

    // Keeping a copy separate leaves a two-copy stack; separating the preferred clears it.
    let g = f.act(&g, A::Separate { asset_id: two.clone() }).unwrap();
    assert_eq!((g.member_count, g.separated_count, g.preferred_id.clone()), (2, 1, None));
    assert_eq!(paths(&g).last().unwrap(), &("two.png".to_string(), true));
    assert_eq!(f.act(&g, A::Prefer { asset_id: two.clone() }).unwrap_err(), DuplicateError::InvalidArgument);
    let g = f.act(&g, A::Separate { asset_id: three.clone() }).unwrap();
    assert!(g.hidden, "one copy left is no longer a stack");
    assert!(f.groups(None, false).is_empty());
    assert_eq!(f.groups(None, true).len(), 1, "separated stacks stay reviewable");
    let g = f.act(&g, A::Rejoin { asset_id: three.clone() }).unwrap();
    let g = f.act(&g, A::Prefer { asset_id: one.clone() }).unwrap();

    // Dismissal hides the stack until a new copy arrives.
    let g = f.act(&g, A::Dismiss).unwrap();
    assert!(g.dismissed && g.hidden);
    assert!(f.groups(None, false).is_empty());
    png(&a.join("four.png"), 8, 0, png::CompressionType::Default);
    f.refresh(&sa);
    let g = f.groups(None, false).remove(0);
    let new: Vec<_> = g.members.iter().filter(|m| m.new_since_dismissed).map(|m| m.image.header.relative_path.as_str()).collect();
    assert_eq!((g.dismissed, g.hidden, new), (true, false, vec!["four.png"]));
    let g = f.act(&g, A::Dismiss).unwrap();
    let g = f.act(&g, A::Restore).unwrap();
    assert!(!g.dismissed && !g.hidden);

    // The preferred copy's bytes change: it leaves the stack and the decision says so.
    png(&a.join("one.png"), 8, 50, png::CompressionType::Default);
    f.refresh(&sa);
    let g = f.groups(None, false).remove(0);
    assert_eq!((g.preferred_id.as_deref(), g.preferred_state), (Some(one.as_str()), PreferredState::Changed));
    assert_ne!(g.representative_id.as_deref(), Some(one.as_str()));
    assert_eq!(g.member_count, 2, "three.png and four.png; two.png stays separated");
    // Changing the bytes back rejoins it with the decision intact.
    png(&a.join("one.png"), 8, 0, png::CompressionType::Default);
    f.refresh(&sa);
    let g = f.groups(None, false).remove(0);
    assert_eq!((g.preferred_state, g.member_count), (PreferredState::Available, 3));

    // Missing after a complete scan, then offline: the decision stays and shows state.
    let parked = f.temp.path().join("parked-one.png");
    fs::rename(a.join("one.png"), &parked).unwrap();
    f.refresh(&sa);
    let g = f.groups(None, false).remove(0);
    assert_eq!((g.preferred_state, g.member_count), (PreferredState::Missing, 2));
    fs::rename(&parked, a.join("one.png")).unwrap();
    f.refresh(&sa);
    let moved = f.temp.path().join("set-unplugged");
    fs::rename(&a, &moved).unwrap();
    probe_image_source(&f.db, &sa).unwrap();
    let g = f.groups(None, false).remove(0);
    assert_eq!(g.preferred_state, PreferredState::Offline);
    assert!(g.members.iter().all(|m| m.image.header.availability == soundshelf_core::image_contracts::Availability::Offline));
    // Relinking to the same files keeps identities, so every decision still applies.
    let relinks = ImageRelinks::default();
    let preview = relinks.prepare(&f.db, &sa, &moved, &AtomicBool::new(false)).unwrap();
    let job = relinks.confirm(&f.db, &preview.id, preview.acknowledgement_required).unwrap();
    f.finish(&job.id);
    let g = f.groups(None, true).remove(0);
    assert_eq!((g.preferred_id.as_deref(), g.preferred_state), (Some(one.as_str()), PreferredState::Available));
    assert_eq!(g.members.iter().find(|m| m.image.header.id == two).map(|m| m.separated), Some(true));

    let after_assets: i64 = f.db.lock().unwrap().db_connection().query_row("SELECT count(*) FROM image_assets", [], |r| r.get(0)).unwrap();
    assert_eq!(after_assets, before_assets + 1, "only four.png was added; grouping removed nothing");
    assert!(moved.join("two.png").exists() && moved.join("three.png").exists());
}

/// R8, A3: decisions travel with portable catalogs; folder snapshots carry only
/// their own copies; tampered references are rejected before anything is written.
#[test]
fn decisions_travel_in_catalogs_and_snapshots_never_name_outside_files() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let a = f.folder("a");
    let b = f.folder("b");
    png(&a.join("x.png"), 8, 3, png::CompressionType::Default);
    fs::copy(a.join("x.png"), a.join("x-copy.png")).unwrap();
    fs::copy(a.join("x.png"), b.join("x.png")).unwrap();
    let sa = f.import(&a);
    let sb = f.import(&b);
    let foreign = f.id(&sb, "x.png");
    let g = f.groups(None, false).remove(0);
    let g = f.act(&g, A::Prefer { asset_id: foreign.clone() }).unwrap();
    let g = f.act(&g, A::Separate { asset_id: f.id(&sa, "x-copy.png") }).unwrap();
    f.act(&g, A::Dismiss).unwrap();

    // Full catalog: everything, and it restores into an empty library.
    let exported = f.db.lock().unwrap().export_portable().unwrap();
    assert_eq!(exported.schema, portable::PORTABLE_SCHEMA_V3, "decisions need v3; older readers refuse it by schema");
    assert_eq!(exported.image_duplicate_decisions.len(), 1);
    let d = &exported.image_duplicate_decisions[0];
    assert_eq!((d.preferred_id.as_deref(), d.dismissed, d.separated.len(), d.reviewed.len()), (Some(foreign.as_str()), true, 1, 2));
    let target = tempfile::tempdir().unwrap();
    let mut restored = Catalog::open(&target.path().join("t.sqlite")).unwrap();
    let text = exported.json().unwrap();
    let data = portable::prepare_import(&text, &BTreeMap::new(), false).unwrap();
    restored.import_portable(&data, &BTreeMap::new(), target.path()).unwrap();
    let page = restored.image_duplicate_groups(None, true, 0, 25).unwrap();
    assert_eq!((page.open_groups, page.hidden_groups), (0, 1), "restored stack is still dismissed");
    let g = &page.groups[0];
    assert_eq!((g.preferred_id.as_deref(), g.separated_count), (Some(foreign.as_str()), 1));

    // Folder snapshot of `a`: its own marks only, no reference to b's copy.
    let source = f.db.lock().unwrap().source(&sa).unwrap();
    source_catalog::save(&f.db, &source).unwrap();
    let snapshot = source_catalog::read(&a).unwrap().unwrap();
    assert_eq!(snapshot.schema, source_catalog::SCHEMA_V3);
    let raw = fs::read_to_string(a.join(".creativeshelf/catalog.json")).unwrap();
    assert!(!raw.contains(&foreign), "a folder snapshot never names another folder's file");
    let d = &snapshot.catalog.image_duplicate_decisions[0];
    assert_eq!((d.preferred_id.clone(), d.separated.len(), d.reviewed.len()), (None, 1, 1));

    // Tampering: a decision naming an image outside the document, or with other bytes.
    let mut bad: PortableCatalog = serde_json::from_str(&text).unwrap();
    bad.image_duplicate_decisions[0].reviewed.push("00000000-0000-4000-8000-0000000000ff".into());
    assert!(bad.validate().is_err());
    let mut bad: PortableCatalog = serde_json::from_str(&text).unwrap();
    bad.image_duplicate_decisions[0].content_hash = "f".repeat(64);
    assert!(bad.validate().is_err());
    let mut bad: PortableCatalog = serde_json::from_str(&text).unwrap();
    let dup = bad.image_duplicate_decisions[0].clone();
    bad.image_duplicate_decisions.push(dup);
    assert!(bad.validate().is_err());
    let mut v2: PortableCatalog = serde_json::from_str(&text).unwrap();
    v2.schema = portable::PORTABLE_SCHEMA_V2.into();
    assert!(v2.validate().unwrap_err().to_string().contains("require the soundshelf-catalog/v3 schema"));
    let mut bad: serde_json::Value = serde_json::from_str(&text).unwrap();
    bad["image_duplicate_decisions"][0]["merge_annotations"] = true.into();
    assert!(portable::preview(&bad.to_string(), false).is_err(), "unknown fields are refused");
}

/// Resource evidence: 100,000 images (25,000 stacks of 2–5 copies) list, count
/// and update within interactive bounds. Release timings go in the review note.
#[test]
fn stacks_stay_interactive_at_the_image_cap() {
    let temp = tempfile::tempdir().unwrap();
    let mut c = Catalog::open(&temp.path().join("db.sqlite")).unwrap();
    let source = c.add_source(temp.path()).unwrap();
    let db = c.db_connection();
    // 100,000 hashed records. The first 70,000 form blocks of 14 holding stacks of
    // 2, 3, 4 and 5 copies (20,000 stacks); the other 30,000 are unique.
    db.execute_batch(&format!(
        "BEGIN;
         CREATE TEMP TABLE plan AS
          WITH RECURSIVE n(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM n WHERE i<99999)
          SELECT printf('%08x-0000-4000-8000-%012x', i+1, i+1) AS id, printf('img/%06d.png', i) AS path,
           printf('%064x', CASE WHEN i < 70000 THEN (i / 14) * 4 + (CASE WHEN i % 14 < 2 THEN 0 WHEN i % 14 < 5 THEN 1 WHEN i % 14 < 9 THEN 2 ELSE 3 END) ELSE 1000000 + i END) AS h
          FROM n;
         INSERT INTO asset_registry(id,kind,source_id,relative_path) SELECT id,'image','{}',path FROM plan;
         INSERT INTO image_contents SELECT DISTINCT h FROM plan;
         INSERT INTO image_assets(id,content_hash,technical_state,availability,enrichment_state,annotation_revision,annotations)
          SELECT id,h,'failed','available','disabled',1,'{{\"revision\":1,\"tags\":[],\"comment\":\"\",\"favorite\":false,\"rating\":null,\"caption\":null}}' FROM plan;
         COMMIT;",
        source.id
    ))
    .unwrap();
    let mut worst = Duration::ZERO;
    let mut first = None;
    for offset in [0, 10_000, 19_975] {
        let start = Instant::now();
        let page = c.image_duplicate_groups(None, false, offset, 25).unwrap();
        worst = worst.max(start.elapsed());
        assert_eq!((page.open_groups, page.extra_copies), (20_000, 50_000));
        assert_eq!(page.groups.len(), 25);
        first.get_or_insert(page);
    }
    let first = first.unwrap();
    assert_eq!(first.groups[0].member_count, 5, "largest stacks first");
    let start = Instant::now();
    let g = &first.groups[0];
    let g = c.update_image_duplicate_group(None, &g.content_hash, 0, &A::Prefer { asset_id: g.members[1].image.header.id.clone() }).unwrap();
    let g = c.update_image_duplicate_group(None, &g.content_hash, g.revision, &A::Dismiss).unwrap();
    let update = start.elapsed() / 2;
    assert!(g.hidden);
    eprintln!("SS-062 duplicate stacks at 100K images / 20K stacks: worst page {worst:?}, update {update:?}");
    // Per-stack reads start from the digest index, never a scan of every image.
    let mut q = c.db_connection().prepare(
        "EXPLAIN QUERY PLAN SELECT r.id FROM image_assets i CROSS JOIN asset_registry r ON r.id=i.id WHERE i.content_hash=?1 AND r.kind='image'",
    ).unwrap();
    let plan: Vec<String> = q.query_map([&g.content_hash], |r| r.get(3)).unwrap().map(|r| r.unwrap()).collect();
    assert!(plan.iter().any(|p| p.contains("image_assets_version")), "{plan:?}");
    drop(q);
    let ceiling = if cfg!(debug_assertions) { Duration::from_secs(5) } else { Duration::from_secs(1) };
    assert!(worst < ceiling && update < ceiling, "{worst:?} {update:?}");
}
