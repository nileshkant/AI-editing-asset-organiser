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

pub struct Catalog { pub(crate) db: Connection }
pub const SCHEMA_VERSION: u32 = 3;

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
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        }
        Ok(Self { db })
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

    pub fn sound(&self, id: &str) -> Result<Sound> { self.all_sounds()?.into_iter().find(|s| s.id == id).ok_or_else(||invalid("Sound not found")) }

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
        let missing: Vec<_> = self.all_sounds()?.into_iter().filter(|s|s.source_id==source.id && !seen.contains(&s.relative_path)).collect();
        let tx = self.db.transaction()?;
        for sound in missing { tx.execute("UPDATE sounds SET status='missing' WHERE id=?1",[sound.id])?; }
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
