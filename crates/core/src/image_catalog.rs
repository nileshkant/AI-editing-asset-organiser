//! Additive image persistence, internal to the core. No image IPC/MCP/UI is enabled.
//! Long-running hashing/decoding belongs outside this module and catalog locks.
use crate::{
    catalog::{Catalog, Source},
    image_contracts::{
        self as contract, Annotations, AssetHeader, Availability, EnrichmentState, ImageAsset,
        ImageProfile, TechnicalState, MAX_BUNDLE_BYTES, MAX_PAGE_SIZE,
    },
    invalid, Result,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const IMAGE_CAPABILITY_ENABLED: bool = false;
const MAX_SCOPES: usize = 128;
const MAX_STORED_IMAGES: usize = 100_000;

const READ_IMAGE: &str = "
 SELECT r.id,r.kind,r.source_id,r.relative_path,i.content_hash,i.technical_state,
 CASE WHEN s.available=0 THEN 'offline' ELSE i.availability END,
 i.enrichment_state,p.profile,i.annotations,i.evidence,i.recipes,i.lineage
 FROM image_assets i JOIN asset_registry r ON r.id=i.id JOIN sources s ON s.id=r.source_id
 LEFT JOIN image_analyses p ON p.content_hash=i.content_hash AND p.analyzer_version=i.analyzer_version";

struct RawImage {
    id: String,
    kind: String,
    source_id: String,
    path: String,
    hash: Option<String>,
    technical: String,
    availability: String,
    enrichment: String,
    profile: Option<String>,
    annotations: String,
    evidence: String,
    recipes: String,
    lineage: Option<String>,
}
fn raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawImage> {
    Ok(RawImage {
        id: row.get(0)?,
        kind: row.get(1)?,
        source_id: row.get(2)?,
        path: row.get(3)?,
        hash: row.get(4)?,
        technical: row.get(5)?,
        availability: row.get(6)?,
        enrichment: row.get(7)?,
        profile: row.get(8)?,
        annotations: row.get(9)?,
        evidence: row.get(10)?,
        recipes: row.get(11)?,
        lineage: row.get(12)?,
    })
}
fn enumeration<T: serde::de::DeserializeOwned>(s: String) -> Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(s))?)
}
fn validate(asset: &ImageAsset) -> Result<()> {
    asset
        .validate()
        .map_err(|e| invalid(&format!("Invalid image contract: {e:?}")))?;
    if serde_json::to_vec(asset)?.len() > MAX_BUNDLE_BYTES {
        return Err(invalid("Image metadata exceeds the 2 MiB record limit"));
    }
    Ok(())
}
impl RawImage {
    fn decode(self) -> Result<ImageAsset> {
        let result = ImageAsset {
            header: AssetHeader {
                id: self.id,
                kind: enumeration(self.kind)?,
                source_id: self.source_id,
                relative_path: self.path,
                content_hash: self.hash,
                technical_state: enumeration(self.technical)?,
                availability: enumeration(self.availability)?,
                enrichment_state: enumeration(self.enrichment)?,
            },
            profile: self.profile.map(|s| serde_json::from_str(&s)).transpose()?,
            annotations: serde_json::from_str(&self.annotations)?,
            evidence: serde_json::from_str(&self.evidence)?,
            recipes: serde_json::from_str(&self.recipes)?,
            lineage: self.lineage.map(|s| serde_json::from_str(&s)).transpose()?,
        };
        validate(&result)?;
        Ok(result)
    }
}
fn load(db: &Connection, id: &str) -> Result<ImageAsset> {
    let value = db
        .query_row(&format!("{READ_IMAGE} WHERE i.id=?1"), [id], raw)
        .optional()?;
    if let Some(value) = value {
        let asset = value.decode()?;
        if let Some(lineage) = &asset.lineage {
            let parent_kind: Option<String> = db
                .query_row(
                    "SELECT kind FROM asset_registry WHERE id=?1",
                    [&lineage.parent_id],
                    |r| r.get(0),
                )
                .optional()?;
            if parent_kind.as_deref() == Some("audio") {
                return Err(invalid("Wrong asset kind in image lineage"));
            }
        }
        return Ok(asset);
    }
    let kind: Option<String> = db
        .query_row("SELECT kind FROM asset_registry WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Err(invalid(if kind.as_deref() == Some("audio") {
        "Wrong asset kind: expected image"
    } else {
        "Image not found"
    }))
}
fn check_source(db: &Connection, source: &Source, relative: &str) -> Result<()> {
    contract::validate_relative(relative).map_err(|_| invalid("Invalid relative image path"))?;
    let valid: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sources s WHERE s.id=?1 AND s.generation=?2 AND s.available=1
         AND (s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=s.id AND f.relative_path=?3)))",
        params![source.id,source.generation,relative],|r| r.get(0))?;
    if !valid {
        return Err(invalid(
            "Source changed, offline, or image outside selected file scope",
        ));
    }
    Ok(())
}
fn hash(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("Invalid image content digest"));
    }
    Ok(())
}
fn version(asset: &ImageAsset, expected: Option<&str>) -> Result<()> {
    if asset.header.content_hash.as_deref() != expected {
        return Err(invalid("Image content version changed"));
    }
    Ok(())
}
/// Summary columns shared by list/gallery queries: registry `r`, assets `i`,
/// sources `s` and the selected analysis `p`.
pub(crate) const SUMMARY_COLUMNS: &str = "r.id,r.kind,r.source_id,r.relative_path,i.content_hash,i.technical_state,
            CASE WHEN s.available=0 THEN 'offline' ELSE i.availability END,i.enrichment_state,
            json_extract(p.profile,'$.width'),json_extract(p.profile,'$.height'),i.annotation_revision";
pub(crate) fn summary(row: &rusqlite::Row<'_>) -> Result<ImageSummary> {
    Ok(ImageSummary {
        header: AssetHeader {
            id: row.get(0)?,
            kind: enumeration(row.get(1)?)?,
            source_id: row.get(2)?,
            relative_path: row.get(3)?,
            content_hash: row.get(4)?,
            technical_state: enumeration(row.get(5)?)?,
            availability: enumeration(row.get(6)?)?,
            enrichment_state: enumeration(row.get(7)?)?,
        },
        width: row.get(8)?,
        height: row.get(9)?,
        annotation_revision: row.get(10)?,
    })
}
pub(crate) fn scoped(source_ids: &[String]) -> Result<()> {
    if source_ids.len() > MAX_SCOPES || source_ids.iter().any(|s| s.is_empty() || s.len() > 256) {
        return Err(invalid("Invalid image source scopes"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageSummary {
    pub header: AssetHeader,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub annotation_revision: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImagePage {
    pub items: Vec<ImageSummary>,
    pub next_after: Option<String>,
}

impl Catalog {
    /// Metadata-only registration; callers must verify bytes and grants before use.
    /// None is allowed for first discovery, never to erase a known version.
    pub fn register_image(
        &mut self,
        source: &Source,
        relative: &str,
        content_hash: Option<&str>,
    ) -> Result<ImageAsset> {
        self.register_image_when(source,relative,content_hash,|_|Ok(()))
    }
    pub(crate) fn register_image_when(&mut self,source:&Source,relative:&str,content_hash:Option<&str>,fence:impl FnOnce(&Connection)->Result<()>)->Result<ImageAsset>{
        if let Some(h) = content_hash {
            hash(h)?;
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_source(&tx, source, relative)?;
        fence(&tx)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT id,kind FROM asset_registry WHERE source_id=?1 AND relative_path=?2",
                params![source.id, relative],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let id;
        if let Some((old_id, kind)) = existing {
            if kind != "image" {
                return Err(invalid("Wrong asset kind at existing source-relative path"));
            }
            id = old_id;
            let mut current = load(&tx, &id)?;
            if current.header.content_hash.as_deref() == content_hash {
                tx.commit()?;
                return Ok(current);
            }
            if content_hash.is_none() {
                return Err(invalid(
                    "Cannot replace a known image version with an unknown digest",
                ));
            }
            for e in &mut current.evidence {
                e.stale = true;
            }
            for r in &mut current.recipes {
                r.stale = true;
            }
            current.header.content_hash = content_hash.map(str::to_owned);
            current.header.technical_state = TechnicalState::Pending;
            current.header.availability = Availability::Available;
            current.header.enrichment_state = if current.evidence.is_empty() {
                EnrichmentState::Disabled
            } else {
                EnrichmentState::Stale
            };
            current.profile = None;
            validate(&current)?;
            tx.execute(
                "INSERT OR IGNORE INTO image_contents VALUES(?1)",
                [content_hash],
            )?;
            tx.execute("UPDATE image_assets SET content_hash=?2,analyzer_version=NULL,technical_state='pending',
                availability='available',enrichment_state=?3,evidence=?4,recipes=?5 WHERE id=?1",
                params![id,content_hash,serde_json::to_value(current.header.enrichment_state)?.as_str(),
                    serde_json::to_string(&current.evidence)?,serde_json::to_string(&current.recipes)?])?;
        } else {
            let count: i64 = tx.query_row("SELECT count(*) FROM image_assets", [], |r| r.get(0))?;
            if count >= MAX_STORED_IMAGES as i64 {
                return Err(invalid("Image catalog record limit reached"));
            }
            id = Uuid::new_v4().to_string();
            if let Some(h) = content_hash {
                tx.execute("INSERT OR IGNORE INTO image_contents VALUES(?1)", [h])?;
            }
            let annotations = Annotations {
                revision: 1,
                tags: vec![],
                comment: String::new(),
                favorite: false,
                rating: None,
                caption: None,
            };
            tx.execute(
                "INSERT INTO asset_registry VALUES(?1,'image',?2,?3)",
                params![id, source.id, relative],
            )?;
            tx.execute("INSERT INTO image_assets(id,content_hash,technical_state,availability,enrichment_state,annotation_revision,annotations)
                VALUES(?1,?2,'pending','available','disabled',1,?3)",params![id,content_hash,serde_json::to_string(&annotations)?])?;
        }
        if let Some(h) = content_hash {
            tx.execute(
                "INSERT OR IGNORE INTO image_asset_versions VALUES(?1,?2,?3)",
                params![id, h, crate::jobs::now_secs()],
            )?;
        }
        let result = load(&tx, &id)?;
        tx.commit()?;
        Ok(result)
    }

    /// Internal full metadata read; adapters must use image_in_sources for scoped clients.
    pub fn image(&self, id: &str, expected_version: Option<&str>) -> Result<ImageAsset> {
        let asset = load(&self.db, id)?;
        self.validate_source_entry(&asset.header.source_id, &asset.header.relative_path)?;
        if let Some(expected) = expected_version {
            version(&asset, Some(expected))?;
        }
        Ok(asset)
    }

    pub fn image_in_sources(
        &self,
        source_ids: &[String],
        id: &str,
        expected_version: Option<&str>,
    ) -> Result<ImageAsset> {
        scoped(source_ids)?;
        let source: Option<String> = self
            .db
            .query_row(
                "SELECT source_id FROM asset_registry WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if !source.is_some_and(|s| source_ids.contains(&s)) {
            return Err(invalid("Image permission denied"));
        }
        self.image(id, expected_version)
    }

    pub fn cached_image_profile(
        &self,
        content_hash: &str,
        analyzer: &str,
    ) -> Result<Option<ImageProfile>> {
        hash(content_hash)?;
        if analyzer.is_empty() || analyzer.len() > 256 {
            return Err(invalid("Invalid image analyzer version"));
        }
        let profile: Option<String> = self
            .db
            .query_row(
                "SELECT profile FROM image_analyses WHERE content_hash=?1 AND analyzer_version=?2",
                params![content_hash, analyzer],
                |r| r.get(0),
            )
            .optional()?;
        profile
            .map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }

    /// Publish version-bound facts supplied by a verified worker, with no decoding here.
    pub fn publish_image(
        &mut self,
        source: &Source,
        id: &str,
        expected_version: &str,
        profile: &ImageProfile,
    ) -> Result<ImageAsset> {
        self.publish_image_when(source,id,expected_version,profile,|_|Ok(()))
    }

    /// A shared transaction for worker ownership/cancellation fences and publication.
    /// The fence may update only bounded job metadata; it performs no file work.
    pub(crate) fn publish_image_when(
        &mut self,source:&Source,id:&str,expected_version:&str,profile:&ImageProfile,
        fence:impl FnOnce(&Connection)->Result<()>,
    )->Result<ImageAsset>{
        hash(expected_version)?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut asset = load(&tx, id)?;
        if asset.header.source_id != source.id {
            return Err(invalid("Image outside selected source"));
        }
        check_source(&tx, source, &asset.header.relative_path)?;
        version(&asset, Some(expected_version))?;
        asset.header.technical_state = TechnicalState::Ready;
        asset.profile = Some(profile.clone());
        validate(&asset)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT profile FROM image_analyses WHERE content_hash=?1 AND analyzer_version=?2",
                params![expected_version, profile.analyzer_version],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(raw) = old {
            if serde_json::from_str::<ImageProfile>(&raw)? != *profile {
                return Err(invalid(
                    "Conflicting image analysis for content/analyzer version",
                ));
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO image_analyses(content_hash,analyzer_version,profile) VALUES(?1,?2,?3)",
            params![
                expected_version,
                profile.analyzer_version,
                serde_json::to_string(profile)?
            ],
        )?;
        fence(&tx)?;
        tx.execute(
            "UPDATE image_assets SET analyzer_version=?2,technical_state='ready',availability='available' WHERE id=?1",
            params![id, profile.analyzer_version],
        )?;
        let result = load(&tx, id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn publish_cached_image(
        &mut self,
        source: &Source,
        id: &str,
        expected_version: &str,
        analyzer: &str,
    ) -> Result<ImageAsset> {
        let profile = self
            .cached_image_profile(expected_version, analyzer)?
            .ok_or_else(|| invalid("Image analysis cache unavailable"))?;
        self.publish_image(source, id, expected_version, &profile)
    }

    /// The supplied annotation revision is the expected old revision, incremented on success.
    /// None as expected_version means "expect unknown", not "ignore version".
    pub fn set_image_annotations(
        &mut self,
        id: &str,
        expected_version: Option<&str>,
        annotations: &Annotations,
    ) -> Result<ImageAsset> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut asset = load(&tx, id)?;
        let allowed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asset_registry r JOIN sources s ON s.id=r.source_id
            WHERE r.id=?1 AND (s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path)))",[id],|r|r.get(0))?;
        if !allowed {
            return Err(invalid("Image outside selected file scope"));
        }
        version(&asset, expected_version)?;
        if asset.annotations.revision != annotations.revision {
            return Err(invalid("Image annotation revision changed"));
        }
        let mut next = annotations.clone();
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("Image annotation revision exhausted"))?;
        asset.annotations = next;
        validate(&asset)?;
        let updated=tx.execute("UPDATE image_assets SET annotation_revision=?2,annotations=?3 WHERE id=?1 AND annotation_revision=?4",
            params![id,asset.annotations.revision,serde_json::to_string(&asset.annotations)?,annotations.revision])?;
        if updated != 1 {
            return Err(invalid("Image annotation revision changed"));
        }
        tx.commit()?;
        Ok(asset)
    }

    /// Metadata deletion only; immutable source/export bytes are never touched.
    pub fn delete_image(
        &mut self,
        source: &Source,
        id: &str,
        expected_version: Option<&str>,
        expected_revision: u32,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let asset = load(&tx, id)?;
        if asset.header.source_id != source.id {
            return Err(invalid("Image outside selected source"));
        }
        check_source(&tx, source, &asset.header.relative_path)?;
        version(&asset, expected_version)?;
        if asset.annotations.revision != expected_revision {
            return Err(invalid("Image annotation revision changed"));
        }
        tx.execute("DELETE FROM asset_registry WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Stable ID keyset pagination. Summaries omit potentially large evidence/recipes.
    pub fn list_images(
        &self,
        source_ids: &[String],
        after: Option<&str>,
        limit: u32,
    ) -> Result<ImagePage> {
        scoped(source_ids)?;
        if limit == 0 || limit > MAX_PAGE_SIZE {
            return Err(invalid("Invalid image page size"));
        }
        if let Some(id) = after {
            if Uuid::parse_str(id).is_err() || id.len() != 36 {
                return Err(invalid("Invalid image page cursor"));
            }
        }
        if source_ids.is_empty() {
            return Ok(ImagePage {
                items: vec![],
                next_after: None,
            });
        }
        let placeholders = vec!["?"; source_ids.len()].join(",");
        let sql=format!("SELECT {SUMMARY_COLUMNS}
            FROM asset_registry r JOIN image_assets i ON i.id=r.id JOIN sources s ON s.id=r.source_id
            LEFT JOIN image_analyses p ON p.content_hash=i.content_hash AND p.analyzer_version=i.analyzer_version
            WHERE r.source_id IN ({placeholders}) AND r.id>?
            AND (s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path))
            ORDER BY r.id LIMIT ?");
        let mut values: Vec<rusqlite::types::Value> =
            source_ids.iter().cloned().map(Into::into).collect();
        values.push(after.unwrap_or("").to_owned().into());
        values.push(i64::from(limit + 1).into());
        let mut statement = self.db.prepare(&sql)?;
        let mut rows = statement.query(rusqlite::params_from_iter(values))?;
        let mut items = vec![];
        let mut bytes = 256;
        let mut more = false;
        while let Some(row) = rows.next()? {
            if items.len() == limit as usize {
                more = true;
                break;
            }
            let item = summary(row)?;
            let size = serde_json::to_vec(&item)?.len() + 1;
            if bytes + size > MAX_BUNDLE_BYTES {
                more = true;
                break;
            }
            bytes += size;
            items.push(item);
        }
        let next_after = if more {
            items.last().map(|i| i.header.id.clone())
        } else {
            None
        };
        Ok(ImagePage { items, next_after })
    }

    pub fn source_has_images(&self, source_id: &str) -> Result<bool> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM asset_registry WHERE source_id=?1 AND kind IN ('image','element'))",[source_id],|r|r.get(0))?)
    }
    pub fn require_audio_only_source(&self, source_id: &str) -> Result<()> {
        let importing:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM image_source_identity WHERE source_id=?1) OR EXISTS(SELECT 1 FROM image_import_jobs j WHERE j.source_id=?1 AND (j.state IN ('queued','running') OR EXISTS(SELECT 1 FROM image_import_items i WHERE i.job_id=j.id AND i.media_kind='image' AND i.state IN ('pending','running'))))",[source_id],|r|r.get(0))?;
        if self.source_has_images(source_id)? || importing {
            return Err(invalid(
                "Image metadata requires mixed catalog support; audio-only operation is blocked",
            ));
        }
        Ok(())
    }
}

/// Validate backup contents before replacement; SQLite integrity alone does not
/// establish typed metadata consistency or complete cross-table registry coverage.
pub(crate) fn validate_image_database(db: &Connection) -> Result<()> {
    let mismatch:i64=db.query_row("SELECT count(*) FROM sounds s LEFT JOIN asset_registry r ON r.id=s.id
        WHERE r.id IS NULL OR r.kind!='audio' OR r.source_id!=s.source_id OR r.relative_path!=s.relative_path",[],|r|r.get(0))?;
    let orphan: i64 = db.query_row(
        "SELECT count(*) FROM asset_registry r WHERE
        (r.kind='audio' AND NOT EXISTS(SELECT 1 FROM sounds s WHERE s.id=r.id)) OR
        (r.kind!='audio' AND NOT EXISTS(SELECT 1 FROM image_assets i WHERE i.id=r.id))",
        [],
        |r| r.get(0),
    )?;
    if mismatch != 0 || orphan != 0 {
        return Err(invalid("Backup asset registry is inconsistent"));
    }
    let mut statement = db.prepare(READ_IMAGE)?;
    let mut rows = statement.query([])?;
    let mut edges = HashMap::new();
    let mut count = 0;
    while let Some(row) = rows.next()? {
        count += 1;
        if count > MAX_STORED_IMAGES {
            return Err(invalid("Image catalog record limit exceeded"));
        }
        let asset = raw(row)?.decode()?;
        let h = &asset.header;
        let allowed: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sources s WHERE s.id=?1 AND (s.scope='folder' OR EXISTS(
            SELECT 1 FROM source_files f WHERE f.source_id=s.id AND f.relative_path=?2)))",
            params![h.source_id, h.relative_path],
            |r| r.get(0),
        )?;
        if !allowed {
            return Err(invalid("Image outside selected file scope"));
        }
        if let Some(hash) = &h.content_hash {
            let known:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM image_asset_versions WHERE asset_id=?1 AND content_hash=?2)",params![h.id,hash],|r|r.get(0))?;
            if !known {
                return Err(invalid("Backup image version history is incomplete"));
            }
        }
        if let Some(lineage) = asset.lineage {
            let kind: Option<String> = db
                .query_row(
                    "SELECT kind FROM asset_registry WHERE id=?1",
                    [&lineage.parent_id],
                    |r| r.get(0),
                )
                .optional()?;
            if kind.as_deref() == Some("audio") {
                return Err(invalid("Wrong asset kind in image lineage"));
            }
            edges.insert(h.id.clone(), lineage.parent_id);
        }
    }
    let mut done = HashSet::new();
    for id in edges.keys() {
        let mut chain = HashSet::new();
        let mut current = id;
        while let Some(parent) = edges.get(current) {
            if done.contains(current) {
                break;
            }
            if chain.len() >= 1000 || !chain.insert(current.clone()) {
                return Err(invalid("Invalid or excessive image lineage"));
            }
            current = parent;
        }
        done.extend(chain);
    }
    Ok(())
}
