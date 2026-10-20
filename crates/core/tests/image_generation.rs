//! SS-055: generation metadata and prompt provenance against real catalogs, the
//! real image worker and real PNG/JPEG/WebP files. Originals are never written.
use image::ImageEncoder;
use serde_json::Value;
use soundshelf_core::{
    catalog::Catalog,
    image_contracts::TechnicalState,
    image_generation::{ExtractionStatus, GenerationField, GenerationFormat, GenerationSharing},
    image_import::ImageImportEngine,
    image_import_preview::{MediaImportPreviews, NativeSelectionKind},
    image_jobs::{ImportMode, ScanKind},
    image_provenance::{FieldOrigin, GenerationSettings, ProvenanceError, ProvenancePatch, ProvenanceView, SearchPolicy},
    image_sources::refresh_image_source,
    portable::PortableCatalog,
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
    root: PathBuf,
    previews: MediaImportPreviews,
    engine: ImageImportEngine,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = dir(temp.path().join("media"));
        let db = Arc::new(Mutex::new(Catalog::open(&temp.path().join("db.sqlite")).unwrap()));
        let engine = ImageImportEngine {
            catalog: db.clone(),
            worker: PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker")),
            tools: None,
        };
        Self { temp, db, root, previews: MediaImportPreviews::default(), engine }
    }
    fn import(&self, paths: &[PathBuf], kind: NativeSelectionKind) -> String {
        let raw = self.previews.approve_native(paths, kind).unwrap();
        let preview = self.previews.prepare(&self.db, &uuid::Uuid::new_v4().to_string(), raw, ImportMode::Images).unwrap();
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
    fn check(&self, source: &str, verify: bool) {
        let job = refresh_image_source(&self.db, source, ScanKind::Listed, verify).unwrap();
        self.finish(&job.id);
    }
    fn id(&self, source: &str, path: &str) -> String {
        self.db
            .lock()
            .unwrap()
            .db_connection()
            .query_row("SELECT id FROM asset_registry WHERE source_id=?1 AND relative_path=?2", [source, path], |r| r.get(0))
            .unwrap()
    }
    fn view(&self, id: &str) -> ProvenanceView {
        self.db.lock().unwrap().image_generation(id).unwrap()
    }
}
fn dir(path: PathBuf) -> PathBuf {
    fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}
fn value(v: &ProvenanceView, field: GenerationField) -> (Option<String>, FieldOrigin) {
    let f = v.fields.iter().find(|f| f.field == field).unwrap();
    (f.value.clone(), f.origin)
}
fn settle() {
    std::thread::sleep(Duration::from_millis(2100));
}
fn hashes(root: &Path) -> BTreeMap<PathBuf, String> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && !e.path().components().any(|c| c.as_os_str() == ".creativeshelf"))
        .map(|e| (e.path().to_owned(), blake3::hash(&fs::read(e.path()).unwrap()).to_hex().to_string()))
        .collect()
}

fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = kind.to_vec();
    crc.extend_from_slice(data);
    out.extend_from_slice(&crc32fast::hash(&crc).to_be_bytes());
    out
}
/// A real 2×2 PNG with extra text chunks before IEND.
fn png(color: u8, texts: &[(&str, &str)]) -> Vec<u8> {
    let mut b = vec![];
    image::codecs::png::PngEncoder::new(&mut b)
        .write_image(&[color, 64, 128, 255].repeat(4), 2, 2, image::ExtendedColorType::Rgba8)
        .unwrap();
    let end = b.len() - 12;
    let mut out = b[..end].to_vec();
    for (key, text) in texts {
        let mut d = key.as_bytes().to_vec();
        d.push(0);
        d.extend(text.chars().map(|c| c as u8));
        out.extend(chunk(b"tEXt", &d));
    }
    out.extend_from_slice(&b[end..]);
    out
}
/// Little-endian TIFF with an EXIF UserComment in UTF-16 (A1111 layout).
fn exif(comment: &str) -> Vec<u8> {
    let mut text = b"UNICODE\0".to_vec();
    for u in comment.encode_utf16() {
        text.extend(u.to_be_bytes());
    }
    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    // IFD0: one entry, the Exif sub-IFD pointer.
    out.extend(1u16.to_le_bytes());
    out.extend([0x69, 0x87, 4, 0]);
    out.extend(1u32.to_le_bytes());
    out.extend(26u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    // Exif IFD at 26: UserComment, data at 44.
    out.extend(1u16.to_le_bytes());
    out.extend([0x86, 0x92, 7, 0]);
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(44u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(text);
    out
}
fn jpeg(comment: &str) -> Vec<u8> {
    let mut j = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut j, 90)
        .write_image(&[200, 100, 50].repeat(4), 2, 2, image::ExtendedColorType::Rgb8)
        .unwrap();
    let mut app = b"Exif\0\0".to_vec();
    app.extend(exif(comment));
    let mut out = j[..2].to_vec();
    out.extend([0xff, 0xe1]);
    out.extend(((app.len() + 2) as u16).to_be_bytes());
    out.extend(app);
    out.extend_from_slice(&j[2..]);
    out
}
fn webp(comment: &str) -> Vec<u8> {
    let mut simple = vec![];
    image::codecs::webp::WebPEncoder::new_lossless(&mut simple)
        .write_image(&[1, 2, 3, 255].repeat(4), 2, 2, image::ExtendedColorType::Rgba8)
        .unwrap();
    let riff = |kind: &[u8; 4], data: &[u8]| {
        let mut v = kind.to_vec();
        v.extend((data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 != 0 {
            v.push(0);
        }
        v
    };
    let mut body = b"WEBP".to_vec();
    body.extend(riff(b"VP8X", &[8, 0, 0, 0, 1, 0, 0, 1, 0, 0]));
    body.extend_from_slice(&simple[12..]);
    body.extend(riff(b"EXIF", &exif(comment)));
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}
const PARAMS: &str = "a red fox in snow\nNegative prompt: blurry\nSteps: 30, Sampler: Euler a, CFG scale: 7, Seed: 98765432109876543210, Model hash: 9f2b, Model: dreamshaper_8";

#[test]
fn embedded_metadata_is_read_on_import_without_changing_readiness_or_bytes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let comfy = r#"{"3":{"class_type":"KSampler","inputs":{"seed":42,"model":["4",0],"positive":["6",0],"negative":["7",0]}},"4":{"class_type":"CheckpointLoaderSimple","inputs":{"ckpt_name":"sdxl.safetensors"}},"6":{"class_type":"CLIPTextEncode","inputs":{"text":"lighthouse"}},"7":{"class_type":"CLIPTextEncode","inputs":{"text":""}}}"#;
    fs::write(f.root.join("a1111.png"), png(10, &[("parameters", PARAMS)])).unwrap();
    fs::write(f.root.join("comfy.png"), png(20, &[("prompt", comfy), ("workflow", r#"{"nodes":[{"type":"Shell","widgets_values":["rm -rf /"]}]}"#)])).unwrap();
    fs::write(f.root.join("broken.png"), png(30, &[("prompt", "{\"3\": [not json"), ("invokeai_metadata", "[")])).unwrap();
    fs::write(f.root.join("plain.png"), png(40, &[("Software", "GIMP"), ("dream", "a cat -S42")])).unwrap();
    fs::write(f.root.join("exif.jpg"), jpeg("猫 🐱\nSteps: 12, Seed: 7")).unwrap();
    fs::write(f.root.join("exif.webp"), webp("café\nSteps: 9, Sampler: DDIM")).unwrap();
    let originals = hashes(&f.root);
    let started = Instant::now();
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders);
    eprintln!("import of 6 images with metadata: {:?}", started.elapsed());

    for name in ["a1111.png", "comfy.png", "broken.png", "plain.png", "exif.jpg", "exif.webp"] {
        let id = f.id(&source, name);
        let image = f.db.lock().unwrap().image(&id, None).unwrap();
        assert_eq!(image.header.technical_state, TechnicalState::Ready, "{name}: metadata never decides readiness");
    }
    assert_eq!(hashes(&f.root), originals, "originals are never written");

    let v = f.view(&f.id(&source, "a1111.png"));
    assert_eq!(v.embedded.as_ref().unwrap().status, ExtractionStatus::Found);
    assert_eq!(value(&v, GenerationField::Prompt), (Some("a red fox in snow".into()), FieldOrigin::Imported));
    assert_eq!(value(&v, GenerationField::Seed).0.as_deref(), Some("98765432109876543210"), "lossless seed text");
    assert_eq!(value(&v, GenerationField::Generator), (None, FieldOrigin::Unknown), "never fabricated");
    assert_eq!(v.fields[0].source.as_ref().unwrap().format, GenerationFormat::A1111Parameters);

    let v = f.view(&f.id(&source, "comfy.png"));
    assert_eq!(value(&v, GenerationField::Prompt).0.as_deref(), Some("lighthouse"));
    assert_eq!(value(&v, GenerationField::NegativePrompt).0.as_deref(), Some(""), "recorded as empty");
    assert_eq!(v.embedded.as_ref().unwrap().uninterpreted[0].key, "workflow");

    let v = f.view(&f.id(&source, "broken.png"));
    assert_eq!(v.embedded.as_ref().unwrap().status, ExtractionStatus::Malformed);
    assert!(v.fields.iter().all(|x| x.value.is_none()));

    let v = f.view(&f.id(&source, "plain.png"));
    assert_eq!(v.embedded.as_ref().unwrap().status, ExtractionStatus::None, "software alone and unknown generators are not claims");

    let v = f.view(&f.id(&source, "exif.jpg"));
    assert_eq!(value(&v, GenerationField::Prompt).0.as_deref(), Some("猫 🐱"));
    let v = f.view(&f.id(&source, "exif.webp"));
    assert_eq!(value(&v, GenerationField::Sampler).0.as_deref(), Some("DDIM"));

    // A quick re-check reuses everything and reads nothing new.
    let rows = |f: &Fixture| -> i64 { f.db.lock().unwrap().db_connection().query_row("SELECT count(*) FROM image_generation_extractions", [], |r| r.get(0)).unwrap() };
    let before = rows(&f);
    f.check(&source, false);
    assert_eq!(rows(&f), before);
    assert_eq!(hashes(&f.root), originals);
}

#[test]
fn upgraded_libraries_read_metadata_once_on_the_next_check() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    fs::write(f.root.join("fox.png"), png(10, &[("parameters", PARAMS)])).unwrap();
    settle();
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders);
    let id = f.id(&source, "fox.png");
    // As in a v15 library: no extraction was ever stored.
    f.db.lock().unwrap().db_connection().execute("DELETE FROM image_generation_extractions", []).unwrap();
    assert!(f.view(&id).embedded.is_none(), "unread is not 'none'");
    f.check(&source, false);
    assert_eq!(value(&f.view(&id), GenerationField::Model).0.as_deref(), Some("dreamshaper_8"));
}

#[test]
fn user_provenance_is_revisioned_and_survives_file_and_claim_changes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let path = f.root.join("fox.png");
    fs::write(&path, png(10, &[("parameters", PARAMS)])).unwrap();
    settle();
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders);
    let id = f.id(&source, "fox.png");
    let edit = |rev: u32, patch: ProvenancePatch| f.db.lock().unwrap().edit_image_generation(&id, rev, &patch);
    let set = |field, v: Option<&str>| ProvenancePatch { set: BTreeMap::from([(field, v.map(str::to_owned))]), confirm: None };

    let v = edit(0, set(GenerationField::Model, Some("my corrected model"))).unwrap();
    assert_eq!((v.revision, value(&v, GenerationField::Model)), (1, (Some("my corrected model".into()), FieldOrigin::User)));
    assert_eq!(edit(0, set(GenerationField::Seed, Some("1"))).unwrap_err(), ProvenanceError::Conflict, "stale revision");
    assert_eq!(edit(1, set(GenerationField::Settings, Some("x"))).unwrap_err(), ProvenanceError::InvalidArgument, "imported-only field");
    assert_eq!(edit(1, set(GenerationField::AiGenerated, Some("probably"))).unwrap_err(), ProvenanceError::InvalidArgument);
    let v = edit(1, ProvenancePatch { confirm: Some(true), ..Default::default() }).unwrap();
    assert!(v.confirmed && !v.confirmation_outdated);
    let v = edit(2, set(GenerationField::AiGenerated, Some("yes"))).unwrap();
    assert_eq!(v.revision, 3);

    // New bytes with a different embedded claim: the claim follows the file,
    // the person's values and their record stay.
    fs::write(&path, png(99, &[("parameters", "a blue fox\nSteps: 5, Seed: 11, Model: other")])).unwrap();
    settle();
    f.check(&source, false);
    let v = f.view(&id);
    assert_eq!(value(&v, GenerationField::Prompt).0.as_deref(), Some("a blue fox"));
    assert_eq!(value(&v, GenerationField::Seed).0.as_deref(), Some("11"));
    assert_eq!(value(&v, GenerationField::Model), (Some("my corrected model".into()), FieldOrigin::User));
    assert_eq!(value(&v, GenerationField::AiGenerated), (Some("yes".into()), FieldOrigin::User));
    assert!(!v.confirmed && v.confirmation_outdated, "approval is not carried to a new claim");
    assert_eq!(v.revision, 3, "a claim refresh never rewrites the user record");
    // Clearing a user value shows the imported claim again.
    let v = edit(3, set(GenerationField::Model, None)).unwrap();
    assert_eq!(value(&v, GenerationField::Model), (Some("other".into()), FieldOrigin::Imported));
}

#[test]
fn sidecar_changes_are_versioned_and_exact_file_grants_never_read_siblings() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    fs::write(f.root.join("fox.png"), png(10, &[])).unwrap();
    fs::write(f.root.join("fox.txt"), "fox from sidecar\nSteps: 20, Seed: 5").unwrap();
    fs::write(f.root.join("notes.png"), png(11, &[])).unwrap();
    fs::write(f.root.join("notes.txt"), "shopping list").unwrap();
    settle();
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders);
    let id = f.id(&source, "fox.png");
    let v = f.view(&id);
    let side = v.sidecar.clone().unwrap();
    assert_eq!((side.relative_path.as_str(), side.present, side.revision), ("fox.txt", true, 1));
    assert_eq!(value(&v, GenerationField::Prompt).0.as_deref(), Some("fox from sidecar"));
    assert!(f.view(&f.id(&source, "notes.png")).sidecar.is_none(), "unrelated text is not a claim");

    // Same bytes rewritten: a new stat but no new revision.
    fs::write(f.root.join("fox.txt"), "fox from sidecar\nSteps: 20, Seed: 5").unwrap();
    settle();
    f.check(&source, false);
    assert_eq!(f.view(&id).sidecar.unwrap().revision, 1);
    // Sidecar-only change, image untouched: detected by a quick check.
    fs::write(f.root.join("fox.txt"), "fox edited\nSteps: 20, Seed: 6").unwrap();
    settle();
    f.check(&source, false);
    let v = f.view(&id);
    assert_eq!(v.sidecar.as_ref().unwrap().revision, 2);
    assert_eq!(value(&v, GenerationField::Seed).0.as_deref(), Some("6"));
    fs::remove_file(f.root.join("fox.txt")).unwrap();
    f.check(&source, false);
    let v = f.view(&id);
    let side = v.sidecar.clone().unwrap();
    assert_eq!((side.present, side.revision), (false, 3), "removal is a new version");
    assert_eq!(value(&v, GenerationField::Seed), (None, FieldOrigin::Unknown), "a removed sidecar supplies nothing");

    // Exact-file selection elsewhere: the sibling sidecar is never read.
    let other = dir(f.temp.path().join("other"));
    fs::write(other.join("cat.png"), png(12, &[])).unwrap();
    fs::write(other.join("cat.txt"), "secret sibling\nSteps: 1").unwrap();
    let files = f.import(&[other.join("cat.png")], NativeSelectionKind::Files);
    let v = f.view(&f.id(&files, "cat.png"));
    assert!(v.sidecar.is_none() && v.fields.iter().all(|x| x.value.is_none()));

    // A sidecar symlink that leaves the source is not followed.
    #[cfg(unix)]
    {
        let outside = dir(f.temp.path().join("outside"));
        fs::write(outside.join("x.txt"), "escaped\nSteps: 1").unwrap();
        fs::write(f.root.join("link.png"), png(13, &[])).unwrap();
        std::os::unix::fs::symlink(outside.join("x.txt"), f.root.join("link.txt")).unwrap();
        settle();
        f.check(&source, false);
        let refreshed = refresh_image_source(&f.db, &source, ScanKind::Discover, false).unwrap();
        f.finish(&refreshed.id);
        assert!(f.view(&f.id(&source, "link.png")).sidecar.is_none());
    }
}

fn portable(f: &Fixture, sharing: GenerationSharing) -> String {
    f.db.lock().unwrap().export_portable_with(sharing).unwrap().json().unwrap()
}
fn import_into(text: &str, root: &Path, source: &str) -> (tempfile::TempDir, Catalog) {
    let d = tempfile::tempdir().unwrap();
    let mut c = Catalog::open(&d.path().join("t.sqlite")).unwrap();
    let data: PortableCatalog = serde_json::from_str(text).unwrap();
    let roots = BTreeMap::from([(source.to_owned(), root.to_string_lossy().into_owned())]);
    c.import_portable(&data, &roots, d.path()).unwrap();
    (d, c)
}

#[test]
fn shared_exports_redact_prompts_and_paths_unless_opted_in() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let comfy = r#"{"3":{"class_type":"KSampler","inputs":{"seed":42,"model":["4",0],"positive":["6",0],"negative":["6",0]}},"4":{"class_type":"CheckpointLoaderSimple","inputs":{"ckpt_name":"/Users/me/models/private.safetensors"}},"6":{"class_type":"CLIPTextEncode","inputs":{"text":"client secret brief"}},"8":{"class_type":"LoadImage","inputs":{"image":"reference-photo.png"}}}"#;
    fs::write(f.root.join("fox.png"), png(10, &[("prompt", comfy)])).unwrap();
    settle();
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders);
    let id = f.id(&source, "fox.png");
    f.db.lock().unwrap().edit_image_generation(&id, 0, &ProvenancePatch {
        set: BTreeMap::from([(GenerationField::NegativePrompt, Some("my private negative".into())), (GenerationField::Generator, Some("ComfyUI 0.3".into()))]),
        confirm: Some(true),
    }).unwrap();
    let before = f.view(&id);

    let shared = portable(&f, GenerationSharing::default());
    for secret in ["client secret brief", "my private negative", "/Users/me", "reference-photo.png"] {
        assert!(!shared.contains(secret), "{secret} leaked into a default export");
    }
    let (_d, target) = import_into(&shared, &f.root, &source);
    let v = target.image_generation(&id).unwrap();
    let prompt = v.fields.iter().find(|x| x.field == GenerationField::Prompt).unwrap();
    assert!(prompt.value.is_none() && prompt.redacted, "redacted is distinct from unknown");
    assert_eq!(value(&v, GenerationField::Seed).0.as_deref(), Some("42"), "settings travel");
    assert_eq!(value(&v, GenerationField::Generator), (Some("ComfyUI 0.3".into()), FieldOrigin::User));
    assert_eq!(v.revision, before.revision, "revisions stay monotonic");

    // With the file mapped, the next check restores embedded prompts from the bytes.
    let target = Arc::new(Mutex::new(target));
    let engine = ImageImportEngine { catalog: target.clone(), worker: PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker")), tools: None };
    let job = refresh_image_source(&target, &source, ScanKind::Listed, true).unwrap();
    for _ in 0..100 {
        if target.lock().unwrap().image_import_job(&job.id).unwrap().terminal() {
            break;
        }
        engine.tick("test", &AtomicBool::new(false)).unwrap();
    }
    let v = target.lock().unwrap().image_generation(&id).unwrap();
    assert_eq!(value(&v, GenerationField::Prompt).0.as_deref(), Some("client secret brief"));
    assert_eq!(value(&v, GenerationField::NegativePrompt).0.as_deref(), Some("client secret brief"), "the user's redacted value is gone; the file's claim shows");

    // Opting in keeps everything, and a round trip reproduces the view.
    let full = portable(&f, GenerationSharing { prompts: true, private_references: true });
    assert!(full.contains("client secret brief") && full.contains("reference-photo.png"));
    let (_d2, target) = import_into(&full, &f.root, &source);
    let mut again = target.image_generation(&id).unwrap();
    again.editable = before.editable;
    assert_eq!(again.fields, before.fields);
    assert_eq!((again.confirmed, again.claims.clone()), (before.confirmed, before.claims.clone()));
    let reexport: Value = serde_json::from_str(&target.export_portable_with(GenerationSharing { prompts: true, private_references: true }).unwrap().json().unwrap()).unwrap();
    let original: Value = serde_json::from_str(&full).unwrap();
    assert_eq!(reexport["images"], original["images"], "lossless round trip");
}

#[test]
fn folder_snapshots_follow_the_library_sharing_setting() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    fs::write(f.root.join("fox.png"), png(10, &[("parameters", PARAMS)])).unwrap();
    settle();
    let source_id = f.import(&[f.root.clone()], NativeSelectionKind::Folders);
    let source = f.db.lock().unwrap().source(&source_id).unwrap();
    source_catalog::save(&f.db, &source).unwrap();
    let snapshot = fs::read_to_string(f.root.join(".creativeshelf/catalog.json")).unwrap();
    assert!(snapshot.contains("dreamshaper_8") && !snapshot.contains("a red fox"), "default snapshot redacts prompts");
    let dirty = |f: &Fixture| -> i64 { f.db.lock().unwrap().db_connection().query_row("SELECT dirty FROM source_catalog_state WHERE source_id=?1", [&source_id], |r| r.get(0)).unwrap() };
    let before = dirty(&f);
    f.db.lock().unwrap().set_generation_settings(GenerationSettings { search: SearchPolicy::Prompts, snapshot: GenerationSharing { prompts: true, private_references: false } }).unwrap();
    assert!(dirty(&f) > before, "a sharing change republishes affected folders");
    source_catalog::save(&f.db, &source).unwrap();
    let snapshot = fs::read_to_string(f.root.join(".creativeshelf/catalog.json")).unwrap();
    assert!(snapshot.contains("a red fox"));
    let id = f.id(&source_id, "fox.png");
    let terms = f.db.lock().unwrap().image_generation_search_text(&id).unwrap();
    assert!(terms.contains(&"a red fox in snow".to_owned()) && terms.contains(&"CFG scale: 7".to_owned()));
}

#[test]
fn derived_elements_inherit_parent_provenance_per_field() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    fs::write(f.root.join("fox.png"), png(10, &[("parameters", PARAMS)])).unwrap();
    settle();
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders);
    let parent = f.id(&source, "fox.png");
    f.db.lock().unwrap().edit_image_generation(&parent, 0, &ProvenancePatch {
        set: BTreeMap::from([(GenerationField::Generator, Some("Forge".into()))]),
        confirm: None,
    }).unwrap();
    let mut doc: Value = serde_json::from_str(&portable(&f, GenerationSharing { prompts: true, private_references: true })).unwrap();
    // A synthetic element cut from the parent (SS-077 creates real ones).
    let mut element = doc["images"][0].clone();
    let child = "33333333-3333-4333-8333-333333333333";
    element["asset"]["header"]["id"] = child.into();
    element["asset"]["header"]["kind"] = "element".into();
    element["asset"]["header"]["relative_path"] = "elements/fox.png".into();
    // Different bytes from the parent, as a real cutout has.
    element["asset"]["header"]["content_hash"] = "d".repeat(64).into();
    element["versions"] = serde_json::json!([{"content_hash": "d".repeat(64), "first_seen": 1}]);
    element["asset"]["lineage"] = serde_json::json!({"parent_id": parent, "parent_version": doc["images"][0]["asset"]["header"]["content_hash"],
        "recipe_id": "55555555-5555-4555-8555-555555555555", "recipe_revision": 1, "mask_hash": "c".repeat(64), "mask_revision": 1});
    element["generation"] = serde_json::json!({"embedded": null, "sidecar": null,
        "provenance": {"revision": 1, "fields": {"seed": "child seed"}, "confirmed": null, "redacted": []}});
    doc["images"].as_array_mut().unwrap().push(element);
    let (_d, target) = import_into(&doc.to_string(), &f.root, &source);
    let v = target.image_generation(child).unwrap();
    assert_eq!(value(&v, GenerationField::Seed), (Some("child seed".into()), FieldOrigin::User), "child-specific value");
    assert_eq!(value(&v, GenerationField::Generator), (Some("Forge".into()), FieldOrigin::Inherited));
    assert_eq!(value(&v, GenerationField::Prompt), (Some("a red fox in snow".into()), FieldOrigin::Inherited));
    assert_eq!(v.inherited_from.as_deref(), Some(parent.as_str()));
}

#[test]
fn restore_refuses_backups_with_invalid_generation_records() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    fs::write(f.root.join("fox.png"), png(10, &[("parameters", PARAMS)])).unwrap();
    settle();
    let source = f.import(&[f.root.clone()], NativeSelectionKind::Folders);
    let id = f.id(&source, "fox.png");
    f.db.lock().unwrap().edit_image_generation(&id, 0, &ProvenancePatch {
        set: BTreeMap::from([(GenerationField::Seed, Some("1".into()))]),
        confirm: None,
    }).unwrap();
    let backup = f.temp.path().join("backup.sqlite");
    f.db.lock().unwrap().backup_database(&backup).unwrap();
    let good = f.temp.path().join("good.sqlite");
    fs::copy(&backup, &good).unwrap();
    {
        let db = rusqlite::Connection::open(&backup).unwrap();
        // Valid JSON with the right revision, but a field users cannot edit.
        db.execute("UPDATE image_generation_provenance SET record=json_set(record,'$.fields.settings','x')", []).unwrap();
    }
    let rollback = f.temp.path().join("rollback.sqlite");
    assert!(f.db.lock().unwrap().restore_database(&backup, &rollback).is_err());
    f.db.lock().unwrap().restore_database(&good, &f.temp.path().join("rollback2.sqlite")).unwrap();
    assert_eq!(value(&f.view(&id), GenerationField::Seed).0.as_deref(), Some("1"));
}

/// Measured costs recorded in docs/reviews/SS-055.md. Run with
/// `cargo test --release -p soundshelf-core --test image_generation -- --nocapture cost`.
#[test]
fn metadata_reader_cost_is_bounded_on_large_and_hostile_files() {
    use soundshelf_core::image_generation::{extract, MAX_SEGMENTS};
    // A large real PNG (≈ 24 MB of noise) with its parameters after the image data.
    let (w, h) = (2_800u32, 2_200u32);
    let mut seed = 0x9e37_79b9u32;
    let pixels: Vec<u8> = (0..w * h * 4)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect();
    let mut big = vec![];
    image::codecs::png::PngEncoder::new_with_quality(&mut big, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::NoFilter)
        .write_image(&pixels, w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    let end = big.len() - 12;
    let mut d = b"parameters\0".to_vec();
    d.extend(PARAMS.as_bytes());
    big.splice(end..end, chunk(b"tEXt", &d));
    let started = Instant::now();
    let runs = 20;
    for _ in 0..runs {
        assert_eq!(extract(&big).status, ExtractionStatus::Found);
    }
    let per = started.elapsed() / runs;
    eprintln!("cost: {} byte PNG, metadata after IDAT: {per:?} per extract", big.len());
    assert!(per < Duration::from_millis(250), "walks chunk headers, never pixels");
    // First-read I/O: the import keeps the hashed bytes instead of streaming them.
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("big.png");
    fs::write(&file, &big).unwrap();
    let (mut stream, mut keep) = (Duration::ZERO, Duration::ZERO);
    for _ in 0..5 {
        let t = Instant::now();
        soundshelf_core::catalog::hash_file(&file).unwrap();
        stream += t.elapsed();
        let t = Instant::now();
        let bytes = fs::read(&file).unwrap();
        blake3::hash(&bytes);
        extract(&bytes);
        keep += t.elapsed();
    }
    eprintln!("cost: streamed hash {:?} vs read+hash+extract {:?} per 24 MB file (warm cache)", stream / 5, keep / 5);

    // Hostile: the maximum number of near-limit compressed chunks.
    let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::best());
    std::io::Write::write_all(&mut z, &vec![b'{'; 900 * 1024]).unwrap();
    let bomb = z.finish().unwrap();
    let mut chunks = vec![];
    for _ in 0..MAX_SEGMENTS + 8 {
        let mut c = b"prompt\0\0".to_vec();
        c.extend(&bomb);
        chunks.push(chunk(b"zTXt", &c));
    }
    let mut hostile = png(1, &[]);
    let end = hostile.len() - 12;
    hostile.splice(end..end, chunks.concat());
    let started = Instant::now();
    let e = extract(&hostile);
    let took = started.elapsed();
    eprintln!("cost: hostile {} byte PNG ({} bombs): {took:?}, status {:?}", hostile.len(), MAX_SEGMENTS + 8, e.status);
    assert!(took < Duration::from_secs(3), "decompression and time budgets stop the work");
}
