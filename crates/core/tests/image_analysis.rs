//! Real codecs and a real child process. All pixels are generated synthetic data.
use image::ImageEncoder;
use soundshelf_core::{
    catalog::{Catalog, Source},
    image_analysis::{
        AnalysisError as E, AnalysisWarning as W, ImageAnalysisRequest, ImageSelection,
        MAX_ENCODED, MAX_PIXELS, WORKER_HEAP,
    },
    image_contracts::{ImageFormat, TechnicalState, Transparency},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
static SERIAL: Mutex<()> = Mutex::new(());
fn worker() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_creativeshelf-image-worker"))
}
fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
struct Fixture {
    dir: tempfile::TempDir,
    c: Catalog,
    s: Source,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media");
        fs::create_dir(&root).unwrap();
        let c = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
        let s = c.add_source(&root).unwrap();
        Self { dir, c, s }
    }
    fn put(&mut self, name: &str, bytes: &[u8]) -> String {
        fs::write(Path::new(&self.s.root).join(name), bytes).unwrap();
        let selection = ImageSelection::capture(&self.c, &self.s.id, name).unwrap();
        let h = selection.hash(&AtomicBool::new(false)).unwrap();
        self.c
            .register_image(&self.s, name, Some(&h))
            .unwrap()
            .header
            .id
    }
    fn request(&self, id: &str) -> ImageAnalysisRequest {
        ImageAnalysisRequest::capture(&self.c, id).unwrap()
    }
}
fn png(w: u32, h: u32, color: png::ColorType, depth: png::BitDepth, pixels: &[u8]) -> Vec<u8> {
    let mut bytes = vec![];
    {
        let mut encoder = png::Encoder::new(&mut bytes, w, h);
        encoder.set_color(color);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
    }
    bytes
}
fn rgba() -> Vec<u8> {
    png(3, 2, png::ColorType::Rgba, png::BitDepth::Eight, &pixels())
}
fn pixels() -> Vec<u8> {
    vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 0, 255, 255, 255, 255, 0,
        255, 255,
    ]
}
fn chunk(kind: &[u8; 4], p: &[u8]) -> Vec<u8> {
    let mut out = (p.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(p);
    out.extend_from_slice(&crc32fast::hash(&out[4..]).to_be_bytes());
    out
}
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
fn error(f: &Fixture, id: &str) -> E {
    match f.request(id).analyze(&worker(), cancel()) {
        Err(e) => e,
        Ok(_) => panic!("Expected isolated image error"),
    }
}

#[test]
fn all_exif_orientations_match_independent_pixel_maps_and_preserve_originals() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let maps = [
        [0, 1, 2, 3, 4, 5],
        [2, 1, 0, 5, 4, 3],
        [5, 4, 3, 2, 1, 0],
        [3, 4, 5, 0, 1, 2],
        [0, 3, 1, 4, 2, 5],
        [3, 0, 4, 1, 5, 2],
        [5, 2, 4, 1, 3, 0],
        [2, 5, 1, 4, 0, 3],
    ];
    for o in 1..=8 {
        let bytes = inject(&rgba(), b"eXIf", &exif(o));
        let name = format!("oriented-{o}.png");
        let id = f.put(&name, &bytes);
        let result = f.request(&id).analyze(&worker(), cancel()).unwrap();
        let a = result.analysis();
        assert_eq!(a.profile.orientation, o);
        assert_eq!((a.profile.raw_width, a.profile.raw_height), (3, 2));
        let (w, h) = if o >= 5 { (2, 3) } else { (3, 2) };
        assert_eq!((a.profile.width, a.profile.height), (w, h));
        assert_eq!(a.aspect_ratio, [w, h]);
        let original = pixels();
        let expected: Vec<u8> = maps[(o - 1) as usize]
            .iter()
            .flat_map(|i| original[i * 4..i * 4 + 4].iter().copied())
            .collect();
        assert_eq!(
            a.display_rgba_hash,
            blake3::hash(&expected).to_hex().to_string()
        );
        let published = result
            .reverify(&AtomicBool::new(false))
            .unwrap()
            .publish(&mut f.c)
            .unwrap();
        assert_eq!(published.header.technical_state, TechnicalState::Ready);
        assert_eq!(fs::read(Path::new(&f.s.root).join(name)).unwrap(), bytes);
    }
}

#[test]
fn alpha_presence_actual_transparency_and_16bit_coverage_are_independent() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    for (name, bytes, channel, trans, transparent, partial, depth) in [
        (
            "rgb.png",
            png(1, 1, png::ColorType::Rgb, png::BitDepth::Eight, &[4, 8, 16]),
            false,
            Transparency::None,
            0,
            0,
            8,
        ),
        (
            "opaque.png",
            png(
                1,
                1,
                png::ColorType::Rgba,
                png::BitDepth::Eight,
                &[4, 8, 16, 255],
            ),
            true,
            Transparency::None,
            0,
            0,
            8,
        ),
        (
            "binary.png",
            png(
                2,
                1,
                png::ColorType::Rgba,
                png::BitDepth::Eight,
                &[4, 8, 16, 0, 4, 8, 16, 255],
            ),
            true,
            Transparency::Binary,
            1,
            0,
            8,
        ),
        (
            "partial.png",
            png(
                2,
                1,
                png::ColorType::Rgba,
                png::BitDepth::Eight,
                &[4, 8, 16, 128, 4, 8, 16, 255],
            ),
            true,
            Transparency::Partial,
            1,
            1,
            8,
        ),
        (
            "gray.png",
            png(1, 1, png::ColorType::Grayscale, png::BitDepth::Eight, &[64]),
            false,
            Transparency::None,
            0,
            0,
            8,
        ),
        (
            "gray-alpha.png",
            png(
                1,
                1,
                png::ColorType::GrayscaleAlpha,
                png::BitDepth::Eight,
                &[64, 128],
            ),
            true,
            Transparency::Partial,
            1,
            1,
            8,
        ),
        (
            "gray16.png",
            png(
                1,
                1,
                png::ColorType::Grayscale,
                png::BitDepth::Sixteen,
                &[128, 128],
            ),
            false,
            Transparency::None,
            0,
            0,
            16,
        ),
        // Alpha 1/65535 rounds to 0 in RGBA8 but must remain partial in measured facts.
        (
            "rgba16.png",
            png(
                1,
                1,
                png::ColorType::Rgba,
                png::BitDepth::Sixteen,
                &[255, 255, 0, 0, 0, 0, 0, 1],
            ),
            true,
            Transparency::Partial,
            1,
            1,
            16,
        ),
    ] {
        let id = f.put(name, &bytes);
        let result = f.request(&id).analyze(&worker(), cancel()).unwrap();
        let a = result.analysis();
        assert_eq!(
            (
                a.profile.alpha_channel,
                a.profile.transparency,
                a.transparent_pixels,
                a.partial_alpha_pixels,
                a.profile.bit_depth
            ),
            (channel, trans, transparent, partial, depth),
            "{name}"
        );
        assert_eq!(a.warnings.contains(&W::HighBitDepthDisplay8), depth == 16);
    }
    // Palette tRNS expands a real alpha channel; bit depth remains the encoded depth.
    let mut bytes = vec![];
    {
        let mut e = png::Encoder::new(&mut bytes, 2, 1);
        e.set_color(png::ColorType::Indexed);
        e.set_depth(png::BitDepth::One);
        e.set_palette(vec![255, 0, 0, 0, 0, 255]);
        e.set_trns(vec![255, 0]);
        e.write_header()
            .unwrap()
            .write_image_data(&[0b0100_0000])
            .unwrap();
    }
    let id = f.put("palette.png", &bytes);
    let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
    assert_eq!(
        (
            a.analysis().profile.bit_depth,
            a.analysis().profile.transparency
        ),
        (1, Transparency::Binary)
    );
}

#[test]
fn jpeg_and_webp_are_real_full_decodes_and_extension_is_only_a_hint() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let mut jpeg = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
        .write_image(
            &[64, 128, 192].repeat(16 * 8),
            16,
            8,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    let mut webp = vec![];
    image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
        .write_image(
            &[64, 128, 192, 128].repeat(16 * 8),
            16,
            8,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    for (name, bytes, format, alpha) in [
        ("wrong.png", jpeg, ImageFormat::Jpeg, false),
        ("correct.webp", webp, ImageFormat::Webp, true),
    ] {
        let id = f.put(name, &bytes);
        let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
        let a = a.analysis();
        assert_eq!(
            (
                a.profile.raw_width,
                a.profile.raw_height,
                a.profile.format,
                a.profile.alpha_channel
            ),
            (16, 8, format, alpha)
        );
        assert_eq!(
            a.warnings.contains(&W::ExtensionMismatch),
            name == "wrong.png"
        );
        let expected = [64u8, 128, 192, if alpha { 128 } else { 255 }];
        for c in a.display_corners {
            for (x, y) in c.into_iter().zip(expected) {
                assert!((i16::from(x) - i16::from(y)).abs() <= 3);
            }
        }
    }
    let bytes = include_bytes!("../../../fixtures/images/solid-lossy.webp");
    let id = f.put("lossy.webp", bytes);
    let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
    assert_eq!(a.analysis().profile.format, ImageFormat::Webp);
    assert_eq!(
        (a.analysis().profile.width, a.analysis().profile.height),
        (16, 8)
    );
}

#[test]
fn icc_srgb_and_wide_gamut_conversion_preserve_straight_alpha() {
    use std::io::Write;
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let source_pixel = [128, 64, 32, 128];
    let base = png(
        1,
        1,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &source_pixel,
    );
    for (name, profile, expected, tolerance) in [
        (
            "srgb.png",
            moxcms::ColorProfile::new_srgb(),
            [128u8, 64, 32, 128],
            1,
        ),
        // Independent matrix/TRC calculation: Adobe RGB(1998) gamma 2.2 -> sRGB.
        (
            "adobe.png",
            moxcms::ColorProfile::new_adobe_rgb(),
            [146u8, 62, 23, 128],
            3,
        ),
    ] {
        let icc = profile.encode().unwrap();
        let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::default());
        z.write_all(&icc).unwrap();
        let mut p = b"generated\0\0".to_vec();
        p.extend(z.finish().unwrap());
        let bytes = inject(&base, b"iCCP", &p);
        let id = f.put(name, &bytes);
        let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
        let a = a.analysis();
        assert!(a
            .profile
            .source_color_profile
            .as_ref()
            .unwrap()
            .starts_with("icc-blake3:"));
        assert!(!a.warnings.contains(&W::AssumedSrgb));
        for (x, y) in a.display_corners[0].into_iter().zip(expected) {
            assert!(
                (i16::from(x) - i16::from(y)).abs() <= tolerance,
                "{name}: {:?}",
                a.display_corners[0]
            );
        }
        assert_eq!(a.partial_alpha_pixels, 1);
        assert_eq!(a.display_corners[0][3], 128);
    }
    let bytes = inject(&base, b"gAMA", &100000u32.to_be_bytes());
    let id = f.put("unsupported-gamma.png", &bytes);
    assert_eq!(error(&f, &id), E::UnsupportedColorProfile);
}

#[test]
fn real_animation_and_corrupt_inputs_fail_without_readiness_or_process_crash() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    for (name, bytes) in [
        (
            "animated.png",
            include_bytes!("../../../fixtures/images/two-frames.png").as_slice(),
        ),
        (
            "animated.webp",
            include_bytes!("../../../fixtures/images/two-frames.webp").as_slice(),
        ),
    ] {
        let id = f.put(name, bytes);
        assert_eq!(error(&f, &id), E::AnimationUnsupported);
        assert_eq!(
            f.c.image(&id, None).unwrap().header.technical_state,
            TechnicalState::Pending
        );
    }
    for (name, bytes) in [(
        "truncated.webp",
        include_bytes!("../../../fixtures/images/solid-lossy.webp").as_slice(),
    )] {
        let id = f.put(name, &bytes[..bytes.len() - 1]);
        assert_eq!(error(&f, &id), E::InvalidData);
    }
    let mut truncated_jpeg = vec![];
    image::codecs::jpeg::JpegEncoder::new(&mut truncated_jpeg)
        .write_image(&[64u8; 3], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    truncated_jpeg.truncate(truncated_jpeg.len() - 2);
    let id = f.put("truncated.jpg", &truncated_jpeg);
    assert_eq!(error(&f, &id), E::InvalidData);
    let full = rgba();
    let mut bad_crc = full.clone();
    bad_crc[40] ^= 1;
    let mut bad_icc = b"bad\0\0".to_vec();
    bad_icc.extend([1, 2, 3]);
    for (i, bytes) in [
        full[..full.len() - 1].to_vec(),
        bad_crc,
        inject(&full, b"eXIf", b"broken EXIF"),
        inject(&full, b"iCCP", &bad_icc),
        b"<svg>unsupported</svg>".to_vec(),
    ]
    .into_iter()
    .enumerate()
    {
        let id = f.put(&format!("bad-{i}.png"), &bytes);
        let e = error(&f, &id);
        assert!(matches!(e, E::InvalidData | E::UnsupportedFormat), "{e:?}");
    }
    // A failed decode does not poison subsequent worker slots.
    let id = f.put("good.png", &rgba());
    assert!(f.request(&id).analyze(&worker(), cancel()).is_ok());
}

#[test]
fn dimensions_metadata_and_encoded_limits_are_enforced_before_decode() {
    use std::io::Write;
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let original = rgba();
    for (w, h, expected) in [
        (0u32, 2u32, E::InvalidData),
        (32769, 1, E::LimitExceeded),
        (8192, 8192, E::LimitExceeded),
    ] {
        let mut ihdr = original[16..29].to_vec();
        ihdr[..4].copy_from_slice(&w.to_be_bytes());
        ihdr[4..8].copy_from_slice(&h.to_be_bytes());
        let mut bytes = original[..8].to_vec();
        bytes.extend(chunk(b"IHDR", &ihdr));
        bytes.extend_from_slice(&original[33..]);
        let id = f.put(&format!("dimensions-{w}.png"), &bytes);
        assert_eq!(error(&f, &id), expected);
    }
    let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::best());
    z.write_all(&vec![b'x'; 1_048_577]).unwrap();
    let mut text = b"text\0\0".to_vec();
    text.extend(z.finish().unwrap());
    for kind in [b"zTXt", b"iCCP"] {
        let id = f.put("metadata-bomb.png", &inject(&original, kind, &text));
        assert_eq!(error(&f, &id), E::LimitExceeded);
    }
    let huge = Path::new(&f.s.root).join("huge.png");
    let file = fs::File::create(&huge).unwrap();
    file.set_len(MAX_ENCODED + 1).unwrap();
    let s = ImageSelection::capture(&f.c, &f.s.id, "huge.png").unwrap();
    assert_eq!(s.hash(&AtomicBool::new(false)), Err(E::LimitExceeded));
}

#[test]
fn source_version_membership_and_publication_fences_reject_stale_work() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let id = f.put("image.png", &rgba());
    let request = f.request(&id);
    fs::write(
        Path::new(&f.s.root).join("image.png"),
        png(1, 1, png::ColorType::Rgb, png::BitDepth::Eight, &[1, 2, 3]),
    )
    .unwrap();
    assert!(matches!(
        request.analyze(&worker(), cancel()),
        Err(E::SourceChanged)
    ));
    let id = f.put("image.png", &rgba());
    let result = f.request(&id).analyze(&worker(), cancel()).unwrap();
    fs::write(
        Path::new(&f.s.root).join("image.png"),
        png(1, 1, png::ColorType::Rgb, png::BitDepth::Eight, &[1, 2, 3]),
    )
    .unwrap();
    assert!(matches!(
        result.reverify(&AtomicBool::new(false)),
        Err(E::SourceChanged)
    ));
    let id = f.put("image.png", &rgba());
    let result = f
        .request(&id)
        .analyze(&worker(), cancel())
        .unwrap()
        .reverify(&AtomicBool::new(false))
        .unwrap();
    f.c.db_connection()
        .execute(
            "UPDATE sources SET generation=generation+1 WHERE id=?1",
            [&f.s.id],
        )
        .unwrap();
    assert!(matches!(result.publish(&mut f.c), Err(E::SourceChanged)));
    assert_eq!(
        f.c.image(&id, None).unwrap().header.technical_state,
        TechnicalState::Pending
    );
    f.c.db_connection()
        .execute("UPDATE sources SET scope='files' WHERE id=?1", [&f.s.id])
        .unwrap();
    assert!(matches!(
        ImageSelection::capture(&f.c, &f.s.id, "sibling.png"),
        Err(E::PermissionDenied)
    ));
    assert!(matches!(
        ImageSelection::capture(&f.c, &f.s.id, "../escape.png"),
        Err(E::PermissionDenied)
    ));
    f.c.set_available(&f.s.id, false).unwrap();
    assert!(matches!(
        ImageAnalysisRequest::capture(&f.c, &id),
        Err(E::PermissionDenied) | Err(E::SourceUnavailable)
    ));
}

#[test]
fn maximum_fixture_peak_memory_cancellation_and_real_worker_deadline_are_bounded() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let side = 4096;
    assert_eq!(u64::from(side) * u64::from(side), MAX_PIXELS);
    let bytes = png(
        side,
        side,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &[32, 64, 128, 128].repeat((side * side) as usize),
    );
    let id = f.put("maximum.png", &bytes);
    let now = Instant::now();
    let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
    eprintln!(
        "SS042 admitted_max_pixels={} peak_worker_heap={} elapsed_ms={}",
        MAX_PIXELS,
        a.analysis().peak_heap_bytes,
        now.elapsed().as_millis()
    );
    assert!(a.analysis().peak_heap_bytes <= WORKER_HEAP as u64);
    assert!(a.analysis().peak_heap_bytes > MAX_PIXELS * 4);
    assert_eq!(a.analysis().partial_alpha_pixels, MAX_PIXELS);
    let now = Instant::now();
    assert!(matches!(
        f.request(&id)
            .analyze_with_deadline(&worker(), cancel(), Duration::from_millis(1)),
        Err(E::TimedOut)
    ));
    assert!(now.elapsed() < Duration::from_secs(3));
    let flag = cancel();
    let change = flag.clone();
    let trigger = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        change.store(true, Ordering::Relaxed);
    });
    let now = Instant::now();
    assert!(matches!(
        f.request(&id).analyze(&worker(), flag),
        Err(E::Cancelled)
    ));
    trigger.join().unwrap();
    assert!(now.elapsed() < Duration::from_secs(3));
    let request = f.request(&id);
    let other = f.request(&id);
    let flag = cancel();
    let stop = flag.clone();
    let running = thread::spawn(move || request.analyze(&worker(), flag));
    let end = Instant::now() + Duration::from_secs(3);
    while !soundshelf_core::image_analysis::worker_busy() {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(other.analyze(&worker(), cancel()), Err(E::Busy)));
    stop.store(true, Ordering::Relaxed);
    assert!(matches!(running.join().unwrap(), Err(E::Cancelled)));
    let stopped = Arc::new(AtomicBool::new(true));
    assert!(matches!(
        f.request(&id).analyze(&worker(), stopped),
        Err(E::Cancelled)
    ));
    assert_eq!(
        fs::read(Path::new(&f.s.root).join("maximum.png")).unwrap(),
        bytes
    );
}

#[cfg(unix)]
#[test]
fn escaped_symlink_and_failed_worker_do_not_publish() {
    use std::os::unix::fs::symlink;
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let id = f.put("image.png", &rgba());
    assert!(matches!(
        f.request(&id)
            .analyze(Path::new("/nonexistent/image-worker"), cancel()),
        Err(E::WorkerUnavailable)
    ));
    fs::remove_file(Path::new(&f.s.root).join("image.png")).unwrap();
    let outside = f.dir.path().join("private.png");
    fs::write(&outside, rgba()).unwrap();
    symlink(&outside, Path::new(&f.s.root).join("image.png")).unwrap();
    assert_eq!(error(&f, &id), E::SourceUnavailable);
}

#[test]
fn exif_metadata_in_jpeg_and_extended_webp_uses_the_same_coordinate_policy() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let mut jpeg = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
        .write_image(
            &[64, 128, 192].repeat(6),
            3,
            2,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    let mut app = b"Exif\0\0".to_vec();
    app.extend(exif(6));
    let mut bytes = jpeg[..2].to_vec();
    bytes.extend([0xff, 0xe1]);
    bytes.extend(((app.len() + 2) as u16).to_be_bytes());
    bytes.extend(app);
    bytes.extend_from_slice(&jpeg[2..]);
    let id = f.put("oriented.jpg", &bytes);
    let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
    assert_eq!(
        (
            a.analysis().profile.orientation,
            a.analysis().profile.width,
            a.analysis().profile.height
        ),
        (6, 2, 3)
    );
    let mut simple = vec![];
    image::codecs::webp::WebPEncoder::new_lossless(&mut simple)
        .write_image(&pixels(), 3, 2, image::ExtendedColorType::Rgba8)
        .unwrap();
    fn riff(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = kind.to_vec();
        v.extend((data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 != 0 {
            v.push(0);
        }
        v
    }
    let mut body = b"WEBP".to_vec();
    body.extend(riff(b"VP8X", &[8, 0, 0, 0, 2, 0, 0, 1, 0, 0]));
    body.extend_from_slice(&simple[12..]);
    body.extend(riff(b"EXIF", &exif(6)));
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((body.len() as u32).to_le_bytes());
    bytes.extend(body);
    let id = f.put("oriented.webp", &bytes);
    let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
    assert_eq!(
        (
            a.analysis().profile.orientation,
            a.analysis().profile.width,
            a.analysis().profile.height
        ),
        (6, 2, 3)
    );
    let expected: Vec<u8> = [3, 0, 4, 1, 5, 2]
        .into_iter()
        .flat_map(|i| pixels()[i * 4..i * 4 + 4].to_vec())
        .collect();
    assert_eq!(
        a.analysis().display_rgba_hash,
        blake3::hash(&expected).to_hex().to_string()
    );
}

#[test]
fn max_16bit_oriented_fixture_is_admitted_within_the_worker_heap() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let data = [0x80, 0x80, 0x40, 0x40, 0x20, 0x20, 0x80, 0x80].repeat(MAX_PIXELS as usize);
    let bytes = inject(
        &png(
            4096,
            4096,
            png::ColorType::Rgba,
            png::BitDepth::Sixteen,
            &data,
        ),
        b"eXIf",
        &exif(6),
    );
    drop(data);
    let id = f.put("maximum16.png", &bytes);
    let now = Instant::now();
    let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
    assert_eq!(a.analysis().profile.bit_depth, 16);
    assert_eq!(a.analysis().partial_alpha_pixels, MAX_PIXELS);
    assert!(a.analysis().peak_heap_bytes <= WORKER_HEAP as u64);
    eprintln!(
        "SS042 admitted_max_16bit_pixels={} peak_worker_heap={} elapsed_ms={}",
        MAX_PIXELS,
        a.analysis().peak_heap_bytes,
        now.elapsed().as_millis()
    );
}

#[test]
fn same_size_same_timestamp_replacement_is_detected_by_bytes() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let before = inject(&rgba(), b"tEXt", b"note\0aaa");
    let after = inject(&rgba(), b"tEXt", b"note\0bbb");
    assert_eq!(before.len(), after.len());
    let id = f.put("same-stat.png", &before);
    let request = f.request(&id);
    let path = Path::new(&f.s.root).join("same-stat.png");
    let stamp = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, &after).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    assert!(matches!(
        request.analyze(&worker(), cancel()),
        Err(E::SourceChanged)
    ));
    assert_eq!(
        f.c.image(&id, None).unwrap().header.technical_state,
        TechnicalState::Pending
    );
}

#[test]
fn lossless_webp_alpha_hints_and_maximum_header_width_cannot_erase_pixels() {
    let _guard = SERIAL.lock().unwrap();
    let mut f = Fixture::new();
    let data = [32u8, 64, 128, 128].repeat(3);
    let mut bytes = vec![];
    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
        .write_image(&data, 3, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert_eq!(&bytes[12..16], b"VP8L");
    bytes[24] &= !0x10;
    let id = f.put("false-hint.webp", &bytes);
    let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
    assert_eq!(a.analysis().partial_alpha_pixels, 3);
    assert_eq!(a.analysis().profile.transparency, Transparency::Partial);
    assert_eq!(
        a.analysis().display_rgba_hash,
        blake3::hash(&data).to_hex().to_string()
    );
    assert_eq!(
        a.analysis().content_hash,
        blake3::hash(&bytes).to_hex().to_string()
    );
    assert_eq!(
        fs::read(Path::new(&f.s.root).join("false-hint.webp")).unwrap(),
        bytes
    );
    let data = [32u8, 64, 128, 255].repeat(16384);
    let mut bytes = vec![];
    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
        .write_image(&data, 16384, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    let id = f.put("maximum-width.webp", &bytes);
    let a = f.request(&id).analyze(&worker(), cancel()).unwrap();
    assert_eq!(
        (a.analysis().profile.width, a.analysis().profile.height),
        (16384, 1)
    );
    assert_eq!(a.analysis().profile.transparency, Transparency::None);
    assert!(a.analysis().profile.alpha_channel);
    assert_eq!(
        a.analysis().display_rgba_hash,
        blake3::hash(&data).to_hex().to_string()
    );
}
