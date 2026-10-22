//! Versioned metadata transfer. Media, credentials, jobs and machine grants never travel.
use crate::{
    catalog::{
        contained, hash_file, path_text, valid_relative, validate_clip_recipe, validate_profile,
        Catalog, Clip, SavedSearch, Sound, ANALYZER,
    },
    image_generation::GenerationSharing,
    invalid, Result,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const PORTABLE_SCHEMA: &str = "soundshelf-catalog/v1";
/// Mixed audio/image catalogs (SS-048). Audio-only exports stay v1 so older
/// readers keep working; an older reader rejects v2 by schema, never by dropping images.
pub const PORTABLE_SCHEMA_V2: &str = "soundshelf-catalog/v2";
/// v2 plus exact-duplicate decisions (SS-062). Written only when there are
/// decisions, so libraries without them keep producing v2 for older readers.
pub const PORTABLE_SCHEMA_V3: &str = "soundshelf-catalog/v3";
const SCHEMA_FAMILY: &str = "soundshelf-catalog/";
pub const LEGACY_SCHEMA: &str = "portable-sound-effects-catalog/v3";
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableSource {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub files: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LegacyEvidence {
    pub sound_id: String,
    pub description: String,
    pub provenance: String,
    pub analyzed_seconds: Option<f64>,
    pub measurement_scope: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableCatalog {
    pub schema: String,
    pub sources: Vec<PortableSource>,
    pub sounds: Vec<Sound>,
    pub clips: Vec<Clip>,
    pub saved_searches: Vec<SavedSearch>,
    pub legacy_evidence: Vec<LegacyEvidence>,
    /// v2 only: image and element records in the SS-040 contract shape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<crate::mixed_catalog::PortableImage>,
    /// v2 only: library-wide creator tag aliases (SS-046). Undo history stays local.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_tag_aliases: Vec<crate::mixed_catalog::PortableTagAlias>,
    /// v3 only: creator decisions on exact-duplicate stacks (SS-062).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_duplicate_decisions: Vec<crate::image_duplicates::PortableDuplicateDecision>,
}
#[derive(Debug, Serialize)]
pub struct Preview {
    pub schema: String,
    pub sources: Vec<PortableSource>,
    pub sounds: usize,
    pub images: usize,
    pub legacy: bool,
}
#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub sources: usize,
    pub sounds: usize,
    pub images: usize,
    pub offline_sources: usize,
    pub legacy: bool,
}

fn unique(ids: impl Iterator<Item = String>, max_bytes: usize) -> Result<()> {
    let mut seen = HashSet::new();
    for id in ids {
        if id.is_empty() || id.len() > max_bytes || !seen.insert(id) {
            return Err(invalid("Empty, oversized or duplicate identity"));
        }
    }
    Ok(())
}
impl PortableCatalog {
    pub fn validate(&self) -> Result<()> {
        let mixed = self.schema == PORTABLE_SCHEMA_V2 || self.schema == PORTABLE_SCHEMA_V3;
        if !mixed && self.schema != PORTABLE_SCHEMA {
            return Err(invalid(newer_or_unsupported(&self.schema)));
        }
        if !mixed && (!self.images.is_empty() || !self.image_tag_aliases.is_empty()) {
            return Err(invalid("Image records require the soundshelf-catalog/v2 schema"));
        }
        if self.schema != PORTABLE_SCHEMA_V3 && !self.image_duplicate_decisions.is_empty() {
            return Err(invalid("Duplicate decisions require the soundshelf-catalog/v3 schema"));
        }
        if self.sounds.len() > 100_000 || self.sources.len() > 10_000 || self.clips.len() > 100_000
        {
            return Err(invalid("Catalog exceeds import limits"));
        }
        unique(self.sources.iter().map(|s| s.id.clone()), 256)?;
        unique(self.sounds.iter().map(|s| s.id.clone()), 256)?;
        unique(self.clips.iter().map(|s| s.id.clone()), 256)?;
        unique(self.saved_searches.iter().map(|s| s.id.clone()), 256)?;
        unique(self.legacy_evidence.iter().map(|s| s.sound_id.clone()), 256)?;
        let sources: BTreeMap<_, _> = self.sources.iter().map(|s| (&s.id, s)).collect();
        let memberships: BTreeMap<_, HashSet<_>> = self
            .sources
            .iter()
            .map(|s| (&s.id, s.files.iter().collect()))
            .collect();
        let sounds: BTreeMap<_, _> = self.sounds.iter().map(|s| (&s.id, s)).collect();
        let mut identities: HashSet<String> = self.sounds.iter().map(|s| s.id.clone()).collect();
        let mut taken: HashSet<(String, String)> = self
            .sounds
            .iter()
            .map(|s| (s.source_id.clone(), s.relative_path.clone()))
            .collect();
        crate::mixed_catalog::validate_images(
            &self.images,
            &self.image_tag_aliases,
            &sources,
            &mut taken,
            &mut identities,
        )?;
        crate::image_duplicates::validate_decisions(&self.image_duplicate_decisions, &self.images)?;
        for source in &self.sources {
            if !["folder", "files"].contains(&source.scope.as_str()) {
                return Err(invalid("Invalid source scope"));
            }
            unique(source.files.iter().cloned(), 32768)?;
            for path in &source.files {
                valid_relative(path)?;
            }
            if source.scope == "folder" && !source.files.is_empty() {
                return Err(invalid("Folder source has selected members"));
            }
        }
        let mut paths = HashSet::new();
        let mut profiles = BTreeMap::new();
        for sound in &self.sounds {
            let source = sources
                .get(&sound.source_id)
                .ok_or_else(|| invalid("Sound references unknown source"))?;
            valid_relative(&sound.relative_path)?;
            if !paths.insert((&sound.source_id, &sound.relative_path)) {
                return Err(invalid("Duplicate source-relative path"));
            }
            if source.scope == "files"
                && !memberships[&sound.source_id].contains(&sound.relative_path)
            {
                return Err(invalid("Sound outside selected file scope"));
            }
            if !["ready", "pending", "missing", "failed"].contains(&sound.status.as_str()) {
                return Err(invalid("Invalid sound status"));
            }
            if sound.content_hash.is_empty() || sound.content_hash.len() > 256 {
                return Err(invalid("Invalid content digest"));
            }
            if sound.user_tags.len() > 64
                || sound.comment.chars().count() > 10_000
                || sound
                    .user_tags
                    .iter()
                    .any(|t| t.chars().count() > 64 || t.chars().any(char::is_control))
            {
                return Err(invalid("Invalid annotations"));
            }
            if let Some(profile) = &sound.profile {
                validate_profile(profile)?;
                if let Some(previous) = profiles.insert(&sound.content_hash, profile) {
                    if previous != profile {
                        return Err(invalid("Conflicting profiles for one content digest"));
                    }
                }
            } else if sound.status == "ready" {
                return Err(invalid("Ready sound lacks measured profile"));
            }
        }
        for clip in &self.clips {
            if !sounds.contains_key(&clip.sound_id)
                || clip.recipe.asset_id != clip.sound_id
                || clip.revision == 0
            {
                return Err(invalid("Invalid clip reference or revision"));
            }
            validate_clip_recipe(&clip.recipe, None)?;
        }
        for search in &self.saved_searches {
            if search
                .query
                .source_ids
                .iter()
                .any(|id| !sources.contains_key(id))
            {
                return Err(invalid("Saved search references unknown source"));
            }
            crate::search::interpret(&search.query)?;
        }
        for note in &self.legacy_evidence {
            if !sounds.contains_key(&note.sound_id)
                || note.provenance != "legacy_filename_inference"
                || note.measurement_scope != "first_up_to_20_seconds_12khz_mono"
                || note
                    .analyzed_seconds
                    .is_some_and(|s| !s.is_finite() || !(0.0..=20.0).contains(&s))
            {
                return Err(invalid("Invalid legacy provenance"));
            }
        }
        Ok(())
    }
    pub fn json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn markdown(&self) -> String {
        fn escape(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\n', " ")
                .replace('\r', " ")
                .replace('`', "\\`")
                .replace('*', "\\*")
                .replace('[', "\\[")
                .replace(']', "\\]")
        }
        let mut out = format!("# CreativeShelf catalog\n\nSchema: {}. Media locations are relative to source IDs; map each source to a local root. This Markdown is a readable companion; use JSON for restore.\n", self.schema);
        for s in &self.sounds {
            out.push_str(&format!("\n## {}\n\nSource: {}\n\nRelative path: {}\n\nStatus: {} · Favorite: {}\n\nTags: {}\n\nComment: {}\n",escape(&s.title),escape(&s.source_id),escape(&s.relative_path),s.status,s.favorite,escape(&s.user_tags.join(", ")),escape(&s.comment)));
            if let Some(p) = &s.profile {
                out.push_str(&format!(
                    "\nMeasured duration: {} seconds\n\n{}\n",
                    p.duration,
                    escape(&p.description)
                ));
            }
            if let Some(n) = self.legacy_evidence.iter().find(|n| n.sound_id == s.id) {
                out.push_str(&format!("\nLegacy filename inference: {}\n\nLegacy measurements cover up to 20 seconds at 12 kHz mono; they are not full-file facts.\n",escape(&n.description)));
            }
        }
        for image in &self.images {
            let a = &image.asset;
            let h = &a.header;
            out.push_str(&format!("\n## Image: {}\n\nSource: {}\n\nRelative path: {}\n\nFavorite: {} · Rating: {}\n\nTags: {}\n\nNotes: {}\n",
                escape(h.relative_path.rsplit('/').next().unwrap_or(&h.relative_path)), escape(&h.source_id), escape(&h.relative_path),
                a.annotations.favorite, a.annotations.rating.map_or("none".into(), |r| r.to_string()),
                escape(&a.annotations.tags.join(", ")), escape(&a.annotations.comment)));
            if let Some(p) = &a.profile {
                out.push_str(&format!("\nMeasured size: {} × {} pixels\n", p.width, p.height));
            }
        }
        out
    }
}
impl Catalog {
    /// Shared export: generation prompts and private references are redacted.
    pub fn export_portable(&self) -> Result<PortableCatalog> {
        self.export_portable_with(GenerationSharing::default())
    }
    /// `sharing` opts generation prompts and private references in (SS-055).
    pub fn export_portable_with(&self, sharing: GenerationSharing) -> Result<PortableCatalog> {
        self.export_audio_portable(None, sharing)
    }
    pub(crate) fn export_portable_for_source(&self, source_id: &str) -> Result<PortableCatalog> {
        let sharing = self.generation_settings().map_err(|_| invalid("Invalid generation settings"))?.snapshot;
        self.export_audio_portable(Some(source_id), sharing)
    }
    fn export_audio_portable(&self, source_id: Option<&str>, sharing: GenerationSharing) -> Result<PortableCatalog> {
        // Consistent snapshot even if a separate connection is writing.
        let tx = self.db.unchecked_transaction()?;
        // Source snapshots carry one folder's members; aliases are library-wide.
        let images = crate::mixed_catalog::export_images(&self.db, source_id, source_id.is_some(), sharing)?;
        let image_tag_aliases = if source_id.is_some() { vec![] } else { crate::mixed_catalog::export_aliases(&self.db)? };
        // Restricted to the exported images, so a folder snapshot names only its own files.
        let image_duplicate_decisions = crate::image_duplicates::export_decisions(&self.db, &images)?;
        let sounds=if let Some(id)=source_id { self.scoped_sounds(&[id.to_owned()])? } else { self.all_sounds()? };
        let ids:HashSet<_>=sounds.iter().map(|s|s.id.clone()).collect();
        let result = PortableCatalog {
            schema: if !image_duplicate_decisions.is_empty() { PORTABLE_SCHEMA_V3 }
                else if images.is_empty() && image_tag_aliases.is_empty() { PORTABLE_SCHEMA } else { PORTABLE_SCHEMA_V2 }.into(),
            sources: self
                .sources()?
                .into_iter()
                .filter(|s|source_id.is_none_or(|id|s.id==id))
                .map(|s| PortableSource {
                    id: s.id,
                    name: s.name,
                    scope: s.scope,
                    files: s.files.into_iter().map(|f| f.relative_path).collect(),
                })
                .collect(),
            sounds,
            clips: self.all_clips()?.into_iter().filter(|c|ids.contains(&c.sound_id)).collect(),
            saved_searches: if source_id.is_some(){vec![]}else{self.saved_searches()?},
            legacy_evidence: self.legacy_evidence()?.into_iter().filter(|e|ids.contains(&e.sound_id)).collect(),
            images,
            image_tag_aliases,
            image_duplicate_decisions,
        };
        tx.commit()?;
        result.validate()?;
        Ok(result)
    }
    pub fn legacy_evidence(&self) -> Result<Vec<LegacyEvidence>> {
        let mut q = self
            .db
            .prepare("SELECT evidence FROM legacy_evidence ORDER BY sound_id")?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .collect()
    }
    pub fn import_portable(
        &mut self,
        data: &PortableCatalog,
        roots: &BTreeMap<String, String>,
        offline_base: &Path,
    ) -> Result<ImportReport> {
        self.import_source_portable(data,roots,offline_base,&[])
    }
    pub(crate) fn import_source_portable(&mut self,data:&PortableCatalog,roots:&BTreeMap<String,String>,offline_base:&Path,history:&[crate::source_catalog::ClipRevision])->Result<ImportReport> {
        data.validate()?;
        if roots
            .keys()
            .any(|id| !data.sources.iter().any(|s| &s.id == id))
        {
            return Err(invalid("Root mapping references unknown source"));
        }
        let mut locations = Vec::new();
        let mut paths: Vec<PathBuf> = self
            .source_headers()?
            .iter()
            .map(|s| PathBuf::from(&s.root))
            .collect();
        if !offline_base.is_absolute() {
            return Err(invalid("Offline catalog location must be absolute"));
        }
        for source in &data.sources {
            let (root, available) = match roots.get(&source.id) {
                Some(root) => {
                    let p = Path::new(root);
                    if !p.is_absolute() || !p.is_dir() {
                        return Err(invalid("Mapped root must be an existing absolute folder"));
                    }
                    (p.canonicalize()?, true)
                }
                None => (offline_base.join(Uuid::new_v4().to_string()), false),
            };
            if paths
                .iter()
                .any(|p| root.starts_with(p) || p.starts_with(&root))
            {
                return Err(invalid(
                    "Mapped root overlaps an existing or imported source",
                ));
            }
            paths.push(root.clone());
            locations.push((source, root, available));
        }
        let tx = self.db.transaction()?;
        for (s, p, available) in &locations {
            tx.execute(
                "INSERT INTO sources(id,name,root,available,scope) VALUES(?1,?2,?3,?4,?5)",
                params![s.id, s.name, path_text(p)?, available, s.scope],
            )?;
            for path in &s.files {
                tx.execute(
                    "INSERT INTO source_files(source_id,relative_path) VALUES(?1,?2)",
                    params![s.id, path],
                )?;
            }
        }
        for s in &data.sounds {
            tx.execute("INSERT INTO sounds(id,source_id,relative_path,title,content_hash,status) VALUES(?1,?2,?3,?4,?5,?6)",params![s.id,s.source_id,s.relative_path,s.title,s.content_hash,s.status])?;
            tx.execute(
                "INSERT INTO annotations(sound_id,tags,comment,favorite) VALUES(?1,?2,?3,?4)",
                params![
                    s.id,
                    serde_json::to_string(&s.user_tags)?,
                    s.comment,
                    s.favorite
                ],
            )?;
            if let Some(p) = &s.profile {
                let previous: Option<String> = tx
                    .query_row(
                        "SELECT profile FROM analyses WHERE content_hash=?1 AND analyzer=?2",
                        params![s.content_hash, ANALYZER],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(raw) = previous {
                    if serde_json::from_str::<crate::catalog::Profile>(&raw)? != *p {
                        return Err(invalid("Import conflicts with cached analysis"));
                    }
                } else {
                    tx.execute(
                        "INSERT INTO analyses(content_hash,analyzer,profile) VALUES(?1,?2,?3)",
                        params![s.content_hash, ANALYZER, serde_json::to_string(p)?],
                    )?;
                }
            }
        }
        for c in &data.clips {
            let r = &c.recipe;
            tx.execute("INSERT INTO clips(id,sound_id,name,asset_version_id,source_sample_rate_hz,start_frame,end_frame,channel_policy,gain_db,fade_in_ms,fade_out_ms,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![c.id,c.sound_id,c.name,r.asset_version_id,r.source_sample_rate_hz,r.start_frame,r.end_frame,r.channel_policy,r.gain_db,r.fade_in_ms,r.fade_out_ms,c.revision,c.created_at,c.updated_at])?;
            tx.execute("INSERT INTO clip_revisions(id,clip_id,revision,asset_version_id,source_sample_rate_hz,start_frame,end_frame,channel_policy,gain_db,fade_in_ms,fade_out_ms,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![Uuid::new_v4().to_string(),c.id,c.revision,r.asset_version_id,r.source_sample_rate_hz,r.start_frame,r.end_frame,r.channel_policy,r.gain_db,r.fade_in_ms,r.fade_out_ms,c.updated_at])?;
        }
        for h in history {
            let r=&h.recipe;
            tx.execute("INSERT INTO clip_revisions(id,clip_id,revision,asset_version_id,source_sample_rate_hz,start_frame,end_frame,channel_policy,gain_db,fade_in_ms,fade_out_ms,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(clip_id,revision) DO NOTHING",params![Uuid::new_v4().to_string(),h.clip_id,h.revision,r.asset_version_id,r.source_sample_rate_hz,r.start_frame,r.end_frame,r.channel_policy,r.gain_db,r.fade_in_ms,r.fade_out_ms,h.created_at])?;
        }
        for s in &data.saved_searches {
            tx.execute(
                "INSERT INTO saved_searches(id,name,query,created_at) VALUES(?1,?2,?3,?4)",
                params![s.id, s.name, serde_json::to_string(&s.query)?, s.created_at],
            )?;
        }
        for n in &data.legacy_evidence {
            tx.execute(
                "INSERT INTO legacy_evidence(sound_id,evidence) VALUES(?1,?2)",
                params![n.sound_id, serde_json::to_string(n)?],
            )?;
        }
        crate::mixed_catalog::import_images(&tx, &data.images, &data.image_tag_aliases)?;
        crate::image_duplicates::import_decisions(&tx, &data.image_duplicate_decisions)?;
        tx.commit()?;
        Ok(ImportReport {
            sources: data.sources.len(),
            sounds: data.sounds.len(),
            images: data.images.len(),
            offline_sources: locations.iter().filter(|(_, _, a)| !*a).count(),
            legacy: !data.legacy_evidence.is_empty(),
        })
    }
}

#[derive(Deserialize)]
struct LegacyCatalog {
    schema: String,
    total_sounds: usize,
    sources: Vec<LegacySource>,
    sounds: Vec<LegacySound>,
}
#[derive(Deserialize)]
struct LegacySource {
    id: String,
    name: String,
}
#[derive(Deserialize)]
struct LegacySound {
    id: String,
    source_id: String,
    relative_path: String,
    title: String,
    #[serde(default)]
    semantic_description: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    audio_profile: Option<LegacyProfile>,
    #[serde(default)]
    user_tags: Vec<String>,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    favorite: bool,
    #[serde(default)]
    content_hash: Option<String>,
}
#[derive(Deserialize)]
struct LegacyProfile {
    analyzed_seconds: Option<f64>,
}

pub fn read_document(path: &Path) -> Result<String> {
    let file = fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(invalid("Catalog exceeds 64 MiB"));
    }
    Ok(text)
}
fn legacy(text: &str) -> Result<LegacyCatalog> {
    let data: LegacyCatalog = serde_json::from_str(text)?;
    if data.schema != LEGACY_SCHEMA
        || data.total_sounds != data.sounds.len()
        || data.sounds.len() > 100_000
        || data.sources.len() > 10_000
    {
        return Err(invalid("Unsupported legacy schema or count mismatch"));
    }
    unique(data.sources.iter().map(|s| s.id.clone()), 256)?;
    unique(data.sounds.iter().map(|s| s.id.clone()), 256)?;
    let mut paths = HashSet::new();
    for s in &data.sounds {
        valid_relative(&s.relative_path)?;
        if !data.sources.iter().any(|r| r.id == s.source_id)
            || !paths.insert((&s.source_id, &s.relative_path))
        {
            return Err(invalid("Invalid legacy source or duplicate path"));
        }
    }
    Ok(data)
}
pub fn preview(text: &str, allow_legacy: bool) -> Result<Preview> {
    if text.len() as u64 > MAX_BYTES {
        return Err(invalid("Catalog exceeds 64 MiB"));
    }
    if let Some(schema) = declared_schema(text) {
        if schema.starts_with(SCHEMA_FAMILY) {
            if ![PORTABLE_SCHEMA, PORTABLE_SCHEMA_V2, PORTABLE_SCHEMA_V3].contains(&schema.as_str()) {
                return Err(invalid(newer_or_unsupported(&schema)));
            }
            // A current-family document never falls back to the legacy parser:
            // its own validation error is the actionable one.
            let data: PortableCatalog = serde_json::from_str(text)?;
            data.validate()?;
            return Ok(Preview {
                schema: data.schema,
                sources: data.sources,
                sounds: data.sounds.len(),
                images: data.images.len(),
                legacy: false,
            });
        }
    }
    if !allow_legacy {
        return Err(invalid(
            "Unsupported portable schema. Legacy import requires explicit opt-in.",
        ));
    }
    let data = legacy(text)?;
    Ok(Preview {
        schema: data.schema,
        sources: data
            .sources
            .into_iter()
            .map(|s| PortableSource {
                id: s.id,
                name: s.name,
                scope: "files".into(),
                files: vec![],
            })
            .collect(),
        sounds: data.sounds.len(),
        images: 0,
        legacy: true,
    })
}
/// Reads only the top-level `schema` string, bounded by the document limit.
fn declared_schema(text: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Head {
        schema: String,
    }
    serde_json::from_str::<Head>(text).ok().map(|h| h.schema)
}
fn newer_or_unsupported(schema: &str) -> &'static str {
    let version = schema
        .strip_prefix(SCHEMA_FAMILY)
        .and_then(|v| v.strip_prefix('v'))
        .and_then(|v| v.parse::<u32>().ok());
    if version.is_some_and(|v| v > 3) {
        "Catalog was written by a newer CreativeShelf; update the app to import it. Nothing was imported."
    } else {
        "Unsupported portable catalog schema"
    }
}
/// Hash legacy media before acquiring the catalog lock. Old path IDs are never content digests.
pub fn prepare_import(
    text: &str,
    roots: &BTreeMap<String, String>,
    allow_legacy: bool,
) -> Result<PortableCatalog> {
    let info = preview(text, allow_legacy)?;
    if !info.legacy {
        return Ok(serde_json::from_str(text)?);
    }
    let old = legacy(text)?;
    let mut data = PortableCatalog {
        schema: PORTABLE_SCHEMA.into(),
        sources: old
            .sources
            .into_iter()
            .map(|s| PortableSource {
                id: s.id,
                name: s.name,
                scope: "files".into(),
                files: vec![],
            })
            .collect(),
        sounds: vec![],
        clips: vec![],
        saved_searches: vec![],
        legacy_evidence: vec![],
        images: vec![],
        image_tag_aliases: vec![],
        image_duplicate_decisions: vec![],
    };
    for s in old.sounds {
        let root = roots
            .get(&s.source_id)
            .ok_or_else(|| invalid("Legacy import requires mapping every source root"))?;
        if !Path::new(root).is_absolute() {
            return Err(invalid("Legacy source root must be absolute"));
        }
        let path = contained(Path::new(root), &s.relative_path)?;
        let hash = hash_file(&path)?;
        if s.content_hash
            .as_ref()
            .is_some_and(|expected| expected != &hash)
        {
            return Err(invalid("Legacy content hash mismatch; nothing imported"));
        }
        let id = Uuid::new_v4().to_string();
        data.sources
            .iter_mut()
            .find(|r| r.id == s.source_id)
            .unwrap()
            .files
            .push(s.relative_path.clone());
        data.legacy_evidence.push(LegacyEvidence {
            sound_id: id.clone(),
            description: if s.semantic_description.is_empty() {
                s.description
            } else {
                s.semantic_description
            },
            provenance: "legacy_filename_inference".into(),
            analyzed_seconds: s
                .audio_profile
                .and_then(|p| p.analyzed_seconds)
                .map(|v| v.min(20.0)),
            measurement_scope: "first_up_to_20_seconds_12khz_mono".into(),
        });
        data.sounds.push(Sound {
            id,
            source_id: s.source_id,
            relative_path: s.relative_path,
            title: s.title,
            content_hash: hash,
            status: "pending".into(),
            profile: None,
            user_tags: s.user_tags,
            comment: s.comment,
            favorite: s.favorite,
        });
    }
    data.validate()?;
    Ok(data)
}
/// Stage beside the destination; publish with an atomic, no-replace hard link.
/// A failed or interrupted write never leaves a partial final catalog.
pub fn write_document(path: &Path, text: &str) -> Result<()> {
    if text.len() as u64 > MAX_BYTES {
        return Err(invalid("Catalog exceeds 64 MiB"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Choose a catalog destination"))?;
    let staging = parent.join(format!(".soundshelf-catalog-{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?;
    let result = file
        .write_all(text.as_bytes())
        .and_then(|_| file.sync_all());
    drop(file);
    let result = result.and_then(|_| fs::hard_link(&staging, path));
    // Only this operation's random staging file is cleaned, never the final pathname.
    let _ = fs::remove_file(&staging);
    result?;
    Ok(())
}
