//! SS-050 image-AI evaluation: a reproducible generated corpus with provenance,
//! held-out labels, caption rubric, retrieval judgments, a lexical baseline,
//! pre-registered thresholds and a bounded runner.
//!
//! The runner reads only images listed in a manifest whose BLAKE3 digest
//! matches, talks only to the adapters it is handed, and stops at an explicit
//! request allowance. It never touches a user's catalog.
use crate::ai_provider::{ProviderError, ProviderKind};
use crate::ai_provider_wire::{ProviderAdapter, Usage, MAX_EMBED_INPUTS};
use crate::image_ai::{describe, insight_text, prepare_pixels, EmbeddingIndex, EmbeddingSpace, Insight};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const CORPUS_ID: &str = "creativeshelf-generated/v1";
pub const MANIFEST_SCHEMA: &str = "creativeshelf-image-eval-manifest/v1";
pub const REPORT_SCHEMA: &str = "creativeshelf-image-eval-report/v1";
/// Shipping UI language. Queries and labels are English only; other languages
/// are not claimed.
pub const LANGUAGES: &[&str] = &["en"];
pub const MAX_MANIFEST_ITEMS: usize = 1000;
pub const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    /// Used for prompt checks and choosing the retrieval cut-off.
    Dev,
    /// Scored once against the pre-registered thresholds.
    Heldout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestItem {
    pub id: String,
    /// Path relative to the manifest's folder.
    pub file: String,
    pub blake3: String,
    pub category: String,
    pub split: Split,
    pub provenance: String,
    /// Name a creator might have given the file; the lexical baseline sees only this.
    pub filename_hint: String,
    /// Gold tags. Each inner list is one fact with accepted spellings.
    pub tags: Vec<Vec<String>>,
    /// Caption rubric: each fact must be named (any spelling)...
    pub caption_required: Vec<Vec<String>>,
    /// ...and none of these words may appear.
    pub caption_forbidden: Vec<String>,
    /// Visibly ambiguous by construction; "uncertain" is an acceptable answer.
    pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub id: String,
    pub text: String,
    pub language: String,
    /// Relevant item IDs. Empty means out of domain: no result is correct.
    pub relevant: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub corpus: String,
    pub license: String,
    pub languages: Vec<String>,
    pub categories_pending: Vec<String>,
    pub items: Vec<ManifestItem>,
    pub queries: Vec<Query>,
}

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("Manifest: {0}")]
    Manifest(String),
    #[error("File {0} does not match its manifest digest")]
    Digest(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image {0} could not be decoded or prepared")]
    Image(String),
}

fn manifest_error(m: impl Into<String>) -> EvalError {
    EvalError::Manifest(m.into())
}

impl Manifest {
    pub fn validate(&self) -> Result<(), EvalError> {
        if self.schema != MANIFEST_SCHEMA {
            return Err(manifest_error(format!("unsupported schema {}", self.schema)));
        }
        if self.items.is_empty() || self.items.len() > MAX_MANIFEST_ITEMS {
            return Err(manifest_error("item count out of range"));
        }
        let mut ids = HashSet::new();
        for i in &self.items {
            let safe = !i.file.is_empty()
                && !i.file.contains('\\')
                && !i.file.starts_with('/')
                && i.file.split('/').all(|s| !s.is_empty() && s != "." && s != "..");
            if !safe {
                return Err(manifest_error(format!("unsafe path for {}", i.id)));
            }
            if !ids.insert(i.id.as_str()) || i.blake3.len() != 64 || i.provenance.is_empty() {
                return Err(manifest_error(format!("invalid item {}", i.id)));
            }
        }
        for q in &self.queries {
            if q.relevant.iter().any(|r| !ids.contains(r.as_str())) {
                return Err(manifest_error(format!("query {} names an unknown item", q.id)));
            }
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self, EvalError> {
        if std::fs::metadata(path)?.len() > MAX_MANIFEST_BYTES {
            return Err(manifest_error("manifest too large"));
        }
        let m: Manifest = serde_json::from_slice(&std::fs::read(path)?).map_err(|e| manifest_error(e.to_string()))?;
        m.validate()?;
        Ok(m)
    }
}

/// Reads one approved image: listed in the manifest and matching its digest.
pub fn read_verified(dir: &Path, item: &ManifestItem) -> Result<Vec<u8>, EvalError> {
    let bytes = std::fs::read(dir.join(&item.file))?;
    if blake3::hash(&bytes).to_hex().as_str() != item.blake3 {
        return Err(EvalError::Digest(item.file.clone()));
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Generated corpus. Integer-only drawing so every platform produces identical
// bytes; the committed manifest's digests prove it.

type Px = [u8; 4];
const WHITE: Px = [255, 255, 255, 255];
const BLACK: Px = [20, 20, 20, 255];
const RED: Px = [220, 30, 30, 255];
const BLUE: Px = [30, 70, 210, 255];
const GREEN: Px = [30, 160, 60, 255];
const YELLOW: Px = [245, 205, 20, 255];
const PURPLE: Px = [130, 50, 170, 255];
const ORANGE: Px = [245, 130, 20, 255];
const GRAY: Px = [128, 128, 128, 255];
const PAPER: Px = [248, 244, 232, 255];
const CLEAR: Px = [0, 0, 0, 0];

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }
}

fn canvas(w: u32, h: u32, c: Px) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba(c))
}
/// Source-over blend with integer arithmetic.
fn blend(img: &mut RgbaImage, x: i64, y: i64, c: Px) {
    if x < 0 || y < 0 || x >= img.width() as i64 || y >= img.height() as i64 {
        return;
    }
    let d = img.get_pixel_mut(x as u32, y as u32);
    let a = c[3] as u32;
    if a == 255 || d.0[3] == 0 {
        d.0 = c;
        return;
    }
    for k in 0..3 {
        d.0[k] = ((c[k] as u32 * a + d.0[k] as u32 * (255 - a)) / 255) as u8;
    }
    d.0[3] = (a + d.0[3] as u32 * (255 - a) / 255).min(255) as u8;
}
fn fill_where(img: &mut RgbaImage, c: Px, inside: impl Fn(i64, i64) -> bool) {
    for y in 0..img.height() as i64 {
        for x in 0..img.width() as i64 {
            if inside(x, y) {
                blend(img, x, y, c);
            }
        }
    }
}
fn disc(img: &mut RgbaImage, cx: i64, cy: i64, r: i64, c: Px) {
    fill_where(img, c, |x, y| (x - cx).pow(2) + (y - cy).pow(2) <= r * r);
}
fn rect(img: &mut RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64, c: Px) {
    fill_where(img, c, |x, y| x >= x0 && x < x1 && y >= y0 && y < y1);
}
fn rounded(img: &mut RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64, r: i64, c: Px) {
    fill_where(img, c, |x, y| {
        if x < x0 || x >= x1 || y < y0 || y >= y1 {
            return false;
        }
        let cx = x.clamp(x0 + r, x1 - 1 - r);
        let cy = y.clamp(y0 + r, y1 - 1 - r);
        (x - cx).pow(2) + (y - cy).pow(2) <= r * r
    });
}
/// Even-odd point in polygon, integer vertices.
fn polygon(img: &mut RgbaImage, pts: &[(i64, i64)], c: Px) {
    fill_where(img, c, |x, y| {
        let (px, py) = (2 * x + 1, 2 * y + 1);
        let mut inside = false;
        for i in 0..pts.len() {
            let (ax, ay) = (2 * pts[i].0, 2 * pts[i].1);
            let (bx, by) = (2 * pts[(i + 1) % pts.len()].0, 2 * pts[(i + 1) % pts.len()].1);
            if (ay > py) != (by > py) && (px * (by - ay) < (bx - ax) * (py - ay) + ax * (by - ay)) == (by > ay) {
                inside = !inside;
            }
        }
        inside
    });
}
fn triangle(img: &mut RgbaImage, cx: i64, cy: i64, s: i64, c: Px) {
    polygon(img, &[(cx, cy - s), (cx + s, cy + s * 3 / 4), (cx - s, cy + s * 3 / 4)], c);
}
/// Five-pointed star from fixed unit coordinates (×1000), no trigonometry.
const STAR: [(i64, i64); 10] =
    [(0, -1000), (225, -309), (951, -309), (363, 118), (588, 809), (0, 382), (-588, 809), (-363, 118), (-951, -309), (-225, -309)];
fn star(img: &mut RgbaImage, cx: i64, cy: i64, r: i64, c: Px) {
    let pts: Vec<_> = STAR.iter().map(|(x, y)| (cx + x * r / 1000, cy + y * r / 1000)).collect();
    polygon(img, &pts, c);
}
fn octagon(img: &mut RgbaImage, cx: i64, cy: i64, r: i64, c: Px) {
    let a = r * 414 / 1000;
    polygon(img, &[(cx - a, cy - r), (cx + a, cy - r), (cx + r, cy - a), (cx + r, cy + a), (cx + a, cy + r), (cx - a, cy + r), (cx - r, cy + a), (cx - r, cy - a)], c);
}
/// Lambert-shaded sphere, light from the upper left. Only +, *, / and sqrt,
/// which IEEE 754 rounds identically everywhere.
fn sphere(img: &mut RgbaImage, cx: i64, cy: i64, r: i64, c: Px) {
    let (lx, ly, lz) = (-0.5f64, -0.6f64, 0.6245f64);
    for y in cy - r..=cy + r {
        for x in cx - r..=cx + r {
            let (dx, dy) = ((x - cx) as f64 / r as f64, (y - cy) as f64 / r as f64);
            let d = dx * dx + dy * dy;
            if d > 1.0 {
                continue;
            }
            let dz = (1.0 - d).sqrt();
            let light = (dx * lx + dy * ly + dz * lz).max(0.0) * 0.8 + 0.2;
            let k = (light * 1000.0) as u32;
            blend(img, x, y, [(c[0] as u32 * k / 1000) as u8, (c[1] as u32 * k / 1000) as u8, (c[2] as u32 * k / 1000) as u8, 255]);
        }
    }
}
fn vertical_gradient(img: &mut RgbaImage, top: Px, bottom: Px) {
    let h = img.height().max(2) - 1;
    for y in 0..img.height() {
        let t = y;
        let c: Px = std::array::from_fn(|k| ((top[k] as u32 * (h - t) + bottom[k] as u32 * t) / h) as u8);
        for x in 0..img.width() {
            img.put_pixel(x, y, Rgba(c));
        }
    }
}
fn noise(img: &mut RgbaImage, seed: u64, amount: u32) {
    let mut r = Lcg(seed);
    for p in img.pixels_mut() {
        let delta = r.below(amount * 2 + 1) as i32 - amount as i32;
        for k in 0..3 {
            p.0[k] = (p.0[k] as i32 + delta).clamp(0, 255) as u8;
        }
    }
}
/// Separable integer box blur, edges clamped.
fn box_blur(img: &mut RgbaImage, radius: i64) {
    for horizontal in [true, false] {
        let src = img.clone();
        let (w, h) = (img.width() as i64, img.height() as i64);
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 4];
                for d in -radius..=radius {
                    let (sx, sy) = if horizontal { ((x + d).clamp(0, w - 1), y) } else { (x, (y + d).clamp(0, h - 1)) };
                    let p = src.get_pixel(sx as u32, sy as u32).0;
                    for k in 0..4 {
                        sum[k] += p[k] as u32;
                    }
                }
                let n = (2 * radius + 1) as u32;
                img.put_pixel(x as u32, y as u32, Rgba(std::array::from_fn(|k| (sum[k] / n) as u8)));
            }
        }
    }
}
/// Watercolor wash: many translucent overlapping dabs, then a soft blur.
fn wash(img: &mut RgbaImage, seed: u64, cx: i64, cy: i64, spread: i64, c: Px) {
    let mut r = Lcg(seed);
    for _ in 0..90 {
        let x = cx + r.below(2 * spread as u32) as i64 - spread;
        let y = cy + r.below(2 * spread as u32) as i64 - spread;
        if (x - cx).pow(2) + (y - cy).pow(2) > spread * spread {
            continue;
        }
        disc(img, x, y, 12 + r.below(22) as i64, [c[0], c[1], c[2], 46]);
    }
}

/// 5×7 glyphs for the words in the corpus; each row is 5 bits, MSB on the left.
fn glyph(ch: char) -> [u8; 7] {
    match ch {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        _ => [0; 7],
    }
}
fn text(img: &mut RgbaImage, x0: i64, y0: i64, scale: i64, word: &str, c: Px) {
    for (i, ch) in word.chars().enumerate() {
        let g = glyph(ch);
        let ox = x0 + i as i64 * 6 * scale;
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5 {
                if bits & (0x10 >> col) != 0 {
                    let (x, y) = (ox + col as i64 * scale, y0 + row as i64 * scale);
                    rect(img, x, y, x + scale, y + scale, c);
                }
            }
        }
    }
}
fn word_width(word: &str, scale: i64) -> i64 {
    word.chars().count() as i64 * 6 * scale - scale
}
fn centered_text(img: &mut RgbaImage, scale: i64, word: &str, c: Px) {
    let (w, h) = (img.width() as i64, img.height() as i64);
    text(img, (w - word_width(word, scale)) / 2, (h - 7 * scale) / 2, scale, word, c);
}
fn window(img: &mut RgbaImage, dark: bool) {
    let (bg, bar, panel) = if dark { ([32, 33, 38, 255], [55, 57, 66, 255], [44, 46, 54, 255]) } else { (WHITE, [222, 224, 230, 255], [242, 243, 246, 255]) };
    let (w, h) = (img.width() as i64, img.height() as i64);
    rect(img, 0, 0, w, h, bg);
    rect(img, 0, 0, w, 28, bar);
    for (i, c) in [[236, 95, 90, 255], [245, 190, 70, 255], [100, 200, 90, 255]].iter().enumerate() {
        disc(img, 16 + i as i64 * 20, 14, 6, *c);
    }
    rect(img, 0, 28, 90, h, panel);
    for i in 0..5 {
        rect(img, 14, 48 + i * 26, 74, 58 + i * 26, if dark { [90, 92, 104, 255] } else { [200, 202, 210, 255] });
    }
}

type Draw = fn() -> RgbaImage;
struct Spec {
    id: &'static str,
    category: &'static str,
    filename_hint: &'static str,
    tags: &'static [&'static [&'static str]],
    required: &'static [&'static [&'static str]],
    forbidden: &'static [&'static str],
    ambiguous: bool,
    draw: Draw,
}

const CIRCLE: &[&str] = &["circle", "round", "disc", "disk", "dot", "ball"];
const SQUARE: &[&str] = &["square", "box", "rectangle"];
const TRIANGLE: &[&str] = &["triangle", "triangular"];
const STARS: &[&str] = &["star", "stars"];
const SPHERE: &[&str] = &["sphere", "ball", "orb", "globe"];
const RED_W: &[&str] = &["red", "crimson", "scarlet"];
const BLUE_W: &[&str] = &["blue", "navy", "cobalt"];
const GREEN_W: &[&str] = &["green"];
const YELLOW_W: &[&str] = &["yellow", "gold", "golden"];
const PURPLE_W: &[&str] = &["purple", "violet", "magenta"];
const ORANGE_W: &[&str] = &["orange"];
const WATERCOLOR: &[&str] = &["watercolor", "watercolour", "painting", "painted", "paint"];
const COLLAGE: &[&str] = &["collage", "mosaic", "patchwork"];
const RENDER: &[&str] = &["3d", "3d render", "render", "rendered", "cgi", "three-dimensional"];
const SCREENSHOT: &[&str] = &["screenshot", "user interface", "ui", "interface", "app", "application", "window", "software"];
const TEXT_W: &[&str] = &["text", "typography", "lettering", "word", "letters"];
const TRANSPARENT: &[&str] = &["transparent background", "transparent", "cutout", "isolated"];
const DARK: &[&str] = &["dark mode", "dark theme", "dark"];
const DOTS: &[&str] = &["dots", "polka dots", "pattern", "dotted"];
const STRIPES: &[&str] = &["stripes", "striped", "lines"];
const SIGN: &[&str] = &["sign", "stop sign", "octagon"];
const SUN: &[&str] = &["sun", "sunset", "sunrise"];
const PANORAMA: &[&str] = &["panorama", "panoramic", "banner", "wide"];
const NOISE: &[&str] = &["noise", "grain", "static", "texture"];
const CUBE: &[&str] = &["cube", "box", "block"];

const SPECS: &[Spec] = &[
    // Flat control shapes.
    Spec { id: "flat-red-circle", category: "flat", filename_hint: "red-ball.png", tags: &[CIRCLE, RED_W], required: &[CIRCLE, RED_W], forbidden: &["blue", "green", "square", "triangle"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, WHITE); disc(&mut i, 192, 128, 80, RED); i } },
    Spec { id: "flat-blue-square", category: "flat", filename_hint: "IMG_0042.png", tags: &[SQUARE, BLUE_W], required: &[SQUARE, BLUE_W], forbidden: &["red", "green", "circle", "triangle"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, WHITE); rect(&mut i, 122, 58, 262, 198, BLUE); i } },
    Spec { id: "flat-green-triangle", category: "flat", filename_hint: "shape_final_v2.png", tags: &[TRIANGLE, GREEN_W], required: &[TRIANGLE, GREEN_W], forbidden: &["red", "blue", "circle", "square"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, WHITE); triangle(&mut i, 192, 128, 90, GREEN); i } },
    Spec { id: "flat-yellow-star", category: "flat", filename_hint: "star.png", tags: &[STARS, YELLOW_W], required: &[STARS, YELLOW_W], forbidden: &["red", "blue", "green", "circle"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, [40, 40, 70, 255]); star(&mut i, 192, 128, 100, YELLOW); i } },
    // Watercolor.
    Spec { id: "wc-red-wash", category: "watercolor", filename_hint: "scan_003.png", tags: &[WATERCOLOR, RED_W], required: &[WATERCOLOR, RED_W], forbidden: &["blue", "green", "photo"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, PAPER); noise(&mut i, 11, 6); wash(&mut i, 12, 192, 128, 90, RED); box_blur(&mut i, 3); i } },
    Spec { id: "wc-blue-wash", category: "watercolor", filename_hint: "sky_wash.png", tags: &[WATERCOLOR, BLUE_W], required: &[WATERCOLOR, BLUE_W], forbidden: &["red", "green", "photo"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, PAPER); noise(&mut i, 21, 6); wash(&mut i, 22, 150, 110, 100, BLUE); wash(&mut i, 23, 260, 150, 70, BLUE); box_blur(&mut i, 3); i } },
    Spec { id: "wc-green-yellow", category: "watercolor", filename_hint: "Untitled-1.png", tags: &[WATERCOLOR, GREEN_W, YELLOW_W], required: &[WATERCOLOR, GREEN_W], forbidden: &["red", "blue", "photo"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, PAPER); noise(&mut i, 31, 6); wash(&mut i, 32, 130, 128, 80, GREEN); wash(&mut i, 33, 260, 128, 80, YELLOW); box_blur(&mut i, 3); i } },
    Spec { id: "wc-purple-circle", category: "watercolor", filename_hint: "moon_study.png", tags: &[WATERCOLOR, PURPLE_W, CIRCLE], required: &[WATERCOLOR, PURPLE_W], forbidden: &["green", "photo"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, PAPER); noise(&mut i, 41, 6); wash(&mut i, 42, 192, 128, 70, PURPLE); disc(&mut i, 192, 128, 60, [130, 50, 170, 70]); box_blur(&mut i, 3); i } },
    // Collage.
    Spec { id: "collage-grid", category: "collage", filename_hint: "moodboard.png", tags: &[COLLAGE, RED_W, BLUE_W, YELLOW_W], required: &[COLLAGE], forbidden: &["photo of a person"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, PAPER); for (k, c) in [RED, BLUE, YELLOW, GREEN, ORANGE, PURPLE].iter().enumerate() { let (x, y) = ((k % 3) as i64 * 124 + 8, (k / 3) as i64 * 124 + 8); rect(&mut i, x, y, x + 118, y + 118, *c); } disc(&mut i, 70, 70, 30, WHITE); triangle(&mut i, 192, 190, 40, WHITE); i } },
    Spec { id: "collage-overlap", category: "collage", filename_hint: "IMG_0107.png", tags: &[COLLAGE, ORANGE_W, BLUE_W], required: &[COLLAGE], forbidden: &["photo of a person"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, PAPER); let mut r = Lcg(51); for k in 0..14 { let (x, y) = (r.below(300) as i64, r.below(190) as i64); let c = [ORANGE, BLUE, [250, 240, 220, 255]][k % 3]; rect(&mut i, x, y, x + 60 + r.below(60) as i64, y + 40 + r.below(50) as i64, c); } noise(&mut i, 52, 10); i } },
    Spec { id: "collage-shapes", category: "collage", filename_hint: "cutouts.png", tags: &[COLLAGE, CIRCLE, TRIANGLE, STARS], required: &[COLLAGE], forbidden: &["photo of a person"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, [235, 225, 205, 255]); noise(&mut i, 61, 12); rect(&mut i, 20, 20, 170, 140, [200, 70, 60, 255]); disc(&mut i, 260, 90, 60, BLUE); triangle(&mut i, 110, 190, 55, GREEN); star(&mut i, 300, 200, 45, YELLOW); i } },
    Spec { id: "collage-stripes", category: "collage", filename_hint: "paper_scraps.png", tags: &[COLLAGE, STRIPES], required: &[COLLAGE], forbidden: &["photo of a person"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, PAPER); for k in 0..8 { rect(&mut i, k * 48, 0, k * 48 + 24, 256, [RED, BLUE, YELLOW, GREEN][(k % 4) as usize]); } rect(&mut i, 60, 70, 320, 190, [250, 248, 240, 255]); disc(&mut i, 190, 130, 45, PURPLE); noise(&mut i, 71, 10); i } },
    // 3D.
    Spec { id: "3d-red-sphere", category: "3d", filename_hint: "render_01.png", tags: &[RENDER, SPHERE, RED_W], required: &[SPHERE, RED_W], forbidden: &["blue", "green", "flat"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, [200, 205, 215, 255]); sphere(&mut i, 192, 128, 90, RED); i } },
    Spec { id: "3d-blue-sphere-floor", category: "3d", filename_hint: "scene_final.png", tags: &[RENDER, SPHERE, BLUE_W], required: &[SPHERE, BLUE_W], forbidden: &["red", "green"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, [210, 215, 225, 255]); vertical_gradient(&mut i, [225, 230, 240, 255], [150, 155, 165, 255]); fill_where(&mut i, [0, 0, 0, 70], |x, y| (x - 200).pow(2) * 9 + (y - 222).pow(2) * 100 <= 90 * 90 * 9); sphere(&mut i, 190, 130, 80, BLUE); i } },
    Spec { id: "3d-green-cube", category: "3d", filename_hint: "asset_cube.png", tags: &[RENDER, CUBE, GREEN_W], required: &[CUBE, GREEN_W], forbidden: &["red", "blue", "sphere"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, [210, 210, 215, 255]); polygon(&mut i, &[(192, 40), (282, 85), (192, 130), (102, 85)], [90, 210, 110, 255]); polygon(&mut i, &[(102, 85), (192, 130), (192, 225), (102, 180)], [30, 150, 60, 255]); polygon(&mut i, &[(192, 130), (282, 85), (282, 180), (192, 225)], [20, 110, 45, 255]); i } },
    Spec { id: "3d-orange-sphere", category: "3d", filename_hint: "ball_v3.png", tags: &[RENDER, SPHERE, ORANGE_W], required: &[SPHERE, ORANGE_W], forbidden: &["blue", "green"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, [40, 40, 48, 255]); sphere(&mut i, 192, 128, 100, ORANGE); i } },
    // Screenshots.
    Spec { id: "ui-login", category: "screenshot", filename_hint: "Screenshot 2026-03-01 at 10.42.png", tags: &[SCREENSHOT, TEXT_W], required: &[SCREENSHOT], forbidden: &["painting", "watercolor"], ambiguous: false,
        draw: || { let mut i = canvas(480, 300, WHITE); window(&mut i, false); rect(&mut i, 170, 110, 420, 140, [235, 236, 240, 255]); rect(&mut i, 170, 160, 420, 190, [235, 236, 240, 255]); rect(&mut i, 170, 215, 300, 250, BLUE); text(&mut i, 180, 224, 3, "LOGIN", WHITE); text(&mut i, 170, 60, 4, "SIGN IN", BLACK); i } },
    Spec { id: "ui-dark-menu", category: "screenshot", filename_hint: "capture.png", tags: &[SCREENSHOT, DARK, TEXT_W], required: &[SCREENSHOT, DARK], forbidden: &["painting", "watercolor"], ambiguous: false,
        draw: || { let mut i = canvas(480, 300, BLACK); window(&mut i, true); text(&mut i, 120, 50, 4, "MENU", [225, 225, 230, 255]); for k in 0..4 { rect(&mut i, 120, 100 + k * 44, 440, 130 + k * 44, [58, 60, 70, 255]); } i } },
    Spec { id: "ui-dialog", category: "screenshot", filename_hint: "bug_report.png", tags: &[SCREENSHOT, TEXT_W], required: &[SCREENSHOT], forbidden: &["painting", "watercolor"], ambiguous: false,
        draw: || { let mut i = canvas(480, 300, WHITE); window(&mut i, false); rect(&mut i, 0, 0, 480, 300, [0, 0, 0, 90]); rounded(&mut i, 140, 90, 380, 230, 12, WHITE); text(&mut i, 170, 120, 3, "SAVE", BLACK); rect(&mut i, 290, 185, 360, 215, BLUE); text(&mut i, 307, 193, 2, "OK", WHITE); i } },
    Spec { id: "ui-spreadsheet", category: "screenshot", filename_hint: "Screen Shot 12.png", tags: &[SCREENSHOT], required: &[SCREENSHOT], forbidden: &["painting", "watercolor"], ambiguous: false,
        draw: || { let mut i = canvas(480, 300, WHITE); window(&mut i, false); for x in 0..8 { rect(&mut i, 100 + x * 47, 40, 101 + x * 47, 300, [200, 200, 205, 255]); } for y in 0..12 { rect(&mut i, 90, 40 + y * 22, 480, 41 + y * 22, [200, 200, 205, 255]); } rect(&mut i, 148, 84, 194, 105, [180, 215, 250, 255]); i } },
    // Text.
    Spec { id: "text-sale", category: "text", filename_hint: "banner_promo.png", tags: &[TEXT_W, RED_W], required: &[&["sale"]], forbidden: &["blue"], ambiguous: false,
        draw: || { let mut i = canvas(384, 160, WHITE); centered_text(&mut i, 10, "SALE", RED); i } },
    Spec { id: "text-hello", category: "text", filename_hint: "IMG_2210.png", tags: &[TEXT_W, YELLOW_W], required: &[&["hello"]], forbidden: &["sale"], ambiguous: false,
        draw: || { let mut i = canvas(384, 160, YELLOW); centered_text(&mut i, 9, "HELLO", BLACK); i } },
    Spec { id: "text-open", category: "text", filename_hint: "shop_sign.png", tags: &[TEXT_W, GREEN_W], required: &[&["open"]], forbidden: &["closed"], ambiguous: false,
        draw: || { let mut i = canvas(384, 160, GREEN); centered_text(&mut i, 10, "OPEN", WHITE); i } },
    Spec { id: "text-stop", category: "text", filename_hint: "traffic.png", tags: &[TEXT_W, SIGN, RED_W], required: &[&["stop"], RED_W], forbidden: &["blue", "green"], ambiguous: false,
        draw: || { let mut i = canvas(300, 300, [190, 215, 235, 255]); octagon(&mut i, 150, 150, 120, RED); centered_text(&mut i, 8, "STOP", WHITE); i } },
    // Transparent backgrounds.
    Spec { id: "alpha-red-circle", category: "transparent", filename_hint: "sticker_red.png", tags: &[TRANSPARENT, CIRCLE, RED_W], required: &[CIRCLE, RED_W], forbidden: &["blue", "green"], ambiguous: false,
        draw: || { let mut i = canvas(256, 256, CLEAR); disc(&mut i, 128, 128, 100, RED); i } },
    Spec { id: "alpha-blue-star", category: "transparent", filename_hint: "icon_star.png", tags: &[TRANSPARENT, STARS, BLUE_W], required: &[STARS, BLUE_W], forbidden: &["red", "green"], ambiguous: false,
        draw: || { let mut i = canvas(256, 256, CLEAR); star(&mut i, 128, 135, 110, BLUE); i } },
    Spec { id: "alpha-green-triangle", category: "transparent", filename_hint: "arrow_up.png", tags: &[TRANSPARENT, TRIANGLE, GREEN_W], required: &[TRIANGLE, GREEN_W], forbidden: &["red", "blue"], ambiguous: false,
        draw: || { let mut i = canvas(256, 256, CLEAR); triangle(&mut i, 128, 128, 100, GREEN); i } },
    Spec { id: "alpha-soft-purple", category: "transparent", filename_hint: "glow.png", tags: &[TRANSPARENT, PURPLE_W, CIRCLE], required: &[PURPLE_W], forbidden: &["green", "yellow"], ambiguous: false,
        draw: || { let mut i = canvas(256, 256, CLEAR); for r in (10..=110).rev().step_by(10) { disc(&mut i, 128, 128, r, [130, 50, 170, (250 - r * 2) as u8]); } i } },
    // Multiple objects.
    Spec { id: "multi-circle-square", category: "multiple", filename_hint: "IMG_0311.png", tags: &[CIRCLE, SQUARE, RED_W, BLUE_W], required: &[CIRCLE, SQUARE], forbidden: &["triangle", "star"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, WHITE); disc(&mut i, 110, 128, 70, RED); rect(&mut i, 220, 58, 360, 198, BLUE); i } },
    Spec { id: "multi-three-stars", category: "multiple", filename_hint: "rating.png", tags: &[STARS, YELLOW_W], required: &[STARS, YELLOW_W], forbidden: &["circle", "red"], ambiguous: false,
        draw: || { let mut i = canvas(384, 160, WHITE); for k in 0..3 { star(&mut i, 82 + k * 110, 85, 50, YELLOW); } i } },
    Spec { id: "multi-three-shapes", category: "multiple", filename_hint: "shapes_all.png", tags: &[TRIANGLE, CIRCLE, SQUARE, GREEN_W, RED_W, BLUE_W], required: &[TRIANGLE, CIRCLE, SQUARE], forbidden: &["star"], ambiguous: false,
        draw: || { let mut i = canvas(480, 200, WHITE); triangle(&mut i, 80, 100, 60, GREEN); disc(&mut i, 240, 100, 60, RED); rect(&mut i, 340, 40, 460, 160, BLUE); i } },
    Spec { id: "multi-dots", category: "multiple", filename_hint: "pattern_tile.png", tags: &[DOTS, BLUE_W], required: &[DOTS, BLUE_W], forbidden: &["red", "triangle"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, WHITE); for y in 0..6 { for x in 0..9 { disc(&mut i, 22 + x * 42 + (y % 2) * 21, 22 + y * 42, 11, BLUE); } } i } },
    // Unusual crops and sizes.
    Spec { id: "crop-panorama-sun", category: "crop", filename_hint: "header_wide.png", tags: &[PANORAMA, SUN, YELLOW_W, ORANGE_W], required: &[SUN], forbidden: &["moon", "blue circle"], ambiguous: false,
        draw: || { let mut i = canvas(1200, 200, WHITE); vertical_gradient(&mut i, [250, 160, 80, 255], [120, 60, 120, 255]); disc(&mut i, 1180, 150, 90, YELLOW); rect(&mut i, 0, 170, 1200, 200, [40, 30, 60, 255]); i } },
    Spec { id: "crop-tall-stripes", category: "crop", filename_hint: "sidebar.png", tags: &[STRIPES, GREEN_W], required: &[STRIPES, GREEN_W], forbidden: &["red", "circle"], ambiguous: false,
        draw: || { let mut i = canvas(200, 900, WHITE); for k in 0..15 { rect(&mut i, 0, k * 60, 200, k * 60 + 30, GREEN); } i } },
    Spec { id: "crop-half-circle", category: "crop", filename_hint: "IMG_0500.png", tags: &[CIRCLE, RED_W], required: &[RED_W], forbidden: &["blue", "green"], ambiguous: false,
        draw: || { let mut i = canvas(384, 256, WHITE); disc(&mut i, 384, 128, 120, RED); i } },
    Spec { id: "crop-tiny-star", category: "crop", filename_hint: "empty_canvas.png", tags: &[STARS, YELLOW_W], required: &[STARS], forbidden: &["red", "circle"], ambiguous: false,
        draw: || { let mut i = canvas(640, 480, [30, 30, 50, 255]); star(&mut i, 520, 90, 14, YELLOW); i } },
    // Deliberately ambiguous.
    Spec { id: "amb-rounded-square", category: "ambiguous", filename_hint: "blob.png", tags: &[RED_W], required: &[RED_W], forbidden: &["blue", "green"], ambiguous: true,
        draw: || { let mut i = canvas(384, 256, WHITE); rounded(&mut i, 112, 48, 272, 208, 60, RED); i } },
    Spec { id: "amb-magenta", category: "ambiguous", filename_hint: "swatch.png", tags: &[SQUARE], required: &[], forbidden: &["green", "yellow"], ambiguous: true,
        draw: || { let mut i = canvas(384, 256, WHITE); rect(&mut i, 92, 28, 292, 228, [190, 40, 140, 255]); i } },
    Spec { id: "amb-noise", category: "ambiguous", filename_hint: "texture_04.png", tags: &[NOISE, &["gray", "grey"]], required: &[], forbidden: &["cat", "dog", "person", "face"], ambiguous: true,
        draw: || { let mut i = canvas(384, 256, GRAY); noise(&mut i, 91, 60); i } },
    Spec { id: "amb-faint", category: "ambiguous", filename_hint: "blank.png", tags: &[], required: &[], forbidden: &["red", "blue", "green", "person"], ambiguous: true,
        draw: || { let mut i = canvas(384, 256, WHITE); disc(&mut i, 192, 128, 60, [244, 244, 244, 255]); i } },
];

const QUERIES: &[(&str, &str, &[&str])] = &[
    ("q-red-circle", "red circle", &["flat-red-circle", "alpha-red-circle", "multi-circle-square", "crop-half-circle"]),
    ("q-blue-square", "blue square", &["flat-blue-square", "multi-circle-square", "multi-three-shapes"]),
    ("q-green-triangle", "green triangle", &["flat-green-triangle", "alpha-green-triangle", "multi-three-shapes"]),
    ("q-stars", "yellow stars", &["flat-yellow-star", "multi-three-stars", "crop-tiny-star"]),
    ("q-watercolor", "watercolor painting", &["wc-red-wash", "wc-blue-wash", "wc-green-yellow", "wc-purple-circle"]),
    ("q-collage", "paper collage", &["collage-grid", "collage-overlap", "collage-shapes", "collage-stripes"]),
    ("q-3d-ball", "3d rendered ball", &["3d-red-sphere", "3d-blue-sphere-floor", "3d-orange-sphere"]),
    ("q-cube", "green cube", &["3d-green-cube"]),
    ("q-screenshot", "app screenshot", &["ui-login", "ui-dark-menu", "ui-dialog", "ui-spreadsheet"]),
    ("q-dark-mode", "dark mode interface", &["ui-dark-menu"]),
    ("q-login", "login screen", &["ui-login"]),
    ("q-sale", "sale banner", &["text-sale"]),
    ("q-stop", "stop sign", &["text-stop"]),
    ("q-transparent", "sticker with transparent background", &["alpha-red-circle", "alpha-blue-star", "alpha-green-triangle", "alpha-soft-purple"]),
    ("q-sunset", "sunset banner", &["crop-panorama-sun"]),
    ("q-dots", "polka dot pattern", &["multi-dots"]),
    ("q-purple", "purple", &["wc-purple-circle", "alpha-soft-purple"]),
    ("q-ood-dog", "photo of a dog on a beach", &[]),
    ("q-ood-invoice", "scanned invoice with a table of prices", &[]),
];

/// Every third item (by position) is development; the rest is held out.
fn split_of(index: usize) -> Split {
    if index % 3 == 0 {
        Split::Dev
    } else {
        Split::Heldout
    }
}

fn owned(sets: &[&[&str]]) -> Vec<Vec<String>> {
    sets.iter().map(|s| s.iter().map(|w| w.to_string()).collect()).collect()
}

pub fn render_item(id: &str) -> Option<RgbaImage> {
    SPECS.iter().find(|s| s.id == id).map(|s| (s.draw)())
}

pub fn encode_png(img: &RgbaImage) -> Vec<u8> {
    let mut bytes = vec![];
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("PNG encoding to memory");
    bytes
}

/// Writes the generated corpus and its manifest into `dir`. Deterministic:
/// the same version writes byte-identical files on every platform.
pub fn generate_corpus(dir: &Path) -> Result<Manifest, EvalError> {
    std::fs::create_dir_all(dir)?;
    let mut items = vec![];
    for (index, s) in SPECS.iter().enumerate() {
        let bytes = encode_png(&(s.draw)());
        let file = format!("{}.png", s.id);
        std::fs::write(dir.join(&file), &bytes)?;
        items.push(ManifestItem {
            id: s.id.into(),
            file,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            category: s.category.into(),
            split: split_of(index),
            provenance: format!("Generated by creativeshelf-image-eval ({CORPUS_ID}); procedural drawing, no third-party media"),
            filename_hint: s.filename_hint.into(),
            tags: owned(s.tags),
            caption_required: owned(s.required),
            caption_forbidden: s.forbidden.iter().map(|w| w.to_string()).collect(),
            ambiguous: s.ambiguous,
        });
    }
    let manifest = Manifest {
        schema: MANIFEST_SCHEMA.into(),
        corpus: CORPUS_ID.into(),
        license: "CC0-1.0 (generated by this repository's code)".into(),
        languages: LANGUAGES.iter().map(|l| l.to_string()).collect(),
        // Real photographs cannot be generated. Add them as a separate,
        // consented manifest (never committed); see docs/IMAGE_AI_EVALUATION.md.
        categories_pending: vec!["photo".into()],
        items,
        queries: QUERIES
            .iter()
            .map(|(id, text, rel)| Query { id: id.to_string(), text: text.to_string(), language: "en".into(), relevant: rel.iter().map(|r| r.to_string()).collect() })
            .collect(),
    };
    manifest.validate()?;
    std::fs::write(dir.join("manifest.json"), manifest_json(&manifest))?;
    Ok(manifest)
}

pub fn manifest_json(m: &Manifest) -> String {
    serde_json::to_string_pretty(m).expect("manifest serializes") + "\n"
}

// ---------------------------------------------------------------------------
// Scoring.

/// Lowercase words, apostrophes dropped, trailing plural "s" removed from words
/// longer than three letters.
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") { w[..w.len() - 1].to_owned() } else { w.to_owned() })
        .collect()
}

/// True when the phrase's words appear consecutively in the text's words.
pub fn mentions(text_words: &[String], phrase: &str) -> bool {
    let p = words(phrase);
    !p.is_empty() && text_words.windows(p.len()).any(|w| w == p.as_slice())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TagScore {
    /// Predicted tags that match a judged vocabulary term.
    pub judged: u32,
    pub correct: u32,
    /// Predicted tags outside the vocabulary: reported, not scored.
    pub unjudged: u32,
    pub gold_facts: u32,
    pub gold_found: u32,
}
impl TagScore {
    pub fn precision(&self) -> Option<f64> {
        (self.judged > 0).then(|| self.correct as f64 / self.judged as f64)
    }
    pub fn recall(&self) -> Option<f64> {
        (self.gold_facts > 0).then(|| self.gold_found as f64 / self.gold_facts as f64)
    }
    fn add(&mut self, o: &TagScore) {
        self.judged += o.judged;
        self.correct += o.correct;
        self.unjudged += o.unjudged;
        self.gold_facts += o.gold_facts;
        self.gold_found += o.gold_found;
    }
}

pub fn vocabulary(m: &Manifest) -> Vec<String> {
    let mut v: Vec<String> = m.items.iter().flat_map(|i| i.tags.iter().flatten().cloned()).collect();
    v.sort();
    v.dedup();
    v
}

/// A predicted tag is judged when it is a vocabulary term (exact words), and
/// correct when it names one of the item's gold facts.
pub fn score_tags(item: &ManifestItem, predicted: &[String], vocab: &[String]) -> TagScore {
    let mut s = TagScore { gold_facts: item.tags.len() as u32, ..Default::default() };
    let mut found = vec![false; item.tags.len()];
    for t in predicted {
        let tw = words(t);
        if !vocab.iter().any(|v| words(v) == tw) {
            s.unjudged += 1;
            continue;
        }
        s.judged += 1;
        if let Some(k) = item.tags.iter().position(|fact| fact.iter().any(|alt| words(alt) == tw)) {
            s.correct += 1;
            found[k] = true;
        }
    }
    s.gold_found = found.iter().filter(|f| **f).count() as u32;
    s
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CaptionScore {
    pub required: u32,
    pub covered: u32,
    pub forbidden_found: Vec<String>,
}
pub fn score_caption(item: &ManifestItem, caption: &str) -> CaptionScore {
    let w = words(caption);
    CaptionScore {
        required: item.caption_required.len() as u32,
        covered: item.caption_required.iter().filter(|fact| fact.iter().any(|alt| mentions(&w, alt))).count() as u32,
        forbidden_found: item.caption_forbidden.iter().filter(|f| mentions(&w, f)).cloned().collect(),
    }
}

/// Ranks documents by how many distinct query words they contain; ties by ID.
pub fn lexical_rank(query: &str, docs: &[(String, String)]) -> Vec<String> {
    let q: HashSet<String> = words(query).into_iter().collect();
    let mut scored: Vec<(usize, &String)> = docs
        .iter()
        .map(|(id, text)| {
            let d: HashSet<String> = words(text).into_iter().collect();
            (q.intersection(&d).count(), id)
        })
        .filter(|(n, _)| *n > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().map(|(_, id)| id.clone()).collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RetrievalScore {
    pub queries: u32,
    pub recall_at_5: f64,
    pub mrr: f64,
    /// Out-of-domain queries that still returned results.
    pub out_of_domain_with_results: u32,
    pub out_of_domain_queries: u32,
}
/// Scores rankings restricted to `pool` (the items of one split).
pub fn score_retrieval(queries: &[Query], rankings: &BTreeMap<String, Vec<String>>, pool: &HashSet<String>) -> RetrievalScore {
    let mut s = RetrievalScore::default();
    let (mut recall, mut rr) = (0.0, 0.0);
    for q in queries {
        let ranked: Vec<&String> = rankings.get(&q.id).map(|r| r.iter().filter(|id| pool.contains(*id)).collect()).unwrap_or_default();
        let relevant: HashSet<&String> = q.relevant.iter().filter(|r| pool.contains(*r)).collect();
        if q.relevant.is_empty() {
            s.out_of_domain_queries += 1;
            if !ranked.is_empty() {
                s.out_of_domain_with_results += 1;
            }
            continue;
        }
        if relevant.is_empty() {
            continue;
        }
        s.queries += 1;
        recall += ranked.iter().take(5).filter(|id| relevant.contains(*id)).count() as f64 / relevant.len().min(5) as f64;
        rr += ranked.iter().position(|id| relevant.contains(*id)).map_or(0.0, |p| 1.0 / (p + 1) as f64);
    }
    if s.queries > 0 {
        s.recall_at_5 = recall / s.queries as f64;
        s.mrr = rr / s.queries as f64;
    }
    s
}

/// Retrieval using only creator file names: the no-AI baseline.
pub fn filename_baseline(m: &Manifest) -> BTreeMap<String, Vec<String>> {
    let docs: Vec<(String, String)> = m.items.iter().map(|i| (i.id.clone(), i.filename_hint.clone())).collect();
    m.queries.iter().map(|q| (q.id.clone(), lexical_rank(&q.text, &docs))).collect()
}

// ---------------------------------------------------------------------------
// Thresholds and the runner.

/// Pre-registered acceptance thresholds (eval/image-ai/thresholds.json),
/// committed before any real-model result exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thresholds {
    pub version: u32,
    pub min_tag_precision: f64,
    pub min_tag_recall: f64,
    pub min_caption_fact_coverage: f64,
    pub max_caption_error_rate: f64,
    pub min_recall_at_5: f64,
    pub min_mrr: f64,
    pub min_mrr_gain_over_filenames: f64,
    pub max_out_of_domain_with_results: u32,
    pub max_failure_rate: f64,
    pub max_p95_latency_ms: u64,
    pub max_mean_input_tokens: u64,
    /// Rule for the embedding cut-off: highest value on the dev split whose dev
    /// recall@5 stays at least this fraction of the uncut recall@5.
    pub similarity_cutoff_keep_recall: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemOutcome {
    pub id: String,
    pub split: Split,
    pub latency_ms: u64,
    pub insight: Option<Insight>,
    pub error: Option<String>,
    pub tags: TagScore,
    pub caption: CaptionScore,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SplitSummary {
    pub items: u32,
    pub failures: u32,
    pub uncertain: u32,
    pub tags: TagScore,
    pub tag_precision: Option<f64>,
    pub tag_recall: Option<f64>,
    pub caption_fact_coverage: Option<f64>,
    pub caption_error_rate: Option<f64>,
    pub p50_latency_ms: u64,
    pub p95_latency_ms: u64,
    pub filename_retrieval: RetrievalScore,
    pub insight_lexical_retrieval: RetrievalScore,
    pub embedding_retrieval: Option<RetrievalScore>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub schema: String,
    pub corpus: String,
    pub manifest_blake3: String,
    pub thresholds_blake3: String,
    pub describer: Option<String>,
    pub describer_model_version: Option<String>,
    pub embedding_space: Option<String>,
    pub similarity_cutoff: Option<f32>,
    pub requests: u32,
    pub request_allowance: u32,
    /// False when the allowance ran out; an incomplete run never passes.
    pub complete: bool,
    pub usage: Usage,
    pub mean_input_tokens: Option<u64>,
    pub dev: SplitSummary,
    pub heldout: SplitSummary,
    pub items: Vec<ItemOutcome>,
    pub verdict: Option<Verdict>,
}

pub struct Describer<'a> {
    pub adapter: &'a dyn ProviderAdapter,
    pub kind: ProviderKind,
    pub model: String,
}
pub struct Embedder<'a> {
    pub adapter: &'a dyn ProviderAdapter,
    pub kind: ProviderKind,
    pub model: String,
}

fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() - 1) * p).div_ceil(100)]
}

fn summarize(m: &Manifest, items: &[ItemOutcome], split: Split, filename: &BTreeMap<String, Vec<String>>, insight_rank: &BTreeMap<String, Vec<String>>, embed_rank: Option<&BTreeMap<String, Vec<String>>>) -> SplitSummary {
    let mine: Vec<&ItemOutcome> = items.iter().filter(|o| o.split == split).collect();
    let pool: HashSet<String> = m.items.iter().filter(|i| i.split == split).map(|i| i.id.clone()).collect();
    let mut s = SplitSummary { items: pool.len() as u32, ..Default::default() };
    let mut latencies = vec![];
    let (mut required, mut covered, mut with_error, mut captioned) = (0u32, 0u32, 0u32, 0u32);
    for o in &mine {
        latencies.push(o.latency_ms);
        match &o.insight {
            None => s.failures += 1,
            Some(i) => {
                if i.uncertain {
                    s.uncertain += 1;
                }
                s.tags.add(&o.tags);
                captioned += 1;
                required += o.caption.required;
                covered += o.caption.covered;
                if !o.caption.forbidden_found.is_empty() {
                    with_error += 1;
                }
            }
        }
    }
    latencies.sort_unstable();
    s.p50_latency_ms = percentile(&latencies, 50);
    s.p95_latency_ms = percentile(&latencies, 95);
    s.tag_precision = s.tags.precision();
    s.tag_recall = s.tags.recall();
    s.caption_fact_coverage = (required > 0).then(|| covered as f64 / required as f64);
    s.caption_error_rate = (captioned > 0).then(|| with_error as f64 / captioned as f64);
    s.filename_retrieval = score_retrieval(&m.queries, filename, &pool);
    s.insight_lexical_retrieval = score_retrieval(&m.queries, insight_rank, &pool);
    s.embedding_retrieval = embed_rank.map(|r| score_retrieval(&m.queries, r, &pool));
    s
}

pub fn judge(r: &Report, t: &Thresholds) -> Verdict {
    let h = &r.heldout;
    let mut f = vec![];
    let mut check = |ok: bool, what: String| {
        if !ok {
            f.push(what)
        }
    };
    check(r.complete, "the run stopped at its request allowance".into());
    check(h.tag_precision.is_some_and(|v| v >= t.min_tag_precision), format!("tag precision {:?} < {}", h.tag_precision, t.min_tag_precision));
    check(h.tag_recall.is_some_and(|v| v >= t.min_tag_recall), format!("tag recall {:?} < {}", h.tag_recall, t.min_tag_recall));
    check(h.caption_fact_coverage.is_some_and(|v| v >= t.min_caption_fact_coverage), format!("caption fact coverage {:?} < {}", h.caption_fact_coverage, t.min_caption_fact_coverage));
    check(h.caption_error_rate.is_some_and(|v| v <= t.max_caption_error_rate), format!("caption error rate {:?} > {}", h.caption_error_rate, t.max_caption_error_rate));
    let failure_rate = if h.items > 0 { h.failures as f64 / h.items as f64 } else { 1.0 };
    check(failure_rate <= t.max_failure_rate, format!("failure rate {failure_rate:.3} > {}", t.max_failure_rate));
    check(h.p95_latency_ms <= t.max_p95_latency_ms, format!("p95 latency {} ms > {}", h.p95_latency_ms, t.max_p95_latency_ms));
    check(r.mean_input_tokens.is_none_or(|v| v <= t.max_mean_input_tokens), format!("mean input tokens {:?} > {}", r.mean_input_tokens, t.max_mean_input_tokens));
    let retrieval = h.embedding_retrieval.as_ref().unwrap_or(&h.insight_lexical_retrieval);
    check(retrieval.recall_at_5 >= t.min_recall_at_5, format!("recall@5 {:.3} < {}", retrieval.recall_at_5, t.min_recall_at_5));
    check(retrieval.mrr >= t.min_mrr, format!("MRR {:.3} < {}", retrieval.mrr, t.min_mrr));
    check(
        retrieval.mrr - h.filename_retrieval.mrr >= t.min_mrr_gain_over_filenames,
        format!("MRR gain over file names {:.3} < {}", retrieval.mrr - h.filename_retrieval.mrr, t.min_mrr_gain_over_filenames),
    );
    check(
        retrieval.out_of_domain_with_results <= t.max_out_of_domain_with_results,
        format!("{} out-of-domain queries returned results", retrieval.out_of_domain_with_results),
    );
    Verdict { passed: f.is_empty(), failures: f }
}

/// Request budget shared by every call in one run.
struct Allowance {
    used: u32,
    max: u32,
}
impl Allowance {
    fn take(&mut self) -> bool {
        if self.used >= self.max {
            return false;
        }
        self.used += 1;
        true
    }
}

pub struct RunInput<'a> {
    pub dir: &'a Path,
    pub manifest: &'a Manifest,
    pub manifest_blake3: String,
    pub thresholds: &'a Thresholds,
    pub thresholds_blake3: String,
    pub request_allowance: u32,
}

/// Runs the evaluation. Each image is read from disk, digest-checked and
/// prepared exactly as the app will send it; one request per image, then
/// batched embeddings. Provider errors are recorded per item, never retried.
pub fn run(input: &RunInput, describer: Option<&Describer>, embedder: Option<&Embedder>) -> Result<Report, EvalError> {
    let m = input.manifest;
    let vocab = vocabulary(m);
    let mut allowance = Allowance { used: 0, max: input.request_allowance };
    let mut usage = Usage::default();
    let mut complete = true;
    let mut model_version = None;
    let mut outcomes = vec![];
    for item in &m.items {
        let mut outcome = ItemOutcome { id: item.id.clone(), split: item.split, latency_ms: 0, insight: None, error: None, tags: TagScore::default(), caption: CaptionScore::default() };
        let bytes = read_verified(input.dir, item)?;
        let Some(d) = describer else {
            outcomes.push(outcome);
            continue;
        };
        let (_, pixels) = crate::image_decode::decode_display(&bytes).map_err(|_| EvalError::Image(item.id.clone()))?;
        let prepared = prepare_pixels(&pixels).map_err(|_| EvalError::Image(item.id.clone()))?;
        if !allowance.take() {
            complete = false;
            outcome.error = Some("REQUEST_ALLOWANCE_EXHAUSTED".into());
            outcomes.push(outcome);
            continue;
        }
        let started = Instant::now();
        let result = describe(d.adapter, &prepared);
        outcome.latency_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(r) => {
                usage.input_tokens = Some(usage.input_tokens.unwrap_or(0) + r.usage.input_tokens.unwrap_or(0));
                usage.output_tokens = Some(usage.output_tokens.unwrap_or(0) + r.usage.output_tokens.unwrap_or(0));
                model_version = r.model_version.clone().or(model_version);
                outcome.tags = score_tags(item, if r.insight.uncertain { &[] } else { &r.insight.tags }, &vocab);
                outcome.caption = score_caption(item, r.insight.caption.as_deref().unwrap_or(""));
                outcome.insight = Some(r.insight);
            }
            Err(e) => outcome.error = Some(e.code().into()),
        }
        outcomes.push(outcome);
    }
    let filename = filename_baseline(m);
    let insight_docs: Vec<(String, String)> = outcomes.iter().filter_map(|o| o.insight.as_ref().map(|i| (o.id.clone(), insight_text(i)))).collect();
    let insight_rank: BTreeMap<String, Vec<String>> = m.queries.iter().map(|q| (q.id.clone(), lexical_rank(&q.text, &insight_docs))).collect();

    let mut space_id = None;
    let mut cutoff = None;
    let mut embed_rank = None;
    if let (Some(e), false) = (embedder, insight_docs.is_empty()) {
        match embed_all(e, &insight_docs, &m.queries, &mut allowance, &mut usage) {
            Ok(Some((index, space, queries))) => {
                space_id = Some(space.id());
                let dev_pool: HashSet<String> = m.items.iter().filter(|i| i.split == Split::Dev).map(|i| i.id.clone()).collect();
                let rank_at = |c: f32| -> Result<BTreeMap<String, Vec<String>>, EvalError> {
                    let mut out = BTreeMap::new();
                    for (q, v) in &queries {
                        let hits = index.search(&space, v, m.items.len(), c).map_err(|e| manifest_error(e.to_string()))?;
                        out.insert(q.clone(), hits.into_iter().map(|(id, _)| id).collect());
                    }
                    Ok(out)
                };
                let base = score_retrieval(&m.queries, &rank_at(-1.0)?, &dev_pool).recall_at_5;
                let mut chosen = -1.0f32;
                for step in 0..100 {
                    let c = step as f32 / 100.0;
                    if score_retrieval(&m.queries, &rank_at(c)?, &dev_pool).recall_at_5 >= base * input.thresholds.similarity_cutoff_keep_recall {
                        chosen = c;
                    }
                }
                cutoff = Some(chosen);
                embed_rank = Some(rank_at(chosen)?);
            }
            Ok(None) => complete = false,
            Err(code) => {
                for o in &mut outcomes {
                    o.error.get_or_insert_with(|| format!("EMBEDDINGS_{code}"));
                }
                complete = false;
            }
        }
    }
    let described = outcomes.iter().filter(|o| o.insight.is_some()).count() as u64;
    let mean_input_tokens = usage.input_tokens.filter(|_| described > 0).map(|t| t / described);
    let mut report = Report {
        schema: REPORT_SCHEMA.into(),
        corpus: m.corpus.clone(),
        manifest_blake3: input.manifest_blake3.clone(),
        thresholds_blake3: input.thresholds_blake3.clone(),
        describer: describer.map(|d| crate::image_ai::model_ref(d.kind, &d.model, None).provider + "/" + &d.model),
        describer_model_version: model_version,
        embedding_space: space_id,
        similarity_cutoff: cutoff,
        requests: allowance.used,
        request_allowance: allowance.max,
        complete,
        usage,
        mean_input_tokens,
        dev: summarize(m, &outcomes, Split::Dev, &filename, &insight_rank, embed_rank.as_ref()),
        heldout: summarize(m, &outcomes, Split::Heldout, &filename, &insight_rank, embed_rank.as_ref()),
        items: outcomes,
        verdict: None,
    };
    if describer.is_some() {
        report.verdict = Some(judge(&report, input.thresholds));
    }
    Ok(report)
}

type EmbedResult = Option<(EmbeddingIndex, EmbeddingSpace, Vec<(String, Vec<f32>)>)>;

fn embed_all(e: &Embedder, docs: &[(String, String)], queries: &[Query], allowance: &mut Allowance, usage: &mut Usage) -> Result<EmbedResult, &'static str> {
    let texts: Vec<String> = docs.iter().map(|(_, t)| t.clone()).chain(queries.iter().map(|q| q.text.clone())).collect();
    let mut vectors = vec![];
    let mut version = None;
    for chunk in texts.chunks(MAX_EMBED_INPUTS) {
        if !allowance.take() {
            return Ok(None);
        }
        let r = e.adapter.embed(chunk).map_err(|err: ProviderError| err.code())?;
        if r.vectors.len() != chunk.len() {
            return Err("MALFORMED_RESPONSE");
        }
        usage.input_tokens = Some(usage.input_tokens.unwrap_or(0) + r.usage.input_tokens.unwrap_or(0));
        version = r.model_version.or(version);
        vectors.extend(r.vectors);
    }
    let dimension = vectors.first().map_or(0, Vec::len);
    let space = EmbeddingSpace::new(e.kind, &e.model, version.as_deref(), dimension);
    let mut index = EmbeddingIndex::new(space.clone());
    for ((id, _), v) in docs.iter().zip(&vectors) {
        index.add(id, &space, v).map_err(|_| "MALFORMED_RESPONSE")?;
    }
    let qv = queries.iter().zip(&vectors[docs.len()..]).map(|(q, v)| (q.id.clone(), v.clone())).collect();
    Ok(Some((index, space, qv)))
}

/// Default location of committed evaluation inputs, relative to the repository.
pub fn repository_eval_dir(repo: &Path) -> PathBuf {
    repo.join("eval").join("image-ai")
}
