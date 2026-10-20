//! SS-080 image tools for MCP clients: find, inspect, preview and tag images in
//! the client's sources. Scope is applied before any match, count or error, a
//! denied ID fails like an unknown one, pixels need their own permission, and
//! creator notes are never returned or matched.
use crate::{
    agent::{args, Access, AgentLibrary, Failure},
    catalog::Catalog,
    image_annotations::{AnnotationError, AnnotationPatch},
    image_contracts::ImageAsset,
    image_gallery::ImageFilter,
    image_preview::{PreviewError, PreviewRequest, PreviewTier},
};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

type Result<T> = std::result::Result<T, Failure>;

/// Image wording for the shared agent failure codes.
fn failure(code: &'static str) -> Failure {
    Failure {
        code,
        message: match code {
            "PERMISSION_DENIED" => "Client permission does not allow this operation",
            "ASSET_CHANGED" => "Image version or annotation revision changed; read it again",
            "ASSET_MISSING" => "Image original is missing and no preview is cached",
            "SOURCE_OFFLINE" => "Source is offline",
            "NOT_READY" => "Image is not ready",
            "RATE_LIMITED" => "Preview queue is busy; retry",
            "INVALID_ARGUMENT" => "Arguments are invalid",
            _ => "Operation failed; check the desktop application",
        },
    }
}

const MAX_TAG_CHANGES: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    source_ids: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    untagged_only: bool,
    #[serde(default)]
    favorites_only: bool,
    #[serde(default)]
    offset: u32,
    #[serde(default = "page_limit")]
    limit: u32,
}
fn page_limit() -> u32 {
    50
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Get {
    id: String,
    version: String,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Size {
    Small,
    Medium,
    Large,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preview {
    id: String,
    version: String,
    #[serde(default)]
    size: Option<Size>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Annotate {
    id: String,
    version: String,
    revision: u32,
    #[serde(default)]
    add_tags: Vec<String>,
    #[serde(default)]
    remove_tags: Vec<String>,
}

/// Metadata a client may see. No notes, absolute paths or evidence internals.
fn view(c: &Catalog, a: &ImageAsset) -> Value {
    let source_available = c.source_header(&a.header.source_id).map(|s| s.available).unwrap_or(false);
    json!({
        "id": a.header.id,
        "version": a.header.content_hash,
        "source_id": a.header.source_id,
        "relative_path": a.header.relative_path,
        "technical_state": a.header.technical_state,
        "availability": a.header.availability,
        "source_available": source_available,
        "width": a.profile.as_ref().map(|p| p.width),
        "height": a.profile.as_ref().map(|p| p.height),
        "format": a.profile.as_ref().map(|p| p.format),
        "transparency": a.profile.as_ref().map(|p| p.transparency),
        "tags": a.annotations.tags,
        "caption": a.annotations.caption,
        "favorite": a.annotations.favorite,
        "rating": a.annotations.rating,
        "revision": a.annotations.revision,
    })
}

fn scoped_image(c: &Catalog, access: &Access, id: &str, version: Option<&str>) -> Result<ImageAsset> {
    // Unknown, out-of-scope and file-scope non-members share one failure.
    let asset = c.image_in_sources(&access.source_ids, id, None).map_err(|_| failure("PERMISSION_DENIED"))?;
    if version.is_some_and(|v| asset.header.content_hash.as_deref() != Some(v)) {
        return Err(failure("ASSET_CHANGED"));
    }
    Ok(asset)
}

fn preview_failure(e: PreviewError) -> Failure {
    failure(match e {
        PreviewError::StaleVersion | PreviewError::SourceChanged => "ASSET_CHANGED",
        PreviewError::NotReady | PreviewError::WorkerUnavailable => "NOT_READY",
        PreviewError::OriginalUnavailable => "ASSET_MISSING",
        PreviewError::Busy | PreviewError::TimedOut => "RATE_LIMITED",
        PreviewError::NotFound | PreviewError::PermissionDenied => "PERMISSION_DENIED",
        PreviewError::InvalidArgument => "INVALID_ARGUMENT",
        _ => "INTERNAL_ERROR",
    })
}

fn annotation_failure(e: AnnotationError) -> Failure {
    failure(match e {
        AnnotationError::Conflict => "ASSET_CHANGED",
        AnnotationError::SourceUnavailable => "SOURCE_OFFLINE",
        AnnotationError::NotFound | AnnotationError::PermissionDenied => "PERMISSION_DENIED",
        AnnotationError::InvalidArgument | AnnotationError::LimitExceeded => "INVALID_ARGUMENT",
        _ => "INTERNAL_ERROR",
    })
}

static PREVIEW_REQUESTS: AtomicU64 = AtomicU64::new(0);

impl AgentLibrary {
    pub(crate) fn images(&self, access: &Access, method: &str, value: Value) -> Result<Value> {
        match method {
            "images.search" => {
                let a: Search = args(value)?;
                if a.limit == 0 || a.limit > 100 || a.offset > 100_000 || a.tags.len() > 64 || a.text.as_ref().is_some_and(|t| t.len() > 512) {
                    return Err(failure("INVALID_ARGUMENT"));
                }
                if a.source_ids.iter().any(|id| !access.source_ids.contains(id)) {
                    return Err(failure("PERMISSION_DENIED"));
                }
                let scope = if a.source_ids.is_empty() { access.source_ids.clone() } else { a.source_ids };
                let filter = ImageFilter { text: a.text, tags: a.tags, untagged_only: a.untagged_only, favorites_only: a.favorites_only };
                let c = self.catalog()?;
                let page = c.filtered_image_ids(&scope, &filter, a.offset, a.limit).map_err(|_| failure("INVALID_ARGUMENT"))?;
                let mut items = vec![];
                for id in &page.ids {
                    // Rows can vanish between the two reads; skip rather than fail the page.
                    if let Ok(asset) = c.image_in_sources(&scope, id, None) {
                        items.push(view(&c, &asset));
                    }
                }
                let next = (a.offset + (page.ids.len() as u32) < page.total).then_some(a.offset + page.ids.len() as u32);
                Ok(json!({"items": items, "total": page.total, "next_offset": next}))
            }
            "images.get" => {
                let a: Get = args(value)?;
                let c = self.catalog()?;
                let asset = scoped_image(&c, access, &a.id, Some(&a.version))?;
                Ok(json!({"image": view(&c, &asset)}))
            }
            "images.preview" => {
                let a: Preview = args(value)?;
                {
                    let c = self.catalog()?;
                    scoped_image(&c, access, &a.id, Some(&a.version))?;
                } // The preview service takes the catalog lock itself.
                let service = self.image_previews.get().ok_or_else(|| failure("NOT_READY"))?;
                let tier = match a.size.unwrap_or(Size::Large) {
                    Size::Small => PreviewTier::Small,
                    Size::Medium => PreviewTier::Medium,
                    Size::Large => PreviewTier::Large,
                };
                let request_id = format!("mcp-{}", PREVIEW_REQUESTS.fetch_add(1, Ordering::Relaxed));
                let preview = service
                    .preview(&PreviewRequest { request_id, asset_id: a.id, version: a.version, tier, tile: None })
                    .map_err(preview_failure)?;
                let m = &preview.meta;
                Ok(json!({
                    "image": {"mime_type": m.mime, "data": base64::engine::general_purpose::STANDARD.encode(preview.bytes.as_slice())},
                    "meta": {"id": m.asset_id, "version": m.version, "width": m.width, "height": m.height,
                        "source_width": m.source_width, "source_height": m.source_height, "transparency": m.transparency,
                        "original_available": m.original_available, "offline_reference": m.offline_reference}
                }))
            }
            "images.annotate" => {
                let a: Annotate = args(value)?;
                if a.add_tags.is_empty() && a.remove_tags.is_empty() || a.add_tags.len() > MAX_TAG_CHANGES || a.remove_tags.len() > MAX_TAG_CHANGES {
                    return Err(failure("INVALID_ARGUMENT"));
                }
                let mut c = self.catalog()?;
                scoped_image(&c, access, &a.id, Some(&a.version))?;
                let patch = AnnotationPatch { add_tags: a.add_tags, remove_tags: a.remove_tags, ..Default::default() };
                let result = c.edit_image_annotations_as(&a.id, a.revision, &patch, Some("MCP client")).map_err(annotation_failure)?;
                Ok(json!({"id": a.id, "revision": result.annotations.revision, "tags": result.annotations.tags, "undo_action_id": result.action_id}))
            }
            _ => Err(failure("INVALID_ARGUMENT")),
        }
    }
}
