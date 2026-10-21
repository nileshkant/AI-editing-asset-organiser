//! Image records in portable catalogs and source-folder snapshots (SS-048).
//! The image payload reuses the SS-040 `ImageAsset` contract; audio payloads keep
//! their v1 meaning. Machine-local stats, grants, jobs and undo history never travel.
use crate::{
    catalog::{valid_relative, Catalog},
    image_analysis::AnalysisWarning,
    image_annotations::{display_tag, tag_key, MAX_ALIASES},
    image_catalog::{load, validate},
    image_contracts::{AssetKind, ImageAsset, ImageProfile, TechnicalState},
    invalid, Result,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_IMAGES: usize = 100_000;
/// Content versions per image; exceeding it fails export instead of truncating.
pub const MAX_VERSIONS: usize = 1_000;
const MAX_TAGS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PortableImageVersion {
    pub content_hash: String,
    pub first_seen: i64,
}
/// One image or element record. `technical_warnings` are the content-bound
/// warnings cached with its analysis, so a warm import needs no decode.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PortableImage {
    pub asset: ImageAsset,
    pub technical_warnings: Vec<AnalysisWarning>,
    pub versions: Vec<PortableImageVersion>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PortableTagAlias {
    pub alias: String,
    pub canonical: String,
}

/// Structural checks that need no database. Database-level invariants (lineage
/// chains, alias chains, registry coverage) are rechecked after insertion.
pub(crate) fn validate_images(
    images: &[PortableImage],
    aliases: &[PortableTagAlias],
    sources: &BTreeMap<&String, &crate::portable::PortableSource>,
    taken: &mut HashSet<(String, String)>,
    ids: &mut HashSet<String>,
) -> Result<()> {
    if images.len() > MAX_IMAGES || aliases.len() > MAX_ALIASES as usize {
        return Err(invalid("Catalog exceeds image import limits"));
    }
    let mut analyses: HashMap<(&str, &str), (&ImageProfile, &[AnalysisWarning])> = HashMap::new();
    for image in images {
        let asset = &image.asset;
        let h = &asset.header;
        validate(asset)?;
        if !ids.insert(h.id.clone()) {
            return Err(invalid("Empty, oversized or duplicate identity"));
        }
        let source = sources
            .get(&h.source_id)
            .ok_or_else(|| invalid("Image references unknown source"))?;
        valid_relative(&h.relative_path)?;
        if !taken.insert((h.source_id.clone(), h.relative_path.clone())) {
            return Err(invalid("Duplicate source-relative path"));
        }
        if source.scope == "files" && !source.files.contains(&h.relative_path) {
            return Err(invalid("Image outside selected file scope"));
        }
        if (h.kind == AssetKind::Element) != asset.lineage.is_some() {
            return Err(invalid("Invalid image kind or lineage"));
        }
        let mut keys = HashSet::new();
        if asset.annotations.tags.len() > MAX_TAGS
            || asset.annotations.tags.iter().any(|t| {
                display_tag(t).ok().flatten().as_deref() != Some(t.as_str())
                    || !keys.insert(tag_key(t))
            })
        {
            return Err(invalid("Invalid image annotations"));
        }
        if image.versions.len() > MAX_VERSIONS
            || h.content_hash.is_some() == image.versions.is_empty()
        {
            return Err(invalid("Invalid image version history"));
        }
        let mut seen = HashSet::new();
        for version in &image.versions {
            if version.content_hash.len() != 64
                || !version.content_hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !seen.insert(version.content_hash.as_str())
            {
                return Err(invalid("Invalid image version history"));
            }
        }
        if h.content_hash.as_ref().is_some_and(|c| !seen.contains(c.as_str())) {
            return Err(invalid("Image version history lacks its current content"));
        }
        match &asset.profile {
            None if !image.technical_warnings.is_empty() => {
                return Err(invalid("Technical warnings require an analysis"))
            }
            None => {}
            Some(profile) => {
                if image.technical_warnings.len() > 8
                    || image.technical_warnings.contains(&AnalysisWarning::ExtensionMismatch)
                {
                    return Err(invalid("Invalid technical warnings"));
                }
                let key = (h.content_hash.as_deref().unwrap_or(""), profile.analyzer_version.as_str());
                if let Some((p, w)) = analyses.insert(key, (profile, &image.technical_warnings)) {
                    if p != profile || w != image.technical_warnings.as_slice() {
                        return Err(invalid("Conflicting image analyses for one content digest"));
                    }
                }
            }
        }
    }
    let mut alias_keys = HashSet::new();
    for alias in aliases {
        let ok = display_tag(&alias.alias).ok().flatten().as_deref() == Some(alias.alias.as_str())
            && display_tag(&alias.canonical).ok().flatten().as_deref() == Some(alias.canonical.as_str())
            && tag_key(&alias.alias) != tag_key(&alias.canonical)
            && alias_keys.insert(tag_key(&alias.alias));
        if !ok {
            return Err(invalid("Invalid image tag alias"));
        }
    }
    if aliases.iter().any(|a| alias_keys.contains(&tag_key(&a.canonical))) {
        return Err(invalid("Image tag aliases may not chain"));
    }
    Ok(())
}

/// Images in one source (or all), ordered for stable diffs. Snapshots pass
/// `members_only`: confirmed-missing and never-hashed records stay local.
pub(crate) fn export_images(
    db: &Connection,
    source_id: Option<&str>,
    members_only: bool,
) -> Result<Vec<PortableImage>> {
    let mut q = db.prepare(
        "SELECT r.id FROM asset_registry r JOIN image_assets i ON i.id=r.id
         WHERE r.kind IN ('image','element') AND (?1 IS NULL OR r.source_id=?1)
         AND (?2=0 OR (i.availability!='missing' AND i.content_hash IS NOT NULL))
         ORDER BY r.source_id,r.relative_path,r.id",
    )?;
    let ids = q
        .query_map(params![source_id, members_only], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if ids.len() > MAX_IMAGES {
        return Err(invalid("Image catalog exceeds the portable record limit"));
    }
    let mut versions = db.prepare(
        "SELECT content_hash,first_seen FROM image_asset_versions WHERE asset_id=?1 ORDER BY first_seen,content_hash",
    )?;
    let mut images = Vec::with_capacity(ids.len());
    for id in ids {
        let mut asset = load(db, &id)?;
        // The stored per-member state is portable; "offline" is derived from the
        // source on read and would otherwise freeze an unmapped root's state.
        let stored: String = db.query_row("SELECT availability FROM image_assets WHERE id=?1", [&id], |r| r.get(0))?;
        asset.header.availability = serde_json::from_value(serde_json::Value::String(stored))?;
        let history = versions
            .query_map([&id], |r| {
                Ok(PortableImageVersion {
                    content_hash: r.get(0)?,
                    first_seen: r.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if history.len() > MAX_VERSIONS {
            return Err(invalid("Image version history exceeds the portable limit"));
        }
        let technical_warnings = match (&asset.profile, &asset.header.content_hash) {
            (Some(p), Some(hash)) => db
                .query_row(
                    "SELECT warnings FROM image_analysis_warnings WHERE content_hash=?1 AND analyzer_version=?2",
                    params![hash, p.analyzer_version],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .map(|w| serde_json::from_str(&w))
                .transpose()?
                .unwrap_or_default(),
            _ => vec![],
        };
        images.push(PortableImage {
            asset,
            technical_warnings,
            versions: history,
        });
    }
    Ok(images)
}
pub(crate) fn export_aliases(db: &Connection) -> Result<Vec<PortableTagAlias>> {
    let mut q = db.prepare("SELECT alias,canonical FROM image_tag_aliases ORDER BY alias_key")?;
    let rows = q
        .query_map([], |r| {
            Ok(PortableTagAlias {
                alias: r.get(0)?,
                canonical: r.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn wire<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("Invalid image state"))
}

/// Inserts inside the caller's import transaction; any failure rolls back
/// sources, sounds and images together. Nothing is overwritten.
pub(crate) fn import_images(
    tx: &Connection,
    images: &[PortableImage],
    aliases: &[PortableTagAlias],
) -> Result<()> {
    let mut sources = HashSet::new();
    for image in images {
        let a = &image.asset;
        let h = &a.header;
        sources.insert(h.source_id.as_str());
        tx.execute(
            "INSERT INTO asset_registry(id,kind,source_id,relative_path) VALUES(?1,?2,?3,?4)",
            params![h.id, wire(&h.kind)?, h.source_id, h.relative_path],
        )?;
        for v in &image.versions {
            tx.execute("INSERT OR IGNORE INTO image_contents VALUES(?1)", [&v.content_hash])?;
        }
        if let (Some(profile), Some(hash)) = (&a.profile, &h.content_hash) {
            let cached: Option<String> = tx
                .query_row(
                    "SELECT profile FROM image_analyses WHERE content_hash=?1 AND analyzer_version=?2",
                    params![hash, profile.analyzer_version],
                    |r| r.get(0),
                )
                .optional()?;
            match cached {
                Some(raw) if serde_json::from_str::<ImageProfile>(&raw)? != *profile => {
                    return Err(invalid("Import conflicts with cached image analysis"))
                }
                Some(_) => {}
                None => {
                    tx.execute(
                        "INSERT INTO image_analyses(content_hash,analyzer_version,profile) VALUES(?1,?2,?3)",
                        params![hash, profile.analyzer_version, serde_json::to_string(profile)?],
                    )?;
                }
            }
            let warnings = serde_json::to_string(&image.technical_warnings)?;
            let old: Option<String> = tx
                .query_row(
                    "SELECT warnings FROM image_analysis_warnings WHERE content_hash=?1 AND analyzer_version=?2",
                    params![hash, profile.analyzer_version],
                    |r| r.get(0),
                )
                .optional()?;
            match old {
                Some(old) if serde_json::from_str::<Vec<AnalysisWarning>>(&old)? != image.technical_warnings => {
                    return Err(invalid("Import conflicts with cached image warnings"))
                }
                Some(_) => {}
                None => {
                    tx.execute(
                        "INSERT INTO image_analysis_warnings VALUES(?1,?2,?3)",
                        params![hash, profile.analyzer_version, warnings],
                    )?;
                }
            }
        }
        tx.execute(
            "INSERT INTO image_assets(id,content_hash,analyzer_version,technical_state,availability,enrichment_state,annotation_revision,annotations,evidence,recipes,lineage)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                h.id,
                h.content_hash,
                a.profile.as_ref().map(|p| p.analyzer_version.clone()),
                wire(&h.technical_state)?,
                wire(&h.availability)?,
                wire(&h.enrichment_state)?,
                a.annotations.revision,
                serde_json::to_string(&a.annotations)?,
                serde_json::to_string(&a.evidence)?,
                serde_json::to_string(&a.recipes)?,
                a.lineage.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;
        for v in &image.versions {
            tx.execute(
                "INSERT INTO image_asset_versions(asset_id,content_hash,first_seen) VALUES(?1,?2,?3)",
                params![h.id, v.content_hash, v.first_seen],
            )?;
        }
    }
    // Watching is never implied: monitoring rows start off, as after an import.
    for id in sources {
        tx.execute(
            "INSERT INTO image_source_monitor(source_id,mode) VALUES(?1,CASE WHEN EXISTS(SELECT 1 FROM sounds WHERE source_id=?1) THEN 'mixed' ELSE 'images' END)
             ON CONFLICT(source_id) DO NOTHING",
            [id],
        )?;
    }
    let now = crate::jobs::now_secs();
    for alias in aliases {
        let key = tag_key(&alias.alias);
        let existing: Option<String> = tx
            .query_row("SELECT canonical FROM image_tag_aliases WHERE alias_key=?1", [&key], |r| r.get(0))
            .optional()?;
        match existing {
            Some(canonical) if tag_key(&canonical) == tag_key(&alias.canonical) => {}
            Some(_) => return Err(invalid("Image tag alias conflicts with this library")),
            None => {
                tx.execute(
                    "INSERT INTO image_tag_aliases(alias_key,alias,canonical,created_at) VALUES(?1,?2,?3,?4)",
                    params![key, alias.alias, alias.canonical, now],
                )?;
            }
        }
    }
    if !images.is_empty() || !aliases.is_empty() {
        // Same typed checks as a database restore, over the combined library.
        crate::image_catalog::validate_image_database(tx)?;
    }
    Ok(())
}

impl Catalog {
    /// Image records currently pending verification or analysis in one source,
    /// plus pending audio in a mixed source: the members a warm import changed.
    pub fn pending_members(&self, source_id: &str) -> Result<Vec<String>> {
        let mut q = self.db.prepare(
            "SELECT r.relative_path FROM asset_registry r JOIN image_assets i ON i.id=r.id
             WHERE r.source_id=?1 AND i.technical_state='pending' AND i.availability='available'
             UNION SELECT relative_path FROM sounds WHERE source_id=?1 AND status='pending'
             ORDER BY 1",
        )?;
        let rows = q
            .query_map([source_id], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// True when the image lane already verified these exact bytes at this stat:
/// the SS-047 stat evidence stands in for the audio lane's scan verification.
pub(crate) fn image_stat_verified(
    db: &Connection,
    asset_id: &str,
    hash: &str,
    size: u64,
    modified_ns: &str,
) -> Result<bool> {
    let stored: Option<(i64, i64)> = db
        .query_row(
            "SELECT size,modified FROM asset_file_stats WHERE asset_id=?1 AND content_hash=?2",
            params![asset_id, hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(stored.is_some_and(|(s, m)| {
        i64::try_from(size).ok() == Some(s) && m.to_string() == modified_ns
    }))
}

/// Mark members whose snapshot stat no longer matches. Unchanged members keep
/// their verified facts with no hash or decode; changed ones wait for a check.
pub(crate) fn apply_fingerprints(
    images: &mut [PortableImage],
    stat: impl Fn(&str) -> Result<FingerprintState>,
) -> Result<()> {
    for image in images {
        match stat(&image.asset.header.relative_path)? {
            FingerprintState::Missing => {
                image.asset.header.availability = crate::image_contracts::Availability::Missing
            }
            FingerprintState::Changed => {
                image.asset.header.technical_state = TechnicalState::Pending;
                image.asset.header.availability = crate::image_contracts::Availability::Available;
            }
            FingerprintState::Same => {
                image.asset.header.availability = crate::image_contracts::Availability::Available
            }
        }
    }
    Ok(())
}
pub(crate) enum FingerprintState {
    Missing,
    Changed,
    Same,
}

#[cfg(test)]
mod persistence_inventory {
    //! R8 hook: every SQLite column must be classified. Adding a durable column
    //! without deciding how it travels fails here, so a later ticket has to
    //! extend portable export, snapshots and restore validation deliberately.
    use super::*;

    /// P = portable catalog and source snapshot; B = database backup only (with
    /// the reason); R = rebuildable or derived.
    const INVENTORY: &[(&str, &str, &[&str])] = &[
        ("sources", "P", &["id", "name", "scope"]),
        ("sources", "B: machine-local root, availability and generation fence", &["root", "available", "generation"]),
        ("source_files", "P", &["source_id", "relative_path", "status"]),
        ("sounds", "P", &["id", "source_id", "relative_path", "title", "content_hash", "status"]),
        ("annotations", "P", &["sound_id", "tags", "comment", "favorite"]),
        ("analyses", "P", &["content_hash", "analyzer", "profile"]),
        ("search_profiles", "R", &["content_hash", "analyzer", "profile"]),
        ("clips", "P", &["id", "sound_id", "name", "asset_version_id", "source_sample_rate_hz", "start_frame", "end_frame", "channel_policy", "gain_db", "fade_in_ms", "fade_out_ms", "revision", "created_at", "updated_at"]),
        ("clip_revisions", "P", &["id", "clip_id", "revision", "asset_version_id", "source_sample_rate_hz", "start_frame", "end_frame", "channel_policy", "gain_db", "fade_in_ms", "fade_out_ms", "created_at"]),
        ("saved_searches", "P", &["id", "name", "query", "created_at"]),
        ("legacy_evidence", "P", &["sound_id", "evidence"]),
        ("jobs", "B: local job history", &["id", "source_id", "state", "status", "completed", "total", "reused", "failed", "current", "errors", "paths", "lease_owner", "lease_until", "created_at", "updated_at", "kind"]),
        ("app_settings", "B: machine preferences", &["key", "value"]),
        ("source_catalog_state", "B: local snapshot revision and retry state", &["source_id", "revision", "dirty", "last_error"]),
        ("asset_registry", "P", &["id", "kind", "source_id", "relative_path"]),
        ("image_contents", "P", &["content_hash"]),
        ("image_analyses", "P", &["content_hash", "analyzer_version", "profile"]),
        ("image_analysis_warnings", "P", &["content_hash", "analyzer_version", "warnings"]),
        ("image_assets", "P", &["id", "content_hash", "analyzer_version", "technical_state", "availability", "enrichment_state", "annotation_revision", "annotations", "evidence", "recipes", "lineage"]),
        ("image_asset_versions", "P", &["asset_id", "content_hash", "first_seen"]),
        ("image_tag_aliases", "P", &["alias_key", "alias", "canonical"]),
        ("image_tag_aliases", "B: local creation time", &["created_at"]),
        ("image_annotation_actions", "B: local undo history (SS-046)", &["seq", "id", "kind", "label", "state", "created_at", "items", "entries", "alias"]),
        ("image_import_jobs", "B: local job history", &["id", "source_id", "source_generation", "selection_key", "mode", "discovery_epoch", "root_stamp", "folder_stamps", "folders", "paths", "state", "phase", "history_truncated", "discovery_complete", "lease_owner", "lease_until", "sequence", "queue_order", "total", "completed", "ready", "reused", "failed", "unsupported", "current", "error_code", "created_at", "updated_at", "scan_kind", "verify", "origin"]),
        ("image_import_items", "B: local job history", &["job_id", "relative_path", "media_kind", "state", "asset_id", "content_hash", "warnings", "error_code", "attempts", "discovered_epoch"]),
        ("image_import_budget", "R", &["id", "rows", "bytes"]),
        ("image_source_identity", "B: machine-local volume and inode binding", &["source_id", "source_generation", "root_stamp"]),
        ("asset_file_stats", "B: machine-local stat evidence (SS-047)", &["asset_id", "content_hash", "volume", "file_id", "size", "modified", "changed", "recorded"]),
        ("image_duplicate_groups", "P", &["content_hash", "preferred_id", "dismissed"]),
        ("image_duplicate_groups", "B: local edit fence (SS-062); imports start at revision 1", &["revision", "updated_at"]),
        ("image_duplicate_marks", "P", &["content_hash", "asset_id", "mark"]),
        ("image_source_monitor", "B: watching is re-enabled deliberately per machine", &["source_id", "watch", "mode", "root_state", "checked_at"]),
        ("image_search_docs", "R: derived search index (SS-058), rebuilt from image records", &["doc", "asset_id", "kind", "favorite", "rating", "width", "height", "orientation", "alpha", "transparency", "format", "technical", "availability", "enrichment", "suggestions", "origins", "added_at"]),
        ("image_search_tags", "R", &["doc", "key", "tag", "state"]),
        ("image_search_dirty", "R", &["asset_id"]),
        ("image_search_fts", "R", &["name", "tags", "suggested", "caption", "notes"]),
        ("image_search_fts_data", "R", &["id", "block"]),
        ("image_search_fts_idx", "R", &["segid", "term", "pgno"]),
        ("image_search_fts_content", "R", &["id", "c0", "c1", "c2", "c3", "c4"]),
        ("image_search_fts_docsize", "R", &["id", "sz"]),
        ("image_search_fts_config", "R", &["k", "v"]),
        ("saved_image_searches", "P", &["id", "name", "query", "created_at", "updated_at"]),
        ("saved_image_searches", "R: derived from name", &["name_key"]),
    ];

    #[test]
    fn every_durable_column_has_a_persistence_decision() {
        let c = Catalog::open(std::path::Path::new(":memory:")).unwrap();
        let db = c.db_connection();
        let mut tables = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = tables
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let mut unclassified = vec![];
        for table in &tables {
            let mut columns = db.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")).unwrap();
            let columns: Vec<String> = columns
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            for column in columns {
                if !INVENTORY.iter().any(|(t, _, cs)| t == table && cs.contains(&column.as_str())) {
                    unclassified.push(format!("{table}.{column}"));
                }
            }
        }
        assert!(
            unclassified.is_empty(),
            "Classify these columns in mixed_catalog::persistence_inventory and extend portable/snapshot/restore paths: {unclassified:?}"
        );
        for (table, _, _) in INVENTORY {
            assert!(tables.iter().any(|t| t == table), "Inventory names a removed table: {table}");
        }
    }

    /// Portable image columns must round-trip through the contract payload.
    #[test]
    fn portable_image_payload_names_every_portable_image_field() {
        let fields = serde_json::to_value(PortableImage {
            asset: serde_json::from_value(
                serde_json::from_str::<serde_json::Value>(include_str!("../../../fixtures/contracts/image-v1.json")).unwrap()["images"][0].clone(),
            )
            .unwrap(),
            technical_warnings: vec![],
            versions: vec![],
        })
        .unwrap();
        let asset = &fields["asset"];
        for key in ["header", "profile", "annotations", "evidence", "recipes", "lineage"] {
            assert!(asset.get(key).is_some(), "{key}");
        }
        for key in ["id", "kind", "source_id", "relative_path", "content_hash", "technical_state", "availability", "enrichment_state"] {
            assert!(asset["header"].get(key).is_some(), "{key}");
        }
        assert!(fields.get("versions").is_some() && fields.get("technical_warnings").is_some());
    }

    fn v14_database(path: &std::path::Path) -> Connection {
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
        ] {
            db.execute_batch(schema).unwrap();
        }
        db.pragma_update(None, "user_version", 14).unwrap();
        db
    }
    fn master(db: &Connection) -> Vec<(String, String)> {
        let mut q = db.prepare("SELECT name,replace(replace(coalesce(sql,''),' ',''),char(10),'') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name").unwrap();
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        rows.collect::<std::result::Result<_, _>>().unwrap()
    }

    #[test]
    fn genuine_v14_upgrade_clears_the_guard_error_and_matches_a_fresh_schema() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v14.sqlite");
        let mut old = Catalog { db: v14_database(&path) };
        let source = old.add_source(d.path()).unwrap();
        let image = old.register_image(&source, "kept.png", Some(&"a".repeat(64))).unwrap();
        let error: String = old.db.query_row("SELECT last_error FROM source_catalog_state WHERE source_id=?1", [&source.id], |r| r.get(0)).unwrap();
        assert!(error.contains("requires mixed catalog support"), "the v14 trigger records the guard");
        drop(old);
        let c = Catalog::open(&path).unwrap();
        assert_eq!(c.db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), crate::catalog::SCHEMA_VERSION);
        assert_eq!(c.image(&image.header.id, None).unwrap(), image);
        let (dirty, error): (i64, String) = c.db.query_row("SELECT dirty,last_error FROM source_catalog_state WHERE source_id=?1", [&source.id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert!(dirty > 0, "the folder still needs its first mixed snapshot");
        assert_eq!(error, "");
        let fresh = Catalog::open(std::path::Path::new(":memory:")).unwrap();
        assert_eq!(master(&c.db), master(&fresh.db), "restore compares schemas exactly");
        let backup = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains(&format!("pre-v{}-", crate::catalog::SCHEMA_VERSION))).unwrap();
        let db = Connection::open(backup).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 14);
        // New image writes keep marking the folder dirty without an error.
        let mut c = c;
        c.register_image(&source, "new.png", Some(&"b".repeat(64))).unwrap();
        let error: String = c.db.query_row("SELECT last_error FROM source_catalog_state WHERE source_id=?1", [&source.id], |r| r.get(0)).unwrap();
        assert_eq!(error, "");
    }

    #[test]
    fn v15_failure_rolls_back_and_keeps_the_v14_catalog_usable() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v14.sqlite");
        let db = v14_database(&path);
        // A trigger the v15 script expects is absent, so its DROP fails mid-migration.
        db.execute_batch("DROP TRIGGER image_import_protect_snapshot;").unwrap();
        drop(db);
        assert!(Catalog::open(&path).is_err());
        let db = Connection::open(&path).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 14);
        let triggers: i64 = db.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name IN ('image_dirty_insert','image_dirty_update')", [], |r| r.get(0)).unwrap();
        assert_eq!(triggers, 2, "the v14 triggers are intact after rollback");
    }
}
