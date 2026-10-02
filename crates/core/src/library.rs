use crate::{
    catalog::{contained, hash_file, path_text, Catalog, Source},
    invalid,
    media::{analyze, MediaTools},
    Result,
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use walkdir::WalkDir;

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Progress {
    pub job_id: String,
    pub source_id: String,
    pub status: String,
    pub completed: usize,
    pub total: usize,
    pub reused: usize,
    pub failed: usize,
    pub current: String,
    pub errors: Vec<String>,
}

pub fn scan(
    catalog: Arc<Mutex<Catalog>>,
    source: Source,
    tools: &MediaTools,
    cancel: Arc<AtomicBool>,
    job_id: String,
    progress: impl FnMut(Progress),
) -> Result<Progress> {
    scan_paths(catalog, source, tools, cancel, job_id, None, progress)
}
pub fn import_scan(
    catalog: Arc<Mutex<Catalog>>,
    source: Source,
    tools: &MediaTools,
    cancel: Arc<AtomicBool>,
    job_id: String,
    mut progress: impl FnMut(Progress),
) -> Result<Progress> {
    if source.scope != "folder" {
        return scan_paths(catalog, source, tools, cancel, job_id, None, progress);
    }
    let root = Path::new(&source.root);
    let Some(snapshot) = crate::source_catalog::read(root)? else {
        return scan_paths(catalog, source, tools, cancel, job_id, None, progress);
    };
    let mut changed = vec![];
    let mut state = Progress {
        job_id: job_id.clone(),
        source_id: source.id.clone(),
        status: "checking catalog".into(),
        total: snapshot.catalog.sounds.len(),
        ..Default::default()
    };
    for s in &snapshot.catalog.sounds {
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("Import cancelled"));
        }
        let stat = crate::source_catalog::fingerprint(root, &s.relative_path)?;
        let c = catalog.lock().map_err(|_| invalid("Catalog unavailable"))?;
        if c.source_generation(&source.id)? != source.generation {
            return Err(invalid("Source moved; discard stale import"));
        }
        match stat {
            None => {
                c.set_status(&s.id, "missing")?;
                state.completed += 1;
            }
            Some(stat)
                if stat == snapshot.fingerprints[&s.relative_path]
                    && c.sound(&s.id).is_ok_and(|local| {
                        local.content_hash == s.content_hash && local.profile.is_some()
                    }) =>
            {
                c.db_connection()
                    .execute("UPDATE sounds SET status='ready' WHERE id=?1", [&s.id])?;
                state.reused += 1;
                state.completed += 1;
            }
            Some(_) => changed.push(s.relative_path.clone()),
        }
        drop(c);
        progress(state.clone());
    }
    if !changed.is_empty() {
        // Internal catalog membership is already validated; not a renderer file selection.
        let result = scan_paths(
            catalog.clone(),
            source.clone(),
            tools,
            cancel.clone(),
            job_id,
            Some(changed),
            |p| {
                let mut merged = p;
                merged.total = state.total;
                merged.completed += state.completed;
                merged.reused += state.reused;
                progress(merged);
            },
        )?;
        state.completed += result.completed;
        state.reused += result.reused;
        state.failed = result.failed;
        state.errors = result.errors;
    } else {
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("Import cancelled"));
        }
        if let Err(e) = crate::source_catalog::save(&catalog, &source) {
            catalog
                .lock()
                .map_err(|_| invalid("Catalog unavailable"))?
                .snapshot_error(&source.id, &e.to_string())?;
            state
                .errors
                .push(format!("Folder catalog needs retry: {e}"));
        }
    }
    state.status = if state.failed > 0 || !state.errors.is_empty() {
        "completed with warnings"
    } else {
        "complete"
    }
    .into();
    progress(state.clone());
    Ok(state)
}

pub fn scan_paths(
    catalog: Arc<Mutex<Catalog>>,
    source: Source,
    tools: &MediaTools,
    cancel: Arc<AtomicBool>,
    job_id: String,
    targets: Option<Vec<String>>,
    mut progress: impl FnMut(Progress),
) -> Result<Progress> {
    tools.validate()?;
    let root = Path::new(&source.root);
    let mut state = Progress {
        job_id: job_id.clone(),
        source_id: source.id.clone(),
        status: "discovering".into(),
        ..Default::default()
    };
    progress(state.clone());
    if !root.is_dir() {
        catalog
            .lock()
            .map_err(|_| invalid("Catalog unavailable"))?
            .set_available(&source.id, false)?;
        return Err(invalid("Source is offline or inaccessible"));
    }
    catalog
        .lock()
        .map_err(|_| invalid("Catalog unavailable"))?
        .set_available(&source.id, true)?;
    let mut files = vec![];
    let mut complete = true;
    if targets.is_some() || source.scope == "files" {
        let paths = targets.clone().unwrap_or_else(|| {
            source
                .files
                .iter()
                .map(|f| f.relative_path.clone())
                .collect()
        });
        for relative in paths {
            catalog
                .lock()
                .map_err(|_| invalid("Catalog unavailable"))?
                .validate_source_entry(&source.id, &relative)?;
            files.push(root.join(relative));
        }
    } else {
        crate::source_catalog::count_walk();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        {
            if cancel.load(Ordering::Relaxed) {
                return Err(invalid("Scan cancelled"));
            }
            match entry {
                Ok(e) if e.file_type().is_file() && supported(e.path()) => {
                    files.push(e.path().to_owned())
                }
                Ok(_) => {}
                Err(e) => {
                    complete = false;
                    if state.errors.len() < 50 {
                        state.errors.push(e.to_string());
                    }
                }
            }
        }
    }
    files.extend(
        source
            .files
            .iter()
            .filter(|f| {
                targets.is_none()
                    && source.scope == "folder"
                    && root.join(&f.relative_path).is_file()
            })
            .map(|f| root.join(&f.relative_path)),
    );
    files.sort();
    files.dedup();
    state.total = files.len();
    state.status = "analyzing".into();
    progress(state.clone());
    let mut seen = vec![];
    let mut verified_fingerprints = std::collections::BTreeMap::new();
    for file in files {
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("Scan cancelled"));
        }
        let relative = path_text(
            file.strip_prefix(root)
                .map_err(|_| invalid("Invalid source path"))?,
        )?
        .replace(std::path::MAIN_SEPARATOR, "/");
        state.current = relative.clone();
        progress(state.clone());
        let result = (|| -> Result<()> {
            let file = contained(root, &relative)?;
            let fingerprint = crate::source_catalog::fingerprint(root, &relative)?
                .ok_or_else(|| invalid("Source file disappeared"))?;
            let hash = hash_file(&file)?;
            if crate::source_catalog::fingerprint(root, &relative)?.as_ref() != Some(&fingerprint) {
                return Err(invalid("File changed while being hashed"));
            }
            // Reconciliation may only preserve a path after it was verified and hashed.
            // A file deleted between discovery and this point must become missing.
            seen.push(relative.clone());
            let id = {
                catalog
                    .lock()
                    .map_err(|_| invalid("Catalog unavailable"))?
                    .register(&source, &relative, &hash)?
            };
            if catalog
                .lock()
                .map_err(|_| invalid("Catalog unavailable"))?
                .has_cached_profile(&hash)?
            {
                verified_fingerprints.insert(relative.clone(), fingerprint);
                state.reused += 1;
                return Ok(());
            }
            match analyze(tools, &file, cancel.clone()) {
                Ok(profile) => {
                    if hash_file(&file)? != hash
                        || crate::source_catalog::fingerprint(root, &relative)?.as_ref()
                            != Some(&fingerprint)
                    {
                        return Err(invalid("File changed while being analyzed"));
                    }
                    catalog
                        .lock()
                        .map_err(|_| invalid("Catalog unavailable"))?
                        .publish(&source, &id, &hash, &profile)?;
                }
                Err(error) => {
                    catalog
                        .lock()
                        .map_err(|_| invalid("Catalog unavailable"))?
                        .set_status(&id, "failed")?;
                    return Err(error);
                }
            }
            verified_fingerprints.insert(relative.clone(), fingerprint);
            Ok(())
        })();
        if let Err(error) = result {
            catalog
                .lock()
                .map_err(|_| invalid("Catalog unavailable"))?
                .set_entry_status(
                    &source.id,
                    &relative,
                    if file.is_file() { "failed" } else { "missing" },
                )?;
            state.failed += 1;
            if state.errors.len() < 50 {
                state.errors.push(format!("{relative}: {error}"));
            }
        }
        state.completed += 1;
        progress(state.clone());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(invalid("Scan cancelled"));
    }
    catalog
        .lock()
        .map_err(|_| invalid("Catalog unavailable"))?
        .reconcile_paths(&source, &seen, targets.as_deref(), complete)?;
    state.status = if state.failed > 0 || !complete {
        "completed with errors"
    } else {
        "complete"
    }
    .into();
    if source.scope == "folder" && complete && state.failed == 0 {
        if let Err(error) =
            crate::source_catalog::save_scanned(&catalog, &source, &verified_fingerprints)
        {
            catalog
                .lock()
                .map_err(|_| invalid("Catalog unavailable"))?
                .snapshot_error(&source.id, &error.to_string())?;
            state
                .errors
                .push(format!("Folder catalog needs retry: {error}"));
            state.status = "completed with warnings".into();
        }
    }
    state.current.clear();
    progress(state.clone());
    Ok(state)
}
pub fn supported(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        [
            "wav", "mp3", "flac", "ogg", "opus", "m4a", "aac", "aif", "aiff", "wma", "caf",
        ]
        .contains(&s.to_lowercase().as_str())
    })
}
