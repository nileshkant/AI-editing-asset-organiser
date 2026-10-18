//! Worker-only preview rendering from SS-042's canonical display pixels.
//! Runs inside the heap-budgeted image worker. Input is stdin bytes; output is a
//! framed JSON batch plus bounded encoded payloads. No paths or writes.
use crate::image_analysis::{AnalysisError as E, ANALYZER, MAX_ENCODED};
use crate::image_preview::{
    inspect_steps, Job, PreviewBatch, PreviewMime, PreviewOutput, PreviewReply, PreviewTier,
    MAX_OUTPUT, TILE_EDGE, WORKER_MAGIC,
};
use image::{
    codecs::{
        jpeg::JpegEncoder,
        png::{CompressionType, FilterType as PngFilter, PngEncoder},
    },
    imageops::{self, FilterType},
    ExtendedColorType, ImageBuffer, ImageEncoder, Rgba, RgbaImage,
};
use std::io::{Read, Write};
type Result<T> = std::result::Result<T, E>;
type Premultiplied = ImageBuffer<Rgba<u16>, Vec<u16>>;

// Resampling straight alpha bleeds hidden RGB from transparent pixels into
// edges, so transparent sources resample in 16-bit premultiplied space.
fn premultiply(src: &RgbaImage) -> Result<Premultiplied> {
    let (w, h) = src.dimensions();
    let n = (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(E::LimitExceeded)?;
    let mut out = Vec::with_capacity(n);
    for p in src.pixels() {
        let a = u32::from(p[3]);
        for c in &p.0[..3] {
            out.push(((u32::from(*c) * a * 65535 + 32512) / 65025) as u16);
        }
        out.push((a * 257) as u16);
    }
    Premultiplied::from_raw(w, h, out).ok_or(E::LimitExceeded)
}
fn unpremultiply(src: &Premultiplied) -> RgbaImage {
    let (w, h) = src.dimensions();
    let mut out = RgbaImage::new(w, h);
    for (d, p) in out.pixels_mut().zip(src.pixels()) {
        let a = u32::from(p[3]);
        if a == 0 {
            continue;
        }
        let channel = |c: u16| ((u32::from(c).min(a) * 255 + a / 2) / a).min(255) as u8;
        *d = Rgba([
            channel(p[0]),
            channel(p[1]),
            channel(p[2]),
            ((a + 128) / 257) as u8,
        ]);
    }
    out
}
/// Downscale only, never upscale. Opaque sources stay in RGBA8 with alpha 255.
fn sizes(rgba: RgbaImage, targets: &[(u32, u32)]) -> Result<Vec<RgbaImage>> {
    let mut out = Vec::with_capacity(targets.len());
    if rgba.pixels().all(|p| p[3] == 255) {
        let mut current = rgba;
        for &(w, h) in targets {
            if current.dimensions() != (w, h) {
                current = imageops::resize(&current, w, h, FilterType::CatmullRom);
                for p in current.pixels_mut() {
                    p[3] = 255;
                }
            }
            out.push(current.clone());
        }
        return Ok(out);
    }
    let mut current = premultiply(&rgba)?;
    drop(rgba);
    for &(w, h) in targets {
        if current.dimensions() != (w, h) {
            current = imageops::resize(&current, w, h, FilterType::CatmullRom);
        }
        out.push(unpremultiply(&current));
    }
    Ok(out)
}
/// JPEG only when every output pixel is opaque; any alpha keeps lossless PNG.
fn encode(tier: PreviewTier, img: &RgbaImage) -> Result<(PreviewMime, bool, Vec<u8>)> {
    let (w, h) = img.dimensions();
    let transparency = img.pixels().any(|p| p[3] != 255);
    let mut bytes = Vec::new();
    if !transparency {
        let rgb: Vec<u8> = img.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
        JpegEncoder::new_with_quality(&mut bytes, tier.quality())
            .write_image(&rgb, w, h, ExtendedColorType::Rgb8)
            .map_err(|_| E::WorkerFailed)?;
        if (bytes.len() as u64) <= u64::from(w) * u64::from(h) * 4 {
            return Ok((PreviewMime::Jpeg, false, bytes));
        }
        bytes.clear();
    }
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, PngFilter::Adaptive)
        .write_image(img.as_raw(), w, h, ExtendedColorType::Rgba8)
        .map_err(|_| E::WorkerFailed)?;
    Ok((PreviewMime::Png, transparency, bytes))
}
fn render(job: Job, bytes: &[u8]) -> Result<(PreviewBatch, Vec<u8>)> {
    let (analysis, rgba) = crate::image_decode::decode_display(bytes)?;
    let (w, h) = rgba.dimensions();
    let expected = job.outputs(w, h).ok_or(E::InvalidData)?;
    let candidates: Vec<Vec<RgbaImage>> = match job {
        Job::Tile(col, row) => {
            let (tw, th) = (expected[0].2, expected[0].3);
            vec![vec![imageops::crop_imm(&rgba, col * TILE_EDGE, row * TILE_EDGE, tw, th).to_image()]]
        }
        Job::Inspect => vec![sizes(rgba, &inspect_steps(w, h))?],
        Job::Thumbs => {
            let targets: Vec<(u32, u32)> = expected.iter().map(|o| (o.2, o.3)).collect();
            sizes(rgba, &targets)?.into_iter().map(|i| vec![i]).collect()
        }
    };
    let mut outputs = Vec::with_capacity(candidates.len());
    let mut payload = Vec::new();
    for ((tier, tile, ..), steps) in expected.into_iter().zip(candidates) {
        // First candidate within the 4 MiB contract ceiling; larger steps come first.
        let mut chosen = None;
        for image in steps {
            let (mime, transparency, encoded) = encode(tier, &image)?;
            if encoded.len() as u64 <= MAX_OUTPUT {
                chosen = Some((image.dimensions(), mime, transparency, encoded));
                break;
            }
        }
        let ((width, height), mime, transparency, encoded) = chosen.ok_or(E::LimitExceeded)?;
        outputs.push(PreviewOutput {
            tier,
            tile,
            width,
            height,
            mime,
            transparency,
            bytes: encoded.len() as u64,
            blake3: blake3::hash(&encoded).to_hex().to_string(),
        });
        payload.extend_from_slice(&encoded);
    }
    Ok((
        PreviewBatch {
            content_hash: analysis.content_hash,
            analyzer: ANALYZER.into(),
            source_width: w,
            source_height: h,
            outputs,
            peak_heap_bytes: 0,
        },
        payload,
    ))
}
pub(crate) fn worker_main(args: &[String], peak: impl Fn() -> usize + std::panic::RefUnwindSafe) {
    let result = std::panic::catch_unwind(|| {
        let job = Job::parse(args).ok_or(E::InvalidData)?;
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(MAX_ENCODED + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| E::InvalidData)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_ENCODED {
            return Err(E::LimitExceeded);
        }
        let (mut batch, payload) = render(job, &bytes)?;
        batch.peak_heap_bytes = peak() as u64;
        Ok((batch, payload))
    })
    .unwrap_or(Err(E::WorkerFailed));
    let (reply, payload) = match result {
        Ok((batch, payload)) => (PreviewReply { result: Ok(batch) }, payload),
        Err(e) => (PreviewReply { result: Err(e) }, Vec::new()),
    };
    let Ok(json) = serde_json::to_vec(&reply) else {
        return;
    };
    let mut out = std::io::stdout().lock();
    let _ = out
        .write_all(WORKER_MAGIC)
        .and_then(|_| out.write_all(&(json.len() as u32).to_le_bytes()))
        .and_then(|_| out.write_all(&json))
        .and_then(|_| out.write_all(&payload))
        .and_then(|_| out.flush());
}
