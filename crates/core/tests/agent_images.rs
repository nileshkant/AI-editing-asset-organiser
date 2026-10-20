//! SS-080 image tools for MCP clients, through the same `AgentLibrary::execute`
//! the MCP server calls, with real image-worker previews. All pixels are synthetic.
use base64::Engine;
use image::ImageEncoder;
use serde_json::{json, Value};
use soundshelf_core::{
    agent::{Access, AgentLibrary, Failure},
    catalog::{Catalog, Source},
    export::ExportService,
    image_analysis::{ImageAnalysisRequest, ImageSelection},
    image_preview::{PreviewCache, PreviewService, DEFAULT_QUOTA},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
};

/// The image worker is a real process; tests share it one at a time, as in image_preview.rs.
static SERIAL: Mutex<()> = Mutex::new(());
fn worker() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker"))
}
fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    let mut out = vec![];
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

struct Fixture {
    _serial: std::sync::MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    catalog: Arc<Mutex<Catalog>>,
    library: Arc<AgentLibrary>,
    public: Source,
    secret: Source,
    previews: Arc<PreviewService>,
}
impl Fixture {
    fn new() -> Self {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        for d in ["public", "secret"] {
            fs::create_dir(dir.path().join(d)).unwrap();
        }
        let c = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
        let public = c.add_source(&dir.path().join("public")).unwrap();
        let secret = c.add_source(&dir.path().join("secret")).unwrap();
        let catalog = Arc::new(Mutex::new(c));
        let exports = Arc::new(ExportService::new(dir.path().join("journal")).unwrap());
        let library = Arc::new(AgentLibrary::new(catalog.clone(), exports, None));
        let previews = Arc::new(PreviewService::new(
            catalog.clone(),
            Arc::new(PreviewCache::new(dir.path().join("previews"), DEFAULT_QUOTA)),
            worker(),
        ));
        Self { _serial: serial, _dir: dir, catalog, library, public, secret, previews }
    }
    /// Register and publish through the SS-042 analysis path, as import does.
    fn image(&self, source: &Source, name: &str, bytes: &[u8]) -> (String, String) {
        fs::write(Path::new(&source.root).join(name), bytes).unwrap();
        let mut c = self.catalog.lock().unwrap();
        let hash = ImageSelection::capture(&c, &source.id, name).unwrap().hash(&AtomicBool::new(false)).unwrap();
        let id = c.register_image(source, name, Some(&hash)).unwrap().header.id;
        ImageAnalysisRequest::capture(&c, &id)
            .unwrap()
            .analyze(&worker(), Arc::new(AtomicBool::new(false)))
            .unwrap()
            .reverify(&AtomicBool::new(false))
            .unwrap()
            .publish(&mut c)
            .unwrap();
        (id, hash)
    }
    fn access(&self) -> Access {
        Access { source_ids: vec![self.public.id.clone()], read: true, ..Default::default() }
    }
    fn call(&self, access: &Access, method: &str, value: Value) -> Result<Value, Failure> {
        self.library.execute("client", access, method, value)
    }
}
fn code(r: Result<Value, Failure>) -> &'static str {
    r.err().map(|f| f.code).unwrap_or("OK")
}
fn ids(v: &Value) -> Vec<String> {
    v["items"].as_array().unwrap().iter().map(|i| i["relative_path"].as_str().unwrap().to_owned()).collect()
}

#[test]
fn search_is_scoped_filtered_paged_and_never_reveals_notes_or_paths() {
    let f = Fixture::new();
    let (red, red_v) = f.image(&f.public, "red poster.png", &png(40, 30, [255, 0, 0, 255]));
    f.image(&f.public, "blue.png", &png(20, 20, [0, 0, 255, 255]));
    f.image(&f.public, "green.png", &png(20, 20, [0, 255, 0, 255]));
    f.image(&f.secret, "secret red.png", &png(20, 20, [255, 0, 0, 255]));
    {
        let mut c = f.catalog.lock().unwrap();
        let patch = soundshelf_core::image_annotations::AnnotationPatch {
            add_tags: vec!["Poster".into()],
            comment: Some("blue moon private note".into()),
            favorite: Some(true),
            ..Default::default()
        };
        c.edit_image_annotations(&red, 1, &patch).unwrap();
    }
    let a = f.access();
    let all = f.call(&a, "images.search", json!({})).unwrap();
    assert_eq!(ids(&all), ["blue.png", "green.png", "red poster.png"]);
    assert_eq!(all["total"], 3);
    // Another client's source never appears, even when the text matches it.
    assert_eq!(ids(&f.call(&a, "images.search", json!({"text": "red"})).unwrap()), ["red poster.png"]);
    // Naming a source outside the pairing is refused, not filtered.
    assert_eq!(code(f.call(&a, "images.search", json!({"source_ids": [f.secret.id]}))), "PERMISSION_DENIED");
    // Notes are neither matched nor returned.
    assert_eq!(f.call(&a, "images.search", json!({"text": "moon"})).unwrap()["total"], 0);
    assert!(!all.to_string().contains("private note"));
    assert!(!all.to_string().contains(&f.public.root), "absolute roots stay hidden");
    assert_eq!(ids(&f.call(&a, "images.search", json!({"tags": ["poster"]})).unwrap()), ["red poster.png"]);
    assert_eq!(ids(&f.call(&a, "images.search", json!({"untagged_only": true})).unwrap()), ["blue.png", "green.png"]);
    assert_eq!(ids(&f.call(&a, "images.search", json!({"favorites_only": true})).unwrap()), ["red poster.png"]);
    let page = f.call(&a, "images.search", json!({"limit": 2})).unwrap();
    assert_eq!((ids(&page).len(), page["next_offset"].clone()), (2, json!(2)));
    let last = f.call(&a, "images.search", json!({"limit": 2, "offset": 2})).unwrap();
    assert_eq!((ids(&last), last["next_offset"].clone()), (vec!["red poster.png".to_owned()], Value::Null));
    let item = &all["items"][2];
    assert_eq!((item["version"].as_str(), item["width"].as_u64(), item["format"].as_str()), (Some(red_v.as_str()), Some(40), Some("png")));
    assert_eq!(item["tags"], json!(["Poster"]));
    for bad in [json!({"limit": 0}), json!({"limit": 101}), json!({"bogus": 1}), json!({"tags": [""]})] {
        assert_eq!(code(f.call(&a, "images.search", bad.clone())), "INVALID_ARGUMENT", "{bad}");
    }
}

#[test]
fn get_and_preview_fail_alike_for_unknown_and_denied_and_check_versions() {
    let f = Fixture::new();
    let (id, v) = f.image(&f.public, "a.png", &png(900, 300, [10, 20, 30, 255]));
    let (hidden, hidden_v) = f.image(&f.secret, "b.png", &png(10, 10, [0, 0, 0, 255]));
    let a = f.access();
    let got = f.call(&a, "images.get", json!({"id": id, "version": v})).unwrap();
    assert_eq!(got["image"]["relative_path"], "a.png");
    let denied = f.call(&a, "images.get", json!({"id": hidden, "version": hidden_v})).unwrap_err();
    let unknown = f.call(&a, "images.get", json!({"id": "00000000-0000-0000-0000-000000000000", "version": hidden_v})).unwrap_err();
    assert_eq!((denied.code, denied.message), (unknown.code, unknown.message));
    let stale = "0".repeat(64);
    assert_eq!(code(f.call(&a, "images.get", json!({"id": id, "version": stale}))), "ASSET_CHANGED");

    // Pixels need their own permission, then a running preview service.
    let request = json!({"id": id, "version": v, "size": "medium"});
    assert_eq!(code(f.call(&a, "images.preview", request.clone())), "PERMISSION_DENIED");
    let with_previews = Access { previews: true, ..a.clone() };
    assert_eq!(code(f.call(&with_previews, "images.preview", request.clone())), "NOT_READY");
    f.library.set_image_previews(f.previews.clone());
    let p = f.call(&with_previews, "images.preview", request).unwrap();
    assert_eq!((p["meta"]["width"].as_u64(), p["meta"]["height"].as_u64()), (Some(256), Some(85)));
    assert_eq!((p["meta"]["source_width"].as_u64(), p["meta"]["version"].as_str()), (Some(900), Some(v.as_str())));
    assert_eq!(p["image"]["mime_type"], "image/jpeg", "opaque previews are JPEG; any transparency keeps PNG");
    let bytes = base64::engine::general_purpose::STANDARD.decode(p["image"]["data"].as_str().unwrap()).unwrap();
    let decoded = image::load_from_memory(&bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (256, 85));
    let large = f.call(&with_previews, "images.preview", json!({"id": id, "version": v})).unwrap();
    assert_eq!(large["meta"]["width"], 512, "large is the default and caps at 512 px");
    assert_eq!(code(f.call(&with_previews, "images.preview", json!({"id": hidden, "version": hidden_v}))), "PERMISSION_DENIED");
    assert_eq!(code(f.call(&with_previews, "images.preview", json!({"id": id, "version": stale}))), "ASSET_CHANGED");
    assert_eq!(code(f.call(&with_previews, "images.preview", json!({"id": id, "version": v, "size": "huge"}))), "INVALID_ARGUMENT");
}

#[test]
fn annotate_needs_edit_detects_conflicts_and_is_undoable_by_the_owner() {
    let f = Fixture::new();
    let (id, v) = f.image(&f.public, "a.png", &png(10, 10, [1, 2, 3, 255]));
    let (hidden, hidden_v) = f.image(&f.secret, "b.png", &png(10, 10, [0, 0, 0, 255]));
    let read_only = f.access();
    let tag = json!({"id": id, "version": v, "revision": 1, "add_tags": ["Sky", "sky", "blue_hour"]});
    assert_eq!(code(f.call(&read_only, "images.annotate", tag.clone())), "PERMISSION_DENIED");
    let edit = Access { edit: true, ..read_only };
    let done = f.call(&edit, "images.annotate", tag.clone()).unwrap();
    assert_eq!((done["revision"].as_u64(), done["tags"].clone()), (Some(2), json!(["Sky", "blue hour"])));
    // Replaying the old revision is a conflict, not a silent merge.
    assert_eq!(code(f.call(&edit, "images.annotate", tag)), "ASSET_CHANGED");
    let removed = f.call(&edit, "images.annotate", json!({"id": id, "version": v, "revision": 2, "remove_tags": ["SKY"]})).unwrap();
    assert_eq!(removed["tags"], json!(["blue hour"]));
    assert_eq!(code(f.call(&edit, "images.annotate", json!({"id": hidden, "version": hidden_v, "revision": 1, "add_tags": ["x"]}))), "PERMISSION_DENIED");
    assert_eq!(code(f.call(&edit, "images.annotate", json!({"id": id, "version": v, "revision": 3}))), "INVALID_ARGUMENT");
    assert_eq!(code(f.call(&edit, "images.annotate", json!({"id": id, "version": v, "revision": 3, "add_tags": ["x"], "notes": "y"}))), "INVALID_ARGUMENT");
    let mut c = f.catalog.lock().unwrap();
    let history = c.image_annotation_history(5).unwrap();
    assert!(history[0].label.starts_with("MCP client: "), "{}", history[0].label);
    assert_eq!(history[0].id, removed["undo_action_id"].as_str().unwrap());
    // The owner can undo the client's latest change from the desktop.
    c.undo_image_annotation_action(&history[0].id).unwrap();
    assert_eq!(c.image_in_sources(&[f.public.id.clone()], &id, None).unwrap().annotations.tags, ["Sky", "blue hour"]);
}

/// End-to-end search cost through `execute`, including item metadata. Registering
/// images one by one takes minutes, so CI's `--include-ignored` run skips it
/// unless asked: `SS080_IMAGES=100000 cargo test --release -p soundshelf-core
/// --test agent_images -- --ignored --nocapture`. The catalog-only measurement
/// is `image_gallery::tests::filtered_search_cost_at_the_image_cap`.
#[test]
#[ignore = "opt-in large-catalog measurement; set SS080_IMAGES"]
fn measure_search_over_a_large_catalog() {
    let Some(n) = std::env::var("SS080_IMAGES").ok().and_then(|v| v.parse::<usize>().ok()) else { return };
    let f = Fixture::new();
    {
        let mut c = f.catalog.lock().unwrap();
        let hash = "a".repeat(64);
        for i in 0..n {
            c.register_image(&f.public, &format!("set{:03}/image{i:06}.png", i % 500), Some(&hash)).unwrap();
        }
    }
    let a = f.access();
    for (label, args) in [
        ("first page, no filter", json!({})),
        ("text matching 1%", json!({"text": "set007"})),
        ("text matching nothing", json!({"text": "zzz"})),
        ("untagged, deep page", json!({"untagged_only": true, "offset": 99_000})),
    ] {
        let mut times = vec![];
        for _ in 0..7 {
            let t = std::time::Instant::now();
            f.call(&a, "images.search", args.clone()).unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        println!("{n} images, {label}: median {:.1} ms, max {:.1} ms", times[3], times[6]);
    }
}

#[test]
fn pairings_saved_before_previews_existed_load_without_pixel_access() {
    let old: Access = serde_json::from_str(r#"{"source_ids":["s"],"read":true,"edit":true,"export":false,"paths":false}"#).unwrap();
    assert!(old.read && old.edit && !old.previews);
    assert!(!AgentLibrary::allows(&old, "images.preview"));
    assert!(AgentLibrary::allows(&old, "images.annotate"));
}
