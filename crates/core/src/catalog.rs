use crate::{invalid, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::{Component, Path, PathBuf}, time::Duration};
use uuid::Uuid;

pub const ANALYZER: &str = "soundshelf-pcm-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub root: String,
    pub generation: i64,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub peak: f32,
    pub rms: f32,
    #[serde(default)]
    pub channel_peaks: Vec<f32>,
    #[serde(default)]
    pub channel_rms: Vec<f32>,
    #[serde(default)]
    pub channel_layout: String,
    pub description: String,
    pub tags: Vec<String>,
    pub waveform: Vec<[f32; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sound {
    pub id: String,
    pub source_id: String,
    pub relative_path: String,
    pub title: String,
    pub content_hash: String,
    pub status: String,
    pub profile: Option<Profile>,
    pub user_tags: Vec<String>,
    pub comment: String,
    pub favorite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedSearch {
    pub id: String,
    pub name: String,
    pub query: crate::search::SearchQuery,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipRecipe {
    pub asset_id: String,
    pub asset_version_id: String,
    pub source_sample_rate_hz: u32,
    pub start_frame: String,
    pub end_frame: String,
    #[serde(default = "default_channel_policy")]
    pub channel_policy: String,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub fade_in_ms: u32,
    #[serde(default)]
    pub fade_out_ms: u32,
}

fn default_channel_policy() -> String {
    "preserve".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Clip {
    pub id: String,
    pub sound_id: String,
    pub name: String,
    pub recipe: ClipRecipe,
    pub revision: u32,
    pub is_stale: bool,
    pub stale_reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct Catalog { pub(crate) db: Connection }
pub const SCHEMA_VERSION: u32 = 4;

impl Catalog {
    pub fn open(path: &Path) -> Result<Self> {
        let mut db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.pragma_update(None, "foreign_keys", true)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION { return Err(invalid("Database belongs to a newer SoundShelf version")); }
        if version == 0 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("schema.sql"))?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        } else {
            let tx = db.transaction()?;
            if version < 2 {
                tx.execute_batch("CREATE TABLE jobs (
 id TEXT PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES sources(id),
 kind TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN('queued','running','complete','failed','cancelled')),
 status TEXT NOT NULL,
 lease_owner TEXT,
 lease_until INTEGER,
 completed INTEGER NOT NULL DEFAULT 0,
 total INTEGER NOT NULL DEFAULT 0,
 reused INTEGER NOT NULL DEFAULT 0,
 failed INTEGER NOT NULL DEFAULT 0,
 current TEXT NOT NULL DEFAULT '',
 errors TEXT NOT NULL DEFAULT '[]',
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 UNIQUE(source_id, kind)
);")?;
            }
            if version < 3 {
                tx.execute_batch("CREATE TABLE saved_searches (
 id TEXT PRIMARY KEY,
 name TEXT NOT NULL UNIQUE,
 query TEXT NOT NULL,
 created_at INTEGER NOT NULL
);")?;
            }
            if version < 4 {
                tx.execute_batch("CREATE TABLE clips (
  id TEXT PRIMARY KEY,
  sound_id TEXT NOT NULL REFERENCES sounds(id),
  name TEXT NOT NULL,
  asset_version_id TEXT NOT NULL,
  source_sample_rate_hz INTEGER NOT NULL,
  start_frame TEXT NOT NULL,
  end_frame TEXT NOT NULL,
  channel_policy TEXT NOT NULL DEFAULT 'preserve',
  gain_db REAL NOT NULL DEFAULT 0.0,
  fade_in_ms INTEGER NOT NULL DEFAULT 0,
  fade_out_ms INTEGER NOT NULL DEFAULT 0,
  revision INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX clips_sound ON clips(sound_id);
CREATE TABLE clip_revisions (
  id TEXT PRIMARY KEY,
  clip_id TEXT NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL,
  asset_version_id TEXT NOT NULL,
  source_sample_rate_hz INTEGER NOT NULL,
  start_frame TEXT NOT NULL,
  end_frame TEXT NOT NULL,
  channel_policy TEXT NOT NULL DEFAULT 'preserve',
  gain_db REAL NOT NULL DEFAULT 0.0,
  fade_in_ms INTEGER NOT NULL DEFAULT 0,
  fade_out_ms INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  UNIQUE(clip_id, revision)
);
CREATE INDEX clip_revisions_clip ON clip_revisions(clip_id);")?;
            }
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        }
        Ok(Self { db })
    }

    pub fn db_connection(&self) -> &Connection {
        &self.db
    }

    pub fn db_connection_mut(&mut self) -> &mut Connection {
        &mut self.db
    }

    pub fn sources(&self) -> Result<Vec<Source>> {
        let mut query = self.db.prepare("SELECT id,name,root,generation,available FROM sources ORDER BY name,id")?;
        let sources = query.query_map([], |r| Ok(Source { id:r.get(0)?, name:r.get(1)?, root:r.get(2)?, generation:r.get(3)?, available:r.get(4)? }))?.collect::<std::result::Result<_,_>>()?;
        Ok(sources)
    }

    pub fn source(&self, id: &str) -> Result<Source> {
        self.sources()?.into_iter().find(|s| s.id == id).ok_or_else(|| invalid("Source not found"))
    }

    pub fn add_source(&self, root: &Path) -> Result<Source> {
        let root = root.canonicalize()?;
        if !root.is_dir() { return Err(invalid("Choose a folder")); }
        for existing in self.sources()? {
            let other = Path::new(&existing.root);
            if other == root { return Ok(existing); }
            if root.starts_with(other) || other.starts_with(&root) { return Err(invalid("This folder overlaps an existing source")); }
        }
        let root_text = path_text(&root)?;
        let source = Source { id:Uuid::new_v4().to_string(), name:root.file_name().unwrap_or_default().to_string_lossy().into_owned(), root:root_text, generation:0, available:true };
        self.db.execute("INSERT INTO sources(id,name,root) VALUES(?1,?2,?3)", params![source.id,source.name,source.root])?;
        Ok(source)
    }

    pub fn set_available(&self, id: &str, available: bool) -> Result<()> {
        self.db.execute("UPDATE sources SET available=?2 WHERE id=?1", params![id,available])?;
        Ok(())
    }

    pub fn relink(&mut self, id: &str, new_root: &Path) -> Result<Source> {
        let source = self.source(id)?;
        let new_root = new_root.canonicalize()?;
        if !new_root.is_dir() { return Err(invalid("Replacement must be a folder")); }
        for other in self.sources()?.into_iter().filter(|s| s.id != id) {
            if new_root.starts_with(&other.root) || Path::new(&other.root).starts_with(&new_root) { return Err(invalid("Replacement overlaps another source")); }
        }
        // Verify content, not timestamps: copies may rewrite mtimes but retain analysis.
        for sound in self.all_sounds()?.into_iter().filter(|s| s.source_id == id && s.status != "missing") {
            let file = contained(&new_root, &sound.relative_path)?;
            if hash_file(&file)? != sound.content_hash { return Err(invalid("Replacement differs from this library. No changes made.")); }
        }
        self.db.execute("UPDATE sources SET root=?2,generation=generation+1,available=1 WHERE id=?1 AND generation=?3", params![id,path_text(&new_root)?,source.generation])?;
        self.source(id)
    }

    pub fn register(&self, source: &Source, relative: &str, hash: &str) -> Result<String> {
        valid_relative(relative)?;
        if self.source(&source.id)?.generation != source.generation { return Err(invalid("Source moved; discard stale scan")); }
        let title = Path::new(relative).file_stem().unwrap_or_default().to_string_lossy().replace('_', " ");
        let id = Uuid::new_v4().to_string();
        let cached = self.cached_profile(hash)?.is_some();
        let status = if cached { "ready" } else { "pending" };
        self.db.execute("INSERT INTO sounds(id,source_id,relative_path,title,content_hash,status) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(source_id,relative_path) DO UPDATE SET content_hash=excluded.content_hash,status=excluded.status,title=excluded.title", params![id,source.id,relative,title,hash,status])?;
        Ok(self.db.query_row("SELECT id FROM sounds WHERE source_id=?1 AND relative_path=?2", params![source.id,relative], |r| r.get(0))?)
    }

    pub fn cached_profile(&self, hash: &str) -> Result<Option<Profile>> {
        let raw: Option<String> = self.db.query_row("SELECT profile FROM analyses WHERE content_hash=?1 AND analyzer=?2", params![hash,ANALYZER], |r| r.get(0)).optional()?;
        raw.map(|s| serde_json::from_str(&s).map_err(Into::into)).transpose()
    }

    pub fn publish(&mut self, source: &Source, id: &str, hash: &str, profile: &Profile) -> Result<()> {
        validate_profile(profile)?;
        let tx = self.db.transaction()?;
        let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sounds s JOIN sources f ON f.id=s.source_id WHERE s.id=?1 AND s.source_id=?2 AND s.content_hash=?3 AND f.generation=?4)",params![id,source.id,hash,source.generation],|r| r.get(0))?;
        if !valid { return Err(invalid("Media or source changed during analysis")); }
        tx.execute("INSERT INTO analyses(content_hash,analyzer,profile) VALUES(?1,?2,?3) ON CONFLICT(content_hash,analyzer) DO UPDATE SET profile=excluded.profile",params![hash,ANALYZER,serde_json::to_string(profile)?])?;
        tx.execute("UPDATE sounds SET status='ready' WHERE id=?1",[id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn all_sounds(&self) -> Result<Vec<Sound>> {
        let mut query = self.db.prepare("SELECT s.id,s.source_id,s.relative_path,s.title,s.content_hash,s.status,a.profile,COALESCE(m.tags,'[]'),COALESCE(m.comment,''),COALESCE(m.favorite,0) FROM sounds s LEFT JOIN analyses a ON a.content_hash=s.content_hash AND a.analyzer=?1 LEFT JOIN annotations m ON m.sound_id=s.id ORDER BY s.title,s.id")?;
        let rows = query.query_map([ANALYZER], |r| Ok((Sound { id:r.get(0)?, source_id:r.get(1)?, relative_path:r.get(2)?, title:r.get(3)?, content_hash:r.get(4)?, status:r.get(5)?, profile:None,user_tags:vec![],comment:r.get(8)?,favorite:r.get(9)? }, r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?)))?;
        let mut sounds = vec![];
        for row in rows { let (mut sound,profile,tags) = row?; sound.profile=profile.map(|s|serde_json::from_str(&s)).transpose()?; sound.user_tags=serde_json::from_str(&tags)?; sounds.push(sound); }
        Ok(sounds)
    }

    /// Direct O(1) SQL primary-key lookup — replaces the previous O(N) full-table scan.
    pub fn sound(&self, id: &str) -> Result<Sound> {
        let row = self.db.query_row(
            "SELECT s.id,s.source_id,s.relative_path,s.title,s.content_hash,s.status,
             a.profile,COALESCE(m.tags,'[]'),COALESCE(m.comment,''),COALESCE(m.favorite,0)
             FROM sounds s
             LEFT JOIN analyses a ON a.content_hash=s.content_hash AND a.analyzer=?2
             LEFT JOIN annotations m ON m.sound_id=s.id
             WHERE s.id=?1",
            params![id, ANALYZER],
            |r| Ok((
                r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?,
                r.get::<_,String>(3)?, r.get::<_,String>(4)?, r.get::<_,String>(5)?,
                r.get::<_,Option<String>>(6)?, r.get::<_,String>(7)?,
                r.get::<_,String>(8)?, r.get::<_,bool>(9)?
            ))
        ).optional()?.ok_or_else(|| invalid("Sound not found"))?;
        let (sid, source_id, relative_path, title, content_hash, status, profile_raw, tags_raw, comment, favorite) = row;
        let profile = profile_raw.map(|s| serde_json::from_str(&s)).transpose()?;
        let user_tags: Vec<String> = serde_json::from_str(&tags_raw)?;
        Ok(Sound { id: sid, source_id, relative_path, title, content_hash, status, profile, user_tags, comment, favorite })
    }

    pub fn ready_sound(&self, id: &str) -> Result<Sound> {
        let sound = self.sound(id)?;
        if sound.status != "ready" { return Err(invalid("Sound is not ready")); }
        let source = self.source(&sound.source_id)?;
        if !source.available { return Err(invalid("Source is offline")); }
        Ok(sound)
    }

    pub fn resolve(&self, id: &str) -> Result<PathBuf> {
        let sound = self.ready_sound(id)?;
        let source = self.source(&sound.source_id)?;
        contained(Path::new(&source.root), &sound.relative_path)
    }

    pub fn annotate(&self, id: &str, tags: &[String], comment: &str, favorite: bool) -> Result<()> {
        let sound = self.sound(id)?;
        if sound.status == "missing" { return Err(invalid("Cannot annotate missing sound")); }
        let source = self.source(&sound.source_id)?;
        if !source.available { return Err(invalid("Cannot annotate sound in offline source")); }
        if comment.chars().count() > 10_000 || tags.len() > 64 { return Err(invalid("Annotation is too large")); }
        let mut normalized = vec![];
        for tag in tags {
            let display = tag.replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" ");
            if display.chars().count() > 64 || display.chars().any(char::is_control) { return Err(invalid("Invalid tag")); }
            if !display.is_empty() && !normalized.iter().any(|t: &String|t.to_lowercase()==display.to_lowercase()) { normalized.push(display); }
        }
        self.db.execute("INSERT INTO annotations(sound_id,tags,comment,favorite) VALUES(?1,?2,?3,?4) ON CONFLICT(sound_id) DO UPDATE SET tags=excluded.tags,comment=excluded.comment,favorite=excluded.favorite",params![id,serde_json::to_string(&normalized)?,comment,favorite])?;
        Ok(())
    }

    pub fn set_status(&self, id: &str, status: &str) -> Result<()> {
        if !["pending","failed","missing"].contains(&status) { return Err(invalid("Invalid status transition")); }
        self.db.execute("UPDATE sounds SET status=?2 WHERE id=?1",params![id,status])?;
        Ok(())
    }

    pub fn reconcile(&mut self, source: &Source, seen: &[String], complete: bool) -> Result<()> {
        if !complete { return Ok(()); }
        if self.source(&source.id)?.generation != source.generation { return Err(invalid("Source moved; discard stale scan")); }
        // Targeted query: only fetch id+path for this source; no full profile deserialization.
        // Collect all rows first so the Statement borrow ends before the mutable transaction borrow.
        let existing: Vec<(String, String)> = {
            let mut stmt = self.db.prepare(
                "SELECT id, relative_path FROM sounds WHERE source_id=?1 AND status != 'missing'"
            )?;
            let x = stmt.query_map([&source.id], |r| {
                Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?))
            })?.collect::<std::result::Result<Vec<_>,_>>()?;
            x
        };
        let to_mark_missing: Vec<String> = existing
            .into_iter()
            .filter(|(_, relative_path)| !seen.contains(relative_path))
            .map(|(id, _)| id)
            .collect();
        let tx = self.db.transaction()?;
        for id in to_mark_missing { tx.execute("UPDATE sounds SET status='missing' WHERE id=?1",[id])?; }
        tx.commit()?;
        Ok(())
    }

    pub fn search(&self, query: &crate::search::SearchQuery) -> Result<crate::search::SearchResults> {
        let online: Vec<String> = self.sources()?.into_iter().filter(|s| s.available).map(|s| s.id).collect();
        crate::search::search(self.all_sounds()?, query, &online)
    }

    pub fn save_search(&self, name: &str, query: &crate::search::SearchQuery) -> Result<SavedSearch> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 100 {
            return Err(invalid("Search name must be between 1 and 100 characters"));
        }
        let id = Uuid::new_v4().to_string();
        let now = crate::jobs::now_secs();
        let query_json = serde_json::to_string(query)?;
        self.db.execute(
            "INSERT INTO saved_searches(id, name, query, created_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET query=excluded.query, created_at=excluded.created_at",
            params![id, trimmed, query_json, now],
        )?;
        let saved_id: String = self.db.query_row("SELECT id FROM saved_searches WHERE name=?1", [trimmed], |r| r.get(0))?;
        Ok(SavedSearch {
            id: saved_id,
            name: trimmed.to_string(),
            query: query.clone(),
            created_at: now,
        })
    }

    pub fn saved_searches(&self) -> Result<Vec<SavedSearch>> {
        let mut stmt = self.db.prepare("SELECT id, name, query, created_at FROM saved_searches ORDER BY name, id")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
        })?;
        let mut list = Vec::new();
        for row in rows {
            let (id, name, q_str, created_at) = row?;
            let query: crate::search::SearchQuery = serde_json::from_str(&q_str).map_err(|e| invalid(&e.to_string()))?;
            list.push(SavedSearch { id, name, query, created_at });
        }
        Ok(list)
    }

    pub fn delete_saved_search(&self, id: &str) -> Result<()> {
        let count = self.db.execute("DELETE FROM saved_searches WHERE id=?1", [id])?;
        if count == 0 {
            return Err(invalid("Saved search not found"));
        }
        Ok(())
    }

    pub fn create_clip(&mut self, sound_id: &str, name: &str, recipe: &ClipRecipe) -> Result<Clip> {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() || trimmed_name.chars().count() > 100 {
            return Err(invalid("Clip name must be between 1 and 100 characters"));
        }
        let sound = self.ready_sound(sound_id)?;
        if sound.content_hash != recipe.asset_version_id {
            return Err(invalid("Recipe asset version does not match current sound content hash"));
        }
        let profile = sound.profile.as_ref().ok_or_else(|| invalid("Sound has no profile"))?;
        if profile.sample_rate != recipe.source_sample_rate_hz {
            return Err(invalid("Recipe sample rate does not match sound profile sample rate"));
        }
        validate_clip_recipe(recipe, Some(profile.frames))?;

        let id = Uuid::new_v4().to_string();
        let now = crate::jobs::now_secs();
        let revision = 1;

        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO clips (id, sound_id, name, asset_version_id, source_sample_rate_hz, start_frame, end_frame, channel_policy, gain_db, fade_in_ms, fade_out_ms, revision, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                id,
                sound_id,
                trimmed_name,
                recipe.asset_version_id,
                recipe.source_sample_rate_hz,
                recipe.start_frame,
                recipe.end_frame,
                recipe.channel_policy,
                recipe.gain_db,
                recipe.fade_in_ms,
                recipe.fade_out_ms,
                revision,
                now,
                now,
            ],
        )?;

        let rev_id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO clip_revisions (id, clip_id, revision, asset_version_id, source_sample_rate_hz, start_frame, end_frame, channel_policy, gain_db, fade_in_ms, fade_out_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                rev_id,
                id,
                revision,
                recipe.asset_version_id,
                recipe.source_sample_rate_hz,
                recipe.start_frame,
                recipe.end_frame,
                recipe.channel_policy,
                recipe.gain_db,
                recipe.fade_in_ms,
                recipe.fade_out_ms,
                now,
            ],
        )?;
        tx.commit()?;

        Ok(Clip {
            id,
            sound_id: sound_id.to_string(),
            name: trimmed_name.to_string(),
            recipe: recipe.clone(),
            revision,
            is_stale: false,
            stale_reason: None,
            created_at: now,
            updated_at: now,
        })
    }

    fn evaluate_clip_staleness(&self, sound: Option<&Sound>, recipe: &ClipRecipe) -> (bool, Option<String>) {
        match sound {
            None => (true, Some("Source sound not found".to_string())),
            Some(s) => {
                if s.status == "missing" {
                    return (true, Some("Source sound is missing from library".to_string()));
                }
                if s.content_hash != recipe.asset_version_id {
                    return (true, Some("Source content version changed".to_string()));
                }
                if let Some(profile) = &s.profile {
                    if profile.sample_rate != recipe.source_sample_rate_hz {
                        return (true, Some("Source sample rate mismatch".to_string()));
                    }
                    if let Ok(end) = parse_frame(&recipe.end_frame) {
                        if end > profile.frames {
                            return (true, Some("Recipe boundaries exceed shortened source frames".to_string()));
                        }
                    }
                }
                (false, None)
            }
        }
    }

    pub fn get_clip(&self, id: &str) -> Result<Clip> {
        let mut stmt = self.db.prepare(
            "SELECT id, sound_id, name, asset_version_id, source_sample_rate_hz, start_frame, end_frame, channel_policy, gain_db, fade_in_ms, fade_out_ms, revision, created_at, updated_at
             FROM clips WHERE id = ?1"
        )?;
        let clip_row = stmt.query_row([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, f32>(8)?,
                r.get::<_, u32>(9)?,
                r.get::<_, u32>(10)?,
                r.get::<_, u32>(11)?,
                r.get::<_, i64>(12)?,
                r.get::<_, i64>(13)?,
            ))
        }).optional()?;

        let (cid, sound_id, name, asset_version_id, source_sample_rate_hz, start_frame, end_frame, channel_policy, gain_db, fade_in_ms, fade_out_ms, revision, created_at, updated_at) =
            clip_row.ok_or_else(|| invalid("Clip not found"))?;

        let recipe = ClipRecipe {
            asset_id: sound_id.clone(),
            asset_version_id,
            source_sample_rate_hz,
            start_frame,
            end_frame,
            channel_policy,
            gain_db,
            fade_in_ms,
            fade_out_ms,
        };

        let sound = self.sound(&sound_id).ok();
        let (is_stale, stale_reason) = self.evaluate_clip_staleness(sound.as_ref(), &recipe);

        Ok(Clip {
            id: cid,
            sound_id,
            name,
            recipe,
            revision,
            is_stale,
            stale_reason,
            created_at,
            updated_at,
        })
    }

    pub fn list_clips_for_sound(&self, sound_id: &str) -> Result<Vec<Clip>> {
        let sound = self.sound(sound_id).ok();
        let mut stmt = self.db.prepare(
            "SELECT id, sound_id, name, asset_version_id, source_sample_rate_hz, start_frame, end_frame, channel_policy, gain_db, fade_in_ms, fade_out_ms, revision, created_at, updated_at
             FROM clips WHERE sound_id = ?1 ORDER BY created_at ASC, name ASC"
        )?;
        let rows = stmt.query_map([sound_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, f32>(8)?,
                r.get::<_, u32>(9)?,
                r.get::<_, u32>(10)?,
                r.get::<_, u32>(11)?,
                r.get::<_, i64>(12)?,
                r.get::<_, i64>(13)?,
            ))
        })?;

        let mut clips = Vec::new();
        for row in rows {
            let (cid, sid, name, asset_version_id, source_sample_rate_hz, start_frame, end_frame, channel_policy, gain_db, fade_in_ms, fade_out_ms, revision, created_at, updated_at) = row?;
            let recipe = ClipRecipe {
                asset_id: sid.clone(),
                asset_version_id,
                source_sample_rate_hz,
                start_frame,
                end_frame,
                channel_policy,
                gain_db,
                fade_in_ms,
                fade_out_ms,
            };
            let (is_stale, stale_reason) = self.evaluate_clip_staleness(sound.as_ref(), &recipe);
            clips.push(Clip {
                id: cid,
                sound_id: sid,
                name,
                recipe,
                revision,
                is_stale,
                stale_reason,
                created_at,
                updated_at,
            });
        }
        Ok(clips)
    }

    pub fn all_clips(&self) -> Result<Vec<Clip>> {
        let all_sounds = self.all_sounds()?;
        let sounds_map: std::collections::HashMap<String, Sound> = all_sounds.into_iter().map(|s| (s.id.clone(), s)).collect();
        let mut stmt = self.db.prepare(
            "SELECT id, sound_id, name, asset_version_id, source_sample_rate_hz, start_frame, end_frame, channel_policy, gain_db, fade_in_ms, fade_out_ms, revision, created_at, updated_at
             FROM clips ORDER BY created_at ASC"
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, f32>(8)?,
                r.get::<_, u32>(9)?,
                r.get::<_, u32>(10)?,
                r.get::<_, u32>(11)?,
                r.get::<_, i64>(12)?,
                r.get::<_, i64>(13)?,
            ))
        })?;

        let mut clips = Vec::new();
        for row in rows {
            let (cid, sid, name, asset_version_id, source_sample_rate_hz, start_frame, end_frame, channel_policy, gain_db, fade_in_ms, fade_out_ms, revision, created_at, updated_at) = row?;
            let recipe = ClipRecipe {
                asset_id: sid.clone(),
                asset_version_id,
                source_sample_rate_hz,
                start_frame,
                end_frame,
                channel_policy,
                gain_db,
                fade_in_ms,
                fade_out_ms,
            };
            let sound = sounds_map.get(&sid);
            let (is_stale, stale_reason) = self.evaluate_clip_staleness(sound, &recipe);
            clips.push(Clip {
                id: cid,
                sound_id: sid,
                name,
                recipe,
                revision,
                is_stale,
                stale_reason,
                created_at,
                updated_at,
            });
        }
        Ok(clips)
    }

    pub fn update_clip(&mut self, id: &str, name: &str, recipe: &ClipRecipe, expected_revision: u32) -> Result<Clip> {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() || trimmed_name.chars().count() > 100 {
            return Err(invalid("Clip name must be between 1 and 100 characters"));
        }
        let current = self.get_clip(id)?;
        if current.revision != expected_revision {
            return Err(invalid("Clip revision conflict: clip was modified concurrently. Please reload."));
        }

        let sound = self.sound(&current.sound_id)?;
        let profile_frames = sound.profile.as_ref().map(|p| p.frames);
        validate_clip_recipe(recipe, profile_frames)?;

        let now = crate::jobs::now_secs();
        let new_revision = current.revision + 1;

        let tx = self.db.transaction()?;
        let affected = tx.execute(
            "UPDATE clips SET name = ?2, asset_version_id = ?3, source_sample_rate_hz = ?4, start_frame = ?5, end_frame = ?6, channel_policy = ?7, gain_db = ?8, fade_in_ms = ?9, fade_out_ms = ?10, revision = ?11, updated_at = ?12
             WHERE id = ?1 AND revision = ?13",
            params![
                id,
                trimmed_name,
                recipe.asset_version_id,
                recipe.source_sample_rate_hz,
                recipe.start_frame,
                recipe.end_frame,
                recipe.channel_policy,
                recipe.gain_db,
                recipe.fade_in_ms,
                recipe.fade_out_ms,
                new_revision,
                now,
                expected_revision,
            ],
        )?;

        if affected == 0 {
            return Err(invalid("Clip revision conflict: clip was modified concurrently. Please reload."));
        }

        let rev_id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO clip_revisions (id, clip_id, revision, asset_version_id, source_sample_rate_hz, start_frame, end_frame, channel_policy, gain_db, fade_in_ms, fade_out_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                rev_id,
                id,
                new_revision,
                recipe.asset_version_id,
                recipe.source_sample_rate_hz,
                recipe.start_frame,
                recipe.end_frame,
                recipe.channel_policy,
                recipe.gain_db,
                recipe.fade_in_ms,
                recipe.fade_out_ms,
                now,
            ],
        )?;
        tx.commit()?;

        self.get_clip(id)
    }

    pub fn rebind_clip(&mut self, id: &str) -> Result<Clip> {
        let current = self.get_clip(id)?;
        let sound = self.ready_sound(&current.sound_id)?;
        let profile = sound.profile.as_ref().ok_or_else(|| invalid("Sound has no profile"))?;

        let start = parse_frame(&current.recipe.start_frame)?;
        if start >= profile.frames {
            return Err(invalid("Cannot rebind: clip start frame is beyond the end of the updated audio file"));
        }
        let end = parse_frame(&current.recipe.end_frame)?;
        let bounded_end = end.min(profile.frames);
        if start >= bounded_end {
            return Err(invalid("Cannot rebind: clip boundaries exceed updated audio file length"));
        }

        let new_recipe = ClipRecipe {
            asset_id: current.sound_id.clone(),
            asset_version_id: sound.content_hash.clone(),
            source_sample_rate_hz: profile.sample_rate,
            start_frame: start.to_string(),
            end_frame: bounded_end.to_string(),
            channel_policy: current.recipe.channel_policy.clone(),
            gain_db: current.recipe.gain_db,
            fade_in_ms: current.recipe.fade_in_ms,
            fade_out_ms: current.recipe.fade_out_ms,
        };

        self.update_clip(id, &current.name, &new_recipe, current.revision)
    }

    pub fn delete_clip(&self, id: &str) -> Result<()> {
        let affected = self.db.execute("DELETE FROM clips WHERE id = ?1", [id])?;
        if affected == 0 {
            return Err(invalid("Clip not found"));
        }
        Ok(())
    }
}

pub fn parse_frame(s: &str) -> Result<u64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(invalid("Frame counter cannot be empty"));
    }
    trimmed.parse::<u64>().map_err(|_| invalid("Frame counter must be a valid non-negative integer"))
}

pub fn validate_clip_recipe(recipe: &ClipRecipe, source_frames: Option<u64>) -> Result<(u64, u64)> {
    if recipe.source_sample_rate_hz == 0 {
        return Err(invalid("Source sample rate must be greater than 0"));
    }
    let start = parse_frame(&recipe.start_frame)?;
    let end = parse_frame(&recipe.end_frame)?;
    if start >= end {
        return Err(invalid("Clip start frame must be strictly less than end frame (duration cannot be zero or negative)"));
    }
    if let Some(total) = source_frames {
        if end > total {
            return Err(invalid("Clip end frame exceeds total source frames"));
        }
    }
    if !recipe.gain_db.is_finite() {
        return Err(invalid("Gain in dB must be finite"));
    }
    if recipe.gain_db < -120.0 || recipe.gain_db > 30.0 {
        return Err(invalid("Gain in dB is out of valid range (-120 dB to +30 dB)"));
    }
    Ok((start, end))
}

pub fn valid_relative(value: &str) -> Result<()> {
    if value.is_empty() || value.contains('\\') || value.contains(':') || value.split('/').any(|s| s.is_empty() || s=="." || s=="..") || Path::new(value).components().any(|c|!matches!(c,Component::Normal(_))) { return Err(invalid("Invalid relative media path")); }
    Ok(())
}
pub fn contained(root: &Path, relative: &str) -> Result<PathBuf> {
    valid_relative(relative)?;
    let root = root.canonicalize()?;
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(root) || !path.is_file() { return Err(invalid("Media is outside its source")); }
    Ok(path)
}
pub fn path_text(path: &Path) -> Result<String> { path.to_str().map(str::to_owned).ok_or_else(||invalid("Paths must be valid Unicode")) }
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = blake3::Hasher::new();
    let mut chunk = [0u8;65536];
    loop { let read = file.read(&mut chunk)?; if read==0 {break;} hash.update(&chunk[..read]); }
    Ok(hash.finalize().to_hex().to_string())
}
fn validate_profile(p: &Profile) -> Result<()> {
    if !p.duration.is_finite() || p.duration<=0.0 || p.sample_rate==0 || p.channels==0 || p.frames==0 || !p.peak.is_finite() || !p.rms.is_finite() || p.peak<0.0 || p.rms<0.0 || p.waveform.is_empty() || p.waveform.iter().any(|b| !b[0].is_finite() || !b[1].is_finite() || b[0]>b[1]) { return Err(invalid("Invalid measured profile")); }
    if !p.channel_peaks.is_empty() && p.channel_peaks.len() != p.channels as usize { return Err(invalid("Channel peaks count does not match channel count")); }
    if !p.channel_rms.is_empty() && p.channel_rms.len() != p.channels as usize { return Err(invalid("Channel rms count does not match channel count")); }
    if p.channel_peaks.iter().any(|v| !v.is_finite() || *v < 0.0) { return Err(invalid("Channel peaks must be finite and non-negative")); }
    if p.channel_rms.iter().any(|v| !v.is_finite() || *v < 0.0) { return Err(invalid("Channel rms must be finite and non-negative")); }
    if p.tags.iter().any(|t| t.contains('_')) { return Err(invalid("Tags must not contain underscore separators")); }
    if p.description.trim().is_empty() { return Err(invalid("Profile description must not be empty")); }
    Ok(())
}
