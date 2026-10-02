//! Per-folder metadata snapshots. Source audio is never copied or rewritten.
use crate::{
    catalog::{Catalog, Source},
    invalid,
    portable::{PortableCatalog, MAX_BYTES},
    Result,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImportIo {
    pub walks: usize,
    pub hashes: usize,
    pub decodes: usize,
}
thread_local! {static IMPORT_IO:std::cell::Cell<ImportIo>=std::cell::Cell::new(ImportIo{walks:0,hashes:0,decodes:0});}
pub fn take_import_io() -> ImportIo {
    IMPORT_IO.with(|c| c.replace(ImportIo::default()))
}
pub(crate) fn count_walk() {
    IMPORT_IO.with(|c| {
        let mut n = c.get();
        n.walks += 1;
        c.set(n);
    });
}
pub(crate) fn count_hash() {
    IMPORT_IO.with(|c| {
        let mut n = c.get();
        n.hashes += 1;
        c.set(n);
    });
}
pub(crate) fn count_decode() {
    IMPORT_IO.with(|c| {
        let mut n = c.get();
        n.decodes += 1;
        c.set(n);
    });
}

pub const SCHEMA: &str = "creativeshelf-source/v1";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub size: u64,
    pub modified_ns: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipRevision {
    pub clip_id: String,
    pub revision: u32,
    pub recipe: crate::catalog::ClipRecipe,
    pub created_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: String,
    pub analyzer: String,
    pub revision: String,
    pub catalog: PortableCatalog,
    pub fingerprints: BTreeMap<String, Fingerprint>,
    pub clip_history: Vec<ClipRevision>,
}
#[derive(Debug, Serialize)]
pub struct SnapshotState {
    pub source_id: String,
    pub dirty: bool,
    pub error: String,
}

fn safe_dir(root: &Path, create: bool) -> Result<PathBuf> {
    let dir = root.join(".creativeshelf");
    if create && !dir.exists() {
        match fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    let metadata = fs::symlink_metadata(&dir)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || dir.canonicalize()? != root.canonicalize()?.join(".creativeshelf")
    {
        return Err(invalid(
            "Source catalog directory must be a local non-symlink folder",
        ));
    }
    Ok(dir)
}
fn nofollow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
}
pub fn read(root: &Path) -> Result<Option<Snapshot>> {
    match fs::symlink_metadata(root.join(".creativeshelf")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    let dir = safe_dir(root, false)?;
    let path = dir.join("catalog.json");
    let mut options = OpenOptions::new();
    options.read(true);
    nofollow(&mut options);
    let file = match options.open(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(f) => f,
    };
    if !file.metadata()?.is_file() {
        return Err(invalid("Source catalog must be a regular file"));
    }
    let mut text = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(invalid("Source catalog exceeds 64 MiB"));
    }
    let snapshot: Snapshot = serde_json::from_str(&text)?;
    snapshot.validate()?;
    Ok(Some(snapshot))
}
impl Snapshot {
    pub fn validate(&self) -> Result<()> {
        if self.schema != SCHEMA
            || self.analyzer != crate::catalog::ANALYZER
            || Uuid::parse_str(&self.revision).is_err()
        {
            return Err(invalid(
                "Unsupported source catalog; use Rebuild catalog to recover",
            ));
        }
        self.catalog.validate()?;
        if self.catalog.sounds.iter().any(|s| {
            s.content_hash.len() != 64 || !s.content_hash.bytes().all(|b| b.is_ascii_hexdigit())
        }) {
            return Err(invalid("Invalid source content digest"));
        }
        if self.catalog.sources.len() != 1
            || self.catalog.sources[0].scope != "folder"
            || !self.catalog.saved_searches.is_empty()
        {
            return Err(invalid(
                "Source catalog must contain exactly one folder and no global saved searches",
            ));
        }
        let members: HashSet<_> = self
            .catalog
            .sounds
            .iter()
            .map(|s| s.relative_path.as_str())
            .collect();
        if members.len() != self.fingerprints.len() {
            return Err(invalid("Source catalog fingerprints do not match members"));
        }
        for (path, stat) in &self.fingerprints {
            crate::catalog::valid_relative(path)?;
            if !members.contains(path.as_str()) || stat.modified_ns.parse::<u128>().is_err() {
                return Err(invalid("Invalid source fingerprint"));
            }
        }
        if self.clip_history.len() > 100_000 {
            return Err(invalid("Too many clip revisions"));
        }
        let mut seen = HashSet::new();
        for revision in &self.clip_history {
            let clip = self
                .catalog
                .clips
                .iter()
                .find(|c| c.id == revision.clip_id)
                .ok_or_else(|| invalid("Revision references unknown clip"))?;
            if revision.revision == 0
                || revision.revision > clip.revision
                || revision.recipe.asset_id != clip.sound_id
                || !seen.insert((&revision.clip_id, revision.revision))
            {
                return Err(invalid("Invalid or duplicate clip revision"));
            }
            crate::catalog::validate_clip_recipe(&revision.recipe, None)?;
            if revision.revision == clip.revision && revision.recipe != clip.recipe {
                return Err(invalid("Current clip revision conflicts with history"));
            }
        }
        Ok(())
    }
}
/// None means confirmed absence; all access/containment errors remain errors.
pub fn fingerprint(root: &Path, relative: &str) -> Result<Option<Fingerprint>> {
    crate::catalog::valid_relative(relative)?;
    // Verify root is readable before interpreting an absent child as deleted.
    let _ = fs::read_dir(root)?;
    let joined = root.join(relative);
    let mut ancestor = joined.as_path();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    return Err(invalid("Source catalog member crosses a symlink"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        if ancestor == root {
            break;
        }
        ancestor = ancestor
            .parent()
            .ok_or_else(|| invalid("Invalid source member"))?;
    }
    let m = match fs::metadata(&joined) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(m) => m,
    };
    if !m.is_file() {
        return Err(invalid("Source member is not a regular audio file"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    nofollow(&mut options);
    let opened = options.open(joined)?;
    if !opened.metadata()?.is_file() {
        return Err(invalid("Source member changed to a non-regular file"));
    }
    let modified = m
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid("Unsupported source timestamp"))?
        .as_nanos();
    Ok(Some(Fingerprint {
        size: m.len(),
        modified_ns: modified.to_string(),
    }))
}
/// Validate all metadata and member access before adding a source. No audio-byte reads.
pub fn import_folder(db: &Arc<Mutex<Catalog>>, root: &Path) -> Result<Source> {
    let root = root.canonicalize()?;
    let Some(mut snapshot) = read(&root)? else {
        return db
            .lock()
            .map_err(|_| invalid("Catalog unavailable"))?
            .add_source(&root);
    };
    for sound in &mut snapshot.catalog.sounds {
        match fingerprint(&root, &sound.relative_path)? {
            None => sound.status = "missing".into(),
            Some(stat) if snapshot.fingerprints[&sound.relative_path] != stat => {
                sound.status = "pending".into()
            }
            Some(_) => {}
        }
    }
    let mut c = db.lock().map_err(|_| invalid("Catalog unavailable"))?;
    if let Some(existing) = c
        .source_headers()?
        .into_iter()
        .find(|s| Path::new(&s.root) == root)
    {
        if existing.scope != "folder" || existing.id != snapshot.catalog.sources[0].id {
            return Err(invalid(
                "Source catalog identity conflicts with this library; use Rebuild catalog",
            ));
        }
        for sound in &snapshot.catalog.sounds {
            let local: Option<String> =
                c.db.query_row(
                    "SELECT id FROM sounds WHERE source_id=?1 AND relative_path=?2",
                    params![existing.id, sound.relative_path],
                    |r| r.get(0),
                )
                .optional()?;
            if local.as_ref().is_some_and(|id| id != &sound.id) {
                return Err(invalid("Source catalog member identity conflicts with this library; rebuild explicitly"));
            }
        }
        c.db.execute("INSERT INTO source_catalog_state(source_id,revision,dirty) VALUES(?1,?2,1) ON CONFLICT(source_id) DO UPDATE SET revision=excluded.revision,dirty=dirty+1",params![existing.id,snapshot.revision])?;
        return Ok(existing); // Local annotations and revisions always win on same-library reimport.
    }
    let id = snapshot.catalog.sources[0].id.clone();
    if let Ok(existing) = c.source(&id) {
        if Path::new(&existing.root).exists() {
            return Err(invalid("Catalog belongs to an existing source; use Relink folder instead of duplicating it"));
        }
        if c.source_headers()?.iter().any(|s| {
            s.id != id && (root.starts_with(&s.root) || Path::new(&s.root).starts_with(&root))
        }) {
            return Err(invalid("Replacement overlaps another source"));
        }
        c.db.execute(
            "UPDATE sources SET root=?2,available=1,generation=generation+1 WHERE id=?1",
            params![id, crate::catalog::path_text(&root)?],
        )?;
    } else {
        let roots = BTreeMap::from([(id.clone(), crate::catalog::path_text(&root)?)]);
        c.import_source_portable(&snapshot.catalog, &roots, &root, &snapshot.clip_history)?;
    }
    c.db.execute("INSERT INTO source_catalog_state(source_id,revision,dirty) VALUES(?1,?2,1) ON CONFLICT(source_id) DO UPDATE SET revision=excluded.revision,dirty=dirty+1",params![id,snapshot.revision])?;
    c.source(&id)
}
impl Catalog {
    pub fn snapshot_states(&self) -> Result<Vec<SnapshotState>> {
        let mut q = self.db.prepare(
            "SELECT source_id,dirty,last_error FROM source_catalog_state ORDER BY source_id",
        )?;
        let rows = q
            .query_map([], |r| {
                Ok(SnapshotState {
                    source_id: r.get(0)?,
                    dirty: r.get::<_, i64>(1)? > 0,
                    error: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    pub fn snapshot_error(&self, id: &str, error: &str) -> Result<()> {
        self.db.execute("INSERT INTO source_catalog_state(source_id,dirty,last_error) VALUES(?1,1,?2) ON CONFLICT(source_id) DO UPDATE SET dirty=MAX(dirty,1),last_error=excluded.last_error",params![id,error])?;
        Ok(())
    }
}
fn lock(root: &Path) -> Result<(PathBuf, File)> {
    let dir = safe_dir(root, true)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    nofollow(&mut options);
    let file = options.open(dir.join("catalog.lock"))?;
    if !file.metadata()?.is_file() {
        return Err(invalid("Invalid catalog lock file"));
    }
    file.try_lock()
        .map_err(|_| invalid("Another application is updating the source catalog; retry later"))?;
    Ok((dir, file))
}
/// I/O happens outside the catalog mutex. A concurrent user edit remains dirty for retry.
pub fn save(db: &Arc<Mutex<Catalog>>, source: &Source) -> Result<()> {
    save_inner(db, source, &BTreeMap::new())
}
pub fn save_scanned(
    db: &Arc<Mutex<Catalog>>,
    source: &Source,
    verified: &BTreeMap<String, Fingerprint>,
) -> Result<()> {
    save_inner(db, source, verified)
}
fn save_inner(
    db: &Arc<Mutex<Catalog>>,
    source: &Source,
    verified: &BTreeMap<String, Fingerprint>,
) -> Result<()> {
    if source.scope != "folder" {
        return Ok(());
    }
    let root = Path::new(&source.root);
    let (dir, _lock) = lock(root)?;
    let disk = read(root)?;
    let (mut catalog, expected, serial, clip_history) = {
        let c = db.lock().map_err(|_| invalid("Catalog unavailable"))?;
        if c.source_generation(&source.id)? != source.generation {
            return Err(invalid("Source moved; discard stale snapshot"));
        }
        let state: Option<(Option<String>, i64)> =
            c.db.query_row(
                "SELECT revision,dirty FROM source_catalog_state WHERE source_id=?1",
                [&source.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (revision, serial) = state.unwrap_or((None, 1));
        let mut data = c.export_portable()?;
        data.sources.retain(|s| s.id == source.id);
        data.sounds
            .retain(|s| s.source_id == source.id && s.status != "missing");
        let ids: HashSet<_> = data.sounds.iter().map(|s| s.id.clone()).collect();
        data.clips.retain(|s| ids.contains(&s.sound_id));
        data.legacy_evidence.retain(|s| ids.contains(&s.sound_id));
        data.saved_searches.clear();
        let mut q=c.db.prepare("SELECT r.clip_id,r.revision,r.asset_version_id,r.source_sample_rate_hz,r.start_frame,r.end_frame,r.channel_policy,r.gain_db,r.fade_in_ms,r.fade_out_ms,r.created_at,c.sound_id FROM clip_revisions r JOIN clips c ON c.id=r.clip_id JOIN sounds s ON s.id=c.sound_id WHERE s.source_id=?1 AND s.status!='missing' ORDER BY r.clip_id,r.revision")?;
        let history = q
            .query_map([&source.id], |r| {
                Ok(ClipRevision {
                    clip_id: r.get(0)?,
                    revision: r.get(1)?,
                    created_at: r.get(10)?,
                    recipe: crate::catalog::ClipRecipe {
                        asset_id: r.get(11)?,
                        asset_version_id: r.get(2)?,
                        source_sample_rate_hz: r.get(3)?,
                        start_frame: r.get(4)?,
                        end_frame: r.get(5)?,
                        channel_policy: r.get(6)?,
                        gain_db: r.get(7)?,
                        fade_in_ms: r.get(8)?,
                        fade_out_ms: r.get(9)?,
                    },
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        (data, revision, serial, history)
    };
    if disk.as_ref().map(|s| &s.revision) != expected.as_ref() {
        return Err(invalid("Source catalog revision changed in another library; local metadata preserved. Reimport or rebuild explicitly."));
    }
    let mut fingerprints = BTreeMap::new();
    for s in &mut catalog.sounds {
        let stat = fingerprint(root, &s.relative_path)?
            .ok_or_else(|| invalid("Source changed during snapshot; retry import before saving"))?;
        if let Some(old) = disk.as_ref().and_then(|d| {
            d.catalog
                .sounds
                .iter()
                .find(|old| old.id == s.id && old.content_hash == s.content_hash)
                .and_then(|_| d.fingerprints.get(&s.relative_path))
        }) {
            if old != &stat && verified.get(&s.relative_path) != Some(&stat) {
                return Err(invalid(
                    "Source changed after analysis; refresh before saving metadata",
                ));
            }
        }
        if verified
            .get(&s.relative_path)
            .is_some_and(|before| before != &stat)
        {
            return Err(invalid(
                "Source changed before snapshot commit; refresh again",
            ));
        }
        fingerprints.insert(s.relative_path.clone(), stat);
    }
    let snapshot = Snapshot {
        schema: SCHEMA.into(),
        analyzer: crate::catalog::ANALYZER.into(),
        revision: Uuid::new_v4().to_string(),
        catalog,
        fingerprints,
        clip_history,
    };
    snapshot.validate()?;
    let bytes = serde_json::to_vec_pretty(&snapshot)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("Source snapshot exceeds 64 MiB"));
    }
    let staged = dir.join(format!(".catalog-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        if safe_dir(root, false)? != dir {
            return Err(invalid("Source catalog directory changed"));
        }
        let c = db.lock().map_err(|_| invalid("Catalog unavailable"))?;
        if c.source_generation(&source.id)? != source.generation {
            return Err(invalid("Source moved; discard stale snapshot"));
        }
        // Recheck revision while holding the writer lock. Rename never follows a destination symlink.
        if read(root)?.as_ref().map(|s| s.revision.clone()) != expected {
            return Err(invalid("Source catalog changed before commit"));
        }
        fs::rename(&staged, dir.join("catalog.json"))?;
        #[cfg(unix)]
        File::open(&dir)?.sync_all()?;
        c.db.execute("INSERT INTO source_catalog_state(source_id,revision,dirty,last_error) VALUES(?1,?2,0,'') ON CONFLICT(source_id) DO UPDATE SET revision=excluded.revision,dirty=CASE WHEN dirty=?3 THEN 0 ELSE dirty END,last_error=''",params![source.id,snapshot.revision,serial])?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(staged);
    }
    result
}
/// Explicit recovery preserves the former snapshot as a sibling backup.
pub fn rebuild(db: &Arc<Mutex<Catalog>>, source: &Source) -> Result<()> {
    let root = Path::new(&source.root);
    let (dir, _lock) = lock(root)?;
    if fs::symlink_metadata(dir.join("catalog.json")).is_ok() {
        fs::rename(
            dir.join("catalog.json"),
            dir.join(format!("catalog-backup-{}.json", Uuid::new_v4())),
        )?;
    }
    let c = db.lock().map_err(|_| invalid("Catalog unavailable"))?;
    c.db.execute("INSERT INTO source_catalog_state(source_id,dirty) VALUES(?1,1) ON CONFLICT(source_id) DO UPDATE SET revision=NULL,dirty=dirty+1,last_error=''",[&source.id])?;
    Ok(())
}
