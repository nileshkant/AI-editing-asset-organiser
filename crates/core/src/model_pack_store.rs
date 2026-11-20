//! SS-051 pack lifecycle: install, verify, activate, update, roll back and
//! uninstall, with durable state in the app data folder.
//!
//! State lives in `model-packs/state.json`, not in the catalog: packs are
//! device-local binaries, so a catalog backup, restore or portable transfer
//! must never claim a pack is present on another computer. Losing the file
//! only means a pack must be installed again; orphaned folders are removed
//! when the store opens.
//!
//! Layout under the store root:
//! `staging/<id>@<version>/…` partial and completed downloads (resumable),
//! `installed/<id>/<version>/…` verified packs, `state.json`.
use crate::model_pack::{
    check_compatibility, Capability, Compatible, Incompat, IncompatCode, Manifest, PackEntry, PackError, Policy, System,
};
use crate::model_pack_fetch::{verify_file, Downloader, Fetcher, Progress};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc, Condvar, Mutex},
    time::Duration,
};

const STATE_VERSION: u32 = 1;
/// Safety margin kept free on top of a download: 5% plus 64 MiB.
fn margin(bytes: u64) -> u64 {
    bytes / 20 + (64 << 20)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackRef {
    pub id: String,
    pub version: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReindexNote {
    pub capability: Capability,
    pub from_space: Option<String>,
    pub to_space: String,
    /// Store generation at which the space changed.
    pub generation: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Installed {
    /// Copy of the trusted entry that was verified, so verification, removal
    /// and display keep working if the manifest later drops the pack.
    entry: PackEntry,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct State {
    version: u32,
    generation: u64,
    installed: Vec<Installed>,
    /// `id@version` to the last failure shown to the user.
    failures: BTreeMap<String, String>,
    active: BTreeMap<Capability, PackRef>,
    pending_reindex: Vec<ReindexNote>,
}

// ------------------------------------------------------------------ leases

#[derive(Default)]
struct LeaseInner {
    counts: BTreeMap<String, usize>,
    draining: std::collections::BTreeSet<String>,
}
#[derive(Default)]
struct LeaseTable {
    inner: Mutex<LeaseInner>,
    released: Condvar,
}
/// Held by any task that is using an active pack (inference, embedding).
/// A pack cannot be replaced or removed while leases exist.
pub struct Lease {
    pub pack: PackRef,
    pub space_id: String,
    pub generation: u64,
    key: String,
    table: Arc<LeaseTable>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut inner = self.table.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = inner.counts.get_mut(&self.key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                inner.counts.remove(&self.key);
            }
        }
        self.table.released.notify_all();
    }
}
impl LeaseTable {
    fn count(&self, key: &str) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).counts.get(key).copied().unwrap_or(0)
    }
    /// Stops new leases for `key` and waits for the running ones to finish.
    fn drain(&self, key: &str, timeout: Duration) -> Result<(), PackError> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.draining.insert(key.to_owned());
        let deadline = std::time::Instant::now() + timeout;
        while inner.counts.get(key).copied().unwrap_or(0) > 0 {
            let now = std::time::Instant::now();
            if now >= deadline {
                let busy = inner.counts.get(key).copied().unwrap_or(0);
                inner.draining.remove(key);
                return Err(PackError::InUse(busy));
            }
            inner = self.released.wait_timeout(inner, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
        Ok(())
    }
    fn finish(&self, key: &str) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).draining.remove(key);
    }
}

// -------------------------------------------------------------- public view

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    NotInstalled,
    Downloading,
    Verifying,
    /// Downloaded and every file matched its trusted digest; not in use.
    Verified,
    Active,
    Failed,
    Incompatible,
}
#[derive(Debug, Clone, Serialize)]
pub struct PackView {
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub capability: Capability,
    pub phase: Phase,
    pub detail: Option<String>,
    pub incompatible: Option<Incompat>,
    /// True when the platform could not report memory, so RAM guidance was not checked.
    pub memory_unchecked: bool,
    pub license_spdx: String,
    pub notice: String,
    pub notice_url: Option<String>,
    pub download_bytes: u64,
    /// Measured size on disk of the installed files, when installed.
    pub installed_bytes: Option<u64>,
    /// Bytes of an interrupted download that can be resumed.
    pub resumable_bytes: u64,
    pub min_ram_bytes: u64,
    pub runtime: String,
    pub progress: Option<(u64, u64)>,
    pub in_use: usize,
    /// A newer version of the same pack is in the trusted list.
    pub update_available: Option<String>,
    pub retired: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct StorageSummary {
    pub installed_bytes: u64,
    pub partial_bytes: u64,
    pub free_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub packs: Vec<PackView>,
    pub storage: StorageSummary,
    pub active: Vec<(Capability, PackRef)>,
    pub generation: u64,
    pub pending_reindex: Vec<ReindexNote>,
    /// Whether this build bundles any inference runtime; without one an active
    /// pack is recorded but cannot yet run.
    pub runtime_available: bool,
}

struct Job {
    key: String,
    progress: Arc<Progress>,
    verifying: std::sync::atomic::AtomicBool,
}

pub struct ModelPacks {
    root: PathBuf,
    manifest: Manifest,
    policy: Policy,
    system: Arc<dyn System>,
    state: Mutex<State>,
    job: Mutex<Option<Arc<Job>>>,
    leases: Arc<LeaseTable>,
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

impl ModelPacks {
    pub fn open(root: PathBuf, manifest: Manifest, policy: Policy, system: Arc<dyn System>) -> Result<Self, PackError> {
        crate::private_directory(&root)?;
        let mut state = match std::fs::read(root.join("state.json")) {
            Ok(raw) => match serde_json::from_slice::<State>(&raw) {
                Ok(s) if s.version == STATE_VERSION => s,
                _ => {
                    // Unreadable or newer state: keep it for inspection, start clean.
                    let _ = std::fs::rename(root.join("state.json"), root.join("state.json.unreadable"));
                    State { version: STATE_VERSION, ..State::default() }
                }
            },
            Err(_) => State { version: STATE_VERSION, ..State::default() },
        };
        state.version = STATE_VERSION;
        let store = Self { root, manifest, policy, system, state: Mutex::new(state), job: Mutex::new(None), leases: Arc::new(LeaseTable::default()) };
        store.reconcile()?;
        Ok(store)
    }

    fn installed_dir(&self, id: &str, version: &str) -> PathBuf {
        self.root.join("installed").join(id).join(version)
    }
    fn staging_dir(&self, entry: &PackEntry) -> PathBuf {
        self.root.join("staging").join(entry.key())
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn persist(&self, state: &State) -> Result<(), PackError> {
        let tmp = self.root.join("state.json.tmp");
        let bytes = serde_json::to_vec_pretty(state).map_err(|e| PackError::Storage(e.to_string()))?;
        {
            let mut file = std::fs::File::create(&tmp)?;
            std::io::Write::write_all(&mut file, &bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, self.root.join("state.json"))?;
        Ok(())
    }

    /// Startup check with no network: marks packs whose files are missing or
    /// resized, removes folders the state does not know, and drops staging for
    /// packs no longer in the trusted list. A broken pack never breaks startup.
    fn reconcile(&self) -> Result<(), PackError> {
        let mut state = self.lock();
        let mut changed = false;
        let installed = std::mem::take(&mut state.installed);
        for record in installed {
            let dir = self.installed_dir(&record.entry.id, &record.entry.version);
            let intact = record.entry.files.iter().all(|f| {
                std::fs::symlink_metadata(dir.join(&f.path)).map(|m| m.is_file() && m.len() == f.bytes).unwrap_or(false)
            });
            if intact {
                state.installed.push(record);
            } else {
                changed = true;
                let key = record.entry.key();
                state.failures.insert(key, "Some pack files are missing or changed. Install it again.".into());
                state.active.retain(|_, r| !(r.id == record.entry.id && r.version == record.entry.version));
                self.remove_owned(&dir);
            }
        }
        // Folders without a record are leftovers of a crash between steps.
        if let Ok(ids) = std::fs::read_dir(self.root.join("installed")) {
            for id in ids.flatten() {
                let Ok(versions) = std::fs::read_dir(id.path()) else { continue };
                for version in versions.flatten() {
                    let known = state.installed.iter().any(|r| {
                        id.file_name().to_str() == Some(&r.entry.id) && version.file_name().to_str() == Some(&r.entry.version)
                    });
                    if !known {
                        self.remove_owned(&version.path());
                    }
                }
                let _ = std::fs::remove_dir(id.path());
            }
        }
        if let Ok(entries) = std::fs::read_dir(self.root.join("staging")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !self.manifest.packs.iter().any(|p| p.key() == name) {
                    self.remove_owned(&entry.path());
                }
            }
        }
        if changed {
            state.generation += 1;
            self.persist(&state)?;
        } else if !self.root.join("state.json").exists() {
            self.persist(&state)?;
        }
        Ok(())
    }

    /// Deletes a path only if it is inside this store's root and is not a link.
    fn remove_owned(&self, path: &Path) {
        let inside = path.starts_with(&self.root) && path != self.root && !path.components().any(|c| matches!(c, std::path::Component::ParentDir));
        if !inside {
            return;
        }
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_symlink() || m.is_file() => {
                let _ = std::fs::remove_file(path);
            }
            Ok(_) => {
                let _ = std::fs::remove_dir_all(path);
            }
            Err(_) => {}
        }
    }

    // ------------------------------------------------------------- views

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    pub fn overview(&self) -> Overview {
        let state = self.lock().clone();
        let job = self.job.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut views = vec![];
        let mut entries: Vec<(PackEntry, bool)> = self.manifest.packs.iter().map(|p| (p.clone(), false)).collect();
        for record in &state.installed {
            if self.manifest.find(&record.entry.id, &record.entry.version).is_none() {
                entries.push((record.entry.clone(), true));
            }
        }
        let mut installed_total = 0;
        let mut partial_total = 0;
        for (entry, retired) in entries {
            let key = entry.key();
            let record = state.installed.iter().find(|r| r.entry.id == entry.id && r.entry.version == entry.version);
            let active = state.active.get(&entry.capability).is_some_and(|r| r.id == entry.id && r.version == entry.version);
            let running = job.as_ref().filter(|j| j.key == key);
            let compat = if retired {
                Err(Incompat { code: IncompatCode::RetiredFromManifest, detail: "It is no longer in the trusted list; it can be removed but not activated".into() })
            } else {
                check_compatibility(&entry.requirements, self.system.as_ref())
            };
            let installed_bytes = record.map(|_| dir_size(&self.installed_dir(&entry.id, &entry.version)));
            let resumable = dir_size(&self.staging_dir(&entry));
            installed_total += installed_bytes.unwrap_or(0);
            partial_total += resumable;
            let (phase, detail) = if let Some(job) = running {
                (if job.verifying.load(Ordering::Relaxed) { Phase::Verifying } else { Phase::Downloading }, None)
            } else if active {
                (Phase::Active, None)
            } else if record.is_some() {
                (Phase::Verified, None)
            } else if let Some(reason) = state.failures.get(&key) {
                (Phase::Failed, Some(reason.clone()))
            } else if compat.is_err() {
                (Phase::Incompatible, None)
            } else {
                (Phase::NotInstalled, None)
            };
            let update = state
                .installed
                .iter()
                .any(|r| r.entry.id == entry.id)
                .then(|| {
                    self.manifest
                        .packs
                        .iter()
                        .filter(|p| p.id == entry.id && crate::model_pack::version_newer(&p.version, &entry.version))
                        .filter(|p| !state.installed.iter().any(|r| r.entry.key() == p.key()))
                        .max_by_key(|p| crate::model_pack::parse_version(&p.version))
                        .map(|p| p.version.clone())
                })
                .flatten()
                .filter(|_| !retired);
            views.push(PackView {
                id: entry.id.clone(),
                version: entry.version.clone(),
                display_name: entry.display_name.clone(),
                capability: entry.capability,
                phase,
                detail,
                memory_unchecked: matches!(&compat, Ok(Compatible { memory_checked: false })),
                incompatible: compat.err(),
                license_spdx: entry.license.spdx.clone(),
                notice: entry.license.notice.clone(),
                notice_url: entry.license.notice_url.clone(),
                download_bytes: entry.download_bytes(),
                installed_bytes,
                resumable_bytes: resumable,
                min_ram_bytes: entry.requirements.min_ram_bytes,
                runtime: entry.requirements.runtime.clone(),
                progress: running.map(|j| (j.progress.received.load(Ordering::Relaxed), j.progress.total)),
                in_use: self.leases.count(&key),
                update_available: update,
                retired,
            });
        }
        Overview {
            packs: views,
            storage: StorageSummary { installed_bytes: installed_total, partial_bytes: partial_total, free_bytes: self.system.free_disk(&self.root) },
            active: state.active.iter().map(|(c, r)| (*c, r.clone())).collect(),
            generation: state.generation,
            pending_reindex: state.pending_reindex.clone(),
            runtime_available: !self.system.runtimes().is_empty(),
        }
    }

    // ----------------------------------------------------------- install

    /// Downloads and verifies a pack from the trusted list. Blocking; run it
    /// on a worker thread and poll `overview` for progress. One install at a
    /// time. Cancelling or a lost connection keeps the verified prefix so the
    /// next call resumes. The installed copy only appears once every file has
    /// matched its trusted size and digest, so an update never disturbs the
    /// pack currently in use.
    pub fn install(&self, id: &str, version: &str, fetcher: &dyn Fetcher) -> Result<(), PackError> {
        let entry = self.manifest.find(id, version).ok_or(PackError::UnknownPack)?.clone();
        check_compatibility(&entry.requirements, self.system.as_ref()).map_err(PackError::Incompatible)?;
        if self.lock().installed.iter().any(|r| r.entry.key() == entry.key()) {
            return Ok(());
        }
        let job = Arc::new(Job { key: entry.key(), progress: Arc::new(Progress::new(entry.download_bytes())), verifying: false.into() });
        {
            let mut slot = self.job.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_some() {
                return Err(PackError::Busy);
            }
            *slot = Some(job.clone());
        }
        let result = self.run_install(&entry, fetcher, &job);
        *self.job.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let mut state = self.lock();
        match &result {
            Ok(()) => {
                state.failures.remove(&entry.key());
            }
            Err(PackError::Cancelled) => {
                state.failures.remove(&entry.key());
            }
            Err(error) => {
                state.failures.insert(entry.key(), error.to_string());
            }
        }
        let _ = self.persist(&state);
        result
    }

    fn run_install(&self, entry: &PackEntry, fetcher: &dyn Fetcher, job: &Job) -> Result<(), PackError> {
        let staging = self.staging_dir(entry);
        std::fs::create_dir_all(&staging)?;
        // Disk check counts what a resumed download already holds.
        let have = dir_size(&staging).min(entry.download_bytes());
        let need = entry.download_bytes() - have + margin(entry.download_bytes());
        if let Some(free) = self.system.free_disk(&self.root) {
            if free < need {
                return Err(PackError::DiskFull { need, free });
            }
        }
        let downloader = Downloader { fetcher, entry, policy: self.policy, progress: &job.progress };
        let mut ranges = None;
        for file in &entry.files {
            let dest = staging.join(&file.path);
            // A completed, previously verified file is kept; anything else is fetched.
            if std::fs::symlink_metadata(&dest).map(|m| m.is_file() && m.len() == file.bytes).unwrap_or(false)
                && verify_file(&dest, file, Some(&job.progress)).is_ok()
            {
                job.progress.received.fetch_add(file.bytes, Ordering::Relaxed);
                continue;
            }
            self.remove_owned(&dest);
            downloader.file(file, &dest, &mut ranges)?;
        }
        job.verifying.store(true, Ordering::Relaxed);
        for file in &entry.files {
            verify_file(&staging.join(&file.path), file, Some(&job.progress))?;
        }
        // No stray files: only listed files may end up in the installed folder.
        let listed: std::collections::BTreeSet<PathBuf> = entry.files.iter().map(|f| staging.join(&f.path)).collect();
        for found in walk_files(&staging) {
            if !listed.contains(&found) {
                let _ = std::fs::remove_file(found);
            }
        }
        let target = self.installed_dir(&entry.id, &entry.version);
        self.remove_owned(&target);
        std::fs::create_dir_all(target.parent().unwrap_or(&self.root))?;
        std::fs::rename(&staging, &target)?;
        let mut state = self.lock();
        state.installed.retain(|r| r.entry.key() != entry.key());
        state.installed.push(Installed { entry: entry.clone() });
        self.persist(&state)?;
        Ok(())
    }

    /// Asks a running install to stop. Its partial download is kept.
    pub fn cancel_install(&self) {
        if let Some(job) = self.job.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
    }
    /// Deletes an interrupted download that is not being resumed right now.
    pub fn discard_partial(&self, id: &str, version: &str) -> Result<(), PackError> {
        let entry = self.manifest.find(id, version).ok_or(PackError::UnknownPack)?;
        if self.job.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|j| j.key == entry.key()) {
            return Err(PackError::Busy);
        }
        self.remove_owned(&self.staging_dir(entry));
        Ok(())
    }

    // ---------------------------------------------------------- activation

    /// Re-verifies every installed file, then makes the pack the active one for
    /// its capability. Replacing another pack waits for its running tasks
    /// (`drain`), changes the generation and, for embedding packs whose vector
    /// space differs, records a pending reindex instead of mixing vectors.
    /// Switching back to a retained earlier version is the rollback path.
    pub fn activate(&self, id: &str, version: &str, drain: Duration) -> Result<u64, PackError> {
        let entry = {
            let state = self.lock();
            state.installed.iter().find(|r| r.entry.id == id && r.entry.version == version).ok_or(PackError::NotInstalled)?.entry.clone()
        };
        if self.manifest.find(id, version).is_none() {
            return Err(PackError::Incompatible(Incompat { code: IncompatCode::RetiredFromManifest, detail: "It is no longer in the trusted list".into() }));
        }
        check_compatibility(&entry.requirements, self.system.as_ref()).map_err(PackError::Incompatible)?;
        let dir = self.installed_dir(id, version);
        for file in &entry.files {
            if let Err(error) = verify_file(&dir.join(&file.path), file, None) {
                self.fail_installed(&entry, &error.to_string());
                return Err(error);
            }
        }
        let previous = self.lock().active.get(&entry.capability).cloned();
        if previous.as_ref().is_some_and(|p| p.id == id && p.version == version) {
            return Ok(self.generation());
        }
        let previous_key = previous.as_ref().map(|p| format!("{}@{}", p.id, p.version));
        if let Some(key) = &previous_key {
            self.leases.drain(key, drain)?;
        }
        let result = (|| {
            let mut state = self.lock();
            // Removed while it was being verified: nothing to activate.
            if !state.installed.iter().any(|r| r.entry.key() == entry.key()) {
                return Err(PackError::NotInstalled);
            }
            let from_space = previous
                .as_ref()
                .and_then(|p| state.installed.iter().find(|r| r.entry.id == p.id && r.entry.version == p.version))
                .map(|r| r.entry.space_id());
            state.active.insert(entry.capability, PackRef { id: id.to_owned(), version: version.to_owned() });
            state.generation += 1;
            let generation = state.generation;
            if entry.capability.produces_vectors() && from_space.as_deref() != Some(entry.space_id().as_str()) {
                if let Some(from) = from_space {
                    state.pending_reindex.retain(|n| n.capability != entry.capability);
                    state.pending_reindex.push(ReindexNote { capability: entry.capability, from_space: Some(from), to_space: entry.space_id(), generation });
                }
            }
            self.persist(&state)?;
            Ok(generation)
        })();
        if let Some(key) = &previous_key {
            self.leases.finish(key);
        }
        result
    }

    fn fail_installed(&self, entry: &PackEntry, reason: &str) {
        let mut state = self.lock();
        state.installed.retain(|r| r.entry.key() != entry.key());
        state.active.retain(|_, r| !(r.id == entry.id && r.version == entry.version));
        state.failures.insert(entry.key(), reason.to_owned());
        state.generation += 1;
        let _ = self.persist(&state);
        drop(state);
        self.remove_owned(&self.installed_dir(&entry.id, &entry.version));
    }

    pub fn deactivate(&self, capability: Capability, drain: Duration) -> Result<(), PackError> {
        let Some(current) = self.lock().active.get(&capability).cloned() else { return Ok(()) };
        let key = format!("{}@{}", current.id, current.version);
        self.leases.drain(&key, drain)?;
        let result = (|| {
            let mut state = self.lock();
            state.active.remove(&capability);
            state.generation += 1;
            self.persist(&state)
        })();
        self.leases.finish(&key);
        result
    }

    /// Removes one installed version and only the files it owns. An active
    /// pack is deactivated first, after its running tasks finish.
    pub fn uninstall(&self, id: &str, version: &str, drain: Duration) -> Result<(), PackError> {
        let entry = {
            let state = self.lock();
            state.installed.iter().find(|r| r.entry.id == id && r.entry.version == version).ok_or(PackError::NotInstalled)?.entry.clone()
        };
        let key = entry.key();
        self.leases.drain(&key, drain)?;
        let result = (|| {
            {
                let mut state = self.lock();
                let was_active = state.active.get(&entry.capability).is_some_and(|r| r.id == id && r.version == version);
                if was_active {
                    state.active.remove(&entry.capability);
                    state.generation += 1;
                }
                state.installed.retain(|r| r.entry.key() != key);
                state.failures.remove(&key);
                // State first: a crash now leaves an orphan folder, cleaned at the next start.
                self.persist(&state)?;
            }
            self.remove_owned(&self.installed_dir(id, version));
            let _ = std::fs::remove_dir(self.root.join("installed").join(id));
            Ok(())
        })();
        self.leases.finish(&key);
        result
    }

    // ------------------------------------------------------------ usage

    /// Takes a lease on the active pack for a capability. Inference and
    /// embedding code must hold the lease for as long as it uses pack files.
    pub fn acquire(&self, capability: Capability) -> Result<Lease, PackError> {
        let state = self.lock();
        let active = state.active.get(&capability).ok_or(PackError::NotActive)?.clone();
        let record = state.installed.iter().find(|r| r.entry.id == active.id && r.entry.version == active.version).ok_or(PackError::NotActive)?;
        let key = record.entry.key();
        let mut inner = self.leases.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.draining.contains(&key) {
            return Err(PackError::Draining);
        }
        *inner.counts.entry(key.clone()).or_insert(0) += 1;
        Ok(Lease { space_id: record.entry.space_id(), generation: state.generation, pack: active, key, table: self.leases.clone() })
    }
    /// Absolute folder of an installed pack, for the runtime that will load it.
    pub fn pack_dir(&self, lease: &Lease) -> PathBuf {
        self.installed_dir(&lease.pack.id, &lease.pack.version)
    }

    /// Clears a pending reindex once the queue that owns reindexing has
    /// scheduled it. Vectors from different spaces are never mixed meanwhile.
    pub fn acknowledge_reindex(&self, capability: Capability, generation: u64) -> Result<(), PackError> {
        let mut state = self.lock();
        state.pending_reindex.retain(|n| !(n.capability == capability && n.generation <= generation));
        self.persist(&state)
    }
}

fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut found = vec![];
    let Ok(entries) = std::fs::read_dir(root) else { return found };
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(t) if t.is_dir() => found.extend(walk_files(&entry.path())),
            Ok(_) => found.push(entry.path()),
            Err(_) => {}
        }
    }
    found
}
