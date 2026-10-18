//! SS-044 previews through the real image-worker process. All pixels are synthetic.
use image::{GenericImageView, ImageEncoder};
use soundshelf_core::{
    catalog::{Catalog, Source},
    image_analysis::{ImageAnalysisRequest, ImageSelection},
    image_contracts::MAX_PREVIEW_BYTES,
    image_preview::{
        CacheState, Preview, PreviewCache, PreviewError as E, PreviewMime, PreviewRequest,
        PreviewService, PreviewTier, DEFAULT_QUOTA,
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
static SERIAL: Mutex<()> = Mutex::new(());
fn worker() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker"))
}
struct Fixture {
    dir: tempfile::TempDir,
    catalog: Arc<Mutex<Catalog>>,
    source: Source,
    service: Arc<PreviewService>,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media");
        fs::create_dir(&root).unwrap();
        let c = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
        let source = c.add_source(&root).unwrap();
        let catalog = Arc::new(Mutex::new(c));
        let service = Arc::new(Self::service_at(&catalog, &dir.path().join("previews"), worker()));
        Self {
            dir,
            catalog,
            source,
            service,
        }
    }
    fn service_at(c: &Arc<Mutex<Catalog>>, root: &Path, worker: PathBuf) -> PreviewService {
        PreviewService::new(
            c.clone(),
            Arc::new(PreviewCache::new(root.into(), DEFAULT_QUOTA)),
            worker,
        )
    }
    fn file(&self, name: &str) -> PathBuf {
        Path::new(&self.source.root).join(name)
    }
    /// Register and publish through the SS-042 analysis path, as SS-043 import does.
    fn ready(&self, name: &str, bytes: &[u8]) -> (String, String) {
        fs::write(self.file(name), bytes).unwrap();
        let mut c = self.catalog.lock().unwrap();
        let hash = ImageSelection::capture(&c, &self.source.id, name)
            .unwrap()
            .hash(&AtomicBool::new(false))
            .unwrap();
        let id = c
            .register_image(&self.source, name, Some(&hash))
            .unwrap()
            .header
            .id;
        let request = ImageAnalysisRequest::capture(&c, &id).unwrap();
        let analyzed = request
            .analyze(&worker(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        analyzed
            .reverify(&AtomicBool::new(false))
            .unwrap()
            .publish(&mut c)
            .unwrap();
        (id, hash)
    }
}
fn request(id: &str, version: &str, tier: PreviewTier, tile: Option<[u32; 2]>) -> PreviewRequest {
    PreviewRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        asset_id: id.into(),
        version: version.into(),
        tier,
        tile,
    }
}
fn get(f: &Fixture, id: &str, v: &str, tier: PreviewTier) -> Result<Preview, E> {
    f.service.preview(&request(id, v, tier, None))
}
fn pixels(p: &Preview) -> image::RgbaImage {
    let format = match p.meta.mime {
        PreviewMime::Png => image::ImageFormat::Png,
        PreviewMime::Jpeg => image::ImageFormat::Jpeg,
    };
    let img = image::load_from_memory_with_format(&p.bytes, format).unwrap();
    assert_eq!(img.dimensions(), (p.meta.width, p.meta.height));
    img.to_rgba8()
}
fn near(a: [u8; 4], b: [u8; 4], tolerance: i16) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (i16::from(*x) - i16::from(y)).abs() <= tolerance)
}
fn encode_png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
    let mut out = vec![];
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}
fn chunk(kind: &[u8; 4], p: &[u8]) -> Vec<u8> {
    let mut out = (p.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(p);
    out.extend_from_slice(&crc32fast::hash(&out[4..]).to_be_bytes());
    out
}
/// Insert an ancillary chunk after IHDR.
fn inject(bytes: &[u8], kind: &[u8; 4], p: &[u8]) -> Vec<u8> {
    let mut result = bytes[..33].to_vec();
    result.extend(chunk(kind, p));
    result.extend_from_slice(&bytes[33..]);
    result
}
fn exif(o: u8) -> Vec<u8> {
    let mut p = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
    p.extend([o, 0, 0, 0, 0, 0, 0, 0]);
    p
}
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
fn quadrants(w: u32, h: u32) -> Vec<u8> {
    encode_png(w, h, |x, y| match (x < w / 2, y < h / 2) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => WHITE,
    })
}

#[test]
fn thumbnail_tiers_apply_orientation_and_color_once_with_bounded_sizes() {
    let _guard = SERIAL.lock().unwrap();
    let f = Fixture::new();
    // EXIF 6 rotates 90° clockwise: new(x, y) = old(y, H-1-x), so a 600x400
    // red/green/blue/white quadrant image becomes 400x600 blue/red/white/green.
    let bytes = inject(&quadrants(600, 400), b"eXIf", &exif(6));
    let (id, v) = f.ready("oriented.png", &bytes);
    for (tier, size) in [
        (PreviewTier::Small, (85, 128)),
        (PreviewTier::Medium, (171, 256)),
        (PreviewTier::Large, (341, 512)),
        (PreviewTier::Inspect, (400, 600)),
    ] {
        let p = get(&f, &id, &v, tier).unwrap();
        assert_eq!((p.meta.width, p.meta.height), size, "{tier:?}");
        assert_eq!((p.meta.source_width, p.meta.source_height), (400, 600));
        assert_eq!(p.meta.mime, PreviewMime::Jpeg);
        assert!(!p.meta.transparency && p.meta.original_available && !p.meta.offline_reference);
        let img = pixels(&p);
        let (w, h) = (size.0, size.1);
        for ((x, y), expected) in [
            ((w / 4, h / 4), BLUE),
            ((3 * w / 4, h / 4), RED),
            ((w / 4, 3 * h / 4), WHITE),
            ((3 * w / 4, 3 * h / 4), GREEN),
        ] {
            assert!(near(img.get_pixel(x, y).0, expected, 8), "{tier:?} {x},{y}");
        }
    }
    // Thumbnail tiers come from one decode; inspect is its own decode.
    assert_eq!(f.service.status().counters.generated, 2);
    // Adobe RGB(1998) converts to sRGB exactly once (independent SS-042 value).
    use std::io::Write;
    let icc = moxcms::ColorProfile::new_adobe_rgb().encode().unwrap();
    let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::default());
    z.write_all(&icc).unwrap();
    let mut p = b"generated\0\0".to_vec();
    p.extend(z.finish().unwrap());
    let (id, v) = f.ready(
        "adobe.png",
        &inject(&encode_png(32, 32, |_, _| [128, 64, 32, 255]), b"iCCP", &p),
    );
    let img = pixels(&get(&f, &id, &v, PreviewTier::Small).unwrap());
    assert!(near(img.get_pixel(16, 16).0, [146, 62, 23, 255], 6), "{:?}", img.get_pixel(16, 16));
    assert_eq!(fs::read(f.file("oriented.png")).unwrap(), bytes);
}

#[test]
fn transparent_previews_keep_actual_alpha_without_edge_bleed_or_baked_matte() {
    let _guard = SERIAL.lock().unwrap();
    let f = Fixture::new();
    // Hidden RGB under zero alpha is bright green; straight-alpha resampling
    // would bleed it into the red edge. A 50% alpha band must stay partial.
    let bytes = encode_png(1024, 256, |x, _| match x {
        0..=511 => [0, 255, 0, 0],
        512..=767 => [255, 0, 0, 255],
        _ => [255, 0, 0, 128],
    });
    let (id, v) = f.ready("alpha.png", &bytes);
    let p = get(&f, &id, &v, PreviewTier::Small).unwrap();
    assert_eq!((p.meta.width, p.meta.height, p.meta.mime), (128, 32, PreviewMime::Png));
    assert!(p.meta.transparency);
    let img = pixels(&p);
    assert_eq!(img.get_pixel(10, 16)[3], 0);
    assert_eq!(img.get_pixel(80, 16).0, RED);
    assert!((i16::from(img.get_pixel(120, 16)[3]) - 128).abs() <= 1);
    for x in 0..128 {
        let px = img.get_pixel(x, 16);
        assert!(px[3] == 0 || px[1] <= 2, "green bled into x={x}: {px:?}");
        // No checkerboard or matte: fully transparent stays transparent black.
        if px[3] == 0 {
            assert_eq!(px.0, [0, 0, 0, 0]);
        }
    }
    // A 1:1 tile keeps exact straight-alpha source pixels.
    let tile = f
        .service
        .preview(&request(&id, &v, PreviewTier::Tile, Some([1, 0])))
        .unwrap();
    assert_eq!((tile.meta.width, tile.meta.height, tile.meta.mime), (512, 256, PreviewMime::Png));
    let t = pixels(&tile);
    assert_eq!(t.get_pixel(0, 0).0, RED);
    assert_eq!(t.get_pixel(511, 255).0, [255, 0, 0, 128]);
    assert_eq!(fs::read(f.file("alpha.png")).unwrap(), bytes);
}

#[test]
fn inspection_and_full_resolution_tiles_are_bounded_and_grid_checked() {
    let _guard = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let bytes = encode_png(4096, 1024, |x, y| [(x / 16) as u8, (y / 4) as u8, 90, 255]);
    let (id, v) = f.ready("wide.png", &bytes);
    let p = get(&f, &id, &v, PreviewTier::Inspect).unwrap();
    assert_eq!((p.meta.width, p.meta.height), (2048, 512));
    let tile = f
        .service
        .preview(&request(&id, &v, PreviewTier::Tile, Some([7, 1])))
        .unwrap();
    assert_eq!((tile.meta.width, tile.meta.height), (512, 512));
    let t = pixels(&tile);
    // Tile (7,1) starts at source pixel (3584, 512): exact 1:1 mapping.
    let expected = |x: u32, y: u32| [(x / 16) as u8, (y / 4) as u8, 90, 255];
    assert!(near(t.get_pixel(0, 0).0, expected(3584, 512), 6));
    assert!(near(t.get_pixel(511, 511).0, expected(4095, 1023), 6));
    // Incompressible transparent pixels exceed the 4 MiB contract ceiling at full
    // size, so inspect steps down to a smaller tier and keeps actual alpha.
    let noise = encode_png(1100, 1100, |x, y| {
        let mut h = x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca77);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        let b = h.to_le_bytes();
        [b[0], b[1], b[2], b[3] | 1]
    });
    let (nid, nv) = f.ready("noise.png", &noise);
    let n = get(&f, &nid, &nv, PreviewTier::Inspect).unwrap();
    assert_eq!((n.meta.width, n.meta.height, n.meta.mime), (768, 768, PreviewMime::Png));
    assert!(n.meta.transparency && n.bytes.len() as u64 <= MAX_PREVIEW_BYTES);
    for bad in [[8, 0], [0, 2]] {
        assert_eq!(
            f.service
                .preview(&request(&id, &v, PreviewTier::Tile, Some(bad)))
                .err(),
            Some(E::InvalidArgument)
        );
    }
    assert_eq!(
        f.service.preview(&request(&id, &v, PreviewTier::Small, Some([0, 0]))).err(),
        Some(E::InvalidArgument)
    );
}

#[test]
fn duplicate_requests_coalesce_and_a_restart_reuses_cache_without_decoding() {
    let _guard = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let bytes = quadrants(900, 700);
    let (id, v) = f.ready("same.png", &bytes);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let s = f.service.clone();
            let r = request(&id, &v, PreviewTier::Medium, None);
            thread::spawn(move || s.preview(&r).map(|p| p.bytes))
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap().unwrap()).collect();
    assert!(results.windows(2).all(|w| w[0] == w[1]));
    let s = f.service.status();
    assert_eq!(s.counters.generated, 1, "{s:?}");
    assert!(s.counters.coalesced >= 1, "{s:?}");
    for tier in [PreviewTier::Small, PreviewTier::Large] {
        assert_eq!(get(&f, &id, &v, tier).unwrap().meta.cache, CacheState::Hit);
    }
    assert_eq!(f.service.status().counters.generated, 1);
    // Restart with an unusable worker: cached tiers still serve, misses report it.
    let restarted =
        Fixture::service_at(&f.catalog, &f.dir.path().join("previews"), "/nonexistent/worker".into());
    let hit = restarted.preview(&request(&id, &v, PreviewTier::Medium, None)).unwrap();
    assert_eq!((hit.meta.cache, hit.bytes.as_slice()), (CacheState::Hit, results[0].as_slice()));
    assert_eq!(
        restarted.preview(&request(&id, &v, PreviewTier::Inspect, None)).err(),
        Some(E::WorkerUnavailable)
    );
    // A corrupted record is repaired by regeneration, never served.
    let dir = f.dir.path().join("previews");
    let record = walk(&dir).into_iter().find(|p| p.ends_with("small.cspv")).unwrap();
    let mut raw = fs::read(&record).unwrap();
    let n = raw.len();
    raw[n - 3] ^= 0x55;
    fs::write(&record, raw).unwrap();
    let fresh = Fixture::service_at(&f.catalog, &dir, worker());
    let p = fresh.preview(&request(&id, &v, PreviewTier::Small, None)).unwrap();
    assert_eq!(p.meta.cache, CacheState::Generated);
    assert_eq!(fresh.status().counters.repaired, 1);
    assert_eq!(fs::read(f.file("same.png")).unwrap(), bytes);
}
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for e in fs::read_dir(dir).unwrap().flatten() {
        if e.file_type().unwrap().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}

#[test]
fn replaced_content_stale_versions_and_revoked_sources_never_serve_old_previews() {
    let _guard = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let first = encode_png(64, 64, |_, _| RED);
    let (id, v1) = f.ready("swap.png", &first);
    assert!(get(&f, &id, &v1, PreviewTier::Small).is_ok());
    // Same path, new bytes: identity is preserved, the old version is fenced.
    let second = encode_png(64, 64, |_, _| BLUE);
    fs::write(f.file("swap.png"), &second).unwrap();
    let v2 = {
        let mut c = f.catalog.lock().unwrap();
        let h = ImageSelection::capture(&c, &f.source.id, "swap.png")
            .unwrap()
            .hash(&AtomicBool::new(false))
            .unwrap();
        assert_eq!(c.register_image(&f.source, "swap.png", Some(&h)).unwrap().header.id, id);
        h
    };
    assert_eq!(get(&f, &id, &v1, PreviewTier::Small).err(), Some(E::StaleVersion));
    assert_eq!(get(&f, &id, &v2, PreviewTier::Small).err(), Some(E::NotReady));
    let (same, v2b) = f.ready("swap.png", &second);
    assert_eq!((same.as_str(), v2b.as_str()), (id.as_str(), v2.as_str()));
    let p = get(&f, &id, &v2, PreviewTier::Small).unwrap();
    assert!(near(pixels(&p).get_pixel(5, 5).0, BLUE, 6));
    // Pruning removes the replaced digest's records only.
    let digests = f.catalog.lock().unwrap().image_content_digests().unwrap();
    assert_eq!(f.service.cache().prune(&digests), 3);
    assert_eq!(get(&f, &id, &v2, PreviewTier::Small).unwrap().meta.cache, CacheState::Hit);
    // Identical bytes at another path share cached pixels but keep their own identity.
    let (twin, tv) = f.ready("twin.png", &second);
    assert_eq!(tv, v2);
    let t = get(&f, &twin, &tv, PreviewTier::Small).unwrap();
    assert_eq!((t.meta.cache, t.meta.asset_id.as_str()), (CacheState::Hit, twin.as_str()));
    // Unknown IDs, a mismatched ID/version pair and malformed IDs leak nothing.
    let unknown = uuid::Uuid::new_v4().to_string();
    assert_eq!(get(&f, &unknown, &v2, PreviewTier::Small).err(), Some(E::NotFound));
    assert_eq!(get(&f, &twin, &v1, PreviewTier::Small).err(), Some(E::StaleVersion));
    assert_eq!(get(&f, "../db.sqlite", &v2, PreviewTier::Small).err(), Some(E::InvalidArgument));
    assert!(f.service.status().counters.stale_rejected >= 2);
    // Revoking the source revokes previews even though cache records remain.
    f.catalog.lock().unwrap().remove_source(&f.source.id, true).unwrap();
    assert_eq!(get(&f, &id, &v2, PreviewTier::Small).err(), Some(E::NotFound));
    assert_eq!(fs::read(f.file("swap.png")).unwrap(), second);
}

#[cfg(unix)]
#[test]
fn a_symlink_swapped_in_after_authorization_cannot_escape_the_source() {
    let _guard = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let bytes = quadrants(64, 64);
    let (id, v) = f.ready("link.png", &bytes);
    let outside = f.dir.path().join("outside.png");
    fs::write(&outside, &bytes).unwrap();
    fs::remove_file(f.file("link.png")).unwrap();
    std::os::unix::fs::symlink(&outside, f.file("link.png")).unwrap();
    assert_eq!(get(&f, &id, &v, PreviewTier::Small).err(), Some(E::OriginalUnavailable));
    assert_eq!(f.service.status().entries, 0);
}

#[test]
fn offline_originals_serve_labeled_cached_references_but_never_decode() {
    let _guard = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let bytes = quadrants(300, 300);
    let (id, v) = f.ready("offline.png", &bytes);
    assert!(get(&f, &id, &v, PreviewTier::Small).unwrap().meta.original_available);
    f.catalog
        .lock()
        .unwrap()
        .db_connection()
        .execute("UPDATE sources SET available=0 WHERE id=?1", [&f.source.id])
        .unwrap();
    let p = get(&f, &id, &v, PreviewTier::Small).unwrap();
    assert!(p.meta.offline_reference && !p.meta.original_available);
    assert_eq!(p.meta.cache, CacheState::Hit);
    assert_eq!(get(&f, &id, &v, PreviewTier::Inspect).err(), Some(E::OriginalUnavailable));
    let s = f.service.status();
    assert_eq!((s.counters.generated, s.counters.offline_served), (1, 1));
}

#[test]
fn cancellation_stops_obsolete_work_while_coalesced_survivors_complete() {
    let _guard = SERIAL.lock().unwrap();
    let f = Fixture::new();
    // Inspect of 2048² needs no resize; decode + encode leaves a cancel window.
    let bytes = encode_png(2048, 2048, |x, y| [(x / 8) as u8, (y / 8) as u8, 90, 255]);
    let (id, v) = f.ready("large.png", &bytes);
    let wait_active = |n| {
        let start = Instant::now();
        while f.service.status().active != n {
            assert!(start.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(5));
        }
    };
    let r = request(&id, &v, PreviewTier::Inspect, None);
    let rid = r.request_id.clone();
    let s = f.service.clone();
    let running = thread::spawn(move || s.preview(&r));
    wait_active(1);
    let start = Instant::now();
    assert!(f.service.cancel(&rid));
    assert_eq!(running.join().unwrap().err(), Some(E::Cancelled));
    assert!(start.elapsed() < Duration::from_secs(3), "{:?}", start.elapsed());
    wait_active(0);
    assert_eq!(f.service.status().entries, 0);
    // Cancelling one of two coalesced requesters leaves shared work running.
    let a = request(&id, &v, PreviewTier::Inspect, None);
    let b = request(&id, &v, PreviewTier::Inspect, None);
    let a_id = a.request_id.clone();
    let (s1, s2) = (f.service.clone(), f.service.clone());
    let first = thread::spawn(move || s1.preview(&a));
    wait_active(1);
    let second = thread::spawn(move || s2.preview(&b));
    let start = Instant::now();
    while f.service.status().counters.coalesced == 0 {
        assert!(start.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(5));
    }
    f.service.cancel(&a_id);
    assert_eq!(first.join().unwrap().err(), Some(E::Cancelled));
    let survivor = second.join().unwrap().unwrap();
    assert_eq!((survivor.meta.width, survivor.meta.height), (2048, 2048));
    assert!(f.service.status().counters.cancelled >= 2);
    assert_eq!(fs::read(f.file("large.png")).unwrap(), bytes);
}

#[test]
fn repeated_browse_reuses_the_cache_for_every_visible_thumbnail() {
    let _guard = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let assets: Vec<_> = (0..40u32)
        .map(|i| f.ready(&format!("browse-{i}.png"), &encode_png(320, 240, |x, y| [i as u8 * 6, (x / 2) as u8, (y / 2) as u8, 255])))
        .collect();
    let pass = || {
        let start = Instant::now();
        let states: Vec<_> = assets
            .iter()
            .map(|(id, v)| get(&f, id, v, PreviewTier::Medium).unwrap().meta.cache)
            .collect();
        (start.elapsed(), states)
    };
    let (cold, cold_states) = pass();
    let (warm, warm_states) = pass();
    assert!(cold_states.iter().all(|s| *s == CacheState::Generated));
    assert!(warm_states.iter().all(|s| *s == CacheState::Hit));
    let s = f.service.status();
    assert_eq!((s.counters.generated, s.counters.hits), (40, 40));
    // Three thumbnail tiers per decode; quota is not reached.
    assert_eq!(s.entries, 120);
    assert!(warm < cold, "warm {warm:?} cold {cold:?}");
    eprintln!("SS-044 browse 40x320x240 medium: cold {cold:?}, warm {warm:?}, cache {} bytes", s.bytes);
}
