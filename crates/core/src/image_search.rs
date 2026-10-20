//! SS-058 exact, model-free image search. A query is a versioned, canonical
//! filter (`ImageQuery`) that later smart collections and agents reuse.
//!
//! Hard predicates (scope, kind, dimensions, states, exclusions) are SQL
//! conditions, never ranking hints: an unknown value (no analysis, no rating,
//! no import date) never satisfies a bound. Text matches whole tokens in the
//! requested fields through an FTS5 index; ranking is an explicit tier, then
//! the gallery's name order, then asset ID. Authorization (source scope and
//! selected-file membership) is applied before totals, facets, per-term counts
//! and suggestions, so restricted sources never influence what a caller sees.
//!
//! The index (`image_search_*`, schema v16) is derived. Writers queue an
//! identity in `image_search_dirty` inside their own transaction; `flush`
//! rebuilds those documents before every read, so an edit is visible to the
//! very next search without full scans, decodes or hashes.
use crate::{
    catalog::Catalog,
    image_annotations::{display_tag, tag_key},
    image_catalog::{scoped, summary, ImageSummary, SUMMARY_COLUMNS},
    image_contracts::{
        Annotations, AssetKind, Availability, EnrichmentState, Evidence, ImageFormat, ImageProfile,
        Origin, ReviewState, TechnicalState, Transparency, MAX_BUNDLE_BYTES, MAX_DIMENSION, MAX_PAGE_SIZE,
    },
    image_gallery::{GallerySort, MAX_GALLERY_OFFSET},
};
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

/// Current canonical query version. Saved queries keep the version they were written with.
pub const QUERY_VERSION: u32 = 1;
pub const MAX_TEXT_CHARS: usize = 512;
/// Terms, phrases and exclusions in one query.
pub const MAX_CLAUSES: usize = 32;
pub const MAX_FILTER_TAGS: usize = 32;
/// Entries in any enum filter list (they are small sets; this bounds hostile input).
pub const MAX_LIST: usize = 16;
pub const MAX_FACET_TAGS: usize = 50;
pub const MAX_SAVED_SEARCHES: i64 = 256;
pub const MAX_SAVED_QUERY_BYTES: usize = 16_384;
/// Dirty documents rebuilt per statement batch inside one savepoint.
const FLUSH_CHUNK: usize = 512;
/// Spelling suggestions per unmatched term.
const MAX_SUGGESTIONS: usize = 3;
/// Private-use separator: a token no query can contain, so phrases never span tags.
const TAG_GAP: &str = " \u{E000} ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SearchError {
    #[error("Invalid image search")]
    InvalidArgument,
    #[error("This search field is not available yet")]
    UnsupportedField,
    #[error("Saved search uses an unsupported query version")]
    UnsupportedVersion,
    #[error("Image search exceeds a limit")]
    LimitExceeded,
    #[error("Saved search not found")]
    NotFound,
    #[error("A saved search with this name already exists")]
    Conflict,
    #[error("Image search failed")]
    Internal,
}
impl From<rusqlite::Error> for SearchError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Internal
    }
}
impl From<serde_json::Error> for SearchError {
    fn from(_: serde_json::Error) -> Self {
        Self::Internal
    }
}
impl From<crate::Error> for SearchError {
    fn from(e: crate::Error) -> Self {
        match e {
            crate::Error::Invalid(_) => Self::InvalidArgument,
            _ => Self::Internal,
        }
    }
}
type Result<T> = std::result::Result<T, SearchError>;
use SearchError as E;

/// Searchable text. OCR, generation metadata and usage notes are reserved for
/// SS-057, SS-055 and SS-056; requesting them fails instead of matching nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextField {
    Name,
    Tags,
    SuggestedTags,
    Caption,
    Notes,
    Ocr,
    Generation,
    UsageNotes,
}
impl TextField {
    fn column(self) -> Option<&'static str> {
        Some(match self {
            Self::Name => "name",
            Self::Tags => "tags",
            Self::SuggestedTags => "suggested",
            Self::Caption => "caption",
            Self::Notes => "notes",
            Self::Ocr | Self::Generation | Self::UsageNotes => return None,
        })
    }
}
/// Fields searched when a query names none: suggestions stay opt-in.
pub const DEFAULT_FIELDS: [TextField; 4] = [TextField::Name, TextField::Tags, TextField::Caption, TextField::Notes];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagState {
    /// User tags and accepted/edited suggestions for the current content.
    #[default]
    Confirmed,
    /// Pending suggestions only.
    Suggested,
    Any,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Landscape,
    Portrait,
    Square,
}
impl Orientation {
    fn name(self) -> &'static str {
        match self {
            Self::Landscape => "landscape",
            Self::Portrait => "portrait",
            Self::Square => "square",
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageSort {
    /// Tier order when text is present, otherwise name A–Z.
    #[default]
    Relevance,
    NameAsc,
    NameDesc,
    RatingDesc,
    AddedDesc,
    AddedAsc,
    PixelsDesc,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFilter {
    /// Has pending suggestions for its current content.
    NeedsReview,
    NoPendingSuggestions,
}

/// Canonical image query (wire, saved-search and smart-collection form).
/// Absent fields mean "no constraint"; every list is an any-of set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ImageQuery {
    pub version: u32,
    pub text: String,
    pub fields: Vec<TextField>,
    /// Empty means images only (the gallery's population).
    pub kinds: Vec<AssetKind>,
    pub source_ids: Vec<String>,
    /// Every tag is required (alias-resolved, case-insensitive exact match).
    pub tags: Vec<String>,
    pub exclude_tags: Vec<String>,
    pub tag_state: TagState,
    pub favorite: Option<bool>,
    pub min_rating: Option<u8>,
    pub max_rating: Option<u8>,
    /// `Some(false)` is unrated only; rating bounds always exclude unrated images.
    pub rated: Option<bool>,
    pub min_width: Option<u32>,
    pub max_width: Option<u32>,
    pub min_height: Option<u32>,
    pub max_height: Option<u32>,
    pub orientations: Vec<Orientation>,
    /// Has an alpha channel (not the same as actual transparency).
    pub alpha_channel: Option<bool>,
    /// Measured transparency of the pixels.
    pub transparency: Vec<Transparency>,
    pub formats: Vec<ImageFormat>,
    pub technical: Vec<TechnicalState>,
    /// Effective availability (an offline source reports `offline`).
    pub availability: Vec<Availability>,
    /// Technically ready, available and on an online source.
    pub ready_only: bool,
    pub enrichment: Vec<EnrichmentState>,
    pub review: Option<ReviewFilter>,
    /// Evidence origins (model, filename, imported, user) for the current content.
    pub provenance: Vec<Origin>,
    /// First import of any content version, Unix seconds, inclusive / exclusive.
    pub added_after: Option<i64>,
    pub added_before: Option<i64>,
    pub sort: ImageSort,
}
impl Default for ImageQuery {
    fn default() -> Self {
        Self {
            version: QUERY_VERSION,
            text: String::new(),
            fields: vec![],
            kinds: vec![],
            source_ids: vec![],
            tags: vec![],
            exclude_tags: vec![],
            tag_state: TagState::Confirmed,
            favorite: None,
            min_rating: None,
            max_rating: None,
            rated: None,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            orientations: vec![],
            alpha_channel: None,
            transparency: vec![],
            formats: vec![],
            technical: vec![],
            availability: vec![],
            ready_only: false,
            enrichment: vec![],
            review: None,
            provenance: vec![],
            added_after: None,
            added_before: None,
            sort: ImageSort::Relevance,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClauseKind {
    Term,
    Phrase,
    Prefix,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clause {
    pub text: String,
    pub kind: ClauseKind,
    pub excluded: bool,
    /// Images in scope matching this clause alone; reported only when nothing matched.
    pub matches: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTag {
    pub requested: String,
    pub tag: String,
    /// The requested spelling is an alias of `tag`.
    pub via_alias: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub term: String,
    /// Indexed words in scope within a small edit distance; never applied automatically.
    pub candidates: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interpretation {
    pub clauses: Vec<Clause>,
    pub fields: Vec<TextField>,
    pub tags: Vec<ResolvedTag>,
    pub exclude_tags: Vec<ResolvedTag>,
    /// Input that contains no searchable characters.
    pub ignored: Vec<String>,
    pub suggestions: Vec<Suggestion>,
    /// When nothing matched: images matching the text alone (same scope), else `None`.
    pub text_only_matches: Option<u32>,
}
/// Why one result is in the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HitExplanation {
    pub id: String,
    /// 0 exact tag, 1 tags, 2 caption, 3 name, 4 notes, 5 across fields; `None` without text.
    pub tier: Option<u8>,
    /// Searched fields containing at least one positive clause.
    pub fields: Vec<TextField>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetCount {
    pub value: String,
    pub count: u32,
}
/// Counts over the matching, authorized set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageFacets {
    pub tags: Vec<FacetCount>,
    pub suggested_tags: Vec<FacetCount>,
    pub orientations: Vec<FacetCount>,
    pub transparency: Vec<FacetCount>,
    /// "1".."5" and "unrated".
    pub ratings: Vec<FacetCount>,
    pub formats: Vec<FacetCount>,
    pub availability: Vec<FacetCount>,
    pub technical: Vec<FacetCount>,
    pub sources: Vec<FacetCount>,
    pub favorites: u32,
    pub needs_review: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSearchPage {
    pub items: Vec<ImageSummary>,
    pub explanations: Vec<HitExplanation>,
    pub offset: u32,
    /// Matching images when this page was read (a live observation, like the gallery).
    pub total: u32,
    pub interpretation: Interpretation,
    pub facets: Option<ImageFacets>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldCapability {
    pub field: TextField,
    pub available: bool,
    /// Ticket that delivers a reserved field.
    pub owner: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedImageSearch {
    pub id: String,
    pub name: String,
    /// `None` when the stored query is from a newer or unsupported version.
    pub query: Option<ImageQuery>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn capabilities() -> Vec<FieldCapability> {
    use TextField::*;
    [Name, Tags, SuggestedTags, Caption, Notes, Ocr, Generation, UsageNotes]
        .into_iter()
        .map(|field| FieldCapability {
            field,
            available: field.column().is_some(),
            owner: match field {
                Ocr => Some("SS-057".into()),
                Generation => Some("SS-055".into()),
                UsageNotes => Some("SS-056".into()),
                _ => None,
            },
        })
        .collect()
}

fn cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x2FA1F)
}
/// Index/query text normalization: NFC, CJK characters as single tokens,
/// private-use and control characters removed. Case and diacritics are folded
/// by the FTS tokenizer on both sides.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.nfc() {
        if c.is_control() || matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0x10FFFF) {
            out.push(' ');
        } else if cjk(c) {
            out.push(' ');
            out.push(c);
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}
fn searchable(folded: &str) -> bool {
    folded.chars().any(char::is_alphanumeric)
}
fn fts_string(folded: &str) -> String {
    format!("\"{}\"", folded.replace('"', "\"\""))
}

struct Parsed {
    text: String,
    kind: ClauseKind,
    excluded: bool,
    fts: String,
}
fn parse_text(text: &str) -> Result<(Vec<Parsed>, Vec<String>)> {
    if text.chars().count() > MAX_TEXT_CHARS || text.contains('\0') {
        return Err(E::LimitExceeded);
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut clauses, mut ignored) = (vec![], vec![]);
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let excluded = chars[i] == '-' && chars.get(i + 1).is_some_and(|c| !c.is_whitespace());
        if excluded {
            i += 1;
        }
        let (raw, kind) = if chars[i] == '"' {
            let start = i + 1;
            i = start;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            let raw: String = chars[start..i].iter().collect();
            i += 1;
            (raw, ClauseKind::Phrase)
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '"' {
                i += 1;
            }
            let raw: String = chars[start..i].iter().collect();
            match raw.strip_suffix('*') {
                Some(stem) if !stem.ends_with('*') => (stem.to_owned(), ClauseKind::Prefix),
                _ => (raw, ClauseKind::Term),
            }
        };
        let folded = fold(&raw);
        if !searchable(&folded) {
            if !raw.trim().is_empty() {
                ignored.push(if excluded { format!("-{raw}") } else { raw });
            }
            continue;
        }
        let shown = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if clauses.iter().any(|c: &Parsed| c.text == shown && c.kind == kind && c.excluded == excluded) {
            continue;
        }
        let mut fts = fts_string(&folded);
        if kind == ClauseKind::Prefix {
            fts.push_str(" *");
        }
        clauses.push(Parsed { text: shown, kind, excluded, fts });
    }
    if clauses.len() > MAX_CLAUSES {
        return Err(E::LimitExceeded);
    }
    Ok((clauses, ignored))
}

fn dedupe<T: Ord + Copy>(list: &[T]) -> Result<Vec<T>> {
    if list.len() > MAX_LIST {
        return Err(E::LimitExceeded);
    }
    Ok(list.iter().copied().collect::<BTreeSet<_>>().into_iter().collect())
}
fn wire<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}
fn wires<T: Serialize>(list: &[T]) -> Vec<Value> {
    let mut out: Vec<String> = list.iter().map(wire).collect();
    out.sort();
    out.dedup();
    out.into_iter().map(Value::from).collect()
}

/// A validated query, ready to compile against the current alias table.
pub struct Plan {
    query: ImageQuery,
    fields: Vec<TextField>,
    clauses: Vec<Parsed>,
    ignored: Vec<String>,
    tags: Vec<(String, String)>,
    exclude_tags: Vec<(String, String)>,
}
impl ImageQuery {
    /// Structural validation (no database). Rejects contradictions instead of
    /// silently returning nothing, and reserved fields instead of ignoring them.
    pub fn validate(&self) -> Result<()> {
        self.plan().map(|_| ())
    }
    pub fn plan(&self) -> Result<Plan> {
        if self.version != QUERY_VERSION {
            return Err(E::UnsupportedVersion);
        }
        let fields = if self.fields.is_empty() { DEFAULT_FIELDS.to_vec() } else { dedupe(&self.fields)? };
        if fields.iter().any(|f| f.column().is_none()) {
            return Err(E::UnsupportedField);
        }
        scoped(&self.source_ids).map_err(|_| E::LimitExceeded)?;
        let (clauses, ignored) = parse_text(&self.text)?;
        let tag_list = |list: &[String]| -> Result<Vec<(String, String)>> {
            if list.len() > MAX_FILTER_TAGS {
                return Err(E::LimitExceeded);
            }
            let mut out: Vec<(String, String)> = vec![];
            for raw in list {
                let tag = display_tag(raw).map_err(|_| E::InvalidArgument)?.ok_or(E::InvalidArgument)?;
                if !out.iter().any(|(_, t)| tag_key(t) == tag_key(&tag)) {
                    out.push((raw.clone(), tag));
                }
            }
            Ok(out)
        };
        let tags = tag_list(&self.tags)?;
        let exclude_tags = tag_list(&self.exclude_tags)?;
        if tags.iter().any(|(_, t)| exclude_tags.iter().any(|(_, x)| tag_key(x) == tag_key(t))) {
            return Err(E::InvalidArgument);
        }
        for r in [self.min_rating, self.max_rating].into_iter().flatten() {
            if !(1..=5).contains(&r) {
                return Err(E::InvalidArgument);
            }
        }
        let rating_bound = self.min_rating.is_some() || self.max_rating.is_some();
        if self.min_rating.zip(self.max_rating).is_some_and(|(a, b)| a > b) || (rating_bound && self.rated == Some(false)) {
            return Err(E::InvalidArgument);
        }
        for (min, max) in [(self.min_width, self.max_width), (self.min_height, self.max_height)] {
            if [min, max].into_iter().flatten().any(|v| v == 0 || v > MAX_DIMENSION) || min.zip(max).is_some_and(|(a, b)| a > b) {
                return Err(E::InvalidArgument);
            }
        }
        if self.added_after.zip(self.added_before).is_some_and(|(a, b)| a >= b)
            || [self.added_after, self.added_before].into_iter().flatten().any(|v| !(0..=crate::image_contracts::MAX_SAFE_INTEGER as i64).contains(&v))
        {
            return Err(E::InvalidArgument);
        }
        for len in [self.kinds.len(), self.orientations.len(), self.transparency.len(), self.formats.len(), self.technical.len(), self.availability.len(), self.enrichment.len(), self.provenance.len()] {
            if len > MAX_LIST {
                return Err(E::LimitExceeded);
            }
        }
        Ok(Plan { query: self.clone(), fields, clauses, ignored, tags, exclude_tags })
    }
    /// Text, tag and structural constraints; `false` means the plain gallery order.
    fn filtered(&self) -> bool {
        let plain = ImageQuery { source_ids: self.source_ids.clone(), sort: self.sort, fields: self.fields.clone(), tag_state: self.tag_state, ..Default::default() };
        let mut probe = self.clone();
        probe.text = probe.text.trim().to_owned();
        probe != plain
    }
}

/// Rebuild queued documents. Runs in a savepoint so a failure leaves the queue intact.
pub(crate) fn flush(db: &Connection) -> Result<usize> {
    let pending: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM image_search_dirty)", [], |r| r.get(0))?;
    if !pending {
        return Ok(0);
    }
    db.execute_batch("SAVEPOINT image_search_flush")?;
    let result = flush_all(db);
    match result {
        Ok(_) => db.execute_batch("RELEASE image_search_flush")?,
        Err(_) => db.execute_batch("ROLLBACK TO image_search_flush; RELEASE image_search_flush")?,
    }
    result
}
struct DocSource {
    kind: String,
    path: String,
    technical: String,
    availability: String,
    enrichment: String,
    annotations: String,
    evidence: String,
    hash: Option<String>,
    profile: Option<String>,
    added: Option<i64>,
}
fn flush_all(db: &Connection) -> Result<usize> {
    let mut done = 0;
    // Many images share one content analysis; parse each profile once per flush.
    let mut profiles: HashMap<String, Option<ImageProfile>> = HashMap::new();
    loop {
        type Row = (String, Option<i64>, Option<DocSource>);
        let rows: Vec<Row> = {
            let mut q = db.prepare_cached(
                "SELECT q.asset_id,(SELECT doc FROM image_search_docs WHERE asset_id=q.asset_id),i.id,r.kind,r.relative_path,
                 i.technical_state,i.availability,i.enrichment_state,i.annotations,i.evidence,i.content_hash,p.profile,
                 (SELECT min(first_seen) FROM image_asset_versions v WHERE v.asset_id=i.id)
                 FROM (SELECT asset_id FROM image_search_dirty LIMIT ?1) q
                 LEFT JOIN image_assets i ON i.id=q.asset_id LEFT JOIN asset_registry r ON r.id=i.id
                 LEFT JOIN image_analyses p ON p.content_hash=i.content_hash AND p.analyzer_version=i.analyzer_version",
            )?;
            let rows = q.query_map([FLUSH_CHUNK as i64], |r| {
                let present: Option<String> = r.get(2)?;
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    match present {
                        None => None,
                        Some(_) => Some(DocSource {
                            kind: r.get(3)?,
                            path: r.get(4)?,
                            technical: r.get(5)?,
                            availability: r.get(6)?,
                            enrichment: r.get(7)?,
                            annotations: r.get(8)?,
                            evidence: r.get(9)?,
                            hash: r.get(10)?,
                            profile: r.get(11)?,
                            added: r.get(12)?,
                        }),
                    },
                ))
            })?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        if rows.is_empty() {
            return Ok(done);
        }
        for (id, old, source) in &rows {
            index_one(db, id, *old, source.as_ref(), &mut profiles)?;
            db.prepare_cached("DELETE FROM image_search_dirty WHERE asset_id=?1")?.execute([id])?;
        }
        done += rows.len();
    }
}
fn index_one(db: &Connection, id: &str, old: Option<i64>, row: Option<&DocSource>, profiles: &mut HashMap<String, Option<ImageProfile>>) -> Result<()> {
    if let Some(doc) = old {
        db.prepare_cached("DELETE FROM image_search_fts WHERE rowid=?1")?.execute([doc])?;
        db.prepare_cached("DELETE FROM image_search_tags WHERE doc=?1")?.execute([doc])?;
    }
    let Some(s) = row else {
        if let Some(doc) = old {
            db.prepare_cached("DELETE FROM image_search_docs WHERE doc=?1")?.execute([doc])?;
        }
        return Ok(());
    };
    // Stored records are validated on write; an unreadable one still gets a
    // document (so it stays findable by name) rather than blocking every search.
    let notes: Option<Annotations> = serde_json::from_str(&s.annotations).ok();
    let evidence: Vec<Evidence> = serde_json::from_str(&s.evidence).unwrap_or_default();
    let profile: Option<ImageProfile> = match &s.profile {
        None => None,
        Some(raw) => profiles.entry(raw.clone()).or_insert_with(|| serde_json::from_str(raw).ok()).clone(),
    };
    let mut confirmed: Vec<String> = vec![];
    let mut suggested: Vec<String> = vec![];
    let (mut captions, mut suggested_captions) = (vec![], vec![]);
    let mut origins = BTreeSet::new();
    let push = |list: &mut Vec<String>, tag: &str| {
        if let Ok(Some(t)) = display_tag(tag) {
            if !list.iter().any(|x| tag_key(x) == tag_key(&t)) {
                list.push(t);
            }
        }
    };
    if let Some(n) = &notes {
        for t in &n.tags {
            push(&mut confirmed, t);
        }
        captions.extend(n.caption.clone());
    }
    // Evidence counts only for the current content and while not stale.
    for e in evidence.iter().filter(|e| !e.stale && Some(&e.content_hash) == s.hash.as_ref()) {
        match e.review_state {
            ReviewState::Rejected => continue,
            ReviewState::Accepted | ReviewState::Edited => {
                e.tags.iter().for_each(|t| push(&mut confirmed, t));
                captions.extend(e.caption.clone());
            }
            ReviewState::Suggested => {
                e.tags.iter().for_each(|t| push(&mut suggested, t));
                suggested_captions.extend(e.caption.clone());
            }
        }
        origins.insert(wire(&e.origin));
    }
    suggested.retain(|t| !confirmed.iter().any(|c| tag_key(c) == tag_key(t)));
    let pending = suggested.len() + suggested_captions.len();
    let (width, height) = profile.as_ref().map(|p| (p.width, p.height)).unzip();
    let orientation = profile.as_ref().map(|p| match p.width.cmp(&p.height) {
        std::cmp::Ordering::Greater => Orientation::Landscape,
        std::cmp::Ordering::Less => Orientation::Portrait,
        std::cmp::Ordering::Equal => Orientation::Square,
    });
    let doc: i64 = db
        .prepare_cached(
            "INSERT INTO image_search_docs(asset_id,kind,favorite,rating,width,height,orientation,alpha,transparency,format,
               technical,availability,enrichment,suggestions,origins,added_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
             ON CONFLICT(asset_id) DO UPDATE SET kind=excluded.kind,favorite=excluded.favorite,rating=excluded.rating,
               width=excluded.width,height=excluded.height,orientation=excluded.orientation,alpha=excluded.alpha,
               transparency=excluded.transparency,format=excluded.format,technical=excluded.technical,
               availability=excluded.availability,enrichment=excluded.enrichment,suggestions=excluded.suggestions,
               origins=excluded.origins,added_at=excluded.added_at
             RETURNING doc",
        )?
        .query_row(
            params![
                id,
                s.kind,
                notes.as_ref().is_some_and(|n| n.favorite),
                notes.as_ref().and_then(|n| n.rating),
                width,
                height,
                orientation.map(Orientation::name),
                profile.as_ref().map(|p| p.alpha_channel),
                profile.as_ref().map(|p| wire(&p.transparency)),
                profile.as_ref().map(|p| wire(&p.format)),
                s.technical,
                s.availability,
                s.enrichment,
                pending as i64,
                format!(",{},", origins.into_iter().collect::<Vec<_>>().join(",")),
                s.added,
            ],
            |r| r.get(0),
        )?;
    let comment = notes.as_ref().map(|n| n.comment.as_str()).unwrap_or("");
    let suggested_text = suggested.iter().chain(&suggested_captions).map(|t| fold(t)).collect::<Vec<_>>().join(TAG_GAP);
    db.prepare_cached("INSERT INTO image_search_fts(rowid,name,tags,suggested,caption,notes) VALUES(?1,?2,?3,?4,?5,?6)")?
        .execute(params![
            doc,
            fold(&s.path),
            confirmed.iter().map(|t| fold(t)).collect::<Vec<_>>().join(TAG_GAP),
            suggested_text,
            captions.iter().map(|c| fold(c)).collect::<Vec<_>>().join(TAG_GAP),
            fold(comment),
        ])?;
    let mut insert = db.prepare_cached("INSERT OR IGNORE INTO image_search_tags(doc,key,tag,state) VALUES(?1,?2,?3,?4)")?;
    for (list, state) in [(&confirmed, "confirmed"), (&suggested, "suggested")] {
        for t in list {
            insert.execute(params![doc, tag_key(t), t, state])?;
        }
    }
    Ok(())
}

fn aliases(db: &Connection) -> Result<HashMap<String, String>> {
    let mut s = db.prepare_cached("SELECT alias_key,canonical FROM image_tag_aliases")?;
    let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}
fn resolve(alias: &HashMap<String, String>, list: &[(String, String)]) -> Vec<ResolvedTag> {
    list.iter()
        .map(|(raw, tag)| match alias.get(&tag_key(tag)) {
            Some(canonical) => ResolvedTag { requested: raw.clone(), tag: canonical.clone(), via_alias: true },
            None => ResolvedTag { requested: raw.clone(), tag: tag.clone(), via_alias: false },
        })
        .collect()
}

/// SQL fragments for one compiled query. `scope` holds authorization; `filters`
/// the caller's hard predicates; `text` the positive/negative token conditions.
struct Compiled {
    scope: String,
    scope_values: Vec<Value>,
    filters: String,
    filter_values: Vec<Value>,
    text: String,
    tier: String,
    tier_values: Vec<Value>,
    /// Positive clauses, OR-ed, per searched column (for explanations).
    any_positive: Option<String>,
    /// Token matches, each evaluated once. An FTS subquery used directly inside
    /// IN can be re-run per candidate row (quadratic on common words).
    ctes: Vec<String>,
    cte_values: Vec<Value>,
    /// Positive token matches drive the join (rare words touch few rows).
    driver: Option<String>,
    empty: bool,
}
impl Compiled {
    /// Register a materialized FTS match and return a subquery over its rows.
    fn matches(&mut self, expr: String) -> String {
        self.materialize("SELECT rowid AS doc FROM image_search_fts WHERE image_search_fts MATCH ?", expr.into())
    }
    fn materialize(&mut self, select: &str, value: Value) -> String {
        let name = format!("m{}", self.ctes.len());
        self.ctes.push(format!("{name} AS MATERIALIZED ({select})"));
        self.cte_values.push(value);
        format!("(SELECT doc FROM {name})")
    }
    /// SQLite otherwise probes the docs' asset_id index once per IN value for
    /// every registry row (10K images × 8K matches took seconds), so the
    /// materialized match set is forced to the outer loop.
    fn from(&self) -> String {
        match &self.driver {
            Some(m) => format!("FROM {m} AS t CROSS JOIN image_search_docs d ON d.doc=t.doc JOIN asset_registry r ON r.id=d.asset_id
 JOIN image_assets i ON i.id=d.asset_id JOIN sources s ON s.id=r.source_id"),
            None => FROM.into(),
        }
    }
    /// The matching set with its sort keys (inserted into the per-search hit table).
    fn hits(&self) -> (String, Vec<Value>) {
        let sql = format!(
            "SELECT d.doc AS doc,{tier} AS tier,lower(r.relative_path) AS k1,r.relative_path AS k2,r.id AS id,
             d.rating AS rating,d.added_at AS added,d.width*d.height AS px,r.source_id AS src,
             CASE WHEN s.available=0 THEN 'offline' ELSE d.availability END AS avail
             {from} WHERE 1 {scope}{filters}{text}",
            tier = self.tier,
            from = self.from(),
            scope = self.scope,
            filters = self.filters,
            text = self.text
        );
        let mut values = self.cte_values.clone();
        values.extend(self.tier_values.clone());
        values.extend(self.scope_values.clone());
        values.extend(self.filter_values.clone());
        (sql, values)
    }
    fn with(&self) -> String {
        if self.ctes.is_empty() { String::new() } else { format!("WITH {} ", self.ctes.join(", ")) }
    }
}
fn single_match(expr: String) -> (String, Vec<Value>) {
    ("WITH m AS MATERIALIZED (SELECT rowid AS doc FROM image_search_fts WHERE image_search_fts MATCH ?) ".into(), vec![expr.into()])
}
const FROM: &str = "FROM image_search_docs d JOIN asset_registry r ON r.id=d.asset_id
 JOIN image_assets i ON i.id=d.asset_id JOIN sources s ON s.id=r.source_id";

fn column_filter(fields: &[TextField]) -> String {
    format!("{{{}}}", fields.iter().filter_map(|f| f.column()).collect::<Vec<_>>().join(" "))
}
fn compile(plan: &Plan, allowed: Option<&[String]>, alias: &HashMap<String, String>) -> Result<(Compiled, Interpretation)> {
    let q = &plan.query;
    let mut c = Compiled {
        scope: String::new(),
        scope_values: vec![],
        filters: String::new(),
        filter_values: vec![],
        text: String::new(),
        tier: "NULL".into(),
        tier_values: vec![],
        any_positive: None,
        ctes: vec![],
        cte_values: vec![],
        driver: None,
        empty: false,
    };
    // Authorization first: the requested sources are narrowed to the allowed set,
    // and a source outside it contributes nothing (indistinguishable from empty).
    let effective: Option<Vec<String>> = match (allowed, q.source_ids.is_empty()) {
        (None, true) => None,
        (None, false) => Some(q.source_ids.clone()),
        (Some(a), true) => Some(a.to_vec()),
        (Some(a), false) => Some(q.source_ids.iter().filter(|s| a.contains(s)).cloned().collect()),
    };
    let kinds = if q.kinds.is_empty() { vec![Value::from("image".to_owned())] } else { wires(&q.kinds) };
    c.scope = format!(
        " AND r.kind IN ({}) AND (s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path))",
        vec!["?"; kinds.len()].join(",")
    );
    c.scope_values.extend(kinds);
    if let Some(ids) = effective {
        if ids.is_empty() {
            c.empty = true;
        } else {
            c.scope.push_str(&format!(" AND r.source_id IN ({})", vec!["?"; ids.len()].join(",")));
            c.scope_values.extend(ids.into_iter().map(Value::from));
        }
    }
    let f = &mut c.filters;
    let v = &mut c.filter_values;
    let any_of = |column: &str, values: Vec<Value>, f: &mut String, v: &mut Vec<Value>| {
        if !values.is_empty() {
            f.push_str(&format!(" AND {column} IN ({})", vec!["?"; values.len()].join(",")));
            v.extend(values);
        }
    };
    any_of("d.orientation", wires(&q.orientations), f, v);
    any_of("d.transparency", wires(&q.transparency), f, v);
    any_of("d.format", wires(&q.formats), f, v);
    any_of("d.technical", wires(&q.technical), f, v);
    any_of("(CASE WHEN s.available=0 THEN 'offline' ELSE d.availability END)", wires(&q.availability), f, v);
    any_of("d.enrichment", wires(&q.enrichment), f, v);
    if let Some(fav) = q.favorite {
        f.push_str(&format!(" AND d.favorite={}", i32::from(fav)));
    }
    if q.rated == Some(false) {
        f.push_str(" AND d.rating IS NULL");
    }
    if q.rated == Some(true) {
        f.push_str(" AND d.rating IS NOT NULL");
    }
    // Comparisons with NULL are never true: unknown never satisfies a bound.
    for (column, op, value) in [
        ("d.rating", ">=", q.min_rating.map(i64::from)),
        ("d.rating", "<=", q.max_rating.map(i64::from)),
        ("d.width", ">=", q.min_width.map(i64::from)),
        ("d.width", "<=", q.max_width.map(i64::from)),
        ("d.height", ">=", q.min_height.map(i64::from)),
        ("d.height", "<=", q.max_height.map(i64::from)),
        ("d.added_at", ">=", q.added_after),
        ("d.added_at", "<", q.added_before),
    ] {
        if let Some(value) = value {
            f.push_str(&format!(" AND {column} {op} ?"));
            v.push(value.into());
        }
    }
    if let Some(alpha) = q.alpha_channel {
        f.push_str(&format!(" AND d.alpha={}", i32::from(alpha)));
    }
    if q.ready_only {
        f.push_str(" AND d.technical='ready' AND d.availability='available' AND s.available=1");
    }
    match q.review {
        Some(ReviewFilter::NeedsReview) => f.push_str(" AND d.suggestions>0"),
        Some(ReviewFilter::NoPendingSuggestions) => f.push_str(" AND d.suggestions=0"),
        None => {}
    }
    if !q.provenance.is_empty() {
        let clauses: Vec<&str> = q.provenance.iter().map(|_| "instr(d.origins,?)>0").collect();
        f.push_str(&format!(" AND ({})", clauses.join(" OR ")));
        v.extend(wires(&q.provenance).into_iter().map(|o| match o {
            Value::Text(t) => Value::Text(format!(",{t},")),
            other => other,
        }));
    }
    let states = match q.tag_state {
        TagState::Confirmed => "('confirmed')",
        TagState::Suggested => "('suggested')",
        TagState::Any => "('confirmed','suggested')",
    };
    let tags = resolve(alias, &plan.tags);
    let exclude_tags = resolve(alias, &plan.exclude_tags);
    for t in &tags {
        f.push_str(&format!(" AND EXISTS(SELECT 1 FROM image_search_tags t WHERE t.doc=d.doc AND t.key=? AND t.state IN {states})"));
        v.push(tag_key(&t.tag).into());
    }
    for t in &exclude_tags {
        f.push_str(&format!(" AND NOT EXISTS(SELECT 1 FROM image_search_tags t WHERE t.doc=d.doc AND t.key=? AND t.state IN {states})"));
        v.push(tag_key(&t.tag).into());
    }
    let cols = column_filter(&plan.fields);
    let positive: Vec<&Parsed> = plan.clauses.iter().filter(|p| !p.excluded).collect();
    let negative: Vec<&Parsed> = plan.clauses.iter().filter(|p| p.excluded).collect();
    if !positive.is_empty() {
        let all = positive.iter().map(|p| p.fts.as_str()).collect::<Vec<_>>().join(" AND ");
        c.driver = Some(c.matches(format!("{cols} : ({all})")));
        c.any_positive = Some(positive.iter().map(|p| p.fts.as_str()).collect::<Vec<_>>().join(" OR "));
        // Tiers: exact tag, then all clauses inside one field in a fixed field order.
        let mut tier = String::from("CASE");
        if plan.fields.contains(&TextField::Tags) && positive.iter().all(|p| p.kind != ClauseKind::Prefix) {
            let whole = positive.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" ");
            if let Ok(Some(tag)) = display_tag(&whole) {
                let tag = alias.get(&tag_key(&tag)).cloned().unwrap_or(tag);
                tier.push_str(" WHEN d.doc IN (SELECT doc FROM image_search_tags WHERE key=? AND state='confirmed') THEN 0");
                c.tier_values.push(tag_key(&tag).into());
            }
        }
        for (field, rank) in [(TextField::Tags, 1), (TextField::Caption, 2), (TextField::Name, 3), (TextField::Notes, 4)] {
            if plan.fields.contains(&field) {
                let m = c.matches(format!("{{{}}} : ({all})", field.column().unwrap_or_default()));
                tier.push_str(&format!(" WHEN d.doc IN {m} THEN {rank}"));
            }
        }
        tier.push_str(" ELSE 5 END");
        c.tier = tier;
    }
    if !negative.is_empty() {
        let any = negative.iter().map(|p| p.fts.as_str()).collect::<Vec<_>>().join(" OR ");
        let m = c.matches(format!("{cols} : ({any})"));
        c.text.push_str(&format!(" AND d.doc NOT IN {m}"));
    }
    // Without text, the first required tag's (indexed) set drives the join instead.
    if let (None, Some(t)) = (&c.driver, tags.first()) {
        let select = format!("SELECT doc FROM image_search_tags WHERE key=? AND state IN {states}");
        c.driver = Some(c.materialize(&select, tag_key(&t.tag).into()));
    }
    let interpretation = Interpretation {
        clauses: plan
            .clauses
            .iter()
            .map(|p| Clause { text: p.text.clone(), kind: p.kind, excluded: p.excluded, matches: None })
            .collect(),
        fields: plan.fields.clone(),
        tags,
        exclude_tags,
        ignored: plan.ignored.clone(),
        suggestions: vec![],
        text_only_matches: None,
    };
    Ok((c, interpretation))
}

fn order_by(sort: ImageSort) -> &'static str {
    match sort {
        ImageSort::Relevance => "h.tier ASC, h.k1 ASC, h.k2 ASC, h.id ASC",
        ImageSort::NameAsc => "h.k1 ASC, h.k2 ASC, h.id ASC",
        ImageSort::NameDesc => "h.k1 DESC, h.k2 DESC, h.id ASC",
        ImageSort::RatingDesc => "h.rating IS NULL, h.rating DESC, h.k1 ASC, h.k2 ASC, h.id ASC",
        ImageSort::AddedDesc => "h.added IS NULL, h.added DESC, h.k1 ASC, h.k2 ASC, h.id ASC",
        ImageSort::AddedAsc => "h.added IS NULL, h.added ASC, h.k1 ASC, h.k2 ASC, h.id ASC",
        ImageSort::PixelsDesc => "h.px IS NULL, h.px DESC, h.k1 ASC, h.k2 ASC, h.id ASC",
    }
}
fn count(db: &Connection, sql: &str, values: Vec<Value>) -> Result<u32> {
    Ok(db.prepare_cached(sql)?.query_row(params_from_iter(values), |r| r.get(0))?)
}
fn facet_rows(db: &Connection, sql: &str) -> Result<Vec<FacetCount>> {
    let mut q = db.prepare_cached(sql)?;
    let rows = q.query_map([], |r| Ok(FacetCount { value: r.get(0)?, count: r.get(1)? }))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}
/// Facet counts over the current result set in one pass (plus one query per
/// tag state). Small sets are driven from the hits; large ones scan the tag
/// table once in key order instead of probing it 100K times.
fn facets(db: &Connection, total: u32) -> Result<ImageFacets> {
    let mut maps: [HashMap<String, u32>; 7] = Default::default();
    let mut out = ImageFacets::default();
    {
        let mut q = db.prepare_cached(
            "SELECT d.orientation,d.transparency,coalesce(CAST(d.rating AS TEXT),'unrated'),d.format,
             h.avail,d.technical,h.src,d.favorite,d.suggestions>0
             FROM temp.image_search_order h CROSS JOIN image_search_docs d ON d.doc=h.doc",
        )?;
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            for (i, map) in maps.iter_mut().enumerate() {
                if let rusqlite::types::ValueRef::Text(v) = row.get_ref(i)? {
                    let v = std::str::from_utf8(v).map_err(|_| E::Internal)?;
                    match map.get_mut(v) {
                        Some(n) => *n += 1,
                        None => {
                            map.insert(v.to_owned(), 1);
                        }
                    }
                }
            }
            out.favorites += u32::from(row.get::<_, bool>(7)?);
            out.needs_review += u32::from(row.get::<_, bool>(8)?);
        }
    }
    let sorted = |map: &mut HashMap<String, u32>| {
        let mut list: Vec<FacetCount> = map.drain().map(|(value, count)| FacetCount { value, count }).collect();
        list.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
        list
    };
    let [o, t, ra, fo, av, te, so] = &mut maps;
    (out.orientations, out.transparency, out.ratings, out.formats) = (sorted(o), sorted(t), sorted(ra), sorted(fo));
    (out.availability, out.technical, out.sources) = (sorted(av), sorted(te), sorted(so));
    let docs: i64 = db.query_row("SELECT count(*) FROM image_search_docs", [], |r| r.get(0))?;
    let tags = |state: &str| {
        let sql = if i64::from(total) * 5 < docs {
            format!("SELECT min(t.tag),count(*) FROM temp.image_search_order h CROSS JOIN image_search_tags t ON t.doc=h.doc
              WHERE t.state='{state}' GROUP BY t.key ORDER BY 2 DESC, t.key ASC LIMIT {MAX_FACET_TAGS}")
        } else {
            format!("SELECT min(t.tag),count(*) FROM image_search_tags t WHERE t.state='{state}'
              AND t.doc IN (SELECT doc FROM temp.image_search_order) GROUP BY t.key ORDER BY 2 DESC, t.key ASC LIMIT {MAX_FACET_TAGS}")
        };
        facet_rows(db, &sql)
    };
    out.tags = tags("confirmed")?;
    out.suggested_tags = tags("suggested")?;
    Ok(out)
}
fn edit_distance_ok(a: &str, b: &str) -> bool {
    let (la, lb) = (a.chars().count(), b.chars().count());
    let budget = if la > 7 { 2 } else { 1 };
    la.abs_diff(lb) <= budget && strsim::damerau_levenshtein(a, b) <= budget
}

impl Catalog {
    /// Bring the derived search index up to date (also run by every search).
    pub fn refresh_image_search_index(&self) -> Result<usize> {
        flush(&self.db)
    }

    /// One window of matching images. `allowed` is the caller's authorized
    /// source set (`None` = desktop owner). `facets` adds counts for the whole
    /// matching set; request it for the first window only.
    pub fn search_images(&self, allowed: Option<&[String]>, query: &ImageQuery, offset: u32, limit: u32, want_facets: bool) -> Result<ImageSearchPage> {
        if limit == 0 || limit > MAX_PAGE_SIZE || offset > MAX_GALLERY_OFFSET {
            return Err(E::InvalidArgument);
        }
        if let Some(a) = allowed {
            scoped(a).map_err(|_| E::LimitExceeded)?;
        }
        let plan = query.plan()?;
        self.db.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS image_search_order(ord INTEGER PRIMARY KEY, doc INTEGER NOT NULL, tier INTEGER,
               id TEXT NOT NULL, src TEXT NOT NULL, avail TEXT NOT NULL);
             CREATE TEMP TABLE IF NOT EXISTS image_search_cache(id INTEGER PRIMARY KEY CHECK(id=1), key TEXT NOT NULL,
               changes INTEGER NOT NULL, data_version INTEGER NOT NULL, total INTEGER NOT NULL);",
        )?;
        // The previous result set is reusable only if nothing was written on this
        // connection (total_changes) or committed by another one (data_version).
        let data_version: i64 = self.db.pragma_query_value(None, "data_version", |r| r.get(0))?;
        let cached: Option<(String, u32)> = self
            .db
            .query_row(
                "SELECT key,total FROM temp.image_search_cache WHERE id=1 AND changes=total_changes() AND data_version=?1",
                [data_version],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        flush(&self.db)?;
        let alias = aliases(&self.db)?;
        let (c, mut interpretation) = compile(&plan, allowed, &alias)?;
        let empty_page = |interpretation| ImageSearchPage { items: vec![], explanations: vec![], offset, total: 0, interpretation, facets: want_facets.then(ImageFacets::default) };
        if c.empty {
            return Ok(empty_page(interpretation));
        }
        // Unfiltered owner browsing in name order is the gallery's indexed path.
        let gallery_sort = match query.sort {
            ImageSort::Relevance | ImageSort::NameAsc => Some(GallerySort::NameAsc),
            ImageSort::NameDesc => Some(GallerySort::NameDesc),
            _ => None,
        };
        if let (false, Some(sort), None, true) = (query.filtered(), gallery_sort, allowed, query.kinds.is_empty()) {
            if !want_facets {
                let scope = (!query.source_ids.is_empty()).then_some(query.source_ids.as_slice());
                let page = self.image_gallery_page(scope, sort, offset, limit)?;
                let explanations = page.items.iter().map(|i| HitExplanation { id: i.header.id.clone(), tier: None, fields: vec![] }).collect();
                return Ok(ImageSearchPage { items: page.items, explanations, offset, total: page.total, interpretation, facets: None });
            }
        }
        let tx = self.db.unchecked_transaction()?;
        let key = serde_json::to_string(&(&plan.query, allowed))?;
        let total = match cached {
            Some((cached_key, total)) if cached_key == key => total,
            _ => {
                // One sort per query and catalog state; later windows are primary-key ranges.
                tx.execute_batch("DELETE FROM temp.image_search_order; DELETE FROM temp.image_search_cache;")?;
                let (sql, values) = c.hits();
                let total = tx
                    .prepare_cached(&format!(
                        "{}INSERT INTO temp.image_search_order(ord,doc,tier,id,src,avail)
                         SELECT row_number() OVER (ORDER BY {order})-1,h.doc,h.tier,h.id,h.src,h.avail FROM ({sql}) h",
                        c.with(),
                        order = order_by(query.sort)
                    ))?
                    .execute(params_from_iter(values))? as u32;
                tx.execute("INSERT INTO temp.image_search_cache(id,key,changes,data_version,total) VALUES(1,?1,0,?2,?3)", params![key, data_version, total])?;
                total
            }
        };
        let sql = format!(
            "SELECT {SUMMARY_COLUMNS},h.tier,h.doc FROM temp.image_search_order h JOIN asset_registry r ON r.id=h.id
             JOIN image_assets i ON i.id=r.id JOIN sources s ON s.id=r.source_id
             LEFT JOIN image_analyses p ON p.content_hash=i.content_hash AND p.analyzer_version=i.analyzer_version
             WHERE h.ord>=?1 AND h.ord<?1+?2 ORDER BY h.ord"
        );
        let (mut items, mut tiers, mut bytes) = (vec![], vec![], 256);
        {
            let mut q = tx.prepare_cached(&sql)?;
            let mut rows = q.query(params![i64::from(offset), i64::from(limit)])?;
            while let Some(row) = rows.next()? {
                let item = summary(row)?;
                let size = serde_json::to_vec(&item)?.len() + 64;
                // Never truncate silently: a short page tells the caller where to resume.
                if bytes + size > MAX_BUNDLE_BYTES {
                    break;
                }
                bytes += size;
                tiers.push((row.get::<_, Option<u8>>(11)?, row.get::<_, i64>(12)?));
                items.push(item);
            }
        }
        let explanations = explain(&tx, &plan, &c, &items, &tiers)?;
        let facets = if want_facets { Some(facets(&tx, total)?) } else { None };
        if total == 0 && offset == 0 && !plan.clauses.is_empty() {
            guidance(&tx, &plan, &c, &mut interpretation)?;
        }
        // Record the connection's change counter as it will read after this
        // one-row update, so any later write (here or by triggers) invalidates the cache.
        tx.execute("UPDATE temp.image_search_cache SET changes=total_changes()+1 WHERE id=1", [])?;
        tx.commit()?;
        Ok(ImageSearchPage { items, explanations, offset, total, interpretation, facets })
    }

    /// Validate a query against the current catalog without running it.
    pub fn image_search_capabilities(&self) -> Vec<FieldCapability> {
        capabilities()
    }

    pub fn saved_image_searches(&self) -> Result<Vec<SavedImageSearch>> {
        let mut q = self.db.prepare("SELECT id,name,query,created_at,updated_at FROM saved_image_searches ORDER BY name_key,id")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get(3)?, r.get(4)?)))?;
        let mut out = vec![];
        for row in rows {
            let (id, name, query, created_at, updated_at) = row?;
            // A query from a newer version is kept intact and reported as unsupported.
            let query = serde_json::from_str::<ImageQuery>(&query).ok().filter(|q| q.validate().is_ok());
            out.push(SavedImageSearch { id, name, query, created_at, updated_at });
        }
        Ok(out)
    }
    /// Create, or replace the query of the search with the same name when `replace`.
    pub fn save_image_search(&mut self, name: &str, query: &ImageQuery, replace: bool) -> Result<SavedImageSearch> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(E::InvalidArgument);
        }
        query.validate()?;
        let json = serde_json::to_string(query)?;
        if json.len() > MAX_SAVED_QUERY_BYTES {
            return Err(E::LimitExceeded);
        }
        let key = tag_key(&name);
        let now = crate::jobs::now_secs();
        let tx = self.db.transaction()?;
        let existing: Option<String> = tx.query_row("SELECT id FROM saved_image_searches WHERE name_key=?1", [&key], |r| r.get(0)).optional()?;
        let id = match existing {
            Some(_) if !replace => return Err(E::Conflict),
            Some(id) => {
                tx.execute("UPDATE saved_image_searches SET name=?2,query=?3,updated_at=?4 WHERE id=?1", params![id, name, json, now])?;
                id
            }
            None => {
                let n: i64 = tx.query_row("SELECT count(*) FROM saved_image_searches", [], |r| r.get(0))?;
                if n >= MAX_SAVED_SEARCHES {
                    return Err(E::LimitExceeded);
                }
                let id = Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO saved_image_searches(id,name,name_key,query,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)",
                    params![id, name, key, json, now],
                )?;
                id
            }
        };
        let saved = tx.query_row("SELECT created_at,updated_at FROM saved_image_searches WHERE id=?1", [&id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        tx.commit()?;
        Ok(SavedImageSearch { id, name, query: Some(query.clone()), created_at: saved.0, updated_at: saved.1 })
    }
    pub fn delete_saved_image_search(&mut self, id: &str) -> Result<()> {
        match self.db.execute("DELETE FROM saved_image_searches WHERE id=?1", [id])? {
            0 => Err(E::NotFound),
            _ => Ok(()),
        }
    }
}

/// Fields that explain each page item. Bounded: one FTS lookup per searched field.
fn explain(db: &Connection, plan: &Plan, c: &Compiled, items: &[ImageSummary], tiers: &[(Option<u8>, i64)]) -> Result<Vec<HitExplanation>> {
    let mut fields_by_doc: HashMap<i64, Vec<TextField>> = HashMap::new();
    if let (Some(any), false) = (&c.any_positive, items.is_empty()) {
        let docs: Vec<Value> = tiers.iter().map(|(_, d)| Value::from(*d)).collect();
        let marks = vec!["?"; docs.len()].join(",");
        for field in &plan.fields {
            let (with, mut values) = single_match(format!("{{{}}} : ({any})", field.column().unwrap_or_default()));
            values.extend(docs.iter().cloned());
            let mut q = db.prepare_cached(&format!("{with}SELECT doc FROM m WHERE doc IN ({marks})"))?;
            let rows = q.query_map(params_from_iter(values), |r| r.get::<_, i64>(0))?;
            for doc in rows {
                fields_by_doc.entry(doc?).or_default().push(*field);
            }
        }
    }
    Ok(items
        .iter()
        .zip(tiers)
        .map(|(item, (tier, doc))| HitExplanation { id: item.header.id.clone(), tier: *tier, fields: fields_by_doc.remove(doc).unwrap_or_default() })
        .collect())
}

/// No-match guidance: per-clause counts and spelling candidates, all computed
/// inside the caller's scope so restricted sources cannot leak through them.
fn guidance(db: &Connection, plan: &Plan, c: &Compiled, out: &mut Interpretation) -> Result<()> {
    let cols = column_filter(&plan.fields);
    let in_scope = |expr: String| -> Result<u32> {
        let (with, mut values) = single_match(expr);
        values.extend(c.scope_values.clone());
        count(db, &format!("{with}SELECT count(*) FROM m t CROSS JOIN image_search_docs d ON d.doc=t.doc JOIN asset_registry r ON r.id=d.asset_id
 JOIN image_assets i ON i.id=d.asset_id JOIN sources s ON s.id=r.source_id WHERE 1 {}", c.scope), values)
    };
    for (clause, parsed) in out.clauses.iter_mut().zip(&plan.clauses) {
        clause.matches = Some(in_scope(format!("{cols} : ({})", parsed.fts))?);
    }
    if !c.filters.is_empty() && (c.driver.is_some() || !c.text.is_empty()) {
        // Text conditions only reference the materialized matches; tier CTEs are unused here.
        let mut values = c.cte_values.clone();
        values.extend(c.scope_values.clone());
        out.text_only_matches = Some(count(db, &format!("{}SELECT count(*) {} WHERE 1 {}{}", c.with(), c.from(), c.scope, c.text), values)?);
    }
    let unmatched: Vec<&Clause> = out.clauses.iter().filter(|c| !c.excluded && c.kind == ClauseKind::Term && c.matches == Some(0)).take(4).collect();
    if unmatched.is_empty() {
        return Ok(());
    }
    db.execute_batch("CREATE VIRTUAL TABLE IF NOT EXISTS temp.image_search_vocab USING fts5vocab(main, image_search_fts, row)")?;
    let mut vocab: Vec<(String, i64)> = {
        let mut q = db.prepare("SELECT term,doc FROM temp.image_search_vocab WHERE length(term)>1")?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    vocab.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut suggestions = vec![];
    for clause in unmatched {
        let term = crate::search::deaccent(&clause.text.to_lowercase());
        let mut candidates = vec![];
        for (word, _) in vocab.iter().filter(|(w, _)| w != &term && edit_distance_ok(&term, w)) {
            // Verify inside scope: a word seen only in restricted sources is never offered.
            if in_scope(format!("{cols} : ({})", fts_string(word)))? > 0 {
                candidates.push(word.clone());
                if candidates.len() == MAX_SUGGESTIONS {
                    break;
                }
            }
        }
        if !candidates.is_empty() {
            suggestions.push(Suggestion { term: clause.text.clone(), candidates });
        }
    }
    out.suggestions = suggestions;
    Ok(())
}

/// Portable form of a saved image search (catalog v2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableSavedImageSearch {
    pub id: String,
    pub name: String,
    pub query: ImageQuery,
    pub created_at: i64,
    pub updated_at: i64,
}
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256 && name.trim() == name && !name.chars().any(char::is_control)
}
pub(crate) fn validate_portable_searches(
    searches: &[PortableSavedImageSearch],
    sources: &std::collections::BTreeMap<&String, &crate::portable::PortableSource>,
) -> crate::Result<()> {
    if searches.len() > MAX_SAVED_SEARCHES as usize {
        return Err(crate::invalid("Catalog exceeds saved image search limits"));
    }
    let (mut ids, mut names) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    for s in searches {
        let ok = Uuid::parse_str(&s.id).is_ok_and(|u| u.hyphenated().to_string() == s.id)
            && ids.insert(s.id.as_str())
            && valid_name(&s.name)
            && names.insert(tag_key(&s.name))
            && s.query.validate().is_ok()
            && serde_json::to_vec(&s.query).is_ok_and(|v| v.len() <= MAX_SAVED_QUERY_BYTES)
            && s.created_at <= s.updated_at;
        if !ok {
            return Err(crate::invalid("Invalid saved image search"));
        }
        if s.query.source_ids.iter().any(|id| !sources.contains_key(id)) {
            return Err(crate::invalid("Saved image search references unknown source"));
        }
    }
    Ok(())
}
pub(crate) fn export_portable_searches(db: &Connection) -> crate::Result<Vec<PortableSavedImageSearch>> {
    let mut q = db.prepare("SELECT id,name,query,created_at,updated_at FROM saved_image_searches ORDER BY name_key,id")?;
    let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get(3)?, r.get(4)?)))?;
    let mut out = vec![];
    for row in rows {
        let (id, name, query, created_at, updated_at) = row?;
        // A query this version cannot read fails the export rather than being dropped.
        let query: ImageQuery = serde_json::from_str(&query).map_err(|_| crate::invalid("Saved image search uses an unsupported query version"))?;
        out.push(PortableSavedImageSearch { id, name, query, created_at, updated_at });
    }
    Ok(out)
}
pub(crate) fn import_portable_searches(tx: &Connection, searches: &[PortableSavedImageSearch]) -> crate::Result<()> {
    for s in searches {
        let taken: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM saved_image_searches WHERE name_key=?1 OR id=?2)", params![tag_key(&s.name), s.id], |r| r.get(0))?;
        if taken {
            return Err(crate::invalid("A saved image search with this name already exists"));
        }
        tx.execute(
            "INSERT INTO saved_image_searches(id,name,name_key,query,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![s.id, s.name, tag_key(&s.name), serde_json::to_string(&s.query)?, s.created_at, s.updated_at],
        )?;
    }
    let n: i64 = tx.query_row("SELECT count(*) FROM saved_image_searches", [], |r| r.get(0))?;
    if n > MAX_SAVED_SEARCHES {
        return Err(crate::invalid("Catalog exceeds saved image search limits"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::Source,
        image_annotations::AnnotationPatch,
        image_contracts::{ConfidenceKind, Coverage},
    };
    use std::time::{Duration, Instant};

    struct Fx {
        dir: tempfile::TempDir,
        c: Catalog,
        a: Source,
        b: Source,
    }
    fn fixture() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let (ra, rb) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&ra).unwrap();
        std::fs::create_dir_all(&rb).unwrap();
        let c = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
        let (a, b) = (c.add_source(&ra).unwrap(), c.add_source(&rb).unwrap());
        Fx { dir, c, a, b }
    }
    fn profile(width: u32, height: u32, alpha: bool, transparency: Transparency) -> ImageProfile {
        ImageProfile {
            raw_width: width,
            raw_height: height,
            orientation: 1,
            width,
            height,
            format: if alpha { ImageFormat::Png } else { ImageFormat::Jpeg },
            encoded_bytes: 1000,
            bit_depth: 8,
            source_color_profile: None,
            alpha_channel: alpha,
            transparency,
            analyzer_version: "test-1".into(),
        }
    }
    impl Fx {
        /// A ready image with measured facts and creator annotations.
        fn image(&mut self, source: &Source, path: &str, size: Option<(u32, u32)>, patch: AnnotationPatch) -> String {
            let hash = blake3::hash(format!("{}{path}", source.id).as_bytes()).to_hex().to_string();
            let id = self.c.register_image(source, path, Some(&hash)).unwrap().header.id;
            if let Some((w, h)) = size {
                self.c.publish_image(source, &id, &hash, &profile(w, h, w == h, if w == h { Transparency::Partial } else { Transparency::None })).unwrap();
            }
            if patch != AnnotationPatch::default() {
                self.c.edit_image_annotations(&id, 1, &patch).unwrap();
            }
            id
        }
        fn caption(&mut self, id: &str, caption: &str) {
            let asset = self.c.image(id, None).unwrap();
            let mut notes = asset.annotations.clone();
            notes.caption = Some(caption.into());
            self.c.set_image_annotations(id, asset.header.content_hash.as_deref(), &notes).unwrap();
        }
        fn evidence(&self, id: &str, tags: &[&str], review: ReviewState, stale: bool) {
            let asset = self.c.image(id, None).unwrap();
            let e = Evidence {
                id: Uuid::new_v4().to_string(),
                content_hash: asset.header.content_hash.clone().unwrap(),
                origin: Origin::Model,
                coverage: Coverage::FullImage,
                tags: tags.iter().map(|t| t.to_string()).collect(),
                caption: None,
                model: None,
                confidence: None,
                confidence_kind: ConfidenceKind::Unavailable,
                review_state: review,
                stale,
                created_at_ms: 1,
            };
            self.c.db.execute("UPDATE image_assets SET evidence=?2 WHERE id=?1", params![id, serde_json::to_string(&vec![e]).unwrap()]).unwrap();
        }
        fn find(&self, q: ImageQuery) -> Vec<String> {
            self.page(q).items.into_iter().map(|i| i.header.relative_path).collect()
        }
        fn page(&self, q: ImageQuery) -> ImageSearchPage {
            self.c.search_images(None, &q, 0, 100, true).unwrap()
        }
    }
    fn text(t: &str) -> ImageQuery {
        ImageQuery { text: t.into(), ..Default::default() }
    }
    fn tags(t: &[&str]) -> AnnotationPatch {
        AnnotationPatch { add_tags: t.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }
    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    #[test]
    fn text_matches_whole_tokens_phrases_prefixes_and_exclusions() {
        let mut f = fixture();
        let a = f.a.clone();
        f.image(&a, "cat.png", Some((10, 10)), tags(&["sky blue", "cat"]));
        f.image(&a, "category.png", Some((10, 10)), tags(&["sky", "blue"]));
        f.image(&a, "dog.png", None, AnnotationPatch { comment: Some("A cat sits near the dog".into()), ..Default::default() });
        // Token boundaries: "cat" never matches "category" without an explicit prefix.
        assert_eq!(sorted(f.find(text("cat"))), ["cat.png", "dog.png"]);
        assert_eq!(sorted(f.find(text("cat*"))), ["cat.png", "category.png", "dog.png"]);
        // A phrase never spans two separate tags.
        assert_eq!(f.find(text("\"sky blue\"")), ["cat.png"]);
        assert_eq!(f.find(text("sky blue")), ["cat.png", "category.png"], "terms are AND-ed across fields");
        assert_eq!(f.find(text("cat -dog")), ["cat.png"], "exclusions apply to every searched field");
        assert_eq!(f.find(text("-cat")), ["category.png"]);
        // Field visibility: notes are searched by default and can be turned off.
        assert_eq!(f.find(ImageQuery { fields: vec![TextField::Tags, TextField::Name], ..text("cat") }), ["cat.png"]);
        let page = f.page(text("cat"));
        let dog = page.items.iter().position(|i| i.header.relative_path == "dog.png").unwrap();
        assert_eq!(page.explanations[dog].fields, [TextField::Notes]);
        assert_eq!(page.explanations[dog].tier, Some(4));
        assert_eq!(page.explanations[0].tier, Some(0), "exact tag ranks first");
        assert_eq!(page.explanations[0].fields, [TextField::Name, TextField::Tags]);
        let parsed = page.interpretation;
        assert_eq!(parsed.clauses.len(), 1);
        assert_eq!(f.page(text("\"\" -- ***")).interpretation.ignored, ["--", "***"]);
    }

    #[test]
    fn unicode_is_normalized_case_and_accent_folded_and_cjk_is_searchable() {
        let mut f = fixture();
        let a = f.a.clone();
        f.image(&a, "Café/Été.png", None, tags(&["Crème Brûlée"]));
        f.image(&a, "東京タワー.webp", None, AnnotationPatch::default());
        f.image(&a, "plain.png", None, tags(&["ÉCOLE"]));
        // Decomposed input (e + combining acute) finds the composed spelling.
        assert_eq!(f.find(text("cafe\u{301}")), ["Café/Été.png"]);
        assert_eq!(f.find(text("CAFE")), ["Café/Été.png"]);
        assert_eq!(f.find(text("\"creme brulee\"")), ["Café/Été.png"]);
        assert_eq!(f.find(text("école")), ["plain.png"]);
        assert_eq!(f.find(text("東京")), ["東京タワー.webp"]);
        assert_eq!(f.find(text("タワー")), ["東京タワー.webp"]);
        assert!(f.find(text("京東")).is_empty(), "CJK characters keep their order");
        // Exact tag filters are case-insensitive by tag identity, not substring.
        assert_eq!(f.find(ImageQuery { tags: vec!["crème brûlée".into()], ..Default::default() }), ["Café/Été.png"]);
        assert!(f.find(ImageQuery { tags: vec!["crème".into()], ..Default::default() }).is_empty());
    }

    #[test]
    fn hard_filters_never_treat_unknown_as_zero_false_or_approved() {
        let mut f = fixture();
        let a = f.a.clone();
        let rated = AnnotationPatch { rating: Some(Some(4)), favorite: Some(true), ..Default::default() };
        f.image(&a, "wide.jpg", Some((1920, 1080)), rated);
        f.image(&a, "tall.jpg", Some((1080, 1920)), AnnotationPatch { rating: Some(Some(1)), ..Default::default() });
        f.image(&a, "square.png", Some((512, 512)), AnnotationPatch::default());
        f.image(&a, "pending.png", None, AnnotationPatch::default());
        let all = |q: ImageQuery| sorted(f.find(q));
        assert_eq!(all(ImageQuery { min_rating: Some(1), ..Default::default() }), ["tall.jpg", "wide.jpg"]);
        assert_eq!(all(ImageQuery { max_rating: Some(3), ..Default::default() }), ["tall.jpg"], "unrated is not zero");
        assert_eq!(all(ImageQuery { rated: Some(false), ..Default::default() }), ["pending.png", "square.png"]);
        assert_eq!(all(ImageQuery { favorite: Some(false), ..Default::default() }).len(), 3);
        // Inclusive numeric boundaries; unknown dimensions never satisfy any bound.
        assert_eq!(all(ImageQuery { min_width: Some(1920), ..Default::default() }), ["wide.jpg"]);
        assert_eq!(all(ImageQuery { max_width: Some(1080), ..Default::default() }), ["square.png", "tall.jpg"]);
        assert_eq!(all(ImageQuery { max_height: Some(32_768), ..Default::default() }).len(), 3);
        assert_eq!(all(ImageQuery { orientations: vec![Orientation::Square, Orientation::Portrait], ..Default::default() }), ["square.png", "tall.jpg"]);
        assert_eq!(all(ImageQuery { alpha_channel: Some(false), ..Default::default() }), ["tall.jpg", "wide.jpg"]);
        assert_eq!(all(ImageQuery { transparency: vec![Transparency::Partial], ..Default::default() }), ["square.png"]);
        assert_eq!(all(ImageQuery { technical: vec![TechnicalState::Pending], ..Default::default() }), ["pending.png"]);
        assert_eq!(all(ImageQuery { formats: vec![ImageFormat::Png], ..Default::default() }), ["square.png"]);
        // Contradictions and out-of-range values are rejected, not silently empty.
        for bad in [
            ImageQuery { min_rating: Some(0), ..Default::default() },
            ImageQuery { min_rating: Some(4), max_rating: Some(2), ..Default::default() },
            ImageQuery { min_rating: Some(2), rated: Some(false), ..Default::default() },
            ImageQuery { min_width: Some(10), max_width: Some(5), ..Default::default() },
            ImageQuery { min_width: Some(MAX_DIMENSION + 1), ..Default::default() },
            ImageQuery { added_after: Some(5), added_before: Some(5), ..Default::default() },
            ImageQuery { tags: vec!["x".into()], exclude_tags: vec!["X".into()], ..Default::default() },
            ImageQuery { tags: vec![" ".into()], ..Default::default() },
        ] {
            assert_eq!(f.c.search_images(None, &bad, 0, 10, false).unwrap_err(), E::InvalidArgument, "{bad:?}");
        }
        // Sorting puts unknown values last, then the gallery's name order.
        let by = |sort| f.find(ImageQuery { sort, ..Default::default() });
        assert_eq!(by(ImageSort::RatingDesc), ["wide.jpg", "tall.jpg", "pending.png", "square.png"]);
        assert_eq!(by(ImageSort::PixelsDesc), ["tall.jpg", "wide.jpg", "square.png", "pending.png"]);
        assert_eq!(by(ImageSort::NameDesc), ["wide.jpg", "tall.jpg", "square.png", "pending.png"]);
    }

    #[test]
    fn suggested_and_confirmed_tags_stay_distinct_and_aliases_resolve() {
        let mut f = fixture();
        let a = f.a.clone();
        let user = f.image(&a, "user.png", None, tags(&["New York"]));
        let suggested = f.image(&a, "suggested.png", None, AnnotationPatch::default());
        let accepted = f.image(&a, "accepted.png", None, AnnotationPatch::default());
        let stale = f.image(&a, "stale.png", None, AnnotationPatch::default());
        let rejected = f.image(&a, "rejected.png", None, AnnotationPatch::default());
        f.evidence(&suggested, &["beach"], ReviewState::Suggested, false);
        f.evidence(&accepted, &["beach"], ReviewState::Accepted, false);
        f.evidence(&stale, &["beach"], ReviewState::Suggested, true);
        f.evidence(&rejected, &["beach"], ReviewState::Rejected, false);
        let tag = |state| sorted(f.find(ImageQuery { tags: vec!["Beach".into()], tag_state: state, ..Default::default() }));
        assert_eq!(tag(TagState::Confirmed), ["accepted.png"]);
        assert_eq!(tag(TagState::Suggested), ["suggested.png"]);
        assert_eq!(tag(TagState::Any), ["accepted.png", "suggested.png"]);
        // Suggestions are only searched as text when the caller opts in.
        assert_eq!(f.find(text("beach")), ["accepted.png"]);
        assert_eq!(sorted(f.find(ImageQuery { fields: vec![TextField::Tags, TextField::SuggestedTags], ..text("beach") })), ["accepted.png", "suggested.png"]);
        assert_eq!(f.find(ImageQuery { review: Some(ReviewFilter::NeedsReview), ..Default::default() }), ["suggested.png"]);
        // Rejected and stale evidence carry no provenance for the current content.
        assert_eq!(sorted(f.find(ImageQuery { provenance: vec![Origin::Model], ..Default::default() })), ["accepted.png", "suggested.png"]);
        let facets = f.page(ImageQuery::default()).facets.unwrap();
        assert_eq!(facets.suggested_tags, [FacetCount { value: "beach".into(), count: 1 }]);
        assert_eq!(facets.needs_review, 1);
        // Library rename keeps an alias; the old spelling still filters by the new tag.
        f.c.rename_image_tag("New York", "NYC", None, true).unwrap();
        let page = f.page(ImageQuery { tags: vec!["new york".into()], ..Default::default() });
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].header.id, user);
        assert_eq!(page.interpretation.tags, [ResolvedTag { requested: "new york".into(), tag: "NYC".into(), via_alias: true }]);
        assert_eq!(f.page(text("new york")).explanations.first().and_then(|e| e.tier), None, "the old words are gone from the text index");
    }

    #[test]
    fn edits_deletes_and_moves_are_visible_to_the_next_search() {
        let mut f = fixture();
        let a = f.a.clone();
        let id = f.image(&a, "one.png", None, AnnotationPatch::default());
        assert!(f.find(text("sunset")).is_empty());
        f.c.edit_image_annotations(&id, 1, &tags(&["sunset"])).unwrap();
        assert_eq!(f.find(text("sunset")), ["one.png"]);
        f.caption(&id, "Golden hour over water");
        assert_eq!(f.find(text("golden")), ["one.png"]);
        f.c.db.execute("UPDATE asset_registry SET relative_path='moved/renamed.png' WHERE id=?1", [&id]).unwrap();
        assert_eq!(f.find(text("renamed")), ["moved/renamed.png"]);
        assert!(f.find(text("one")).is_empty());
        let rev = f.c.image(&id, None).unwrap().annotations.revision;
        let hash = f.c.image(&id, None).unwrap().header.content_hash;
        f.c.delete_image(&a, &id, hash.as_deref(), rev).unwrap();
        assert!(f.find(text("sunset")).is_empty());
        let docs: i64 = f.c.db.query_row("SELECT count(*) FROM image_search_docs", [], |r| r.get(0)).unwrap();
        let fts: i64 = f.c.db.query_row("SELECT count(*) FROM image_search_fts", [], |r| r.get(0)).unwrap();
        assert_eq!((docs, fts), (0, 0), "deleted images leave no index rows");
        // Audio stays out of the image index entirely.
        assert_eq!(f.c.db.query_row::<i64, _, _>("SELECT count(*) FROM image_search_dirty", [], |r| r.get(0)).unwrap(), 0);
    }

    #[test]
    fn authorization_bounds_results_counts_facets_and_guidance() {
        let mut f = fixture();
        let (a, b) = (f.a.clone(), f.b.clone());
        f.image(&a, "public.png", Some((10, 10)), tags(&["shared"]));
        f.image(&b, "secret-sunset.png", Some((10, 10)), tags(&["shared", "classified"]));
        let allowed = [a.id.clone()];
        let run = |q: &ImageQuery| f.c.search_images(Some(&allowed), q, 0, 100, true).unwrap();
        // Asking for a restricted source is indistinguishable from an empty one.
        let page = run(&ImageQuery { source_ids: vec![b.id.clone()], ..Default::default() });
        assert_eq!((page.total, page.facets.clone().unwrap()), (0, ImageFacets::default()));
        let page = run(&ImageQuery { source_ids: vec![a.id.clone(), b.id.clone()], ..Default::default() });
        assert_eq!(page.total, 1);
        let facets = page.facets.unwrap();
        assert_eq!(facets.tags, [FacetCount { value: "shared".into(), count: 1 }]);
        assert_eq!(facets.sources, [FacetCount { value: a.id.clone(), count: 1 }]);
        // Per-term counts and spelling candidates never reveal restricted words.
        let page = run(&text("classifed"));
        assert_eq!(page.interpretation.clauses[0].matches, Some(0));
        assert!(page.interpretation.suggestions.is_empty());
        let owner = f.c.search_images(None, &text("classifed"), 0, 10, false).unwrap();
        assert_eq!(owner.interpretation.suggestions, [Suggestion { term: "classifed".into(), candidates: vec!["classified".into()] }]);
        assert!(run(&text("sunset")).items.is_empty());
        // Selected-file sources list only their exact members.
        f.c.db.execute("UPDATE sources SET scope='files' WHERE id=?1", [&a.id]).unwrap();
        assert_eq!(run(&ImageQuery::default()).total, 0);
        f.c.db.execute("INSERT INTO source_files(source_id,relative_path) VALUES(?1,'public.png')", [&a.id]).unwrap();
        assert_eq!(run(&ImageQuery::default()).total, 1);
    }

    #[test]
    fn ready_selection_excludes_missing_failed_pending_and_offline_images() {
        let mut f = fixture();
        let (a, b) = (f.a.clone(), f.b.clone());
        f.image(&a, "ready.png", Some((4, 4)), AnnotationPatch::default());
        let missing = f.image(&a, "missing.png", Some((4, 4)), AnnotationPatch::default());
        let failed = f.image(&a, "failed.png", None, AnnotationPatch::default());
        f.image(&a, "pending.png", None, AnnotationPatch::default());
        f.image(&b, "offline.png", Some((4, 4)), AnnotationPatch::default());
        f.c.db.execute("UPDATE image_assets SET availability='missing' WHERE id=?1", [&missing]).unwrap();
        f.c.db.execute("UPDATE image_assets SET technical_state='failed' WHERE id=?1", [&failed]).unwrap();
        f.c.db.execute("UPDATE sources SET available=0 WHERE id=?1", [&b.id]).unwrap();
        assert_eq!(f.find(ImageQuery { ready_only: true, ..Default::default() }), ["ready.png"]);
        assert_eq!(f.find(ImageQuery { availability: vec![Availability::Offline], ..Default::default() }), ["offline.png"]);
        let facets = f.page(ImageQuery::default()).facets.unwrap();
        assert!(facets.availability.contains(&FacetCount { value: "offline".into(), count: 1 }));
        assert!(facets.technical.contains(&FacetCount { value: "failed".into(), count: 1 }));
    }

    #[test]
    fn windows_are_stable_and_stitch_without_gaps() {
        let mut f = fixture();
        let a = f.a.clone();
        for i in 0..23 {
            let r = AnnotationPatch { rating: Some(Some((i % 5) as u8 + 1)), ..tags(&["set"]) };
            f.image(&a, &format!("{:02}-Same.png", i % 7), None, AnnotationPatch::default());
            f.image(&a, &format!("dir{i}/x.png"), None, r);
        }
        for sort in [ImageSort::Relevance, ImageSort::RatingDesc, ImageSort::NameDesc, ImageSort::AddedDesc] {
            let q = ImageQuery { sort, ..text("png") };
            let all: Vec<String> = f.c.search_images(None, &q, 0, 100, false).unwrap().items.into_iter().map(|i| i.header.id).collect();
            let mut stitched = vec![];
            for offset in (0..all.len() as u32).step_by(6) {
                stitched.extend(f.c.search_images(None, &q, offset, 6, false).unwrap().items.into_iter().map(|i| i.header.id));
            }
            assert_eq!(stitched, all, "{sort:?}");
            assert_eq!(f.c.search_images(None, &q, 0, 100, false).unwrap().items.into_iter().map(|i| i.header.id).collect::<Vec<_>>(), all);
        }
        for (offset, limit) in [(0, 0), (0, MAX_PAGE_SIZE + 1), (MAX_GALLERY_OFFSET + 1, 10)] {
            assert!(f.c.search_images(None, &ImageQuery::default(), offset, limit, false).is_err());
        }
        // The unfiltered owner path is the gallery's and agrees with it exactly.
        let gallery = f.c.image_gallery_page(None, GallerySort::NameAsc, 0, 100).unwrap();
        let search = f.c.search_images(None, &ImageQuery::default(), 0, 100, false).unwrap();
        let faceted = f.c.search_images(None, &ImageQuery::default(), 0, 100, true).unwrap();
        assert_eq!((search.total, &search.items), (gallery.total, &gallery.items));
        assert_eq!((faceted.total, &faceted.items), (gallery.total, &gallery.items));
    }

    #[test]
    fn contract_rejects_unknown_fields_versions_and_reserved_fields() {
        let q: ImageQuery = serde_json::from_str(r#"{"text":"x","min_width":5}"#).unwrap();
        assert_eq!((q.version, q.min_width, q.sort), (QUERY_VERSION, Some(5), ImageSort::Relevance));
        assert!(serde_json::from_str::<ImageQuery>(r#"{"semantic":true}"#).is_err());
        assert!(serde_json::from_str::<ImageQuery>(r#"{"fields":["vibes"]}"#).is_err());
        assert!(serde_json::from_str::<ImageQuery>(r#"{"min_width":-1}"#).is_err());
        assert_eq!(ImageQuery { version: 2, ..Default::default() }.validate(), Err(E::UnsupportedVersion));
        for field in [TextField::Ocr, TextField::Generation, TextField::UsageNotes] {
            assert_eq!(ImageQuery { fields: vec![field], ..Default::default() }.validate(), Err(E::UnsupportedField));
        }
        assert_eq!(text(&(0..40).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ")).validate(), Err(E::LimitExceeded));
        assert_eq!(text(&"x".repeat(MAX_TEXT_CHARS + 1)).validate(), Err(E::LimitExceeded));
        assert_eq!(ImageQuery { tags: vec!["t".into(); MAX_FILTER_TAGS + 1], ..Default::default() }.validate(), Err(E::LimitExceeded));
        let caps = capabilities();
        assert_eq!(caps.iter().filter(|c| c.available).count(), 5);
        assert_eq!(caps.iter().find(|c| c.field == TextField::Ocr).unwrap().owner.as_deref(), Some("SS-057"));
        // Canonical serialization is stable (smart collections and saved searches store it).
        let json = serde_json::to_value(ImageQuery::default()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["sort"], "relevance");
        assert_eq!(serde_json::from_value::<ImageQuery>(json).unwrap(), ImageQuery::default());
    }

    #[test]
    fn no_match_guidance_reports_counts_and_text_only_matches() {
        let mut f = fixture();
        let a = f.a.clone();
        f.image(&a, "beach.png", Some((5, 5)), tags(&["beach"]));
        let page = f.page(ImageQuery { min_width: Some(100), ..text("beach") });
        assert_eq!(page.total, 0);
        assert_eq!(page.interpretation.clauses[0].matches, Some(1));
        assert_eq!(page.interpretation.text_only_matches, Some(1));
        let page = f.page(text("beach zebra"));
        assert_eq!(page.interpretation.clauses.iter().map(|c| c.matches).collect::<Vec<_>>(), [Some(1), Some(0)]);
        assert!(page.interpretation.suggestions.is_empty(), "no near word exists");
    }

    #[test]
    fn saved_searches_round_trip_portably_and_stay_out_of_source_snapshots() {
        let mut f = fixture();
        let a = f.a.clone();
        f.image(&a, "x.png", Some((5, 5)), tags(&["beach"]));
        let q = ImageQuery { tags: vec!["beach".into()], min_width: Some(2), source_ids: vec![a.id.clone()], ..Default::default() };
        let saved = f.c.save_image_search("  Beach   picks ", &q, false).unwrap();
        assert_eq!(saved.name, "Beach picks");
        assert_eq!(f.c.save_image_search("beach PICKS", &q, false).unwrap_err(), E::Conflict);
        let replaced = f.c.save_image_search("beach PICKS", &ImageQuery { min_width: Some(3), ..q.clone() }, true).unwrap();
        assert_eq!((replaced.id.as_str(), replaced.name.as_str()), (saved.id.as_str(), "beach PICKS"));
        assert_eq!(f.c.save_image_search("bad", &ImageQuery { fields: vec![TextField::Ocr], ..Default::default() }, false).unwrap_err(), E::UnsupportedField);
        // A future-version query is listed as unsupported, never rewritten or dropped.
        f.c.db.execute("INSERT INTO saved_image_searches VALUES('00000000-0000-4000-8000-000000000001','Future','future','{\"version\":9}',1,1)", []).unwrap();
        let list = f.c.saved_image_searches().unwrap();
        assert_eq!(list.iter().map(|s| (s.name.as_str(), s.query.is_some())).collect::<Vec<_>>(), [("beach PICKS", true), ("Future", false)]);
        assert!(f.c.export_portable().is_err(), "an unreadable saved search fails export instead of vanishing");
        f.c.delete_saved_image_search("00000000-0000-4000-8000-000000000001").unwrap();
        assert_eq!(f.c.delete_saved_image_search("00000000-0000-4000-8000-000000000001").unwrap_err(), E::NotFound);
        let exported = f.c.export_portable().unwrap();
        assert_eq!(exported.schema, crate::portable::PORTABLE_SCHEMA_V2);
        assert_eq!(exported.saved_image_searches.len(), 1);
        assert!(f.c.export_portable_for_source(&a.id).unwrap().saved_image_searches.is_empty());
        // Restore into a fresh catalog; the search runs against restored records.
        let other = tempfile::tempdir().unwrap();
        let mut restored = Catalog::open(&other.path().join("r.sqlite")).unwrap();
        let roots = [(a.id.clone(), f.a.root.clone()), (f.b.id.clone(), f.b.root.clone())].into_iter().collect();
        restored.import_portable(&exported, &roots, other.path()).unwrap();
        let back = restored.saved_image_searches().unwrap();
        assert_eq!(back[0].query.as_ref().unwrap().min_width, Some(3));
        assert_eq!(restored.search_images(None, back[0].query.as_ref().unwrap(), 0, 10, false).unwrap().total, 1);
        // Importing the same catalog again cannot silently overwrite a saved search.
        let mut doc = exported.clone();
        doc.saved_image_searches[0].query.source_ids = vec!["unknown".into()];
        assert!(doc.validate().is_err());
        let mut v1 = exported.clone();
        v1.schema = crate::portable::PORTABLE_SCHEMA.into();
        v1.images.clear();
        v1.image_tag_aliases.clear();
        assert!(v1.validate().is_err(), "saved image searches require catalog v2");
        let _ = f.dir.path();
    }

    fn v15_database(path: &std::path::Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        for schema in [
            include_str!("schema.sql"),
            include_str!("source_catalog_schema.sql"),
            include_str!("settings_schema.sql"),
            include_str!("search_profile_schema.sql"),
            include_str!("image_schema.sql"),
            include_str!("image_jobs_schema.sql"),
            include_str!("gallery_schema.sql"),
            include_str!("annotation_schema.sql"),
            include_str!("image_refresh_schema.sql"),
            include_str!("mixed_catalog_schema.sql"),
        ] {
            db.execute_batch(schema).unwrap();
        }
        db.pragma_update(None, "user_version", 15).unwrap();
        db
    }
    fn master(db: &Connection) -> Vec<(String, String)> {
        let mut q = db.prepare("SELECT name,replace(replace(coalesce(sql,''),' ',''),char(10),'') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name").unwrap();
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        rows.collect::<std::result::Result<_, _>>().unwrap()
    }
    #[test]
    fn genuine_v15_upgrade_indexes_existing_images_with_a_backup() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v15.sqlite");
        let mut old = Catalog { db: v15_database(&path) };
        let source = old.add_source(d.path()).unwrap();
        let image = old.register_image(&source, "Harbour.png", Some(&"a".repeat(64))).unwrap();
        let mut notes = image.annotations.clone();
        notes.tags = vec!["boats".into()];
        let image = old.set_image_annotations(&image.header.id, Some(&"a".repeat(64)), &notes).unwrap();
        drop(old);
        let c = Catalog::open(&path).unwrap();
        assert_eq!(c.db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), crate::catalog::SCHEMA_VERSION);
        assert_eq!(c.image(&image.header.id, None).unwrap(), image);
        // Indexed during the upgrade: nothing is left for the first search to do.
        assert_eq!(c.db.query_row::<i64, _, _>("SELECT count(*) FROM image_search_dirty", [], |r| r.get(0)).unwrap(), 0);
        assert_eq!(c.search_images(None, &text("boats harbour"), 0, 10, false).unwrap().total, 1);
        let fresh = Catalog::open(std::path::Path::new(":memory:")).unwrap();
        assert_eq!(master(&c.db), master(&fresh.db), "restore compares schemas exactly");
        let backup = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains(&format!("pre-v{}-", crate::catalog::SCHEMA_VERSION))).unwrap();
        let db = Connection::open(backup).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 15);
        assert_eq!(db.query_row::<i64, _, _>("SELECT count(*) FROM sqlite_master WHERE name LIKE 'image_search%'", [], |r| r.get(0)).unwrap(), 0);
    }
    #[test]
    fn v16_failure_rolls_back_and_keeps_the_v15_catalog_usable() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v15.sqlite");
        let db = v15_database(&path);
        db.execute_batch("CREATE TABLE saved_image_searches(collision TEXT); INSERT INTO saved_image_searches VALUES('kept');").unwrap();
        drop(db);
        assert!(Catalog::open(&path).is_err());
        let db = Connection::open(&path).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 15);
        assert_eq!(db.query_row::<String, _, _>("SELECT collision FROM saved_image_searches", [], |r| r.get(0)).unwrap(), "kept");
        assert_eq!(db.query_row::<i64, _, _>("SELECT count(*) FROM sqlite_master WHERE name LIKE 'image_search%'", [], |r| r.get(0)).unwrap(), 0);
    }

    /// Synthetic ready images: `n` rows over 1,000 distinct analyses, with tags,
    /// ratings, notes and favorites varied deterministically (no media).
    fn synthetic(c: &Catalog, source: &str, n: u32) {
        let tx = c.db.unchecked_transaction().unwrap();
        tx.execute(
            "WITH RECURSIVE n(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM n WHERE i<999)
             INSERT INTO image_contents SELECT printf('%064x',i) FROM n",
            [],
        )
        .unwrap();
        for i in 0..1000u32 {
            let (w, h) = (100 + i * 7 % 3000, 100 + i * 13 % 3000);
            let p = ImageProfile { analyzer_version: "synthetic".into(), ..profile(w, h, i % 4 == 0, if i % 8 == 0 { Transparency::Partial } else { Transparency::None }) };
            tx.execute("INSERT INTO image_analyses VALUES(printf('%064x',?1),'synthetic',?2)", params![i, serde_json::to_string(&p).unwrap()]).unwrap();
        }
        tx.execute(
            "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?1)
             INSERT INTO asset_registry(id,kind,source_id,relative_path)
             SELECT printf('00000000-0000-4000-8000-%012d',i),'image',?2,printf('shoot-%03d/IMG_%06d-夕焼け.png',i%500,i) FROM n",
            params![n, source],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO image_assets(id,content_hash,analyzer_version,technical_state,availability,enrichment_state,annotation_revision,annotations)
             SELECT id,printf('%064x',rowid%1000),'synthetic','ready','available','disabled',1,
               json_object('revision',1,
                 'tags',json_array(CASE rowid%10 WHEN 0 THEN 'sunset' WHEN 1 THEN 'beach' ELSE 'studio' END,printf('series %d',rowid%200)),
                 'comment',CASE WHEN rowid%97=0 THEN 'client approved hero shot' ELSE '' END,
                 'favorite',json(CASE WHEN rowid%50=0 THEN 'true' ELSE 'false' END),
                 'rating',CASE WHEN rowid%3=0 THEN NULL ELSE rowid%5+1 END,'caption',NULL)
             FROM asset_registry WHERE kind='image' AND source_id=?1",
            [source],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO image_asset_versions SELECT i.id,i.content_hash,1000000+r.rowid FROM image_assets i JOIN asset_registry r ON r.id=i.id WHERE r.source_id=?1",
            [source],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    fn plan(c: &Catalog, sql: &str, values: Vec<Value>) -> Vec<String> {
        let mut q = c.db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let rows = q.query_map(params_from_iter(values), |r| r.get(3)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    /// T3/R7 scale evidence at 10K and the 100K image cap: index build cost,
    /// representative query latency and plans (no JSON or media access per row).
    /// Debug builds (CI) run 10K; the 100K figures in docs/reviews/SS-058.md come from
    /// `cargo test --release -p soundshelf-core --lib large_libraries -- --nocapture`.
    #[test]
    fn large_libraries_search_through_indexes_within_budget() {
        let sizes: &[u32] = if cfg!(debug_assertions) { &[10_000] } else { &[10_000, 100_000] };
        for &n in sizes {
            let mut f = fixture();
            let a = f.a.clone();
            synthetic(&f.c, &a.id, n);
            let start = Instant::now();
            let indexed = f.c.refresh_image_search_index().unwrap();
            let build = start.elapsed();
            assert_eq!(indexed, n as usize);
            let queries: Vec<(&str, ImageQuery)> = vec![
                ("rare note term", text("approved")),
                ("common tag term", text("studio")),
                ("CJK substring", text("夕焼")),
                ("phrase + exclusion", text("\"series 7\" -beach")),
                ("prefix", text("shoot-01*")),
                ("exact tag filter", ImageQuery { tags: vec!["Sunset".into()], ..Default::default() }),
                ("structural filters", ImageQuery { min_rating: Some(4), orientations: vec![Orientation::Landscape], min_width: Some(1000), ..Default::default() }),
                ("rating sort, deep window", ImageQuery { sort: ImageSort::RatingDesc, ..Default::default() }),
                ("no match + guidance", text("sunsett")),
            ];
            let mut worst = Duration::ZERO;
            for (label, q) in &queries {
                for (offset, facets) in [(0, true), (n / 2 / 100 * 100, false)] {
                    let start = Instant::now();
                    let page = f.c.search_images(None, q, offset.min(MAX_GALLERY_OFFSET), 100, facets).unwrap();
                    let elapsed = start.elapsed();
                    worst = worst.max(elapsed);
                    eprintln!("SS-058 {n} images · {label} · offset {offset} · facets {facets}: {} hits in {elapsed:?}", page.total);
                }
            }
            eprintln!("SS-058 {n} images: index build {build:?}, worst query {worst:?}");
            // Generous debug-build ceilings that still catch a regression to row-by-row JSON scans.
            assert!(build < Duration::from_secs(60), "{build:?}");
            assert!(worst < Duration::from_secs(5), "{worst:?}");
            // An edit re-indexes one document, not the library.
            let id = format!("00000000-0000-4000-8000-{:012}", 1);
            f.c.edit_image_annotations(&id, 1, &tags(&["fresh-edit"])).unwrap();
            let start = Instant::now();
            assert_eq!(f.c.search_images(None, &text("fresh"), 0, 10, false).unwrap().total, 1);
            eprintln!("SS-058 {n} images: search after one edit {:?}", start.elapsed());
            // Plans of the exact compiled statements: text is driven by the FTS
            // match set with primary-key joins; tag filters probe the tag key index.
            let compiled = |q: &ImageQuery| {
                let plan_ = q.plan().unwrap();
                let (c, _) = compile(&plan_, None, &HashMap::new()).unwrap();
                let (sql, values) = c.hits();
                plan(&f.c, &format!("{}{sql}", c.with()), values)
            };
            let p = compiled(&text("approved"));
            assert!(p.iter().any(|l| l.contains("VIRTUAL TABLE INDEX")), "{p:?}");
            assert!(p.iter().any(|l| l == "SCAN m0"), "{p:?}");
            assert!(p.iter().any(|l| l.starts_with("SEARCH d USING INTEGER PRIMARY KEY")), "{p:?}");
            let scans = |p: &[String], tables: &[&str]| p.iter().any(|l| l.starts_with("SCAN ") && tables.contains(&l.split_whitespace().nth(1).unwrap_or("")));
            assert!(!scans(&p, &["d", "r", "i", "s", "image_assets"]), "{p:?}");
            let p = compiled(&ImageQuery { tags: vec!["sunset".into()], ..Default::default() });
            assert!(p.iter().any(|l| l.contains("image_search_tags USING PRIMARY KEY")) && p.iter().any(|l| l == "SCAN m0"), "{p:?}");
            assert!(!scans(&p, &["i", "image_assets"]) && !p.iter().any(|l| l.contains("image_analyses")), "{p:?}");
        }
    }
}
