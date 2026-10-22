//! SS-055 prompt provenance in the catalog: content-bound embedded claims,
//! asset-bound sidecar claims and user-authored provenance, merged into one
//! attributed view. See docs/GENERATION_METADATA.md.
//!
//! Imported claims and user entries are stored apart. Re-reading a changed file
//! or sidecar replaces only the claim; user values and confirmations are never
//! rewritten by it. A confirmation names the exact claim versions it confirmed,
//! so a later claim shows as unconfirmed instead of inheriting approval.
use crate::{
    catalog::{contained, Catalog},
    image_generation::{
        extract_sidecar, local_path, ClaimSource, ExtractionStatus, GenerationClaim, GenerationExtraction,
        GenerationField, GenerationSharing, MAX_PROMPT_BYTES, MAX_SHORT_BYTES, MAX_SIDECAR_BYTES, PARSER,
    },
    invalid,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    io::Read,
    path::Path,
};

/// Serialized user record (enforced by SQLite too).
pub const MAX_PROVENANCE_BYTES: usize = 256 * 1024;
/// Parent hops followed for derivative inheritance (lineage is bounded at 1,000).
pub const MAX_INHERITANCE_DEPTH: usize = 32;
const SETTINGS_KEY: &str = "image_generation";

fn nullable<'de, D, T>(d: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProvenanceError {
    #[error("Invalid provenance request")]
    InvalidArgument,
    #[error("Image not found")]
    NotFound,
    #[error("Image is outside its approved scope")]
    PermissionDenied,
    #[error("Provenance changed elsewhere; refresh and retry")]
    Conflict,
    #[error("Source is offline or the original is unavailable")]
    SourceUnavailable,
    #[error("Provenance exceeds a limit")]
    LimitExceeded,
    #[error("Provenance operation failed")]
    Internal,
}
impl From<rusqlite::Error> for ProvenanceError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Internal
    }
}
impl From<serde_json::Error> for ProvenanceError {
    fn from(_: serde_json::Error) -> Self {
        Self::Internal
    }
}
type Result<T> = std::result::Result<T, ProvenanceError>;
use ProvenanceError as E;

/// Fields a person can enter or correct. Settings, references and the IPTC
/// source type are imported-only; `AiGenerated` is user-only.
pub const USER_FIELDS: [GenerationField; 8] = [
    GenerationField::Prompt,
    GenerationField::NegativePrompt,
    GenerationField::Generator,
    GenerationField::Model,
    GenerationField::Seed,
    GenerationField::Sampler,
    GenerationField::Created,
    GenerationField::AiGenerated,
];

/// The imported claim versions a person confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmation {
    #[serde(deserialize_with = "nullable")]
    pub content_hash: Option<String>,
    pub parser: String,
    #[serde(deserialize_with = "nullable")]
    pub sidecar_revision: Option<u32>,
    pub confirmed_at: i64,
}
/// User-authored provenance. A field absent from `fields` was never entered;
/// an empty string is an explicit "recorded as empty".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserProvenance {
    pub revision: u32,
    pub fields: BTreeMap<GenerationField, String>,
    #[serde(deserialize_with = "nullable")]
    pub confirmed: Option<Confirmation>,
    /// Fields removed from this record by a shared export.
    pub redacted: Vec<GenerationField>,
}
impl UserProvenance {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        if self.revision == 0 || self.redacted.len() > 16 {
            return Err("Invalid provenance record");
        }
        for (field, value) in &self.fields {
            if !USER_FIELDS.contains(field) {
                return Err("Field is not user-editable");
            }
            validate_value(*field, value).map_err(|_| "Invalid provenance value")?;
        }
        if let Some(c) = &self.confirmed {
            if c.parser.is_empty() || c.parser.len() > 256 || c.content_hash.as_ref().is_some_and(|h| !digest(h)) {
                return Err("Invalid provenance confirmation");
            }
        }
        if serde_json::to_vec(self).map_or(true, |b| b.len() > MAX_PROVENANCE_BYTES) {
            return Err("Provenance record is too large");
        }
        Ok(())
    }
}
fn digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn validate_value(field: GenerationField, value: &str) -> Result<()> {
    let max = if matches!(field, GenerationField::Prompt | GenerationField::NegativePrompt) {
        MAX_PROMPT_BYTES
    } else {
        MAX_SHORT_BYTES
    };
    if value.len() > max {
        return Err(E::LimitExceeded);
    }
    let multiline = matches!(field, GenerationField::Prompt | GenerationField::NegativePrompt);
    if value.chars().any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t'))) {
        return Err(E::InvalidArgument);
    }
    if field == GenerationField::AiGenerated && !matches!(value, "yes" | "no") {
        return Err(E::InvalidArgument);
    }
    Ok(())
}

/// One edit from the revision the editor last read. `set` maps a field to a
/// value (stored as entered) or to `null` (remove the user's value so the
/// imported claim, if any, shows again).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenancePatch {
    #[serde(default)]
    pub set: BTreeMap<GenerationField, Option<String>>,
    /// `true` confirms the claims currently shown; `false` withdraws confirmation.
    #[serde(default)]
    pub confirm: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldOrigin {
    User,
    Imported,
    /// From the parent of a derived element (`inherited_from`).
    Inherited,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldView {
    pub field: GenerationField,
    pub value: Option<String>,
    pub origin: FieldOrigin,
    /// Which claim supplied an imported value.
    pub source: Option<ClaimSource>,
    /// Another claim records a different value.
    pub conflicting: bool,
    /// Removed by a shared export before this library imported it.
    pub redacted: bool,
    /// A date with no recorded time zone.
    pub uncertain: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SidecarView {
    pub relative_path: String,
    pub present: bool,
    pub revision: u32,
    pub extraction: GenerationExtraction,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvenanceView {
    pub asset_id: String,
    /// User record revision for optimistic edits; 0 before the first edit.
    pub revision: u32,
    /// `None`: this content version has not been read for metadata yet.
    pub embedded: Option<GenerationExtraction>,
    pub sidecar: Option<SidecarView>,
    pub fields: Vec<FieldView>,
    /// Every claim behind the view, for origin visibility.
    pub claims: Vec<GenerationClaim>,
    /// Imported claims exist and the person confirmed these exact versions.
    pub confirmed: bool,
    /// The person once confirmed an earlier version of the claims.
    pub confirmation_outdated: bool,
    pub inherited_from: Option<String>,
    pub editable: bool,
    pub settings: GenerationSettings,
}

/// Which generation text later image search (SS-058) may index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchPolicy {
    /// Nothing from generation metadata is searchable.
    Exclude,
    /// Generator, model, sampler, seed and non-prompt settings.
    #[default]
    Metadata,
    /// Metadata plus prompts and negative prompts.
    Prompts,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationSettings {
    pub search: SearchPolicy,
    /// What `.creativeshelf/catalog.json` folder snapshots carry.
    pub snapshot: GenerationSharing,
}

/// The stored extraction for this content, or `None` when it must be (re)read:
/// never read, read by an older parser, or imported with redactions.
pub(crate) fn needs_extraction(db: &Connection, hash: &str) -> crate::Result<bool> {
    let e: Option<String> = db
        .query_row(
            "SELECT extraction FROM image_generation_extractions WHERE content_hash=?1 AND parser_version=?2",
            params![hash, PARSER],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match e {
        None => true,
        Some(e) => serde_json::from_str::<GenerationExtraction>(&e).map_or(true, |e| !e.redacted.is_empty()),
    })
}
/// Content-bound: identical bytes read by the same parser give the same claim.
/// A redacted import is replaced once the real bytes are read.
pub(crate) fn store_extraction(db: &Connection, hash: &str, e: &GenerationExtraction) -> crate::Result<()> {
    e.validate().map_err(invalid)?;
    db.execute(
        "INSERT INTO image_generation_extractions(content_hash,parser_version,extraction) VALUES(?1,?2,?3)
         ON CONFLICT(content_hash,parser_version) DO UPDATE SET extraction=excluded.extraction
         WHERE json_array_length(json_extract(image_generation_extractions.extraction,'$.redacted'))>0",
        params![hash, e.parser, serde_json::to_string(e)?],
    )?;
    Ok(())
}
fn extraction(db: &Connection, hash: &str) -> Result<Option<GenerationExtraction>> {
    let e: Option<String> = db
        .query_row(
            "SELECT extraction FROM image_generation_extractions WHERE content_hash=?1 AND parser_version=?2",
            params![hash, PARSER],
            |r| r.get(0),
        )
        .optional()?;
    Ok(e.map(|e| serde_json::from_str(&e)).transpose()?)
}

/// The A1111 sidecar path for an image: `<stem>.txt` in the same folder.
pub fn sidecar_path(relative: &str) -> Option<String> {
    let (dir, name) = relative.rsplit_once('/').map_or(("", relative), |(d, n)| (d, n));
    let stem = name.rsplit_once('.').map(|(s, _)| s).filter(|s| !s.is_empty())?;
    let file = format!("{stem}.txt");
    Some(if dir.is_empty() { file } else { format!("{dir}/{file}") })
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SidecarRow {
    relative_path: String,
    present: bool,
    hash: Option<String>,
    revision: u32,
    size: Option<i64>,
    modified: Option<i64>,
}
pub(crate) fn sidecar_row(db: &Connection, asset_id: &str) -> crate::Result<Option<SidecarRow>> {
    Ok(db
        .query_row(
            "SELECT relative_path,present,sidecar_hash,revision,size,modified FROM image_generation_sidecars WHERE asset_id=?1",
            [asset_id],
            |r| {
                Ok(SidecarRow {
                    relative_path: r.get(0)?,
                    present: r.get(1)?,
                    hash: r.get(2)?,
                    revision: r.get(3)?,
                    size: r.get(4)?,
                    modified: r.get(5)?,
                })
            },
        )
        .optional()?)
}
/// Filesystem observation of one sidecar, made without the catalog lock.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SidecarObservation {
    /// Exact-file sources never read siblings; nothing is recorded.
    NotAuthorized,
    Absent,
    /// Same size and modification time as the stored row.
    Unchanged,
    Read {
        hash: Option<String>,
        size: i64,
        modified: i64,
        extraction: GenerationExtraction,
    },
}
fn stat(path: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(path).ok()?;
    let modified = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())?;
    Some((i64::try_from(m.len()).ok()?, modified))
}
/// Reads the sidecar beside `image_relative` when the source grant covers it:
/// a folder source's recursive grant does; exact-file selections never expand
/// to siblings. Containment is resolved again here (symlinks included).
pub(crate) fn observe_sidecar(
    root: &Path,
    folder_scope: bool,
    image_relative: &str,
    prior: Option<&SidecarRow>,
) -> SidecarObservation {
    if !folder_scope {
        return SidecarObservation::NotAuthorized;
    }
    let Some(relative) = sidecar_path(image_relative) else { return SidecarObservation::Absent };
    let Ok(path) = contained(root, &relative) else { return SidecarObservation::Absent };
    let Some((size, modified)) = stat(&path) else { return SidecarObservation::Absent };
    if prior.is_some_and(|p| p.present && p.relative_path == relative && p.size == Some(size) && p.modified == Some(modified)) {
        return SidecarObservation::Unchanged;
    }
    let name = relative.rsplit('/').next().unwrap_or(&relative);
    if size as u64 > MAX_SIDECAR_BYTES {
        return SidecarObservation::Read {
            hash: None,
            size,
            modified,
            extraction: extract_sidecar(name, &vec![0; MAX_SIDECAR_BYTES as usize + 1]),
        };
    }
    let mut bytes = Vec::new();
    let read = std::fs::File::open(&path).and_then(|f| f.take(MAX_SIDECAR_BYTES + 1).read_to_end(&mut bytes));
    if read.is_err() {
        return SidecarObservation::Absent;
    }
    // A write while reading leaves the old row; the next check sees a new stat.
    if stat(&path) != Some((size, modified)) || bytes.len() as i64 != size {
        return SidecarObservation::Unchanged;
    }
    SidecarObservation::Read {
        hash: Some(blake3::hash(&bytes).to_hex().to_string()),
        size,
        modified,
        extraction: extract_sidecar(name, &bytes),
    }
}
/// Records an observation unless the row changed since `prior` was read.
/// A changed sidecar gets a new revision; the same bytes only refresh the stat.
pub(crate) fn apply_sidecar(
    db: &Connection,
    asset_id: &str,
    image_relative: &str,
    prior: Option<&SidecarRow>,
    seen: SidecarObservation,
) -> crate::Result<bool> {
    let current = sidecar_row(db, asset_id)?;
    if current.as_ref() != prior {
        return Ok(false);
    }
    let now = crate::jobs::now_secs();
    match (seen, current) {
        (SidecarObservation::NotAuthorized | SidecarObservation::Unchanged, _) | (SidecarObservation::Absent, None) => Ok(false),
        (SidecarObservation::Absent, Some(row)) if !row.present => Ok(false),
        (SidecarObservation::Absent, Some(row)) => {
            db.execute(
                "UPDATE image_generation_sidecars SET present=0,revision=?2,observed_at=?3,size=NULL,modified=NULL WHERE asset_id=?1",
                params![asset_id, row.revision.checked_add(1).ok_or_else(|| invalid("Sidecar revision overflow"))?, now],
            )?;
            Ok(true)
        }
        (SidecarObservation::Read { hash, size, modified, extraction }, current) => {
            extraction.validate().map_err(invalid)?;
            let relative = sidecar_path(image_relative).ok_or_else(|| invalid("Invalid sidecar path"))?;
            if let Some(row) = &current {
                if row.present && row.hash.is_some() && row.hash == hash && row.relative_path == relative {
                    db.execute("UPDATE image_generation_sidecars SET size=?2,modified=?3,observed_at=?4 WHERE asset_id=?1", params![asset_id, size, modified, now])?;
                    return Ok(false);
                }
            }
            // An unrelated text file next to a new image is not recorded at all.
            if current.is_none() && extraction.status == ExtractionStatus::None {
                return Ok(false);
            }
            let revision = current.map_or(Some(1), |r| r.revision.checked_add(1)).ok_or_else(|| invalid("Sidecar revision overflow"))?;
            db.execute(
                "INSERT INTO image_generation_sidecars(asset_id,relative_path,present,sidecar_hash,revision,extraction,observed_at,size,modified)
                 VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(asset_id) DO UPDATE SET relative_path=excluded.relative_path,present=1,sidecar_hash=excluded.sidecar_hash,
                 revision=excluded.revision,extraction=excluded.extraction,observed_at=excluded.observed_at,size=excluded.size,modified=excluded.modified",
                params![asset_id, relative, hash, revision, serde_json::to_string(&extraction)?, now, size, modified],
            )?;
            Ok(true)
        }
    }
}

fn user_record(db: &Connection, id: &str) -> Result<Option<UserProvenance>> {
    let r: Option<String> = db
        .query_row("SELECT record FROM image_generation_provenance WHERE asset_id=?1", [id], |r| r.get(0))
        .optional()?;
    Ok(r.map(|r| serde_json::from_str(&r)).transpose()?)
}
struct Own {
    kind: String,
    hash: Option<String>,
    parent: Option<String>,
    embedded: Option<GenerationExtraction>,
    sidecar: Option<SidecarView>,
    user: Option<UserProvenance>,
}
fn own(db: &Connection, id: &str) -> Result<Own> {
    let row: Option<(String, Option<String>, Option<String>)> = db
        .query_row(
            "SELECT r.kind,i.content_hash,json_extract(i.lineage,'$.parent_id') FROM asset_registry r JOIN image_assets i ON i.id=r.id WHERE r.id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (kind, hash, parent) = row.ok_or(E::NotFound)?;
    let embedded = hash.as_deref().map(|h| extraction(db, h)).transpose()?.flatten();
    let sidecar = db
        .query_row(
            "SELECT relative_path,present,revision,extraction FROM image_generation_sidecars WHERE asset_id=?1",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, u32>(2)?, r.get::<_, String>(3)?)),
        )
        .optional()?
        .map(|(relative_path, present, revision, e)| -> Result<SidecarView> {
            Ok(SidecarView { relative_path, present, revision, extraction: serde_json::from_str(&e)? })
        })
        .transpose()?;
    Ok(Own { kind, hash, parent, embedded, sidecar, user: user_record(db, id)? })
}
fn claims_of(o: &Own) -> Vec<GenerationClaim> {
    let mut out: Vec<GenerationClaim> = o.embedded.iter().flat_map(|e| e.claims.clone()).collect();
    // A removed sidecar no longer supplies values; its last claim stays visible in `sidecar`.
    if let Some(s) = o.sidecar.as_ref().filter(|s| s.present) {
        out.extend(s.extraction.claims.iter().cloned());
    }
    out
}
fn claim_value(c: &GenerationClaim, field: GenerationField) -> Option<(String, bool)> {
    let f = &c.fields;
    let v = match field {
        GenerationField::Prompt => f.prompt.clone(),
        GenerationField::NegativePrompt => f.negative_prompt.clone(),
        GenerationField::Generator => f.generator.clone(),
        GenerationField::Model => f.model.clone(),
        GenerationField::Seed => f.seed.clone(),
        GenerationField::Sampler => f.sampler.clone(),
        GenerationField::Created => {
            return f.created.as_ref().map(|t| match &t.utc_offset {
                Some(o) => (format!("{} {o}", t.value), false),
                None => (t.value.clone(), true),
            })
        }
        GenerationField::DigitalSourceType => f.digital_source_type.clone(),
        _ => None,
    };
    v.map(|v| (v, false))
}
const VIEW_FIELDS: [GenerationField; 9] = [
    GenerationField::Prompt,
    GenerationField::NegativePrompt,
    GenerationField::Generator,
    GenerationField::Model,
    GenerationField::Seed,
    GenerationField::Sampler,
    GenerationField::Created,
    GenerationField::DigitalSourceType,
    GenerationField::AiGenerated,
];
fn confirmation_matches(c: &Confirmation, o: &Own) -> bool {
    c.parser == PARSER
        && c.content_hash == o.hash
        && c.sidecar_revision == o.sidecar.as_ref().filter(|s| s.present).map(|s| s.revision)
}
/// The image's own record, resolved fields, its claims, and the parent it inherited from.
type Resolved = (Own, Vec<FieldView>, Vec<GenerationClaim>, Option<String>);
/// Own values first (user, then embedded claims, then sidecar claims); a
/// derived element falls back per field to its parent chain.
fn resolve(db: &Connection, id: &str) -> Result<Resolved> {
    let o = own(db, id)?;
    let claims = claims_of(&o);
    let redacted: HashSet<GenerationField> = o
        .user
        .iter()
        .flat_map(|u| u.redacted.iter().copied())
        .chain(o.embedded.iter().flat_map(|e| e.redacted.iter().copied()))
        .chain(o.sidecar.iter().flat_map(|s| s.extraction.redacted.iter().copied()))
        .collect();
    let mut fields = vec![];
    for field in VIEW_FIELDS {
        let mut view = FieldView { field, value: None, origin: FieldOrigin::Unknown, source: None, conflicting: false, redacted: false, uncertain: false };
        let imported: Vec<(&GenerationClaim, (String, bool))> = claims.iter().filter_map(|c| claim_value(c, field).map(|v| (c, v))).collect();
        view.conflicting = imported.iter().any(|(_, v)| v.0 != imported[0].1 .0);
        if let Some(v) = o.user.as_ref().and_then(|u| u.fields.get(&field)) {
            view.value = Some(v.clone());
            view.origin = FieldOrigin::User;
        } else if let Some((c, (v, uncertain))) = imported.first() {
            view.value = Some(v.clone());
            view.origin = FieldOrigin::Imported;
            view.source = Some(c.source.clone());
            view.uncertain = *uncertain;
        } else {
            view.redacted = redacted.contains(&field);
        }
        fields.push(view);
    }
    // Derivatives keep their parent's provenance for every field they lack.
    let mut inherited_from = None;
    let mut parent = o.parent.clone();
    let mut seen = HashSet::from([id.to_owned()]);
    for _ in 0..MAX_INHERITANCE_DEPTH {
        if fields.iter().all(|f| f.origin != FieldOrigin::Unknown) {
            break;
        }
        let Some(p) = parent.take().filter(|p| seen.insert(p.clone())) else { break };
        let Ok(po) = own(db, &p) else { break };
        let pclaims = claims_of(&po);
        for view in fields.iter_mut().filter(|f| f.origin == FieldOrigin::Unknown) {
            let user = po.user.as_ref().and_then(|u| u.fields.get(&view.field)).cloned().map(|v| (v, false, None));
            let imported = || pclaims.iter().find_map(|c| claim_value(c, view.field).map(|(v, u)| (v, u, Some(c.source.clone()))));
            if let Some((v, uncertain, source)) = user.or_else(imported) {
                view.value = Some(v);
                view.origin = FieldOrigin::Inherited;
                view.source = source;
                view.uncertain = uncertain;
                view.redacted = false;
                inherited_from.get_or_insert_with(|| p.clone());
            }
        }
        parent = po.parent;
    }
    Ok((o, fields, claims, inherited_from))
}
fn editable(db: &Connection, id: &str) -> Result<()> {
    let row: Option<(String, bool, bool, String)> = db
        .query_row(
            "SELECT r.kind,s.available,(s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f
              WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path)),i.availability
             FROM asset_registry r JOIN sources s ON s.id=r.source_id JOIN image_assets i ON i.id=r.id WHERE r.id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((kind, online, member, availability)) = row else { return Err(E::NotFound) };
    if kind == "audio" {
        return Err(E::NotFound);
    }
    if !member {
        return Err(E::PermissionDenied);
    }
    // Same rule as annotations: no edits while the original is offline or missing.
    if !online || availability != "available" {
        return Err(E::SourceUnavailable);
    }
    Ok(())
}

impl Catalog {
    pub fn generation_settings(&self) -> Result<GenerationSettings> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT value FROM app_settings WHERE key=?1", [SETTINGS_KEY], |r| r.get(0))
            .optional()?;
        Ok(raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default())
    }
    pub fn set_generation_settings(&mut self, settings: GenerationSettings) -> Result<GenerationSettings> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let before: Option<String> = tx.query_row("SELECT value FROM app_settings WHERE key=?1", [SETTINGS_KEY], |r| r.get(0)).optional()?;
        let next = serde_json::to_string(&settings)?;
        tx.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![SETTINGS_KEY, next],
        )?;
        // Snapshot sharing changed: every folder with generation data rewrites its snapshot.
        let old: GenerationSettings = before.and_then(|b| serde_json::from_str(&b).ok()).unwrap_or_default();
        if old.snapshot != settings.snapshot {
            tx.execute(
                "UPDATE source_catalog_state SET dirty=dirty+1 WHERE source_id IN (SELECT DISTINCT r.source_id FROM asset_registry r JOIN image_assets i ON i.id=r.id
                 WHERE EXISTS(SELECT 1 FROM image_generation_extractions g WHERE g.content_hash=i.content_hash)
                 OR EXISTS(SELECT 1 FROM image_generation_sidecars s WHERE s.asset_id=r.id)
                 OR EXISTS(SELECT 1 FROM image_generation_provenance p WHERE p.asset_id=r.id))",
                [],
            )?;
        }
        tx.commit()?;
        Ok(settings)
    }
    /// Attributed generation provenance for one image or element.
    pub fn image_generation(&self, id: &str) -> Result<ProvenanceView> {
        let (o, fields, claims, inherited_from) = resolve(&self.db, id)?;
        if o.kind == "audio" {
            return Err(E::NotFound);
        }
        let confirmation = o.user.as_ref().and_then(|u| u.confirmed.as_ref());
        let confirmed = !claims.is_empty() && confirmation.is_some_and(|c| confirmation_matches(c, &o));
        Ok(ProvenanceView {
            asset_id: id.into(),
            revision: o.user.as_ref().map_or(0, |u| u.revision),
            embedded: o.embedded.clone(),
            sidecar: o.sidecar.clone(),
            fields,
            claims,
            confirmed,
            confirmation_outdated: confirmation.is_some() && !confirmed,
            inherited_from,
            editable: editable(&self.db, id).is_ok(),
            settings: self.generation_settings()?,
        })
    }
    /// Optimistic edit: a revision mismatch is a `Conflict`, never a merge.
    pub fn edit_image_generation(&mut self, id: &str, expected_revision: u32, patch: &ProvenancePatch) -> Result<ProvenanceView> {
        if patch.set.len() > USER_FIELDS.len() {
            return Err(E::InvalidArgument);
        }
        for (field, value) in &patch.set {
            if !USER_FIELDS.contains(field) {
                return Err(E::InvalidArgument);
            }
            if let Some(v) = value {
                validate_value(*field, v)?;
            }
        }
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        editable(&tx, id)?;
        let current = user_record(&tx, id)?;
        if current.as_ref().map_or(0, |u| u.revision) != expected_revision {
            return Err(E::Conflict);
        }
        let mut next = current.clone().unwrap_or(UserProvenance { revision: 0, fields: BTreeMap::new(), confirmed: None, redacted: vec![] });
        for (field, value) in &patch.set {
            match value {
                Some(v) => next.fields.insert(*field, v.clone()),
                None => next.fields.remove(field),
            };
            // Entering a value replaces an export redaction for that field.
            next.redacted.retain(|r| r != field);
        }
        match patch.confirm {
            Some(true) => {
                let (o, _, claims, _) = resolve(&tx, id)?;
                if claims.is_empty() {
                    return Err(E::InvalidArgument);
                }
                next.confirmed = Some(Confirmation {
                    content_hash: o.hash.clone(),
                    parser: PARSER.into(),
                    sidecar_revision: o.sidecar.as_ref().filter(|s| s.present).map(|s| s.revision),
                    confirmed_at: crate::jobs::now_secs(),
                });
            }
            Some(false) => next.confirmed = None,
            None => {}
        }
        if current.as_ref().is_some_and(|c| c.fields == next.fields && c.confirmed == next.confirmed && c.redacted == next.redacted)
            || (current.is_none() && next.fields.is_empty() && next.confirmed.is_none())
        {
            drop(tx);
            return self.image_generation(id);
        }
        next.revision = expected_revision.checked_add(1).ok_or(E::LimitExceeded)?;
        next.validate().map_err(|_| E::LimitExceeded)?;
        tx.execute(
            "INSERT INTO image_generation_provenance(asset_id,revision,record,updated_at) VALUES(?1,?2,?3,?4)
             ON CONFLICT(asset_id) DO UPDATE SET revision=excluded.revision,record=excluded.record,updated_at=excluded.updated_at",
            params![id, next.revision, serde_json::to_string(&next)?, crate::jobs::now_secs()],
        )?;
        tx.commit()?;
        self.image_generation(id)
    }
    /// Generation text one image contributes to search under the library policy.
    /// SS-058 consumes this; it is computed on demand, so a policy change needs
    /// no reindex.
    pub fn image_generation_search_text(&self, id: &str) -> Result<Vec<String>> {
        let policy = self.generation_settings()?.search;
        if policy == SearchPolicy::Exclude {
            return Ok(vec![]);
        }
        let (_, fields, claims, _) = resolve(&self.db, id)?;
        let mut out = vec![];
        for f in fields {
            let prompt = matches!(f.field, GenerationField::Prompt | GenerationField::NegativePrompt);
            if prompt && policy != SearchPolicy::Prompts {
                continue;
            }
            if let Some(v) = f.value.filter(|v| !v.is_empty() && !local_path(v)) {
                out.push(v);
            }
        }
        for c in claims {
            for (k, v) in c.fields.settings {
                if policy == SearchPolicy::Prompts || !crate::image_generation::prompt_setting(&k) {
                    out.push(format!("{k}: {v}"));
                }
            }
        }
        out.dedup();
        Ok(out)
    }
}

/// Generation data for one image in portable JSON and folder snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableGeneration {
    /// Claim read from the current content version.
    #[serde(deserialize_with = "nullable")]
    pub embedded: Option<GenerationExtraction>,
    #[serde(deserialize_with = "nullable")]
    pub sidecar: Option<PortableSidecar>,
    #[serde(deserialize_with = "nullable")]
    pub provenance: Option<UserProvenance>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableSidecar {
    pub relative_path: String,
    pub present: bool,
    #[serde(deserialize_with = "nullable")]
    pub sidecar_hash: Option<String>,
    pub revision: u32,
    pub extraction: GenerationExtraction,
}
fn redact_user(u: &UserProvenance, sharing: GenerationSharing) -> UserProvenance {
    let mut out = u.clone();
    for (field, value) in &u.fields {
        let prompt = matches!(field, GenerationField::Prompt | GenerationField::NegativePrompt);
        if (prompt && !sharing.prompts) || (!sharing.private_references && local_path(value)) {
            out.fields.remove(field);
            if !out.redacted.contains(field) {
                out.redacted.push(*field);
            }
        }
    }
    out.redacted.sort();
    out
}
pub(crate) fn export_generation(
    db: &Connection,
    id: &str,
    hash: Option<&str>,
    sharing: GenerationSharing,
) -> crate::Result<Option<PortableGeneration>> {
    let embedded = hash
        .map(|h| extraction(db, h))
        .transpose()
        .map_err(|_| invalid("Invalid stored generation metadata"))?
        .flatten()
        .map(|e| e.redacted_for(sharing));
    let sidecar = db
        .query_row(
            "SELECT relative_path,present,sidecar_hash,revision,extraction FROM image_generation_sidecars WHERE asset_id=?1",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, u32>(3)?, r.get::<_, String>(4)?)),
        )
        .optional()?
        .map(|(relative_path, present, sidecar_hash, revision, e)| -> crate::Result<PortableSidecar> {
            let e: GenerationExtraction = serde_json::from_str(&e)?;
            Ok(PortableSidecar { relative_path, present, sidecar_hash, revision, extraction: e.redacted_for(sharing) })
        })
        .transpose()?;
    let provenance = user_record(db, id)
        .map_err(|_| invalid("Invalid stored provenance"))?
        .map(|u| redact_user(&u, sharing));
    if embedded.is_none() && sidecar.is_none() && provenance.is_none() {
        return Ok(None);
    }
    Ok(Some(PortableGeneration { embedded, sidecar, provenance }))
}
pub(crate) fn validate_portable(g: &PortableGeneration, relative: &str, has_hash: bool) -> crate::Result<()> {
    if let Some(e) = &g.embedded {
        if !has_hash {
            return Err(invalid("Generation metadata requires a content version"));
        }
        e.validate().map_err(invalid)?;
    }
    if let Some(s) = &g.sidecar {
        s.extraction.validate().map_err(invalid)?;
        if Some(s.relative_path.as_str()) != sidecar_path(relative).as_deref()
            || s.revision == 0
            || s.sidecar_hash.as_deref().is_some_and(|h| !digest(h))
        {
            return Err(invalid("Invalid generation sidecar record"));
        }
    }
    if let Some(u) = &g.provenance {
        u.validate().map_err(invalid)?;
    }
    Ok(())
}
/// Inside the caller's import transaction. Embedded claims are content-bound:
/// a different claim for the same bytes and parser fails the import.
pub(crate) fn import_generation(tx: &Connection, id: &str, hash: Option<&str>, g: &PortableGeneration) -> crate::Result<()> {
    if let (Some(e), Some(hash)) = (&g.embedded, hash) {
        let existing: Option<String> = tx
            .query_row(
                "SELECT extraction FROM image_generation_extractions WHERE content_hash=?1 AND parser_version=?2",
                params![hash, e.parser],
                |r| r.get(0),
            )
            .optional()?;
        match existing.map(|x| serde_json::from_str::<GenerationExtraction>(&x)).transpose()? {
            None => {
                e.validate().map_err(invalid)?;
                tx.execute(
                    "INSERT INTO image_generation_extractions(content_hash,parser_version,extraction) VALUES(?1,?2,?3)",
                    params![hash, e.parser, serde_json::to_string(e)?],
                )?;
            }
            Some(old) if old == *e => {}
            // The same bytes, one copy redacted: keep or upgrade to the complete one.
            Some(old) if old.redacted_for(GenerationSharing::default()) == e.redacted_for(GenerationSharing::default()) => {
                if !old.redacted.is_empty() && e.redacted.is_empty() {
                    store_extraction(tx, hash, e)?;
                }
            }
            Some(_) => return Err(invalid("Import conflicts with stored generation metadata")),
        }
    }
    if let Some(s) = &g.sidecar {
        tx.execute(
            "INSERT INTO image_generation_sidecars(asset_id,relative_path,present,sidecar_hash,revision,extraction,observed_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![id, s.relative_path, s.present, s.sidecar_hash, s.revision, serde_json::to_string(&s.extraction)?, crate::jobs::now_secs()],
        )?;
    }
    if let Some(u) = &g.provenance {
        tx.execute(
            "INSERT INTO image_generation_provenance(asset_id,revision,record,updated_at) VALUES(?1,?2,?3,?4)",
            params![id, u.revision, serde_json::to_string(u)?, crate::jobs::now_secs()],
        )?;
    }
    Ok(())
}
/// Typed checks for database restore: every stored record must parse and validate.
pub(crate) fn validate_generation_tables(db: &Connection) -> crate::Result<()> {
    let mut q = db.prepare("SELECT extraction FROM image_generation_extractions UNION ALL SELECT extraction FROM image_generation_sidecars")?;
    let mut rows = q.query([])?;
    while let Some(r) = rows.next()? {
        let e: GenerationExtraction = serde_json::from_str(&r.get::<_, String>(0)?).map_err(|_| invalid("Invalid stored generation metadata"))?;
        e.validate().map_err(invalid)?;
    }
    let mut q = db.prepare("SELECT record,revision FROM image_generation_provenance")?;
    let mut rows = q.query([])?;
    while let Some(r) = rows.next()? {
        let u: UserProvenance = serde_json::from_str(&r.get::<_, String>(0)?).map_err(|_| invalid("Invalid stored provenance"))?;
        if u.revision != r.get::<_, u32>(1)? {
            return Err(invalid("Invalid stored provenance"));
        }
        u.validate().map_err(invalid)?;
    }
    Ok(())
}
