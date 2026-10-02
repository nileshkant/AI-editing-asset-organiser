//! Lossless clip rendering. Final files are published with no-replace hard links;
//! decoding, validation and hashing run outside the catalog lock.
use crate::{
    catalog::{path_text, validate_clip_recipe, Catalog, Clip, Profile},
    invalid,
    media::{analyze, run_stream, MediaTools},
    Result,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportOptions {
    #[serde(default)]
    pub format: ExportFormat,
    pub sample_rate: Option<u32>,
    pub fade_in_ms: Option<u32>,
    pub fade_out_ms: Option<u32>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    #[default]
    Wav,
    Flac,
}
impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
        }
    }
    fn codec(self) -> &'static str {
        match self {
            Self::Wav => "pcm_s24le",
            Self::Flac => "flac",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportManifest {
    pub schema_version: u32,
    pub clip_id: String,
    pub clip_revision: u32,
    pub name: String,
    pub recipe: crate::catalog::ClipRecipe,
    pub options: ExportOptions,
    /// Relative to the manifest; the handoff is usable after SoundShelf closes.
    pub media_file: String,
    pub content_hash: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: String,
}
#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub path: String,
    pub manifest_path: String,
    pub frames: String,
    pub sound_id: Option<String>,
    pub warning: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct DestinationGrant {
    pub id: String,
    pub path: String,
}

/// Single concurrent render, with one short-lived, one-use native dialog grant.
/// No arbitrary destination path is accepted over IPC.
pub struct ExportService {
    journal: PathBuf,
    state: Mutex<ExportState>,
}
#[derive(Default)]
struct ExportState {
    grant: Option<(DestinationGrant, std::time::Instant)>,
    active: Option<(String, Arc<AtomicBool>)>,
}
impl ExportService {
    pub fn new(journal: PathBuf) -> Result<Self> {
        fs::create_dir_all(&journal)?;
        recover(&journal)?;
        Ok(Self {
            journal,
            state: Mutex::new(ExportState::default()),
        })
    }
    pub fn grant(&self, path: &Path, format: ExportFormat) -> Result<DestinationGrant> {
        if !path.is_absolute() {
            return Err(invalid("Export destination must be absolute"));
        }
        let parent = path
            .parent()
            .ok_or_else(|| invalid("Choose an export file"))?
            .canonicalize()?;
        let path = parent.join(
            path.file_name()
                .ok_or_else(|| invalid("Choose an export filename"))?,
        );
        let path = destination(&path, format)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("Export state unavailable"))?;
        if state.active.is_some() {
            return Err(invalid("An export is already running"));
        }
        let grant = DestinationGrant {
            id: Uuid::new_v4().to_string(),
            path: path_text(&path)?,
        };
        state.grant = Some((grant.clone(), std::time::Instant::now()));
        Ok(grant)
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("Export state unavailable"))?;
        if let Some((active, flag)) = &state.active {
            if active == id {
                flag.store(true, Ordering::Relaxed);
                return Ok(());
            }
        }
        if state
            .grant
            .as_ref()
            .is_some_and(|(grant, _)| grant.id == id)
        {
            state.grant = None;
            return Ok(());
        }
        Err(invalid("Export is no longer running"))
    }
    pub fn shutdown(&self) {
        if let Ok(state) = self.state.lock() {
            if let Some((_, flag)) = &state.active {
                flag.store(true, Ordering::Relaxed);
            }
        }
    }
    pub fn export(
        &self,
        catalog: &Mutex<Catalog>,
        tools: &MediaTools,
        id: &str,
        clip_id: &str,
        revision: u32,
        options: ExportOptions,
    ) -> Result<ExportResult> {
        let (path, cancel) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| invalid("Export state unavailable"))?;
            if state.active.is_some() {
                return Err(invalid("An export is already running"));
            }
            let (grant, created) = state
                .grant
                .as_ref()
                .ok_or_else(|| invalid("Choose an export destination first"))?;
            if grant.id != id || created.elapsed() > Duration::from_secs(600) {
                return Err(invalid("Export destination grant is invalid or expired"));
            }
            let path = PathBuf::from(&grant.path);
            state.grant = None;
            let flag = Arc::new(AtomicBool::new(false));
            state.active = Some((id.to_owned(), flag.clone()));
            (path, flag)
        };
        // Reset even on early validation or I/O errors.
        let _active = ActiveExport(self);
        let (clip, source, profile) = {
            let c = catalog.lock().map_err(|_| invalid("Catalog unavailable"))?;
            let clip = c.get_clip(clip_id)?;
            if clip.is_stale || clip.revision != revision {
                return Err(invalid("Clip changed or is stale; reload before exporting"));
            }
            let sound = c.ready_sound(&clip.sound_id)?;
            let source = c.resolve(&clip.sound_id)?;
            (
                clip,
                source,
                sound
                    .profile
                    .ok_or_else(|| invalid("Sound has no profile"))?,
            )
        };
        let (manifest, measured) = render(
            tools,
            &source,
            &clip,
            &profile,
            &path,
            options,
            cancel,
            &self.journal,
        )?;
        // Catalog publication failure never destroys successfully exported media.
        let indexed = catalog
            .lock()
            .map_err(|_| invalid("Catalog unavailable"))
            .and_then(|mut c| c.index_export(&path, &manifest, &measured));
        let (sound_id, warning) = match indexed {
            Ok(id) => (Some(id), None),
            Err(e) => (None, Some(format!("Media exported successfully, but catalog indexing failed: {e}. Import the destination folder to retry."))),
        };
        Ok(ExportResult {
            path: path_text(&path)?,
            manifest_path: path_text(&sidecar(&path))?,
            frames: manifest.frames,
            sound_id,
            warning,
        })
    }
}
struct ActiveExport<'a>(&'a ExportService);
impl Drop for ActiveExport<'_> {
    fn drop(&mut self) {
        if let Ok(mut s) = self.0.state.lock() {
            s.active = None;
        }
    }
}
fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".soundshelf.json");
    PathBuf::from(name)
}
fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
        Ok(_) => Err(invalid(
            "Destination already exists; choose a different filename",
        )),
    }
}
fn destination(path: &Path, format: ExportFormat) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid("Export destination must be absolute"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Choose an export file"))?
        .canonicalize()?;
    if !parent.is_dir() {
        return Err(invalid("Export parent must be a directory"));
    }
    let name = path
        .file_name()
        .ok_or_else(|| invalid("Choose an export filename"))?;
    let canonical = parent.join(name);
    if canonical != path {
        return Err(invalid(
            "Export destination parent changed or contains a symlink; choose it again",
        ));
    }
    let path = canonical;
    if path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .as_deref()
        != Some(format.extension())
    {
        return Err(invalid(
            "Destination extension must match the selected format",
        ));
    }
    absent(&path)?;
    absent(&sidecar(&path))?;
    Ok(path)
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(invalid("Export cancelled"))
    } else {
        Ok(())
    }
}
fn cancellable_hash(path: &Path, cancel: &AtomicBool) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = blake3::Hasher::new();
    let mut buf = [0u8; 65536];
    loop {
        check_cancel(cancel)?;
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hash.finalize().to_hex().to_string())
}

/// Stage beside the destination so hard-link publication cannot cross volumes.
/// Journal precedes staging. Cleanup removes only our two known temp files,
/// never recursively walks a user folder, and never deletes a final export.
#[derive(Serialize, Deserialize)]
struct JournalEntry {
    dir: PathBuf,
    destination: PathBuf,
}
struct Staging {
    dir: PathBuf,
    journal: PathBuf,
    destination: PathBuf,
}
impl Staging {
    fn new(destination: &Path, journal_root: &Path) -> Result<Self> {
        let parent = destination
            .parent()
            .ok_or_else(|| invalid("Export parent missing"))?;
        let id = Uuid::new_v4().to_string();
        let dir = parent.join(format!(".soundshelf-export-{id}"));
        let journal = journal_root.join(format!("{id}.json"));
        let mut entry = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&journal)?;
        entry.write_all(&serde_json::to_vec(&JournalEntry {
            dir: dir.clone(),
            destination: destination.to_owned(),
        })?)?;
        entry.sync_all()?;
        let staging = Self {
            dir,
            journal,
            destination: destination.to_owned(),
        };
        fs::create_dir(&staging.dir)?;
        Ok(staging)
    }
}
fn cleanup(dir: &Path, destination: &Path) {
    if !fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return;
    }
    // Crash between the sidecar link and the audio link: remove only a sidecar
    // identical to our still-staged manifest, and only when audio is absent.
    if fs::symlink_metadata(destination).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        if fs::metadata(dir.join("manifest")).is_ok_and(|m| m.len() <= 16384) {
            if let Ok(staged) = fs::read(dir.join("manifest")) {
                let sidecar = sidecar(destination);
                if fs::metadata(&sidecar).is_ok_and(|m| m.len() <= 16384)
                    && fs::read(&sidecar).is_ok_and(|bytes| bytes == staged)
                {
                    let _ = fs::remove_file(sidecar);
                }
            }
        }
    }
    if fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        for name in ["media", "manifest"] {
            let _ = fs::remove_file(dir.join(name));
        }
        let _ = fs::remove_dir(dir);
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        cleanup(&self.dir, &self.destination);
        if !self.dir.exists() && self.dir.parent().is_some_and(Path::is_dir) {
            let _ = fs::remove_file(&self.journal);
        }
    }
}
pub fn recover(journal_root: &Path) -> Result<()> {
    for entry in fs::read_dir(journal_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if Uuid::parse_str(id).is_err() || path.extension().and_then(|s| s.to_str()) != Some("json")
        {
            continue;
        }
        if entry.metadata()?.len() > 16384 {
            continue;
        }
        let Ok(entry) = serde_json::from_slice::<JournalEntry>(&fs::read(&path)?) else {
            continue;
        };
        let dir = entry.dir;
        if entry.destination.parent() != dir.parent() {
            continue;
        }
        if dir.is_absolute()
            && dir.file_name().and_then(|s| s.to_str()) == Some(&format!(".soundshelf-export-{id}"))
        {
            cleanup(&dir, &entry.destination);
            if !dir.exists() && dir.parent().is_some_and(Path::is_dir) {
                fs::remove_file(&path)?;
            }
        }
    }
    Ok(())
}

pub fn render(
    tools: &MediaTools,
    source: &Path,
    clip: &Clip,
    profile: &Profile,
    path: &Path,
    options: ExportOptions,
    cancel: Arc<AtomicBool>,
    journal_root: &Path,
) -> Result<(ExportManifest, Profile)> {
    tools.validate()?;
    check_cancel(&cancel)?;
    if clip.is_stale || clip.sound_id != clip.recipe.asset_id {
        return Err(invalid("Clip is stale or belongs to another sound"));
    }
    if profile.sample_rate != clip.recipe.source_sample_rate_hz {
        return Err(invalid("Source sample rate changed"));
    }
    let path = destination(path, options.format)?;
    if path == source.canonicalize()? {
        return Err(invalid("Original media cannot be overwritten"));
    }
    let mut recipe = clip.recipe.clone();
    recipe.fade_in_ms = options.fade_in_ms.unwrap_or(recipe.fade_in_ms);
    recipe.fade_out_ms = options.fade_out_ms.unwrap_or(recipe.fade_out_ms);
    let (start, end) = validate_clip_recipe(&recipe, Some(profile.frames))?;
    if !["preserve", "mono", "stereo"].contains(&recipe.channel_policy.as_str()) {
        return Err(invalid("Unsupported channel policy"));
    }
    let rate = options.sample_rate.unwrap_or(profile.sample_rate);
    if !(8000..=384000).contains(&rate) {
        return Err(invalid("Unsupported export sample rate"));
    }
    let duration_ms = (end - start) as u128 * 1000 / profile.sample_rate as u128;
    if recipe.fade_in_ms as u128 > duration_ms || recipe.fade_out_ms as u128 > duration_ms {
        return Err(invalid("Fade exceeds clip duration"));
    }
    let expected = ((end - start) as u128 * rate as u128 + profile.sample_rate as u128 / 2)
        / profile.sample_rate as u128;
    let expected = u64::try_from(expected).map_err(|_| invalid("Export is too long"))?;
    if expected == 0 {
        return Err(invalid("Selection is shorter than one output frame"));
    }
    if cancellable_hash(source, &cancel)? != recipe.asset_version_id {
        return Err(invalid(
            "Source content changed; rescan and rebind the clip",
        ));
    }
    let staging = Staging::new(&path, journal_root)?;
    let media = staging.dir.join("media");
    let manifest_file = staging.dir.join("manifest");
    let mut filters = vec![
        format!("atrim=start_sample={start}:end_sample={end}"),
        "asetpts=PTS-STARTPTS".into(),
        format!("volume={}dB", recipe.gain_db),
    ];
    // Apply fades in source frames before rate conversion, avoiding timestamp rounding.
    if recipe.fade_in_ms > 0 {
        filters.push(format!(
            "afade=t=in:ss=0:ns={}",
            recipe.fade_in_ms as u64 * profile.sample_rate as u64 / 1000
        ));
    }
    if recipe.fade_out_ms > 0 {
        let fade = recipe.fade_out_ms as u64 * profile.sample_rate as u64 / 1000;
        filters.push(format!("afade=t=out:ss={}:ns={fade}", end - start - fade));
    }
    if rate != profile.sample_rate {
        filters.push(format!("aresample={rate}"));
    }
    // Trim resampler tail to the nearest output frame. Never pad a truncated
    // decode: validation must reject a short output rather than conceal it.
    filters.push(format!("atrim=end_sample={expected}"));
    let mut cmd = Command::new(&tools.ffmpeg);
    cmd.args(["-v", "error", "-nostdin", "-n", "-threads", "1", "-i"])
        .arg(source)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-map_metadata",
            "-1",
            "-af",
            &filters.join(","),
            "-ar",
            &rate.to_string(),
        ]);
    let channels = match recipe.channel_policy.as_str() {
        "mono" => 1,
        "stereo" => 2,
        _ => profile.channels,
    };
    cmd.args([
        "-ac",
        &channels.to_string(),
        "-c:a",
        options.format.codec(),
        "-f",
        options.format.extension(),
    ])
    .arg(&media);
    run_stream(cmd, cancel.clone(), Duration::from_secs(7200), |r| {
        let mut buf = [0; 4096];
        while r.read(&mut buf)? != 0 {}
        Ok(())
    })
    .map_err(|e| {
        invalid(&format!(
            "Export encoder failed: {e}. Check available disk space and destination permissions."
        ))
    })?;
    let measured = analyze(tools, &media, cancel.clone())?;
    if measured.frames != expected || measured.sample_rate != rate || measured.channels != channels
    {
        return Err(invalid(
            "Export validation failed: frame count, rate or channels differ",
        ));
    }
    // A shortened decode must not be silently padded into a valid-looking export.
    // Recheck full source identity after processing to reject concurrent replacement.
    if cancellable_hash(source, &cancel)? != recipe.asset_version_id {
        return Err(invalid("Source changed during export"));
    }
    let hash = cancellable_hash(&media, &cancel)?;
    let manifest = ExportManifest {
        schema_version: 1,
        clip_id: clip.id.clone(),
        clip_revision: clip.revision,
        name: clip.name.clone(),
        recipe,
        options,
        media_file: path.file_name().unwrap().to_string_lossy().into_owned(),
        content_hash: hash,
        sample_rate: rate,
        channels,
        frames: expected.to_string(),
    };
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest_file)?;
    f.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    f.sync_all()?;
    File::open(&media)?.sync_all()?;
    check_cancel(&cancel)?;
    // hard_link is an atomic create-if-absent, unlike rename on Unix. A race
    // with an existing destination can never replace that file (or a symlink).
    fs::hard_link(&manifest_file, sidecar(&path))?;
    if let Err(e) =
        check_cancel(&cancel).and_then(|_| fs::hard_link(&media, &path).map_err(Into::into))
    {
        let _ = fs::remove_file(sidecar(&path));
        return Err(e);
    }
    // Publication is the cancellation boundary; after this point success wins.
    Ok((manifest, measured))
}
impl Catalog {
    pub fn index_export(
        &mut self,
        path: &Path,
        manifest: &ExportManifest,
        profile: &Profile,
    ) -> Result<String> {
        let path = path.canonicalize()?;
        let source = self
            .sources()?
            .into_iter()
            .find(|s| path.starts_with(&s.root));
        let source = match source {
            Some(s) => s,
            None => self.add_source(path.parent().unwrap())?,
        };
        let relative = path_text(
            path.strip_prefix(&source.root)
                .map_err(|_| invalid("Export outside destination"))?,
        )?
        .replace('\\', "/");
        crate::catalog::validate_profile(profile)?;
        let id = Uuid::new_v4().to_string();
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO analyses(content_hash,analyzer,profile) VALUES(?1,?2,?3) ON CONFLICT(content_hash,analyzer) DO NOTHING",
            rusqlite::params![manifest.content_hash, crate::catalog::ANALYZER, serde_json::to_string(profile)?])?;
        tx.execute("INSERT INTO sounds(id,source_id,relative_path,title,content_hash,status) VALUES(?1,?2,?3,?4,?5,'ready') ON CONFLICT(source_id,relative_path) DO UPDATE SET content_hash=excluded.content_hash,status='ready',title=excluded.title",
            rusqlite::params![id, source.id, relative, manifest.name, manifest.content_hash])?;
        let id: String = tx.query_row(
            "SELECT id FROM sounds WHERE source_id=?1 AND relative_path=?2",
            rusqlite::params![source.id, relative],
            |row| row.get(0),
        )?;
        tx.execute("INSERT INTO annotations(sound_id,tags,comment) VALUES(?1,?2,?3) ON CONFLICT(sound_id) DO NOTHING",
            rusqlite::params![id, serde_json::to_string(&["exported clip"])? , format!("{}: exported clip {} revision {}", manifest.name, manifest.clip_id, manifest.clip_revision)])?;
        tx.commit()?;
        Ok(id)
    }
}
