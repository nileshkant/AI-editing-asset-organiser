//! SS-055 generation metadata: a bounded, allowlisted reader of the generation
//! settings some image tools embed in PNG, JPEG and WebP containers or write to
//! an A1111-style `<stem>.txt` sidecar. See docs/GENERATION_METADATA.md.
//!
//! Everything here is a *claim* read from bytes the user already selected. The
//! reader never looks at pixels, never fetches a URL, never runs an embedded
//! workflow and never opens a path named inside the metadata. Unknown stays
//! `None`; an explicitly empty value is `Some("")`. Oversized values become
//! unknown with an issue; nothing is truncated.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Read,
    time::{Duration, Instant},
};

/// Bumped whenever extraction output for the same bytes could change.
pub const PARSER: &str = "creativeshelf-generation-v1";
/// Text segments (PNG text chunks, EXIF/XMP payloads) examined per file.
pub const MAX_SEGMENTS: usize = 64;
/// Raw bytes of one metadata segment.
pub const MAX_SEGMENT_BYTES: usize = 1024 * 1024;
/// Decompressed bytes across every compressed segment of one file.
pub const MAX_INFLATED_BYTES: usize = 1024 * 1024;
/// Prompts and negative prompts.
pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
/// Seeds, model names, samplers, generators, dates, reference values.
pub const MAX_SHORT_BYTES: usize = 1024;
pub const MAX_SETTING_KEY_BYTES: usize = 64;
pub const MAX_SETTINGS: usize = 64;
pub const MAX_REFERENCES: usize = 32;
pub const MAX_CLAIMS: usize = 8;
pub const MAX_ISSUES: usize = 16;
pub const MAX_UNINTERPRETED: usize = 8;
pub const MAX_JSON_DEPTH: usize = 32;
pub const MAX_GRAPH_NODES: usize = 4096;
/// Hops followed through a ComfyUI graph from a sampler to its model loader.
const MAX_GRAPH_HOPS: usize = 16;
/// Sidecars read per image, and the largest one read.
pub const MAX_SIDECARS: usize = 1;
pub const MAX_SIDECAR_BYTES: u64 = 64 * 1024;
/// Wall-clock budget for interpreting one file's metadata.
pub const PARSE_BUDGET: Duration = Duration::from_secs(2);
/// Serialized extraction stored per content version (enforced by SQLite too).
pub const MAX_STORED_BYTES: usize = 512 * 1024;

// Missing and explicit null differ: every nullable stored field is required.
fn nullable<'de, D, T>(d: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(d)
}

/// Documented allowlist (docs/GENERATION_METADATA.md). Anything else is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationFormat {
    /// AUTOMATIC1111-style "parameters" text (also Forge, SD.Next and sidecars).
    A1111Parameters,
    /// ComfyUI API "prompt" graph JSON. Read as data; never executed.
    ComfyuiPrompt,
    /// InvokeAI "invokeai_metadata" JSON.
    InvokeaiMetadata,
    /// NovelAI "Comment" JSON with its "Description"/"Source" text.
    NovelaiComment,
    /// IPTC digital source type in XMP (for example `trainedAlgorithmicMedia`).
    XmpDigitalSource,
    /// Software tag (EXIF, PNG text or XMP CreatorTool), reported only with another claim.
    SoftwareTag,
    /// EXIF original date and offset, reported only with another claim.
    ExifDatetime,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    PngText,
    PngCompressedText,
    PngInternationalText,
    PngExif,
    JpegExif,
    JpegXmp,
    WebpExif,
    WebpXmp,
    Sidecar,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationField {
    Prompt,
    NegativePrompt,
    Generator,
    Model,
    Seed,
    Sampler,
    Created,
    Settings,
    References,
    DigitalSourceType,
    /// User-declared only: "yes" or "no". Never inferred.
    AiGenerated,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    ModelHash,
    Lora,
    Image,
    Other,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub kind: ReferenceKind,
    pub value: String,
    /// Local file names or paths: redacted from shared exports unless opted in.
    pub private: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationTime {
    /// As written (for example `2026:10:20 12:00:00`), never reinterpreted.
    pub value: String,
    /// `None`: the time zone is unknown, so the instant is uncertain.
    #[serde(deserialize_with = "nullable")]
    pub utc_offset: Option<String>,
}
/// `None` is unknown (not recorded); `Some("")` is recorded as empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationFields {
    #[serde(deserialize_with = "nullable")]
    pub prompt: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub negative_prompt: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub generator: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub model: Option<String>,
    /// Lossless text: seeds can exceed every JSON-safe integer.
    #[serde(deserialize_with = "nullable")]
    pub seed: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub sampler: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub created: Option<GenerationTime>,
    pub settings: BTreeMap<String, String>,
    pub references: Vec<Reference>,
    #[serde(deserialize_with = "nullable")]
    pub digital_source_type: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimSource {
    pub format: GenerationFormat,
    pub container: Container,
    /// PNG keyword, EXIF tag name, `xmp` or the sidecar file name.
    pub key: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationClaim {
    pub source: ClaimSource,
    pub fields: GenerationFields,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCode {
    /// Structure or JSON could not be read; the rest of the file still was.
    Malformed,
    /// A count, length, depth, decompression or time limit was reached.
    LimitExceeded,
    UnsupportedEncoding,
    /// Several candidates (for example two samplers); left unknown.
    Ambiguous,
    /// A number that cannot be kept exactly (for example a float seed).
    InexactNumber,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionIssue {
    #[serde(deserialize_with = "nullable")]
    pub container: Option<Container>,
    pub key: String,
    pub code: IssueCode,
    #[serde(deserialize_with = "nullable")]
    pub field: Option<GenerationField>,
}
/// Recognized structured data kept only as its size (for example a ComfyUI
/// "workflow" graph): its presence is shown, its contents are never run or stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Uninterpreted {
    pub container: Container,
    pub key: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    /// No allowlisted generation metadata.
    None,
    Found,
    /// Only unreadable candidates were found.
    Malformed,
    /// The file or sidecar exceeded a reader limit before anything was found.
    LimitExceeded,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationExtraction {
    pub parser: String,
    pub status: ExtractionStatus,
    pub claims: Vec<GenerationClaim>,
    pub issues: Vec<ExtractionIssue>,
    pub uninterpreted: Vec<Uninterpreted>,
    /// Fields removed by a shared export; distinct from never recorded.
    pub redacted: Vec<GenerationField>,
}
impl GenerationExtraction {
    fn empty(status: ExtractionStatus) -> Self {
        Self {
            parser: PARSER.into(),
            status,
            claims: vec![],
            issues: vec![],
            uninterpreted: vec![],
            redacted: vec![],
        }
    }
    /// Structural validation for stored or imported records.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.parser.is_empty() || self.parser.len() > 256 || self.parser.chars().any(char::is_control) {
            return Err("Invalid generation parser");
        }
        if self.claims.len() > MAX_CLAIMS
            || self.issues.len() > MAX_ISSUES
            || self.uninterpreted.len() > MAX_UNINTERPRETED
            || self.redacted.len() > 16
        {
            return Err("Generation metadata exceeds a record limit");
        }
        if (self.status == ExtractionStatus::Found) != !self.claims.is_empty() {
            return Err("Generation status does not match its claims");
        }
        for c in &self.claims {
            short(&c.source.key).map_err(|_| "Invalid generation claim source")?;
            c.fields.validate()?;
        }
        for i in &self.issues {
            short(&i.key).map_err(|_| "Invalid generation issue")?;
        }
        for u in &self.uninterpreted {
            short(&u.key).map_err(|_| "Invalid generation data")?;
        }
        if serde_json::to_vec(self).map_or(true, |b| b.len() > MAX_STORED_BYTES) {
            return Err("Generation metadata exceeds its stored size");
        }
        Ok(())
    }
}
fn plain(s: &str, max: usize) -> Result<(), ()> {
    // Prompts keep newlines and tabs; other control characters are rejected.
    if s.len() > max || s.chars().any(|c| c.is_control() && c != '\n' && c != '\t' && c != '\r') {
        return Err(());
    }
    Ok(())
}
fn short(s: &str) -> Result<(), ()> {
    if s.len() > MAX_SHORT_BYTES || s.chars().any(char::is_control) {
        return Err(());
    }
    Ok(())
}
impl GenerationFields {
    pub fn validate(&self) -> Result<(), &'static str> {
        let bad = "Invalid generation field";
        for p in [&self.prompt, &self.negative_prompt].into_iter().flatten() {
            plain(p, MAX_PROMPT_BYTES).map_err(|_| bad)?;
        }
        for v in [&self.generator, &self.model, &self.seed, &self.sampler, &self.digital_source_type]
            .into_iter()
            .flatten()
        {
            short(v).map_err(|_| bad)?;
        }
        if let Some(t) = &self.created {
            short(&t.value).map_err(|_| bad)?;
            if let Some(o) = &t.utc_offset {
                short(o).map_err(|_| bad)?;
            }
        }
        if self.settings.len() > MAX_SETTINGS || self.references.len() > MAX_REFERENCES {
            return Err(bad);
        }
        for (k, v) in &self.settings {
            if k.is_empty() || k.len() > MAX_SETTING_KEY_BYTES {
                return Err(bad);
            }
            short(k).map_err(|_| bad)?;
            short(v).map_err(|_| bad)?;
        }
        for r in &self.references {
            if r.value.is_empty() {
                return Err(bad);
            }
            short(&r.value).map_err(|_| bad)?;
        }
        Ok(())
    }
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Values that name a location on someone's machine: absolute POSIX paths, home
/// paths, Windows drive or UNC paths and file URLs. Inert data, never opened.
pub fn local_path(value: &str) -> bool {
    let v = value.trim();
    let b = v.as_bytes();
    v.starts_with('/')
        || v.starts_with('~')
        || v.starts_with("\\\\")
        || v.to_ascii_lowercase().starts_with("file:")
        || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}
/// Setting keys that carry prompt text (for example A1111 "Hires prompt").
pub fn prompt_setting(key: &str) -> bool {
    key.to_ascii_lowercase().contains("prompt")
}

/// What a shared export (portable JSON, folder snapshot) may carry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationSharing {
    /// Prompts, negative prompts and prompt-bearing settings.
    pub prompts: bool,
    /// Local file names and absolute paths (reference images, model paths).
    pub private_references: bool,
}
/// Removes what `sharing` does not allow, recording each removed field so a
/// reader can tell "removed on export" from "never recorded".
pub fn redact_fields(f: &mut GenerationFields, sharing: GenerationSharing, redacted: &mut Vec<GenerationField>) {
    let mut mark = |field| {
        if !redacted.contains(&field) {
            redacted.push(field);
        }
    };
    if !sharing.prompts {
        if f.prompt.take().is_some() {
            mark(GenerationField::Prompt);
        }
        if f.negative_prompt.take().is_some() {
            mark(GenerationField::NegativePrompt);
        }
        let before = f.settings.len();
        f.settings.retain(|k, _| !prompt_setting(k));
        if f.settings.len() != before {
            mark(GenerationField::Settings);
        }
    }
    if !sharing.private_references {
        let before = f.references.len();
        f.references.retain(|r| !r.private && !local_path(&r.value));
        if f.references.len() != before {
            mark(GenerationField::References);
        }
        for (field, value) in [
            (GenerationField::Generator, &mut f.generator),
            (GenerationField::Model, &mut f.model),
            (GenerationField::Sampler, &mut f.sampler),
        ] {
            if value.as_deref().is_some_and(local_path) {
                *value = None;
                mark(field);
            }
        }
        let before = f.settings.len();
        f.settings.retain(|_, v| !local_path(v));
        if f.settings.len() != before {
            mark(GenerationField::Settings);
        }
    }
}
impl GenerationExtraction {
    pub fn redacted_for(&self, sharing: GenerationSharing) -> Self {
        let mut out = self.clone();
        for claim in &mut out.claims {
            redact_fields(&mut claim.fields, sharing, &mut out.redacted);
        }
        out.redacted.sort();
        out
    }
}

/// Reader state shared across one file: issue list, budgets and deadline.
struct Reader {
    issues: Vec<ExtractionIssue>,
    uninterpreted: Vec<Uninterpreted>,
    segments: usize,
    inflated: usize,
    deadline: Instant,
    exhausted: bool,
}
impl Reader {
    fn new(budget: Duration) -> Self {
        Self {
            issues: vec![],
            uninterpreted: vec![],
            segments: 0,
            inflated: 0,
            deadline: Instant::now() + budget,
            exhausted: false,
        }
    }
    fn issue(&mut self, container: Option<Container>, key: &str, code: IssueCode, field: Option<GenerationField>) {
        if code == IssueCode::LimitExceeded {
            self.exhausted = true;
        }
        let key = clip_key(key);
        let issue = ExtractionIssue { container, key, code, field };
        if self.issues.len() < MAX_ISSUES && !self.issues.contains(&issue) {
            self.issues.push(issue);
        }
    }
    /// One more segment within count and time budgets.
    fn admit(&mut self, container: Container, key: &str) -> bool {
        self.segments += 1;
        if self.segments > MAX_SEGMENTS || Instant::now() > self.deadline {
            self.issue(Some(container), key, IssueCode::LimitExceeded, None);
            return false;
        }
        true
    }
    fn inflate(&mut self, data: &[u8], container: Container, key: &str) -> Option<Vec<u8>> {
        let left = MAX_INFLATED_BYTES.saturating_sub(self.inflated);
        let mut out = Vec::new();
        let read = flate2::read::ZlibDecoder::new(data)
            .take(left as u64 + 1)
            .read_to_end(&mut out);
        if read.is_err() {
            self.issue(Some(container), key, IssueCode::Malformed, None);
            return None;
        }
        if out.len() > left {
            self.issue(Some(container), key, IssueCode::LimitExceeded, None);
            return None;
        }
        self.inflated += out.len();
        Some(out)
    }
}
/// Keys are bytes from the file; keep them short, printable and lossless enough to show.
fn clip_key(key: &str) -> String {
    let clean: String = key.chars().filter(|c| !c.is_control()).collect();
    if clean.len() <= 80 {
        return clean;
    }
    let mut end = 80;
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    clean[..end].to_owned()
}

/// One text payload found in a container.
struct Segment {
    container: Container,
    key: String,
    text: String,
}
#[derive(Default)]
struct Found {
    segments: Vec<Segment>,
    exif: Vec<(Container, Vec<u8>)>,
    xmp: Vec<(Container, String)>,
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}
fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn png(bytes: &[u8], r: &mut Reader, out: &mut Found) {
    let mut pos = 8;
    while pos + 12 <= bytes.len() {
        let Some(len) = be32(bytes, pos).map(|n| n as usize) else { break };
        let Some(end) = pos.checked_add(12).and_then(|n| n.checked_add(len)).filter(|n| *n <= bytes.len()) else {
            r.issue(Some(Container::PngText), "chunk", IssueCode::Malformed, None);
            return;
        };
        let kind = &bytes[pos + 4..pos + 8];
        let data = &bytes[pos + 8..end - 4];
        pos = end;
        if kind == b"IEND" {
            return;
        }
        let container = match kind {
            b"tEXt" => Container::PngText,
            b"zTXt" => Container::PngCompressedText,
            b"iTXt" => Container::PngInternationalText,
            b"eXIf" => Container::PngExif,
            _ => continue,
        };
        if crc32fast::hash(&bytes[end - 8 - len..end - 4]) != be32(bytes, end - 4).unwrap_or(0) {
            r.issue(Some(container), "chunk", IssueCode::Malformed, None);
            continue;
        }
        if len > MAX_SEGMENT_BYTES {
            r.issue(Some(container), "chunk", IssueCode::LimitExceeded, None);
            continue;
        }
        if container == Container::PngExif {
            if r.admit(container, "eXIf") {
                out.exif.push((container, data.to_vec()));
            }
            continue;
        }
        let Some(zero) = data.iter().position(|b| *b == 0) else {
            r.issue(Some(container), "chunk", IssueCode::Malformed, None);
            continue;
        };
        let key = latin1(&data[..zero]);
        if key.is_empty() || key.len() > 79 {
            r.issue(Some(container), "chunk", IssueCode::Malformed, None);
            continue;
        }
        let tail = &data[zero + 1..];
        if !wanted_key(&key) {
            continue;
        }
        if !r.admit(container, &key) {
            return;
        }
        let text = match container {
            Container::PngText => Some(latin1(tail)),
            Container::PngCompressedText => match tail.split_first() {
                Some((0, rest)) => r.inflate(rest, container, &key).map(|b| latin1(&b)),
                _ => {
                    r.issue(Some(container), &key, IssueCode::Malformed, None);
                    None
                }
            },
            _ => itxt(tail, r, &key),
        };
        if let Some(text) = text {
            out.segments.push(Segment { container, key, text });
        }
    }
}
fn itxt(tail: &[u8], r: &mut Reader, key: &str) -> Option<String> {
    let c = Container::PngInternationalText;
    let malformed = |r: &mut Reader| {
        r.issue(Some(c), key, IssueCode::Malformed, None);
        None
    };
    let (&flag, rest) = tail.split_first()?;
    let Some((&method, rest)) = rest.split_first() else { return malformed(r) };
    if flag > 1 || method != 0 {
        return malformed(r);
    }
    let Some(lang) = rest.iter().position(|b| *b == 0) else { return malformed(r) };
    let rest = &rest[lang + 1..];
    let Some(translated) = rest.iter().position(|b| *b == 0) else { return malformed(r) };
    let raw = &rest[translated + 1..];
    let bytes = if flag == 1 { r.inflate(raw, c, key)? } else { raw.to_vec() };
    match String::from_utf8(bytes) {
        Ok(s) => Some(s),
        Err(_) => {
            r.issue(Some(c), key, IssueCode::UnsupportedEncoding, None);
            None
        }
    }
}
/// PNG text keywords that the allowlist interprets.
fn wanted_key(key: &str) -> bool {
    matches!(
        key,
        "parameters" | "prompt" | "workflow" | "invokeai_metadata" | "Comment" | "Description" | "Source" | "Software"
            | "XML:com.adobe.xmp"
    )
}

fn jpeg(bytes: &[u8], r: &mut Reader, out: &mut Found) {
    let mut pos = 2;
    // Metadata precedes the first scan; entropy-coded data is never walked.
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xff {
            r.issue(Some(Container::JpegExif), "marker", IssueCode::Malformed, None);
            return;
        }
        let marker = bytes[pos + 1];
        if marker == 0xff {
            pos += 1;
            continue;
        }
        if marker == 0xda || marker == 0xd9 {
            return;
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            pos += 2;
            continue;
        }
        let Some(len) = be16(bytes, pos + 2).map(usize::from) else { return };
        let Some(end) = (pos + 2).checked_add(len).filter(|e| *e <= bytes.len() && len >= 2) else {
            r.issue(Some(Container::JpegExif), "marker", IssueCode::Malformed, None);
            return;
        };
        let data = &bytes[pos + 4..end];
        pos = end;
        if marker != 0xe1 {
            continue;
        }
        if let Some(exif) = data.strip_prefix(b"Exif\0\0") {
            if r.admit(Container::JpegExif, "Exif") {
                out.exif.push((Container::JpegExif, exif.to_vec()));
            }
        } else if let Some(xmp) = data.strip_prefix(b"http://ns.adobe.com/xap/1.0/\0") {
            if r.admit(Container::JpegXmp, "xmp") {
                match String::from_utf8(xmp.to_vec()) {
                    Ok(s) => out.xmp.push((Container::JpegXmp, s)),
                    Err(_) => r.issue(Some(Container::JpegXmp), "xmp", IssueCode::UnsupportedEncoding, None),
                }
            }
        }
    }
}
fn webp(bytes: &[u8], r: &mut Reader, out: &mut Found) {
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let kind = &bytes[pos..pos + 4];
        let Some(n) = le32(bytes, pos + 4).map(|n| n as usize) else { return };
        let Some(end) = (pos + 8).checked_add(n).filter(|e| *e <= bytes.len()) else {
            r.issue(Some(Container::WebpExif), "chunk", IssueCode::Malformed, None);
            return;
        };
        let data = &bytes[pos + 8..end];
        pos = end + (n & 1);
        let container = match kind {
            b"EXIF" => Container::WebpExif,
            b"XMP " => Container::WebpXmp,
            _ => continue,
        };
        if n > MAX_SEGMENT_BYTES {
            r.issue(Some(container), "chunk", IssueCode::LimitExceeded, None);
            continue;
        }
        if !r.admit(container, if container == Container::WebpExif { "EXIF" } else { "xmp" }) {
            return;
        }
        if container == Container::WebpExif {
            out.exif.push((container, data.strip_prefix(b"Exif\0\0").unwrap_or(data).to_vec()));
        } else {
            match String::from_utf8(data.to_vec()) {
                Ok(s) => out.xmp.push((container, s)),
                Err(_) => r.issue(Some(container), "xmp", IssueCode::UnsupportedEncoding, None),
            }
        }
    }
}

/// The EXIF tags this reader uses, from a bounded TIFF walk.
#[derive(Default)]
struct Exif {
    software: Option<String>,
    user_comment: Option<Vec<u8>>,
    date: Option<String>,
    offset: Option<String>,
}
fn exif(p: &[u8]) -> Option<Exif> {
    let little = match p.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = p.get(at..at + 2)?.try_into().ok()?;
        Some(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = p.get(at..at + 4)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    // Entry value bytes: inline when they fit in four bytes, else at an offset.
    let value = |entry: usize| -> Option<&[u8]> {
        let kind = u16_at(entry + 2)?;
        let count = u32_at(entry + 4)? as usize;
        let unit = match kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 => 4,
            _ => return None,
        };
        let size = count.checked_mul(unit)?;
        if size > MAX_SEGMENT_BYTES {
            return None;
        }
        if size <= 4 {
            p.get(entry + 8..entry + 8 + size)
        } else {
            let at = u32_at(entry + 8)? as usize;
            p.get(at..at.checked_add(size)?)
        }
    };
    let ascii = |b: &[u8]| -> String { latin1(b.split(|c| *c == 0).next().unwrap_or(b)) };
    let mut out = Exif::default();
    let ifd0 = u32_at(4)? as usize;
    let mut sub = None;
    let entries = |at: usize| -> Option<(usize, usize)> {
        let n = usize::from(u16_at(at)?);
        (n <= 512).then_some((at + 2, n))
    };
    let (start, n) = entries(ifd0)?;
    for i in 0..n {
        let e = start + i * 12;
        match u16_at(e)? {
            0x0131 => out.software = value(e).map(ascii),
            0x8769 => sub = u32_at(e + 8).map(|v| v as usize),
            _ => {}
        }
    }
    if let Some(at) = sub.filter(|at| *at != ifd0) {
        let (start, n) = entries(at)?;
        for i in 0..n {
            let e = start + i * 12;
            match u16_at(e)? {
                0x9286 => out.user_comment = value(e).map(<[u8]>::to_vec),
                0x9003 => out.date = value(e).map(ascii),
                0x9011 => out.offset = value(e).map(ascii),
                _ => {}
            }
        }
    }
    Some(out)
}
/// EXIF UserComment: an 8-byte character code then the text. A1111 writes
/// "UNICODE" as UTF-16 (usually big-endian); byte order is detected.
fn user_comment(b: &[u8]) -> Result<String, IssueCode> {
    let (code, text) = if b.len() >= 8 { b.split_at(8) } else { return Err(IssueCode::Malformed) };
    let trimmed = |s: String| s.trim_end_matches('\0').to_owned();
    match code {
        b"ASCII\0\0\0" => Ok(trimmed(latin1(text))),
        b"UNICODE\0" => {
            if text.len() % 2 != 0 {
                return Err(IssueCode::UnsupportedEncoding);
            }
            let zeros_even = text.iter().step_by(2).filter(|c| **c == 0).count();
            let zeros_odd = text.iter().skip(1).step_by(2).filter(|c| **c == 0).count();
            let units: Vec<u16> = text
                .chunks_exact(2)
                .map(|c| if zeros_even >= zeros_odd { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
                .collect();
            String::from_utf16(&units).map(trimmed).map_err(|_| IssueCode::UnsupportedEncoding)
        }
        [0, 0, 0, 0, 0, 0, 0, 0] => String::from_utf8(text.to_vec()).map(trimmed).map_err(|_| IssueCode::UnsupportedEncoding),
        _ => Err(IssueCode::UnsupportedEncoding),
    }
}

/// XMP is searched as text for two properties. No XML parser runs, so no
/// entities expand and no external resources are resolved.
fn xmp_property(xmp: &str, name: &str) -> Option<String> {
    for (i, _) in xmp.match_indices(name) {
        let rest = &xmp[i + name.len()..];
        let value = if let Some(v) = rest.strip_prefix("=\"") {
            v.split('"').next()
        } else if let Some(v) = rest.strip_prefix('>') {
            let v = v.trim_start();
            let v = v.strip_prefix("<rdf:Alt>").map(str::trim_start).unwrap_or(v);
            let v = match v.strip_prefix("<rdf:li") {
                Some(li) => li.split_once('>').map(|(_, t)| t).unwrap_or(""),
                None => v,
            };
            v.split('<').next()
        } else {
            None
        };
        if let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) {
            return Some(v.to_owned());
        }
    }
    None
}

/// Text for a short field: printable and bounded, else unknown with an issue.
fn short_field(r: &mut Reader, src: &ClaimSource, field: GenerationField, value: &str) -> Option<String> {
    let v = value.trim();
    if v.len() > MAX_SHORT_BYTES {
        r.issue(Some(src.container), &src.key, IssueCode::LimitExceeded, Some(field));
        return None;
    }
    if v.chars().any(char::is_control) {
        r.issue(Some(src.container), &src.key, IssueCode::Malformed, Some(field));
        return None;
    }
    Some(v.to_owned())
}
fn prompt_field(r: &mut Reader, src: &ClaimSource, field: GenerationField, value: &str) -> Option<String> {
    if value.len() > MAX_PROMPT_BYTES {
        r.issue(Some(src.container), &src.key, IssueCode::LimitExceeded, Some(field));
        return None;
    }
    let v: String = value.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect();
    Some(v.trim().to_owned())
}
fn setting(r: &mut Reader, src: &ClaimSource, f: &mut GenerationFields, key: &str, value: &str) {
    let key = key.trim();
    if key.is_empty() || key.len() > MAX_SETTING_KEY_BYTES || key.chars().any(char::is_control) {
        r.issue(Some(src.container), &src.key, IssueCode::LimitExceeded, Some(GenerationField::Settings));
        return;
    }
    if f.settings.len() >= MAX_SETTINGS && !f.settings.contains_key(key) {
        r.issue(Some(src.container), &src.key, IssueCode::LimitExceeded, Some(GenerationField::Settings));
        return;
    }
    if let Some(v) = short_field(r, src, GenerationField::Settings, value) {
        f.settings.insert(key.to_owned(), v);
    }
}
fn reference(r: &mut Reader, src: &ClaimSource, f: &mut GenerationFields, kind: ReferenceKind, value: &str, private: bool) {
    let Some(v) = short_field(r, src, GenerationField::References, value).filter(|v| !v.is_empty()) else { return };
    if f.references.len() >= MAX_REFERENCES {
        r.issue(Some(src.container), &src.key, IssueCode::LimitExceeded, Some(GenerationField::References));
        return;
    }
    let private = private || local_path(&v);
    let item = Reference { kind, value: v, private };
    if !f.references.contains(&item) {
        f.references.push(item);
    }
}

/// A1111 "parameters": prompt lines, an optional "Negative prompt:" section and
/// a final settings line that starts with "Steps:".
fn a1111(text: &str, r: &mut Reader, src: &ClaimSource) -> GenerationFields {
    let mut f = GenerationFields::default();
    let text = text.replace("\r\n", "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    let settings = match lines.iter().rposition(|l| l.trim_start().starts_with("Steps:")) {
        Some(i) => {
            let tail = lines.split_off(i).join("\n");
            Some(tail)
        }
        None => None,
    };
    let negative = lines.iter().position(|l| l.starts_with("Negative prompt:"));
    let (prompt, neg) = match negative {
        Some(i) => {
            let mut rest = lines[i..].to_vec();
            rest[0] = rest[0].trim_start_matches("Negative prompt:");
            (lines[..i].join("\n"), Some(rest.join("\n")))
        }
        None => (lines.join("\n"), None),
    };
    f.prompt = prompt_field(r, src, GenerationField::Prompt, &prompt);
    f.negative_prompt = neg.and_then(|n| prompt_field(r, src, GenerationField::NegativePrompt, &n));
    if let Some(line) = settings {
        for (key, value) in settings_pairs(&line) {
            match key.as_str() {
                "Seed" => f.seed = short_field(r, src, GenerationField::Seed, &value),
                "Sampler" => f.sampler = short_field(r, src, GenerationField::Sampler, &value),
                "Model" => f.model = short_field(r, src, GenerationField::Model, &value),
                "Model hash" => reference(r, src, &mut f, ReferenceKind::ModelHash, &value, false),
                _ => setting(r, src, &mut f, &key, &value),
            }
        }
    }
    f
}
/// `Key: value, Key: "quoted, value"` as A1111 writes it (JSON-style quoting).
fn settings_pairs(line: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i] == ',' || chars[i].is_whitespace()) {
            i += 1;
        }
        let start = i;
        while i < chars.len() && chars[i] != ':' && chars[i] != ',' {
            i += 1;
        }
        if i >= chars.len() || chars[i] != ':' {
            break;
        }
        let key: String = chars[start..i].iter().collect();
        i += 1;
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        let mut value = String::new();
        if chars.get(i) == Some(&'"') {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                value.push(chars[i]);
                i += 1;
            }
            i += 1;
        } else {
            while i < chars.len() && chars[i] != ',' {
                value.push(chars[i]);
                i += 1;
            }
        }
        out.push((key.trim().to_owned(), value.trim().to_owned()));
    }
    out
}

/// Nesting depth before parsing, so hostile JSON cannot exhaust the parser.
fn json_depth_ok(text: &str) -> bool {
    let (mut depth, mut string, mut escape) = (0usize, false, false);
    for b in text.bytes() {
        if string {
            match (escape, b) {
                (true, _) => escape = false,
                (false, b'\\') => escape = true,
                (false, b'"') => string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_JSON_DEPTH {
                    return false;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
}
fn json(text: &str, r: &mut Reader, src: &ClaimSource) -> Option<serde_json::Map<String, serde_json::Value>> {
    if !json_depth_ok(text) {
        r.issue(Some(src.container), &src.key, IssueCode::LimitExceeded, None);
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(m)) => Some(m),
        _ => {
            r.issue(Some(src.container), &src.key, IssueCode::Malformed, None);
            None
        }
    }
}
/// Scalars as exact text. A non-integral or out-of-range number is inexact.
fn scalar(r: &mut Reader, src: &ClaimSource, field: GenerationField, v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => short_field(r, src, field, s),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Number(n) if n.is_u64() || n.is_i64() => Some(n.to_string()),
        serde_json::Value::Number(n) if field != GenerationField::Seed => Some(n.to_string()),
        serde_json::Value::Number(_) => {
            r.issue(Some(src.container), &src.key, IssueCode::InexactNumber, Some(field));
            None
        }
        _ => None,
    }
}
fn comfyui(text: &str, r: &mut Reader, src: &ClaimSource) -> Option<GenerationFields> {
    use serde_json::Value;
    let graph = json(text, r, src)?;
    if graph.len() > MAX_GRAPH_NODES {
        r.issue(Some(src.container), &src.key, IssueCode::LimitExceeded, None);
        return None;
    }
    let class = |n: &Value| n.get("class_type").and_then(Value::as_str).unwrap_or("").to_owned();
    let inputs = |n: &Value| n.get("inputs").and_then(Value::as_object).cloned().unwrap_or_default();
    let linked = |v: Option<&Value>| -> Option<&Value> {
        let id = v?.as_array()?.first()?;
        let key = match id {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return None,
        };
        graph.get(&key)
    };
    let mut f = GenerationFields::default();
    let mut ids: Vec<&String> = graph.keys().collect();
    ids.sort();
    for id in &ids {
        let node = &graph[*id];
        if class(node) == "LoadImage" {
            if let Some(name) = inputs(node).get("image").and_then(Value::as_str) {
                reference(r, src, &mut f, ReferenceKind::Image, name, true);
            }
        }
    }
    let samplers: Vec<&Value> = ids
        .iter()
        .map(|id| &graph[*id])
        .filter(|n| matches!(class(n).as_str(), "KSampler" | "KSamplerAdvanced"))
        .collect();
    let loaders: Vec<String> = ids
        .iter()
        .filter_map(|id| {
            let i = inputs(&graph[*id]);
            i.get("ckpt_name").or_else(|| i.get("unet_name")).and_then(Value::as_str).map(str::to_owned)
        })
        .collect();
    if samplers.len() != 1 {
        if samplers.len() > 1 {
            r.issue(Some(src.container), &src.key, IssueCode::Ambiguous, Some(GenerationField::Prompt));
        }
        if loaders.len() == 1 {
            f.model = short_field(r, src, GenerationField::Model, &loaders[0]);
        }
        return (!f.is_empty()).then_some(f);
    }
    let s = inputs(samplers[0]);
    if let Some(v) = s.get("seed").or_else(|| s.get("noise_seed")).filter(|v| !v.is_array()) {
        f.seed = scalar(r, src, GenerationField::Seed, v);
    }
    if let Some(v) = s.get("sampler_name") {
        f.sampler = scalar(r, src, GenerationField::Sampler, v);
    }
    for key in ["steps", "cfg", "scheduler", "denoise"] {
        if let Some(v) = s.get(key).filter(|v| !v.is_array()).and_then(|v| scalar(r, src, GenerationField::Settings, v)) {
            setting(r, src, &mut f, key, &v);
        }
    }
    let text_of = |n: Option<&Value>| -> Option<String> {
        let i = inputs(n?);
        i.get("text").or_else(|| i.get("text_g")).and_then(Value::as_str).map(str::to_owned)
    };
    f.prompt = text_of(linked(s.get("positive"))).and_then(|t| prompt_field(r, src, GenerationField::Prompt, &t));
    f.negative_prompt =
        text_of(linked(s.get("negative"))).and_then(|t| prompt_field(r, src, GenerationField::NegativePrompt, &t));
    let mut node = linked(s.get("model"));
    for _ in 0..MAX_GRAPH_HOPS {
        let Some(n) = node else { break };
        let i = inputs(n);
        if let Some(lora) = i.get("lora_name").and_then(Value::as_str) {
            reference(r, src, &mut f, ReferenceKind::Lora, lora, false);
        }
        if let Some(name) = i.get("ckpt_name").or_else(|| i.get("unet_name")).and_then(Value::as_str) {
            f.model = short_field(r, src, GenerationField::Model, name);
            break;
        }
        node = linked(i.get("model"));
    }
    Some(f)
}
fn object_fields(
    m: &serde_json::Map<String, serde_json::Value>,
    r: &mut Reader,
    src: &ClaimSource,
    f: &mut GenerationFields,
    skip: &[&str],
) {
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort();
    for k in keys {
        if skip.contains(&k.as_str()) {
            continue;
        }
        let v = &m[k];
        if v.is_object() || v.is_array() || v.is_null() {
            continue;
        }
        if let Some(text) = scalar(r, src, GenerationField::Settings, v) {
            setting(r, src, f, k, &text);
        }
    }
}
fn invokeai(text: &str, r: &mut Reader, src: &ClaimSource) -> Option<GenerationFields> {
    let m = json(text, r, src)?;
    let mut f = GenerationFields::default();
    let text_field = |r: &mut Reader, key, field| {
        m.get(key).and_then(serde_json::Value::as_str).and_then(|t| prompt_field(r, src, field, t))
    };
    f.prompt = text_field(r, "positive_prompt", GenerationField::Prompt);
    f.negative_prompt = text_field(r, "negative_prompt", GenerationField::NegativePrompt);
    if let Some(v) = m.get("seed") {
        f.seed = scalar(r, src, GenerationField::Seed, v);
    }
    if let Some(v) = m.get("scheduler") {
        f.sampler = scalar(r, src, GenerationField::Sampler, v);
    }
    if let Some(model) = m.get("model") {
        let name = model.get("name").or_else(|| model.get("model_name")).or(Some(model)).and_then(serde_json::Value::as_str);
        f.model = name.and_then(|n| short_field(r, src, GenerationField::Model, n));
        if let Some(hash) = model.get("hash").and_then(serde_json::Value::as_str) {
            reference(r, src, &mut f, ReferenceKind::ModelHash, hash, false);
        }
    }
    if let Some(v) = m.get("app_version").and_then(serde_json::Value::as_str) {
        f.generator = short_field(r, src, GenerationField::Generator, &format!("InvokeAI {v}"));
    }
    object_fields(&m, r, src, &mut f, &["positive_prompt", "negative_prompt", "seed", "scheduler", "app_version"]);
    Some(f)
}
fn novelai(text: &str, r: &mut Reader, src: &ClaimSource) -> Option<GenerationFields> {
    let m = json(text, r, src)?;
    let mut f = GenerationFields::default();
    if let Some(p) = m.get("prompt").and_then(serde_json::Value::as_str) {
        f.prompt = prompt_field(r, src, GenerationField::Prompt, p);
    }
    if let Some(p) = m.get("uc").and_then(serde_json::Value::as_str) {
        f.negative_prompt = prompt_field(r, src, GenerationField::NegativePrompt, p);
    }
    if let Some(v) = m.get("seed") {
        f.seed = scalar(r, src, GenerationField::Seed, v);
    }
    if let Some(v) = m.get("sampler") {
        f.sampler = scalar(r, src, GenerationField::Sampler, v);
    }
    object_fields(&m, r, src, &mut f, &["prompt", "uc", "seed", "sampler"]);
    Some(f)
}

fn claim(format: GenerationFormat, container: Container, key: &str) -> ClaimSource {
    ClaimSource { format, container, key: clip_key(key) }
}
/// Reads the allowlisted metadata in one still image's bytes.
pub fn extract(bytes: &[u8]) -> GenerationExtraction {
    extract_within(bytes, PARSE_BUDGET)
}
pub(crate) fn extract_within(bytes: &[u8], budget: Duration) -> GenerationExtraction {
    let mut r = Reader::new(budget);
    let mut found = Found::default();
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes, &mut r, &mut found);
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg(bytes, &mut r, &mut found);
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        webp(bytes, &mut r, &mut found);
    } else {
        return GenerationExtraction::empty(ExtractionStatus::None);
    }
    interpret(found, r)
}
fn interpret(found: Found, mut r: Reader) -> GenerationExtraction {
    let mut claims: Vec<GenerationClaim> = vec![];
    let mut software: Option<(ClaimSource, String)> = None;
    let mut dates: Option<(ClaimSource, GenerationTime)> = None;
    let push = |claims: &mut Vec<GenerationClaim>, r: &mut Reader, source: ClaimSource, fields: GenerationFields| {
        if fields.is_empty() {
            return;
        }
        if claims.len() >= MAX_CLAIMS {
            r.issue(Some(source.container), &source.key, IssueCode::LimitExceeded, None);
            return;
        }
        claims.push(GenerationClaim { source, fields });
    };
    let novelai_software = found
        .segments
        .iter()
        .any(|s| s.key == "Software" && s.text.trim().starts_with("NovelAI"));
    for s in &found.segments {
        if Instant::now() > r.deadline {
            r.issue(Some(s.container), &s.key, IssueCode::LimitExceeded, None);
            break;
        }
        match s.key.as_str() {
            "parameters" => {
                let src = claim(GenerationFormat::A1111Parameters, s.container, &s.key);
                let f = a1111(&s.text, &mut r, &src);
                push(&mut claims, &mut r, src, f);
            }
            "prompt" => {
                let src = claim(GenerationFormat::ComfyuiPrompt, s.container, &s.key);
                if let Some(f) = comfyui(&s.text, &mut r, &src) {
                    push(&mut claims, &mut r, src, f);
                }
            }
            "workflow" => {
                if r.uninterpreted.len() < MAX_UNINTERPRETED {
                    r.uninterpreted.push(Uninterpreted { container: s.container, key: s.key.clone(), bytes: s.text.len() as u64 });
                }
            }
            "invokeai_metadata" => {
                let src = claim(GenerationFormat::InvokeaiMetadata, s.container, &s.key);
                if let Some(f) = invokeai(&s.text, &mut r, &src) {
                    push(&mut claims, &mut r, src, f);
                }
            }
            "Comment" if novelai_software => {
                let src = claim(GenerationFormat::NovelaiComment, s.container, &s.key);
                if let Some(mut f) = novelai(&s.text, &mut r, &src) {
                    let text = |k: &str| found.segments.iter().find(|x| x.key == k).map(|x| x.text.clone());
                    if f.prompt.is_none() {
                        f.prompt = text("Description").and_then(|t| prompt_field(&mut r, &src, GenerationField::Prompt, &t));
                    }
                    f.model = text("Source").and_then(|t| short_field(&mut r, &src, GenerationField::Model, &t));
                    f.generator = text("Software").and_then(|t| short_field(&mut r, &src, GenerationField::Generator, &t));
                    push(&mut claims, &mut r, src, f);
                }
            }
            "Software" if !novelai_software => {
                let src = claim(GenerationFormat::SoftwareTag, s.container, &s.key);
                if let Some(v) = short_field(&mut r, &src, GenerationField::Generator, &s.text) {
                    software.get_or_insert((src, v));
                }
            }
            "XML:com.adobe.xmp" => xmp(&s.text, s.container, &mut r, &mut claims, &mut software),
            _ => {}
        }
    }
    for (container, x) in &found.xmp {
        xmp(x, *container, &mut r, &mut claims, &mut software);
    }
    for (container, raw) in &found.exif {
        let Some(e) = exif(raw) else {
            r.issue(Some(*container), "Exif", IssueCode::Malformed, None);
            continue;
        };
        if let Some(comment) = &e.user_comment {
            match user_comment(comment) {
                // Only text in the A1111 layout is a generation claim; any other
                // comment is ordinary EXIF and is not interpreted.
                Ok(text) if text.lines().any(|l| l.trim_start().starts_with("Steps:")) => {
                    let src = claim(GenerationFormat::A1111Parameters, *container, "UserComment");
                    let f = a1111(&text, &mut r, &src);
                    push(&mut claims, &mut r, src, f);
                }
                Ok(_) => {}
                Err(code) => r.issue(Some(*container), "UserComment", code, None),
            }
        }
        if let Some(sw) = &e.software {
            let src = claim(GenerationFormat::SoftwareTag, *container, "Software");
            if let Some(v) = short_field(&mut r, &src, GenerationField::Generator, sw).filter(|v| !v.is_empty()) {
                software.get_or_insert((src, v));
            }
        }
        if let Some(date) = &e.date {
            let src = claim(GenerationFormat::ExifDatetime, *container, "DateTimeOriginal");
            if let Some(value) = short_field(&mut r, &src, GenerationField::Created, date).filter(|v| !v.is_empty()) {
                let utc_offset = e.offset.as_deref().map(str::trim).filter(|o| !o.is_empty()).map(str::to_owned);
                dates.get_or_insert((src, GenerationTime { value, utc_offset }));
            }
        }
    }
    // A software name or capture date alone says nothing about generation.
    if !claims.is_empty() {
        if let Some((src, v)) = software {
            push(&mut claims, &mut r, src, GenerationFields { generator: Some(v), ..Default::default() });
        }
        if let Some((src, t)) = dates {
            push(&mut claims, &mut r, src, GenerationFields { created: Some(t), ..Default::default() });
        }
    }
    finish(claims, r)
}
fn xmp(
    text: &str,
    container: Container,
    r: &mut Reader,
    claims: &mut Vec<GenerationClaim>,
    software: &mut Option<(ClaimSource, String)>,
) {
    if let Some(v) = xmp_property(text, "DigitalSourceType") {
        let term = v.rsplit('/').next().unwrap_or(&v).to_owned();
        let src = claim(GenerationFormat::XmpDigitalSource, container, "xmp");
        if !term.is_empty() && term.len() <= 128 && term.bytes().all(|b| b.is_ascii_alphanumeric()) {
            if claims.len() < MAX_CLAIMS {
                claims.push(GenerationClaim {
                    source: src,
                    fields: GenerationFields { digital_source_type: Some(term), ..Default::default() },
                });
            }
        } else {
            r.issue(Some(container), "xmp", IssueCode::Malformed, Some(GenerationField::DigitalSourceType));
        }
    }
    if let Some(v) = xmp_property(text, "xmp:CreatorTool") {
        let src = claim(GenerationFormat::SoftwareTag, container, "xmp:CreatorTool");
        if let Some(v) = short_field(r, &src, GenerationField::Generator, &v) {
            software.get_or_insert((src, v));
        }
    }
}
fn finish(claims: Vec<GenerationClaim>, r: Reader) -> GenerationExtraction {
    let status = if !claims.is_empty() {
        ExtractionStatus::Found
    } else if r.exhausted {
        ExtractionStatus::LimitExceeded
    } else if r.issues.is_empty() {
        ExtractionStatus::None
    } else {
        ExtractionStatus::Malformed
    };
    let mut out = GenerationExtraction::empty(status);
    out.claims = claims;
    out.issues = r.issues;
    out.uninterpreted = r.uninterpreted;
    // Never store a record the database would refuse: drop to "limit" instead.
    if out.validate().is_err() {
        let mut limited = GenerationExtraction::empty(ExtractionStatus::LimitExceeded);
        limited.issues.push(ExtractionIssue { container: None, key: "record".into(), code: IssueCode::LimitExceeded, field: None });
        return limited;
    }
    out
}

/// The A1111 sidecar layout (`<stem>.txt` beside the image).
pub fn extract_sidecar(name: &str, bytes: &[u8]) -> GenerationExtraction {
    let mut r = Reader::new(PARSE_BUDGET);
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        r.issue(Some(Container::Sidecar), name, IssueCode::LimitExceeded, None);
        return finish(vec![], r);
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t.trim_start_matches('\u{feff}'),
        Err(_) => {
            r.issue(Some(Container::Sidecar), name, IssueCode::UnsupportedEncoding, None);
            return finish(vec![], r);
        }
    };
    // Only the A1111 layout is a claim; an arbitrary text file next to an image is not.
    if !text.lines().any(|l| l.trim_start().starts_with("Steps:")) {
        return finish(vec![], r);
    }
    let src = claim(GenerationFormat::A1111Parameters, Container::Sidecar, name);
    let f = a1111(text, &mut r, &src);
    let claims = if f.is_empty() { vec![] } else { vec![GenerationClaim { source: src, fields: f }] };
    finish(claims, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc = kind.to_vec();
        crc.extend_from_slice(data);
        out.extend_from_slice(&crc32fast::hash(&crc).to_be_bytes());
        out
    }
    fn text_chunk(key: &str, text: &str) -> Vec<u8> {
        let mut d = key.as_bytes().to_vec();
        d.push(0);
        d.extend(text.chars().map(|c| c as u8));
        png_chunk(b"tEXt", &d)
    }
    fn png_with(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend(png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
        for c in chunks {
            out.extend(c);
        }
        out.extend(png_chunk(b"IEND", &[]));
        out
    }
    const PARAMS: &str = "a red fox, \"snow\"\nsecond line\nNegative prompt: blurry\nSteps: 30, Sampler: DPM++ 2M, CFG scale: 7, Seed: 18446744073709551616123, Size: 512x768, Model hash: abc123, Model: dreamshaper_8, Lora hashes: \"fox: 1a2b, snow: 3c4d\", Hires prompt: \"secret upscale\", Version: v1.10.1";

    #[test]
    fn a1111_parameters_map_every_documented_field_losslessly() {
        let e = extract(&png_with(&[text_chunk("parameters", PARAMS)]));
        assert_eq!(e.status, ExtractionStatus::Found);
        let f = &e.claims[0].fields;
        assert_eq!(e.claims[0].source.format, GenerationFormat::A1111Parameters);
        assert_eq!(f.prompt.as_deref(), Some("a red fox, \"snow\"\nsecond line"));
        assert_eq!(f.negative_prompt.as_deref(), Some("blurry"));
        assert_eq!(f.seed.as_deref(), Some("18446744073709551616123"), "larger than u64, kept as text");
        assert_eq!(f.sampler.as_deref(), Some("DPM++ 2M"));
        assert_eq!(f.model.as_deref(), Some("dreamshaper_8"));
        assert_eq!(f.settings["Lora hashes"], "fox: 1a2b, snow: 3c4d");
        assert_eq!(f.settings["CFG scale"], "7");
        assert_eq!(f.references, vec![Reference { kind: ReferenceKind::ModelHash, value: "abc123".into(), private: false }]);
        assert_eq!(f.generator, None, "no software tag: generator stays unknown");
        assert_eq!(f.created, None);
    }
    #[test]
    fn empty_and_absent_values_stay_distinct() {
        let e = extract(&png_with(&[text_chunk("parameters", "cat\nNegative prompt: \nSteps: 4")]));
        assert_eq!(e.claims[0].fields.negative_prompt.as_deref(), Some(""), "recorded as empty");
        let e = extract(&png_with(&[text_chunk("parameters", "cat\nSteps: 4")]));
        assert_eq!(e.claims[0].fields.negative_prompt, None, "never recorded");
        assert_eq!(e.claims[0].fields.seed, None);
        let e = extract(&png_with(&[]));
        assert_eq!(e.status, ExtractionStatus::None);
        assert!(e.claims.is_empty() && e.issues.is_empty());
    }
    #[test]
    fn unicode_prompts_survive_international_and_compressed_chunks() {
        let prompt = "猫と桜 🌸 — café\nSteps: 20, Seed: 7";
        let mut d = b"parameters\0\x01\x00\0\0".to_vec();
        let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::default());
        std::io::Write::write_all(&mut z, prompt.as_bytes()).unwrap();
        d.extend(z.finish().unwrap());
        let e = extract(&png_with(&[png_chunk(b"iTXt", &d)]));
        assert_eq!(e.claims[0].fields.prompt.as_deref(), Some("猫と桜 🌸 — café"));
        assert_eq!(e.claims[0].source.container, Container::PngInternationalText);
    }
    #[test]
    fn compression_bombs_and_oversized_prompts_stop_without_truncation() {
        let mut d = b"parameters\0\0".to_vec();
        let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::best());
        std::io::Write::write_all(&mut z, &vec![b'a'; MAX_INFLATED_BYTES + 10]).unwrap();
        d.extend(z.finish().unwrap());
        assert!(d.len() < 16 * 1024, "small on disk");
        let e = extract(&png_with(&[png_chunk(b"zTXt", &d)]));
        assert_eq!(e.status, ExtractionStatus::LimitExceeded);
        let huge = format!("{}\nSteps: 1, Seed: 3", "x".repeat(MAX_PROMPT_BYTES + 1));
        let e = extract(&png_with(&[text_chunk("parameters", &huge)]));
        let f = &e.claims[0].fields;
        assert_eq!(f.prompt, None, "too long: unknown, not cut");
        assert_eq!(f.seed.as_deref(), Some("3"), "the rest still reads");
        assert!(e.issues.iter().any(|i| i.code == IssueCode::LimitExceeded && i.field == Some(GenerationField::Prompt)));
    }
    #[test]
    fn segment_count_is_bounded() {
        let chunks: Vec<_> = (0..MAX_SEGMENTS + 5).map(|_| text_chunk("Software", "x")).collect();
        let e = extract(&png_with(&chunks));
        assert!(e.issues.iter().any(|i| i.code == IssueCode::LimitExceeded));
    }
    #[test]
    fn malformed_json_and_chunks_are_reported_and_isolated() {
        let mut bad_crc = text_chunk("parameters", "lost\nSteps: 1");
        let n = bad_crc.len();
        bad_crc[n - 1] ^= 1;
        let e = extract(&png_with(&[bad_crc, text_chunk("prompt", "{not json"), text_chunk("invokeai_metadata", "[1,2]")]));
        assert_eq!(e.status, ExtractionStatus::Malformed);
        assert_eq!(e.issues.len(), 3);
        let deep = format!("{}{}", "[".repeat(MAX_JSON_DEPTH + 1), "]".repeat(MAX_JSON_DEPTH + 1));
        let e = extract(&png_with(&[text_chunk("prompt", &deep), text_chunk("parameters", "ok\nSteps: 2")]));
        assert_eq!(e.status, ExtractionStatus::Found, "one bad segment never hides another");
        assert!(e.issues.iter().any(|i| i.code == IssueCode::LimitExceeded));
        // Truncated container: chunk length past the end.
        let mut cut = png_with(&[text_chunk("parameters", "a\nSteps: 1")]);
        cut.truncate(cut.len() - 20);
        assert_eq!(extract(&cut).status, ExtractionStatus::Malformed);
    }
    #[test]
    fn comfyui_graph_is_read_as_data_and_workflows_are_never_run() {
        let graph = serde_json::json!({
            "3": {"class_type": "KSampler", "inputs": {"seed": 156680208700286u64, "steps": 20, "cfg": 8, "sampler_name": "euler", "scheduler": "normal", "denoise": 1, "model": ["10", 0], "positive": ["6", 0], "negative": ["7", 0]}},
            "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "v1-5-pruned.safetensors"}},
            "10": {"class_type": "LoraLoader", "inputs": {"lora_name": "fox.safetensors", "model": ["4", 0]}},
            "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "a fox"}},
            "7": {"class_type": "CLIPTextEncode", "inputs": {"text": "bad hands"}},
            "8": {"class_type": "LoadImage", "inputs": {"image": "C:\\Users\\me\\ref.png"}},
            "9": {"class_type": "SaveImage", "inputs": {"filename_prefix": "https://example.invalid/x?y"}}
        });
        let workflow = r#"{"nodes":[{"type":"ExecutePython","widgets_values":["import os; os.system('rm -rf /')"]}]}"#;
        let e = extract(&png_with(&[text_chunk("prompt", &graph.to_string()), text_chunk("workflow", workflow)]));
        let f = &e.claims[0].fields;
        assert_eq!(f.prompt.as_deref(), Some("a fox"));
        assert_eq!(f.negative_prompt.as_deref(), Some("bad hands"));
        assert_eq!(f.seed.as_deref(), Some("156680208700286"));
        assert_eq!(f.model.as_deref(), Some("v1-5-pruned.safetensors"));
        assert!(f.references.contains(&Reference { kind: ReferenceKind::Lora, value: "fox.safetensors".into(), private: false }));
        assert!(f.references.contains(&Reference { kind: ReferenceKind::Image, value: "C:\\Users\\me\\ref.png".into(), private: true }));
        assert_eq!(e.uninterpreted, vec![Uninterpreted { container: Container::PngText, key: "workflow".into(), bytes: workflow.len() as u64 }]);
        let stored = serde_json::to_string(&e).unwrap();
        assert!(!stored.contains("os.system") && !stored.contains("example.invalid"), "workflow contents are not stored");
    }
    #[test]
    fn ambiguous_graphs_and_inexact_seeds_stay_unknown() {
        let graph = serde_json::json!({
            "1": {"class_type": "KSampler", "inputs": {"seed": 1}},
            "2": {"class_type": "KSampler", "inputs": {"seed": 2}},
            "3": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "m.ckpt"}}
        });
        let e = extract(&png_with(&[text_chunk("prompt", &graph.to_string())]));
        let f = &e.claims[0].fields;
        assert_eq!((f.seed.as_deref(), f.model.as_deref()), (None, Some("m.ckpt")));
        assert!(e.issues.iter().any(|i| i.code == IssueCode::Ambiguous));
        let e = extract(&png_with(&[text_chunk("invokeai_metadata", r#"{"positive_prompt":"x","seed":1.5e30}"#)]));
        assert_eq!(e.claims[0].fields.seed, None);
        assert!(e.issues.iter().any(|i| i.code == IssueCode::InexactNumber));
    }
    #[test]
    fn invokeai_and_novelai_metadata_map_with_their_own_attribution() {
        let invoke = r#"{"positive_prompt":"castle","negative_prompt":"","seed":4294967297,"model":{"name":"sdxl","hash":"h1"},"steps":30,"cfg_scale":6.5,"scheduler":"dpmpp_2m","app_version":"5.4.0"}"#;
        let e = extract(&png_with(&[text_chunk("invokeai_metadata", invoke)]));
        let f = &e.claims[0].fields;
        assert_eq!((f.prompt.as_deref(), f.negative_prompt.as_deref(), f.seed.as_deref()), (Some("castle"), Some(""), Some("4294967297")));
        assert_eq!((f.model.as_deref(), f.generator.as_deref()), (Some("sdxl"), Some("InvokeAI 5.4.0")));
        assert_eq!(f.settings["cfg_scale"], "6.5");
        let e = extract(&png_with(&[
            text_chunk("Software", "NovelAI"),
            text_chunk("Source", "NovelAI Diffusion V4 ABC123"),
            text_chunk("Description", "a lighthouse"),
            text_chunk("Comment", r#"{"prompt":"a lighthouse","uc":"lowres","seed":99,"steps":28,"sampler":"k_euler"}"#),
        ]));
        let c = &e.claims[0];
        assert_eq!(c.source.format, GenerationFormat::NovelaiComment);
        assert_eq!(c.fields.generator.as_deref(), Some("NovelAI"));
        assert_eq!(c.fields.model.as_deref(), Some("NovelAI Diffusion V4 ABC123"));
        assert_eq!(c.fields.negative_prompt.as_deref(), Some("lowres"));
    }
    #[test]
    fn unknown_generators_and_plain_metadata_are_not_claims() {
        // A camera photo: software and date, but no generation format.
        let e = extract(&png_with(&[text_chunk("Software", "Darktable"), text_chunk("Comment", "{\"prompt\":\"x\"}"), text_chunk("Title", "x")]));
        assert_eq!(e.status, ExtractionStatus::None, "{e:?}");
        // Unknown generator keyword in an unrecognized chunk.
        let e = extract(&png_with(&[text_chunk("dream", "a cat -s50 -S42")]));
        assert_eq!(e.status, ExtractionStatus::None);
        // Software is attached once a real claim exists.
        let e = extract(&png_with(&[text_chunk("Software", "Forge"), text_chunk("parameters", "a\nSteps: 1")]));
        assert_eq!(e.claims[1].fields.generator.as_deref(), Some("Forge"));
        assert_eq!(e.claims[1].source.format, GenerationFormat::SoftwareTag);
    }
    fn exif_blob(little: bool, comment: &[u8], software: &str, date: Option<(&str, Option<&str>)>) -> Vec<u8> {
        let w16 = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
        let w32 = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut ifd0 = vec![];
        let mut data = vec![];
        let sw: Vec<u8> = [software.as_bytes(), b"\0"].concat();
        let mut exif_entries: Vec<(u16, u16, Vec<u8>)> = vec![(0x9286, 7, comment.to_vec())];
        if let Some((d, o)) = date {
            exif_entries.push((0x9003, 2, [d.as_bytes(), b"\0"].concat()));
            if let Some(o) = o {
                exif_entries.push((0x9011, 2, [o.as_bytes(), b"\0"].concat()));
            }
        }
        let ifd0_len = 2 + 2 * 12 + 4;
        let exif_at = 8 + ifd0_len;
        let exif_len = 2 + exif_entries.len() * 12 + 4;
        let mut data_at = exif_at + exif_len;
        let mut place = |bytes: &[u8], data: &mut Vec<u8>| -> u32 {
            let at = data_at as u32;
            data.extend_from_slice(bytes);
            data_at += bytes.len();
            at
        };
        ifd0.extend(w16(2));
        ifd0.extend(w16(0x0131));
        ifd0.extend(w16(2));
        ifd0.extend(w32(sw.len() as u32));
        ifd0.extend(w32(place(&sw, &mut data)));
        ifd0.extend(w16(0x8769));
        ifd0.extend(w16(4));
        ifd0.extend(w32(1));
        ifd0.extend(w32(exif_at as u32));
        ifd0.extend(w32(0));
        let mut sub = w16(exif_entries.len() as u16).to_vec();
        for (tag, kind, bytes) in &exif_entries {
            sub.extend(w16(*tag));
            sub.extend(w16(*kind));
            sub.extend(w32(bytes.len() as u32));
            sub.extend(w32(place(bytes, &mut data)));
        }
        sub.extend(w32(0));
        let mut out = if little { b"II*\0".to_vec() } else { b"MM\0*".to_vec() };
        out.extend(w32(8));
        out.extend(ifd0);
        out.extend(sub);
        out.extend(data);
        out
    }
    fn utf16(prefix_be: bool, s: &str) -> Vec<u8> {
        let mut out = b"UNICODE\0".to_vec();
        for u in s.encode_utf16() {
            out.extend(if prefix_be { u.to_be_bytes() } else { u.to_le_bytes() });
        }
        out
    }
    #[test]
    fn jpeg_and_webp_exif_user_comments_carry_a1111_parameters() {
        let comment = utf16(true, "a 🦊 fox\nSteps: 20, Seed: 42");
        let blob = exif_blob(false, &comment, "Stable Diffusion", Some(("2026:10:20 09:15:00", None)));
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpg.extend(((blob.len() + 8) as u16).to_be_bytes());
        jpg.extend(b"Exif\0\0");
        jpg.extend(&blob);
        jpg.extend([0xff, 0xda, 0, 2, 0xff, 0xd9]);
        let e = extract(&jpg);
        assert_eq!(e.claims[0].fields.prompt.as_deref(), Some("a 🦊 fox"));
        assert_eq!(e.claims[0].source.key, "UserComment");
        let date = e.claims.iter().find_map(|c| c.fields.created.clone()).unwrap();
        assert_eq!(date, GenerationTime { value: "2026:10:20 09:15:00".into(), utc_offset: None }, "time zone unknown");
        let blob = exif_blob(true, &utf16(false, "b\nSteps: 1"), "Tool9", Some(("2026:01:01 00:00:00", Some("+05:30"))));
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend(b"EXIF");
        webp.extend((blob.len() as u32).to_le_bytes());
        webp.extend(&blob);
        if blob.len() % 2 == 1 {
            webp.push(0);
        }
        let e = extract(&webp);
        assert_eq!(e.claims[0].source.container, Container::WebpExif);
        assert_eq!(e.claims[0].fields.prompt.as_deref(), Some("b"));
        let date = e.claims.iter().find_map(|c| c.fields.created.clone()).unwrap();
        assert_eq!(date.utc_offset.as_deref(), Some("+05:30"));
        // A plain photo comment is not a generation claim.
        let blob = exif_blob(true, b"ASCII\0\0\0Holiday", "Camera", Some(("2026:01:01 00:00:00", None)));
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend(b"EXIF");
        webp.extend((blob.len() as u32).to_le_bytes());
        webp.extend(&blob);
        assert_eq!(extract(&webp).status, ExtractionStatus::None);
    }
    #[test]
    fn xmp_digital_source_type_is_an_embedded_claim_without_xml_expansion() {
        let xmp = r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "aaaaaaaa">]><x:xmpmeta><rdf:Description Iptc4xmpExt:DigitalSourceType="http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia" xmp:CreatorTool="Firefly"/></x:xmpmeta>"#;
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xe1];
        let payload = [b"http://ns.adobe.com/xap/1.0/\0".as_slice(), xmp.as_bytes()].concat();
        jpg.extend(((payload.len() + 2) as u16).to_be_bytes());
        jpg.extend(&payload);
        jpg.extend([0xff, 0xd9]);
        let e = extract(&jpg);
        assert_eq!(e.claims[0].fields.digital_source_type.as_deref(), Some("trainedAlgorithmicMedia"));
        assert_eq!(e.claims[1].fields.generator.as_deref(), Some("Firefly"));
    }
    #[test]
    fn non_images_and_pixels_never_produce_claims() {
        assert_eq!(extract(b"Steps: 20, Seed: 1").status, ExtractionStatus::None);
        assert_eq!(extract(&[]).status, ExtractionStatus::None);
        // Image data that merely contains the text is not metadata.
        let mut idat = png_chunk(b"IDAT", b"parameters\0a\nSteps: 1");
        idat.extend(png_chunk(b"tEXt", b"Title\0Steps: 1"));
        assert_eq!(extract(&png_with(&[idat])).status, ExtractionStatus::None);
    }
    #[test]
    fn redaction_removes_prompts_and_private_references_and_records_it() {
        let graph = serde_json::json!({
            "3": {"class_type": "KSampler", "inputs": {"seed": 5, "model": ["4", 0], "positive": ["6", 0], "negative": ["6", 0]}},
            "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "/home/me/models/private.ckpt"}},
            "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "my secret client brief"}},
            "8": {"class_type": "LoadImage", "inputs": {"image": "ref.png"}}
        });
        let e = extract(&png_with(&[text_chunk("prompt", &graph.to_string()), text_chunk("parameters", PARAMS)]));
        let shared = e.redacted_for(GenerationSharing::default());
        let text = serde_json::to_string(&shared).unwrap();
        for secret in ["my secret client brief", "a red fox", "blurry", "secret upscale", "/home/me", "ref.png"] {
            assert!(!text.contains(secret), "{secret} leaked");
        }
        assert_eq!(shared.claims[0].fields.seed.as_deref(), Some("5"), "settings survive");
        assert_eq!(
            shared.redacted,
            vec![GenerationField::Prompt, GenerationField::NegativePrompt, GenerationField::Model, GenerationField::Settings, GenerationField::References]
        );
        let full = e.redacted_for(GenerationSharing { prompts: true, private_references: true });
        assert_eq!(full, e, "opting in keeps everything");
        shared.validate().unwrap();
    }
    #[test]
    fn sidecars_need_the_a1111_layout_and_a_size_limit() {
        let e = extract_sidecar("fox.txt", b"\xef\xbb\xbfa fox\nSteps: 10, Seed: 9");
        assert_eq!(e.claims[0].source.container, Container::Sidecar);
        assert_eq!(e.claims[0].fields.prompt.as_deref(), Some("a fox"));
        assert_eq!(extract_sidecar("notes.txt", b"shopping list").status, ExtractionStatus::None);
        let big = vec![b'a'; MAX_SIDECAR_BYTES as usize + 1];
        assert_eq!(extract_sidecar("big.txt", &big).status, ExtractionStatus::LimitExceeded);
        assert_eq!(extract_sidecar("bad.txt", b"\xff\xfe\nSteps: 1").status, ExtractionStatus::Malformed);
    }
    #[test]
    fn parser_time_budget_stops_work() {
        let chunks: Vec<_> = (0..10).map(|_| text_chunk("parameters", "a\nSteps: 1")).collect();
        let e = extract_within(&png_with(&chunks), Duration::ZERO);
        assert!(e.issues.iter().any(|i| i.code == IssueCode::LimitExceeded));
    }
}
