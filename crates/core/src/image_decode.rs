//! Worker-only decoding. Format structure/metadata are bounded before codec allocation.
use crate::image_analysis::{
    AnalysisError as E, AnalysisWarning as W, ImageAnalysis, ANALYZER, MAX_DECODED, MAX_ENCODED,
    MAX_METADATA, MAX_PIXELS,
};
use crate::image_contracts::{ImageFormat, ImageProfile, Transparency, MAX_DIMENSION};
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use moxcms::{ColorProfile, DataColorSpace, Layout};
use std::{
    borrow::Cow,
    io::{Cursor, Read},
};
type Result<T> = std::result::Result<T, E>;
struct Header {
    format: ImageFormat,
    width: u32,
    height: u32,
    depth: u8,
    space: String,
    icc: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
    srgb: bool,
    non_srgb: bool,
    metadata: usize,
}
impl Header {
    fn new(format: ImageFormat) -> Self {
        Self {
            format,
            width: 0,
            height: 0,
            depth: 8,
            space: "rgb".into(),
            icc: None,
            exif: None,
            srgb: false,
            non_srgb: false,
            metadata: 0,
        }
    }
    fn metadata(&mut self, n: usize) -> Result<()> {
        self.metadata = self.metadata.checked_add(n).ok_or(E::LimitExceeded)?;
        if self.metadata > MAX_METADATA {
            return Err(E::LimitExceeded);
        }
        Ok(())
    }
    fn dimensions(&mut self, w: u32, h: u32) -> Result<()> {
        dimensions(w, h)?;
        if self.width != 0 && (self.width != w || self.height != h) {
            return Err(E::InvalidData);
        }
        self.width = w;
        self.height = h;
        Ok(())
    }
    fn exif(&mut self, p: &[u8]) -> Result<()> {
        if self.exif.is_some() {
            return Err(E::InvalidData);
        }
        orientation(p)?;
        self.exif = Some(p.to_vec());
        Ok(())
    }
    fn icc(&mut self, p: &[u8]) -> Result<()> {
        if self.icc.is_some() {
            return Err(E::InvalidData);
        }
        validate_icc(p)?;
        self.icc = Some(p.to_vec());
        Ok(())
    }
}
fn dimensions(w: u32, h: u32) -> Result<()> {
    let pixels = u64::from(w)
        .checked_mul(u64::from(h))
        .ok_or(E::LimitExceeded)?;
    if w == 0 || h == 0 {
        return Err(E::InvalidData);
    }
    if w > MAX_DIMENSION
        || h > MAX_DIMENSION
        || pixels > MAX_PIXELS
        || pixels.checked_mul(8).ok_or(E::LimitExceeded)? > MAX_DECODED
    {
        return Err(E::LimitExceeded);
    }
    Ok(())
}
fn be32(p: &[u8]) -> Result<u32> {
    Ok(u32::from_be_bytes(
        p.get(..4)
            .ok_or(E::InvalidData)?
            .try_into()
            .map_err(|_| E::InvalidData)?,
    ))
}
fn le32(p: &[u8]) -> Result<u32> {
    Ok(u32::from_le_bytes(
        p.get(..4)
            .ok_or(E::InvalidData)?
            .try_into()
            .map_err(|_| E::InvalidData)?,
    ))
}
fn inflate(p: &[u8], h: &mut Header) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(p)
        .take((MAX_METADATA + 1) as u64)
        .read_to_end(&mut out)
        .map_err(|_| E::InvalidData)?;
    h.metadata(out.len())?;
    Ok(out)
}
fn split_zero(p: &[u8]) -> Result<(&[u8], &[u8])> {
    let n = p.iter().position(|b| *b == 0).ok_or(E::InvalidData)?;
    Ok((&p[..n], &p[n + 1..]))
}
fn png(bytes: &[u8]) -> Result<Header> {
    let mut h = Header::new(ImageFormat::Png);
    let mut pos = 8;
    let mut saw_end = false;
    let mut saw_data = false;
    let mut chunks = 0;
    while pos < bytes.len() {
        chunks += 1;
        if chunks > 65536 {
            return Err(E::LimitExceeded);
        }
        let len = be32(bytes.get(pos..).ok_or(E::InvalidData)?)? as usize;
        let end = pos
            .checked_add(12)
            .and_then(|n| n.checked_add(len))
            .filter(|n| *n <= bytes.len())
            .ok_or(E::InvalidData)?;
        let kind = &bytes[pos + 4..pos + 8];
        let p = &bytes[pos + 8..end - 4];
        if !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err(E::InvalidData);
        }
        if crc32fast::hash(&bytes[pos + 4..end - 4]) != be32(&bytes[end - 4..end])? {
            return Err(E::InvalidData);
        }
        if pos == 8 && kind != b"IHDR" {
            return Err(E::InvalidData);
        }
        if kind != b"IDAT" {
            h.metadata(len)?;
        }
        match kind {
            b"IHDR" => {
                if h.width != 0 || len != 13 {
                    return Err(E::InvalidData);
                }
                h.dimensions(be32(p)?, be32(&p[4..])?)?;
                h.depth = p[8];
                h.space = match p[9] {
                    0 | 4 => "gray",
                    2 | 3 | 6 => "rgb",
                    _ => return Err(E::InvalidData),
                }
                .into();
            }
            b"acTL" | b"fcTL" | b"fdAT" => return Err(E::AnimationUnsupported),
            b"IDAT" => saw_data = true,
            b"IEND" => {
                if len != 0 || end != bytes.len() {
                    return Err(E::InvalidData);
                }
                saw_end = true;
            }
            b"eXIf" => h.exif(p)?,
            b"iCCP" => {
                let (name, tail) = split_zero(p)?;
                if name.is_empty() || name.len() > 79 || tail.first() != Some(&0) {
                    return Err(E::InvalidData);
                }
                let profile = inflate(&tail[1..], &mut h)?;
                h.icc(&profile)?;
            }
            b"zTXt" => {
                let (_, tail) = split_zero(p)?;
                if tail.first() != Some(&0) {
                    return Err(E::InvalidData);
                }
                inflate(&tail[1..], &mut h)?;
            }
            b"iTXt" => {
                let (_, tail) = split_zero(p)?;
                if tail.len() < 2 || tail[0] > 1 || tail[1] != 0 {
                    return Err(E::InvalidData);
                }
                let (_, tail2) = split_zero(&tail[2..])?;
                let (_, text) = split_zero(tail2)?;
                if tail[0] == 1 {
                    inflate(text, &mut h)?;
                }
            }
            b"sRGB" => {
                if len != 1 || p[0] > 3 {
                    return Err(E::InvalidData);
                }
                h.srgb = true;
            }
            b"gAMA" => {
                if len != 4 {
                    return Err(E::InvalidData);
                }
                h.non_srgb |= be32(p)? != 45455;
            }
            b"cHRM" => {
                let standard = [31270, 32900, 64000, 33000, 30000, 60000, 15000, 6000];
                if len != 32 {
                    return Err(E::InvalidData);
                }
                h.non_srgb |= p
                    .chunks_exact(4)
                    .zip(standard)
                    .any(|(b, v)| be32(b).ok() != Some(v));
            }
            b"cICP" => {
                h.non_srgb |= p != [1, 13, 0, 1];
            }
            _ => {}
        }
        if saw_end {
            break;
        }
        pos = end;
    }
    if !saw_end || !saw_data || h.icc.is_some() && h.srgb {
        return Err(E::InvalidData);
    }
    Ok(h)
}
fn jpeg(bytes: &[u8]) -> Result<Header> {
    let mut h = Header::new(ImageFormat::Jpeg);
    let mut pos = 2;
    let mut end = false;
    let mut scans = 0;
    let mut icc_parts = std::collections::BTreeMap::new();
    let mut icc_total = 0;
    while pos < bytes.len() {
        if bytes[pos] != 0xff {
            return Err(E::InvalidData);
        }
        while bytes.get(pos) == Some(&0xff) {
            pos += 1;
        }
        let marker = *bytes.get(pos).ok_or(E::InvalidData)?;
        pos += 1;
        if marker == 0xd9 {
            end = pos == bytes.len();
            break;
        }
        if (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if matches!(marker, 0 | 0xd8) {
            return Err(E::InvalidData);
        }
        let length = u16::from_be_bytes(
            bytes
                .get(pos..pos + 2)
                .ok_or(E::InvalidData)?
                .try_into()
                .map_err(|_| E::InvalidData)?,
        ) as usize;
        if length < 2 {
            return Err(E::InvalidData);
        }
        let p = bytes
            .get(pos + 2..pos.checked_add(length).ok_or(E::InvalidData)?)
            .ok_or(E::InvalidData)?;
        pos += length;
        if (0xe0..=0xef).contains(&marker) || marker == 0xfe {
            h.metadata(p.len())?;
        }
        if matches!(
            marker,
            0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
        ) {
            return Err(E::UnsupportedFormat);
        }
        if matches!(marker, 0xc0 | 0xc1 | 0xc2) {
            if p.len() < 6 || h.width != 0 {
                return Err(E::InvalidData);
            }
            h.depth = p[0];
            if h.depth != 8 {
                return Err(E::UnsupportedFormat);
            }
            h.dimensions(
                u16::from_be_bytes([p[3], p[4]]) as u32,
                u16::from_be_bytes([p[1], p[2]]) as u32,
            )?;
            h.space = match p[5] {
                1 => "gray",
                3 => "ycbcr_or_rgb",
                4 => return Err(E::UnsupportedColorProfile),
                _ => return Err(E::InvalidData),
            }
            .into();
        }
        if marker == 0xe1 && p.starts_with(b"Exif\0\0") {
            h.exif(&p[6..])?;
        }
        if marker == 0xe2 && p.starts_with(b"ICC_PROFILE\0") {
            if p.len() < 14
                || p[12] == 0
                || p[13] == 0
                || p[12] > p[13]
                || icc_total != 0 && icc_total != p[13]
            {
                return Err(E::InvalidData);
            }
            icc_total = p[13];
            if icc_parts.insert(p[12], p[14..].to_vec()).is_some() {
                return Err(E::InvalidData);
            }
        }
        if marker == 0xe2 && p.starts_with(b"MPF\0") {
            return Err(E::AnimationUnsupported);
        }
        if marker == 0xda {
            scans += 1;
            if scans > 256 {
                return Err(E::LimitExceeded);
            }
            loop {
                if pos >= bytes.len() {
                    return Err(E::InvalidData);
                }
                if bytes[pos] == 0xff {
                    let next = *bytes.get(pos + 1).ok_or(E::InvalidData)?;
                    if next == 0 || (0xd0..=0xd7).contains(&next) {
                        pos += 2;
                        continue;
                    }
                    break;
                }
                pos += 1;
            }
        }
    }
    if !end || scans == 0 || h.width == 0 {
        return Err(E::InvalidData);
    }
    if icc_total != 0 {
        if icc_parts.len() != icc_total as usize {
            return Err(E::InvalidData);
        }
        let mut profile = Vec::new();
        for (_, p) in icc_parts {
            profile.extend_from_slice(&p);
        }
        h.icc(&profile)?;
    }
    Ok(h)
}
fn webp(bytes: &[u8]) -> Result<Header> {
    if bytes.len() < 12
        || bytes.get(8..12) != Some(b"WEBP")
        || u64::from(le32(&bytes[4..])?) + 8 != bytes.len() as u64
    {
        return Err(E::InvalidData);
    }
    let mut h = Header::new(ImageFormat::Webp);
    let mut pos = 12;
    let mut image = false;
    while pos < bytes.len() {
        let kind = bytes.get(pos..pos + 4).ok_or(E::InvalidData)?;
        let n = le32(bytes.get(pos + 4..).ok_or(E::InvalidData)?)? as usize;
        let data_end = pos
            .checked_add(8)
            .and_then(|p| p.checked_add(n))
            .filter(|p| *p <= bytes.len())
            .ok_or(E::InvalidData)?;
        let p = &bytes[pos + 8..data_end];
        if !matches!(kind, b"VP8 " | b"VP8L" | b"ALPH") {
            h.metadata(n)?;
        }
        match kind {
            b"ANIM" | b"ANMF" => return Err(E::AnimationUnsupported),
            b"VP8X" => {
                if n != 10 {
                    return Err(E::InvalidData);
                }
                if p[0] & 2 != 0 {
                    return Err(E::AnimationUnsupported);
                }
                let w = 1 + u32::from_le_bytes([p[4], p[5], p[6], 0]);
                let v = 1 + u32::from_le_bytes([p[7], p[8], p[9], 0]);
                h.dimensions(w, v)?;
            }
            b"VP8L" => {
                if n < 5 || p[0] != 0x2f {
                    return Err(E::InvalidData);
                }
                if image {
                    return Err(E::InvalidData);
                }
                image = true;
                let bits = le32(&p[1..])?;
                h.dimensions((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)?;
            }
            b"VP8 " => {
                if n < 10 || p[3..6] != [0x9d, 0x01, 0x2a] {
                    return Err(E::InvalidData);
                }
                if image {
                    return Err(E::InvalidData);
                }
                image = true;
                h.dimensions(
                    u16::from_le_bytes([p[6], p[7]]) as u32 & 0x3fff,
                    u16::from_le_bytes([p[8], p[9]]) as u32 & 0x3fff,
                )?;
            }
            b"EXIF" => h.exif(p.strip_prefix(b"Exif\0\0").unwrap_or(p))?,
            b"ICCP" => h.icc(p)?,
            _ => {}
        }
        pos = data_end + (n & 1);
        if pos > bytes.len() {
            return Err(E::InvalidData);
        }
    }
    if !image || h.width == 0 {
        return Err(E::InvalidData);
    }
    Ok(h)
}
fn validate_icc(p: &[u8]) -> Result<()> {
    if p.len() < 132
        || p.len() > MAX_METADATA
        || be32(p)? as usize != p.len()
        || p.get(36..40) != Some(b"acsp")
    {
        return Err(E::InvalidData);
    }
    let tags = be32(&p[128..])? as usize;
    if tags > 4096 || 132 + tags * 12 > p.len() {
        return Err(E::InvalidData);
    }
    for tag in p[132..132 + tags * 12].chunks_exact(12) {
        let start = be32(&tag[4..])? as usize;
        let n = be32(&tag[8..])? as usize;
        if start.checked_add(n).is_none_or(|end| end > p.len()) {
            return Err(E::InvalidData);
        }
    }
    Ok(())
}
fn orientation(p: &[u8]) -> Result<u8> {
    if p.len() < 8 {
        return Err(E::InvalidData);
    }
    let little = match &p[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(E::InvalidData),
    };
    let short = |s: &[u8]| -> Result<u16> {
        let a = s
            .get(..2)
            .ok_or(E::InvalidData)?
            .try_into()
            .map_err(|_| E::InvalidData)?;
        Ok(if little {
            u16::from_le_bytes(a)
        } else {
            u16::from_be_bytes(a)
        })
    };
    let long = |s: &[u8]| -> Result<u32> {
        let a = s
            .get(..4)
            .ok_or(E::InvalidData)?
            .try_into()
            .map_err(|_| E::InvalidData)?;
        Ok(if little {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        })
    };
    if short(&p[2..])? != 42 {
        return Err(E::InvalidData);
    }
    let start = long(&p[4..])? as usize;
    if start == 0 {
        return Ok(1);
    }
    let rest = p.get(start..).ok_or(E::InvalidData)?;
    let count = short(rest)? as usize;
    if count > 4096 {
        return Err(E::LimitExceeded);
    }
    let entries = rest.get(2..2 + count * 12).ok_or(E::InvalidData)?;
    if rest.get(2 + count * 12..2 + count * 12 + 4).is_none() {
        return Err(E::InvalidData);
    }
    let mut value = None;
    for entry in entries.chunks_exact(12) {
        let tag = short(entry)?;
        let ty = short(&entry[2..])?;
        let n = long(&entry[4..])? as usize;
        let size = match ty {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => 0,
        };
        let bytes = n.checked_mul(size).ok_or(E::InvalidData)?;
        if bytes > 4 {
            let offset = long(&entry[8..])? as usize;
            if offset.checked_add(bytes).is_none_or(|end| end > p.len()) {
                return Err(E::InvalidData);
            }
        }
        if tag == 0x0112 {
            if value.is_some() || ty != 3 || n != 1 {
                return Err(E::InvalidData);
            }
            let v = short(&entry[8..])?;
            if !(1..=8).contains(&v) {
                return Err(E::InvalidData);
            }
            value = Some(v as u8);
        }
    }
    Ok(value.unwrap_or(1))
}
fn alpha(image: &DynamicImage) -> (u64, u64) {
    let mut transparent = 0;
    let mut partial = 0;
    let mut add = |a: u16, max: u16| {
        if a < max {
            transparent += 1;
        }
        if a > 0 && a < max {
            partial += 1;
        }
    };
    match image {
        DynamicImage::ImageLumaA8(i) => {
            for p in i.pixels() {
                add(p[1] as u16, 255);
            }
        }
        DynamicImage::ImageRgba8(i) => {
            for p in i.pixels() {
                add(p[3] as u16, 255);
            }
        }
        DynamicImage::ImageLumaA16(i) => {
            for p in i.pixels() {
                add(p[1], 65535);
            }
        }
        DynamicImage::ImageRgba16(i) => {
            for p in i.pixels() {
                add(p[3], 65535);
            }
        }
        _ => {}
    }
    (transparent, partial)
}
// image-webp's simple VP8L dispatch uses the non-authoritative alpha hint
// to choose RGB and has a width=16384 header edge case. An ephemeral extended
// container makes dimensions explicit and always retains VP8L's actual ARGB plane.
// Original payloads/metadata, input hash and the source file remain unchanged.
fn webp_decode_container<'a>(bytes: &'a [u8], h: &Header) -> Result<Cow<'a, [u8]>> {
    if h.format != ImageFormat::Webp {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut pos = 12;
    let mut flags = 0u8;
    let mut lossless = false;
    let mut alpha = false;
    while pos < bytes.len() {
        let n = le32(&bytes[pos + 4..])? as usize;
        match &bytes[pos..pos + 4] {
            b"VP8L" => lossless = true,
            b"ALPH" => alpha = true,
            b"ICCP" => flags |= 0x20,
            b"EXIF" => flags |= 0x08,
            b"XMP " => flags |= 0x04,
            _ => {}
        }
        pos += 8 + n + (n & 1);
    }
    if lossless && alpha {
        return Err(E::InvalidData);
    }
    if lossless || alpha {
        flags |= 0x10;
    }
    let mut result = Vec::with_capacity(bytes.len() + 18);
    result.extend_from_slice(b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0");
    result.extend_from_slice(&[flags, 0, 0, 0]);
    result.extend_from_slice(&(h.width - 1).to_le_bytes()[..3]);
    result.extend_from_slice(&(h.height - 1).to_le_bytes()[..3]);
    pos = 12;
    while pos < bytes.len() {
        let n = le32(&bytes[pos + 4..])? as usize;
        let end = pos + 8 + n + (n & 1);
        if &bytes[pos..pos + 4] != b"VP8X" {
            result.extend_from_slice(&bytes[pos..end]);
        }
        pos = end;
    }
    let size = (result.len() - 8) as u32;
    result[4..8].copy_from_slice(&size.to_le_bytes());
    Ok(Cow::Owned(result))
}
pub(crate) fn decode(bytes: &[u8]) -> Result<ImageAnalysis> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_ENCODED {
        return Err(E::LimitExceeded);
    }
    let h = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)?
    } else if bytes.starts_with(b"\xff\xd8") {
        jpeg(bytes)?
    } else if bytes.starts_with(b"RIFF") {
        webp(bytes)?
    } else {
        return Err(E::UnsupportedFormat);
    };
    let format = match h.format {
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Webp => image::ImageFormat::WebP,
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED);
    let decoder_bytes = webp_decode_container(bytes, &h)?;
    let mut reader = ImageReader::with_format(Cursor::new(decoder_bytes.as_ref()), format);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|_| E::InvalidData)?;
    if decoder.dimensions() != (h.width, h.height) || decoder.total_bytes() > MAX_DECODED {
        return Err(E::LimitExceeded);
    }
    let has_alpha = decoder.color_type().has_alpha();
    let raw = DynamicImage::from_decoder(decoder).map_err(|_| E::InvalidData)?;
    let (transparent, partial) = alpha(&raw);
    let mut warnings = vec![];
    if h.depth > 8 {
        warnings.push(W::HighBitDepthDisplay8);
    }
    let mut rgba = raw.to_rgba8();
    drop(raw);
    let profile = if let Some(icc) = h.icc.as_ref() {
        let source = ColorProfile::new_from_slice(icc).map_err(|_| E::InvalidData)?;
        let layout = match source.color_space {
            DataColorSpace::Rgb => Layout::Rgba,
            DataColorSpace::Gray if h.space == "gray" => Layout::Gray,
            _ => return Err(E::UnsupportedColorProfile),
        };
        if source.color_space == DataColorSpace::Rgb && h.space == "gray" {
            return Err(E::UnsupportedColorProfile);
        }
        let transform = source
            .create_transform_8bit(
                layout,
                &ColorProfile::new_srgb(),
                Layout::Rgba,
                Default::default(),
            )
            .map_err(|_| E::UnsupportedColorProfile)?;
        let stride = h.width as usize * 4;
        let mut out = vec![0u8; stride];
        let mut gray = vec![0u8; h.width as usize];
        for row in rgba.as_mut().chunks_exact_mut(stride) {
            let input = if layout == Layout::Gray {
                for (g, p) in gray.iter_mut().zip(row.chunks_exact(4)) {
                    *g = p[0];
                }
                &gray[..]
            } else {
                &row[..]
            };
            transform
                .transform(input, &mut out)
                .map_err(|_| E::UnsupportedColorProfile)?;
            for (dest, original) in out.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                dest[3] = original[3];
            }
            row.copy_from_slice(&out);
        }
        Some(format!("icc-blake3:{}", blake3::hash(icc).to_hex()))
    } else {
        if h.non_srgb && !h.srgb {
            return Err(E::UnsupportedColorProfile);
        }
        if !h.srgb {
            warnings.push(W::AssumedSrgb);
        }
        None
    };
    let orientation = h
        .exif
        .as_ref()
        .map(|p| orientation(p))
        .transpose()?
        .unwrap_or(1);
    let mut display = DynamicImage::ImageRgba8(rgba);
    display.apply_orientation(
        image::metadata::Orientation::from_exif(orientation).ok_or(E::InvalidData)?,
    );
    let rgba = display.as_rgba8().ok_or(E::InvalidData)?;
    let (w, v) = rgba.dimensions();
    let corners = [
        rgba.get_pixel(0, 0).0,
        rgba.get_pixel(w - 1, 0).0,
        rgba.get_pixel(0, v - 1).0,
        rgba.get_pixel(w - 1, v - 1).0,
    ];
    let mut a = w;
    let mut b = v;
    while b != 0 {
        let n = a % b;
        a = b;
        b = n;
    }
    Ok(ImageAnalysis {
        content_hash: blake3::hash(bytes).to_hex().to_string(),
        profile: ImageProfile {
            raw_width: h.width,
            raw_height: h.height,
            orientation,
            width: w,
            height: v,
            format: h.format,
            encoded_bytes: bytes.len() as u64,
            bit_depth: h.depth,
            source_color_profile: profile,
            alpha_channel: has_alpha,
            transparency: if partial > 0 {
                Transparency::Partial
            } else if transparent > 0 {
                Transparency::Binary
            } else {
                Transparency::None
            },
            analyzer_version: ANALYZER.into(),
        },
        aspect_ratio: [w / a, v / a],
        transparent_pixels: transparent,
        partial_alpha_pixels: partial,
        source_color_space: h.space,
        warnings,
        display_rgba_hash: blake3::hash(rgba.as_raw()).to_hex().to_string(),
        display_corners: corners,
        peak_heap_bytes: 0,
    })
}
