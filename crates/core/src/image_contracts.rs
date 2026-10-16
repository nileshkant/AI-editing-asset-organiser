//! SS-040 data contracts only. No image commands, storage, decoding or capabilities.
//! See docs/IMAGE_CONTRACTS.md. Validation is structural, never a filesystem grant.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const CONTRACT_SCHEMA: &str = "creativeshelf-image-contract/v1";
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_PATH_BYTES: usize = 32_768;
pub const MAX_TEXT_BYTES: usize = 40_000;
pub const MAX_TAG_BYTES: usize = 256;
pub const MAX_TAGS: usize = 64;
pub const MAX_RECORDS: usize = 1_000;
pub const MAX_EVIDENCE: usize = 128;
pub const MAX_RECIPES: usize = 128;
pub const MAX_DIMENSION: u32 = 32_768;
pub const MAX_PIXELS: u64 = 100_000_000;
pub const MAX_ENCODED_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_BUNDLE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PREVIEW_DIMENSION: u32 = 2_048;
pub const MAX_PREVIEW_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_PAGE_SIZE: u32 = 100;

// Missing and explicit null are different: all nullable wire fields are required.
fn nullable<'de, D, T>(d: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(d)
}
macro_rules! wire_enum {
    ($name:ident { $($v:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($v),+ }
    };
}
wire_enum!(AssetKind { Image, Element });
wire_enum!(TechnicalState {
    Pending,
    Ready,
    Failed
});
wire_enum!(Availability {
    Available,
    Offline,
    Missing,
    Inaccessible
});
wire_enum!(EnrichmentState {
    Disabled,
    Queued,
    Running,
    Complete,
    Failed,
    Cancelled,
    Stale
});
wire_enum!(Scope { Folder, Files });
wire_enum!(ImageFormat { Jpeg, Png, Webp });
wire_enum!(Transparency {
    None,
    Binary,
    Partial
});
wire_enum!(Origin {
    Model,
    Filename,
    Imported,
    User
});
wire_enum!(ReviewState {
    Suggested,
    Accepted,
    Edited,
    Rejected
});
wire_enum!(ConfidenceKind {
    Unavailable,
    Reported,
    Calibrated
});
wire_enum!(Coverage {
    FullImage,
    Preview,
    Element
});
wire_enum!(ColorPolicy { Srgb });

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetHeader {
    pub id: String,
    pub kind: AssetKind,
    pub source_id: String,
    pub relative_path: String,
    #[serde(deserialize_with = "nullable")]
    pub content_hash: Option<String>,
    pub technical_state: TechnicalState,
    pub availability: Availability,
    pub enrichment_state: EnrichmentState,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageProfile {
    pub raw_width: u32,
    pub raw_height: u32,
    pub orientation: u8,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub encoded_bytes: u64,
    pub bit_depth: u8,
    #[serde(deserialize_with = "nullable")]
    pub source_color_profile: Option<String>,
    pub alpha_channel: bool,
    pub transparency: Transparency,
    pub analyzer_version: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotations {
    pub revision: u32,
    pub tags: Vec<String>,
    pub comment: String,
    pub favorite: bool,
    #[serde(deserialize_with = "nullable")]
    pub rating: Option<u8>,
    #[serde(deserialize_with = "nullable")]
    pub caption: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
    pub revision: String,
    pub preprocessing: String,
    pub output_schema: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: String,
    pub content_hash: String,
    pub origin: Origin,
    pub coverage: Coverage,
    pub tags: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    pub caption: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub model: Option<ModelRef>,
    #[serde(deserialize_with = "nullable")]
    pub confidence: Option<f64>,
    pub confidence_kind: ConfidenceKind,
    pub review_state: ReviewState,
    pub stale: bool,
    pub created_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelRect {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRecipe {
    pub id: String,
    pub asset_id: String,
    pub asset_version: String,
    pub revision: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub crop: PixelRect,
    pub output_width: u32,
    pub output_height: u32,
    pub allow_upscale: bool,
    pub color_policy: ColorPolicy,
    pub stale: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivedLineage {
    pub parent_id: String,
    pub parent_version: String,
    pub recipe_id: String,
    pub recipe_revision: u32,
    pub mask_hash: String,
    pub mask_revision: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageAsset {
    pub header: AssetHeader,
    #[serde(deserialize_with = "nullable")]
    pub profile: Option<ImageProfile>,
    pub annotations: Annotations,
    pub evidence: Vec<Evidence>,
    pub recipes: Vec<ImageRecipe>,
    #[serde(deserialize_with = "nullable")]
    pub lineage: Option<DerivedLineage>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractSource {
    pub id: String,
    pub scope: Scope,
    pub files: Vec<String>,
}
/// Conformance bundle only: not a new portable-catalog or source-sidecar format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractBundle {
    pub schema: String,
    pub sources: Vec<ContractSource>,
    pub audio_ids: Vec<String>,
    pub images: Vec<ImageAsset>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ImageEditArgs {
    pub asset_id: String,
    pub expected_content_hash: String,
    pub expected_revision: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContractError {
    InvalidArgument,
    UnsupportedSchema,
    LimitExceeded,
    InvalidPath,
    WrongAssetKind,
    IdentityConflict,
    InvalidReference,
    StaleVersion,
    InvalidRange,
}
type Result<T> = std::result::Result<T, ContractError>;
fn require(ok: bool, err: ContractError) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(err)
    }
}
fn text(s: &str, max: usize, empty: bool) -> Result<()> {
    require(
        (empty || !s.is_empty())
            && s.len() <= max
            && !s.chars().any(|c| c <= '\u{001f}' || c == '\u{007f}'),
        ContractError::InvalidArgument,
    )
}
fn id(s: &str) -> Result<()> {
    text(s, 256, false)
}
fn uuid(s: &str) -> Result<()> {
    require(
        uuid::Uuid::parse_str(s).is_ok_and(|v| !v.is_nil() && v.hyphenated().to_string() == s),
        ContractError::InvalidArgument,
    )
}
fn digest(s: &str) -> Result<()> {
    require(
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        ContractError::InvalidArgument,
    )
}
pub fn validate_relative(s: &str) -> Result<()> {
    require(
        !s.is_empty()
            && s.len() <= MAX_PATH_BYTES
            && !s.contains(['\\', ':'])
            && !s.chars().any(|c| c <= '\u{001f}' || c == '\u{007f}')
            && s.split('/').all(|p| !p.is_empty() && p != "." && p != ".."),
        ContractError::InvalidPath,
    )
}
fn dimensions(w: u32, h: u32) -> Result<()> {
    let pixels = u64::from(w)
        .checked_mul(u64::from(h))
        .ok_or(ContractError::LimitExceeded)?;
    require(
        w > 0 && h > 0 && w <= MAX_DIMENSION && h <= MAX_DIMENSION && pixels <= MAX_PIXELS,
        ContractError::LimitExceeded,
    )
}
fn tags(values: &[String]) -> Result<()> {
    require(values.len() <= MAX_TAGS, ContractError::LimitExceeded)?;
    let mut seen = HashSet::new();
    for v in values {
        text(v, MAX_TAG_BYTES, false)?;
        require(
            v.trim() == v && seen.insert(v),
            ContractError::InvalidArgument,
        )?;
    }
    Ok(())
}
fn caption(value: &Option<String>) -> Result<()> {
    if let Some(v) = value {
        require(
            v.len() <= MAX_TEXT_BYTES && !v.contains('\0'),
            ContractError::LimitExceeded,
        )?;
    }
    Ok(())
}
impl ImageEditArgs {
    pub fn validate(&self) -> Result<()> {
        uuid(&self.asset_id)?;
        digest(&self.expected_content_hash)?;
        require(self.expected_revision > 0, ContractError::InvalidArgument)
    }
}
impl ImageAsset {
    pub fn validate(&self) -> Result<()> {
        let h = &self.header;
        uuid(&h.id)?;
        id(&h.source_id)?;
        validate_relative(&h.relative_path)?;
        if let Some(hash) = &h.content_hash {
            digest(hash)?;
        }
        require(
            h.technical_state != TechnicalState::Ready
                || (h.content_hash.is_some() && self.profile.is_some()),
            ContractError::InvalidArgument,
        )?;
        if let Some(p) = &self.profile {
            require(h.content_hash.is_some(), ContractError::InvalidArgument)?;
            dimensions(p.raw_width, p.raw_height)?;
            dimensions(p.width, p.height)?;
            require(
                (1..=8).contains(&p.orientation),
                ContractError::InvalidArgument,
            )?;
            let expected = if p.orientation >= 5 {
                (p.raw_height, p.raw_width)
            } else {
                (p.raw_width, p.raw_height)
            };
            require((p.width, p.height) == expected, ContractError::InvalidRange)?;
            require(
                p.encoded_bytes > 0 && p.encoded_bytes <= MAX_ENCODED_BYTES,
                ContractError::LimitExceeded,
            )?;
            require(
                [1, 2, 4, 8, 10, 12, 16].contains(&p.bit_depth),
                ContractError::InvalidArgument,
            )?;
            if let Some(c) = &p.source_color_profile {
                text(c, 256, false)?;
            }
            text(&p.analyzer_version, 256, false)?;
            require(
                p.alpha_channel || p.transparency == Transparency::None,
                ContractError::InvalidArgument,
            )?;
            require(
                p.format != ImageFormat::Jpeg
                    || (!p.alpha_channel && p.transparency == Transparency::None),
                ContractError::InvalidArgument,
            )?;
        }
        let a = &self.annotations;
        require(a.revision > 0, ContractError::InvalidArgument)?;
        tags(&a.tags)?;
        caption(&a.caption)?;
        require(
            a.comment.len() <= MAX_TEXT_BYTES && !a.comment.contains('\0'),
            ContractError::LimitExceeded,
        )?;
        require(
            a.rating.is_none_or(|r| (1..=5).contains(&r)),
            ContractError::InvalidArgument,
        )?;
        require(
            self.evidence.len() <= MAX_EVIDENCE && self.recipes.len() <= MAX_RECIPES,
            ContractError::LimitExceeded,
        )?;
        let mut evidence_ids = HashSet::new();
        for e in &self.evidence {
            uuid(&e.id)?;
            digest(&e.content_hash)?;
            tags(&e.tags)?;
            caption(&e.caption)?;
            require(evidence_ids.insert(&e.id), ContractError::IdentityConflict)?;
            require(
                e.stale || h.content_hash.as_ref() == Some(&e.content_hash),
                ContractError::StaleVersion,
            )?;
            require(
                e.created_at_ms <= MAX_SAFE_INTEGER,
                ContractError::LimitExceeded,
            )?;
            require(
                (e.origin == Origin::Model) == e.model.is_some(),
                ContractError::InvalidArgument,
            )?;
            if let Some(m) = &e.model {
                for v in [
                    &m.provider,
                    &m.model,
                    &m.revision,
                    &m.preprocessing,
                    &m.output_schema,
                ] {
                    text(v, 256, false)?;
                }
            }
            require(
                (e.confidence_kind == ConfidenceKind::Unavailable) == e.confidence.is_none(),
                ContractError::InvalidArgument,
            )?;
            if let Some(c) = e.confidence {
                require(
                    c.is_finite() && (0.0..=1.0).contains(&c) && e.origin == Origin::Model,
                    ContractError::InvalidArgument,
                )?;
            }
        }
        let mut recipe_ids = HashSet::new();
        if h.enrichment_state == EnrichmentState::Complete {
            require(
                h.technical_state == TechnicalState::Ready
                    && self.evidence.iter().any(|e| {
                        e.origin == Origin::Model
                            && !e.stale
                            && h.content_hash.as_ref() == Some(&e.content_hash)
                    }),
                ContractError::InvalidArgument,
            )?;
        }
        for r in &self.recipes {
            uuid(&r.id)?;
            uuid(&r.asset_id)?;
            digest(&r.asset_version)?;
            require(recipe_ids.insert(&r.id), ContractError::IdentityConflict)?;
            require(r.asset_id == h.id, ContractError::InvalidReference)?;
            require(r.revision > 0, ContractError::InvalidArgument)?;
            require(
                r.stale || h.content_hash.as_ref() == Some(&r.asset_version),
                ContractError::StaleVersion,
            )?;
            dimensions(r.source_width, r.source_height)?;
            dimensions(r.output_width, r.output_height)?;
            if !r.stale {
                let p = self
                    .profile
                    .as_ref()
                    .ok_or(ContractError::InvalidReference)?;
                require(
                    (r.source_width, r.source_height) == (p.width, p.height),
                    ContractError::InvalidRange,
                )?;
            }
            let c = &r.crop;
            require(
                c.left < c.right
                    && c.top < c.bottom
                    && c.right <= r.source_width
                    && c.bottom <= r.source_height,
                ContractError::InvalidRange,
            )?;
            require(
                r.allow_upscale
                    || (r.output_width <= c.right - c.left && r.output_height <= c.bottom - c.top),
                ContractError::InvalidRange,
            )?;
        }
        require(
            (h.kind == AssetKind::Element) == self.lineage.is_some(),
            ContractError::WrongAssetKind,
        )?;
        if let Some(l) = &self.lineage {
            uuid(&l.parent_id)?;
            digest(&l.parent_version)?;
            uuid(&l.recipe_id)?;
            digest(&l.mask_hash)?;
            require(l.parent_id != h.id, ContractError::InvalidReference)?;
            require(
                l.recipe_revision > 0 && l.mask_revision > 0,
                ContractError::InvalidArgument,
            )?;
        }
        Ok(())
    }
}
impl ContractBundle {
    // JavaScript represents 6, 6.0 and 6e0 as the same integral number.
    // Normalize integral safe JSON floats before typed integer deserialization.
    fn normalize_numbers(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Number(n) if n.is_f64() => {
                if let Some(v) = n.as_f64() {
                    if v.is_finite() && v.fract() == 0.0 && v >= 0.0 && v <= MAX_SAFE_INTEGER as f64
                    {
                        *value = serde_json::Value::from(v as u64);
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(Self::normalize_numbers),
            serde_json::Value::Object(fields) => {
                fields.values_mut().for_each(Self::normalize_numbers)
            }
            _ => {}
        }
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() <= MAX_BUNDLE_BYTES,
            ContractError::LimitExceeded,
        )?;
        let mut value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ContractError::InvalidArgument)?;
        Self::normalize_numbers(&mut value);
        require(
            value.get("schema").and_then(|s| s.as_str()) == Some(CONTRACT_SCHEMA),
            ContractError::UnsupportedSchema,
        )?;
        let bundle: Self =
            serde_json::from_value(value).map_err(|_| ContractError::InvalidArgument)?;
        bundle.validate()?;
        Ok(bundle)
    }
    pub fn validate(&self) -> Result<()> {
        require(
            self.schema == CONTRACT_SCHEMA,
            ContractError::UnsupportedSchema,
        )?;
        require(
            self.sources.len() <= MAX_RECORDS
                && self.audio_ids.len() <= MAX_RECORDS
                && self.images.len() <= MAX_RECORDS,
            ContractError::LimitExceeded,
        )?;
        let mut sources = HashMap::new();
        for s in &self.sources {
            id(&s.id)?;
            require(
                sources.insert(&s.id, s).is_none(),
                ContractError::IdentityConflict,
            )?;
            require(s.files.len() <= MAX_RECORDS, ContractError::LimitExceeded)?;
            require(
                s.scope != Scope::Folder || s.files.is_empty(),
                ContractError::InvalidArgument,
            )?;
            let mut paths = HashSet::new();
            for p in &s.files {
                validate_relative(p)?;
                require(paths.insert(p), ContractError::IdentityConflict)?;
            }
        }
        let mut ids = HashSet::new();
        for audio in &self.audio_ids {
            id(audio)?;
            require(ids.insert(audio), ContractError::IdentityConflict)?;
        }
        let mut paths = HashSet::new();
        let images: HashMap<_, _> = self.images.iter().map(|i| (&i.header.id, i)).collect();
        for asset in &self.images {
            asset.validate()?;
            let h = &asset.header;
            require(ids.insert(&h.id), ContractError::IdentityConflict)?;
            require(
                paths.insert((&h.source_id, &h.relative_path)),
                ContractError::IdentityConflict,
            )?;
            let source = sources
                .get(&h.source_id)
                .ok_or(ContractError::InvalidReference)?;
            require(
                source.scope != Scope::Files || source.files.contains(&h.relative_path),
                ContractError::InvalidReference,
            )?;
            let mut ancestors = HashSet::from([&h.id]);
            let mut current = asset;
            while let Some(l) = &current.lineage {
                require(
                    !self.audio_ids.contains(&l.parent_id),
                    ContractError::WrongAssetKind,
                )?;
                require(
                    ancestors.insert(&l.parent_id),
                    ContractError::InvalidReference,
                )?;
                match images.get(&l.parent_id) {
                    Some(parent) => current = parent,
                    None => break,
                }
            }
        }
        Ok(())
    }
}
