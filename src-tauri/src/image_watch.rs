//! SS-047 native opt-in folder watching. OS events only nominate paths; the
//! core planner settles them and the import lane does the work. A full or
//! failed event queue, a backend rescan flag or a reconnect becomes one
//! complete refresh, so missed events can never be mistaken for deletions.
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use soundshelf_core::{
    catalog::Catalog,
    image_jobs::{ImageImportJob, ImportError as E},
    image_sources::{watch_probe, RootState, WatchScope},
    image_watch::{observe, WatchPlanner},
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

/// Bounded hand-off from OS callback threads; overflow sets a per-source flag.
const EVENT_QUEUE: usize = 4096;
const TICK: Duration = Duration::from_millis(200);
const SYNC_EVERY: Duration = Duration::from_secs(2);
const PROBE_EVERY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchState {
    Watching,
    Paused,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WatchStatus {
    pub state: WatchState,
    pub error: Option<String>,
}
enum Signal {
    Path(String),
    Rescan,
}
struct Active {
    scope: WatchScope,
    watcher: Option<RecommendedWatcher>,
    planner: WatchPlanner,
    lost: Arc<AtomicBool>,
}
type Sink = Arc<dyn Fn(ImageImportJob) + Send + Sync>;
pub struct ImageWatchers {
    stop: Arc<AtomicBool>,
    wake: Arc<AtomicBool>,
    status: Arc<Mutex<HashMap<String, WatchStatus>>>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}
impl ImageWatchers {
    pub fn new(catalog: Arc<Mutex<Catalog>>, enabled: bool, sink: Option<Sink>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(AtomicBool::new(true));
        let status = Arc::new(Mutex::new(HashMap::new()));
        let thread = enabled.then(|| {
            let (stop, wake, status) = (stop.clone(), wake.clone(), status.clone());
            thread::spawn(move || Loop::new(catalog, status, sink).run(&stop, &wake))
        });
        Self {
            stop,
            wake,
            status,
            thread: Mutex::new(thread),
        }
    }
    /// Re-read watch preferences and source generations on the next tick.
    pub fn nudge(&self) {
        self.wake.store(true, Ordering::Relaxed);
    }
    pub fn status(&self, id: &str) -> Option<WatchStatus> {
        self.status.lock().ok()?.get(id).cloned()
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut handle) = self.thread.lock() {
            if let Some(h) = handle.take() {
                let _ = h.join();
            }
        }
    }
}
impl Drop for ImageWatchers {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Loop {
    catalog: Arc<Mutex<Catalog>>,
    status: Arc<Mutex<HashMap<String, WatchStatus>>>,
    sink: Option<Sink>,
    sender: SyncSender<(String, i64, Signal)>,
    events: Receiver<(String, i64, Signal)>,
    active: HashMap<String, Active>,
}
impl Loop {
    fn new(
        catalog: Arc<Mutex<Catalog>>,
        status: Arc<Mutex<HashMap<String, WatchStatus>>>,
        sink: Option<Sink>,
    ) -> Self {
        let (sender, events) = sync_channel(EVENT_QUEUE);
        Self {
            catalog,
            status,
            sink,
            sender,
            events,
            active: HashMap::new(),
        }
    }
    fn run(mut self, stop: &AtomicBool, wake: &AtomicBool) {
        let mut synced = Instant::now() - SYNC_EVERY;
        let mut probed = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if wake.swap(false, Ordering::Relaxed) || synced.elapsed() >= SYNC_EVERY {
                self.sync();
                synced = Instant::now();
            }
            if probed.elapsed() >= PROBE_EVERY {
                self.probe();
                probed = Instant::now();
            }
            self.drain();
            self.plan();
            thread::sleep(TICK);
        }
        // Dropping each watcher stops its OS subscription before shutdown returns.
        self.active.clear();
    }
    fn report(&self, id: &str, state: WatchState, error: Option<String>) {
        if let Ok(mut s) = self.status.lock() {
            s.insert(id.into(), WatchStatus { state, error });
        }
    }
    /// Match running watchers to the enabled sources and their current generation.
    fn sync(&mut self) {
        let Ok(scopes) = self.catalog.lock().map_err(|_| ()).and_then(|c| c.image_watch_scopes().map_err(|_| ())) else {
            return;
        };
        let wanted: HashMap<_, _> = scopes.into_iter().map(|s| (s.source_id.clone(), s)).collect();
        self.active.retain(|id, a| {
            wanted.get(id).is_some_and(|s| s.generation == a.scope.generation && s.root == a.scope.root)
        });
        if let Ok(mut s) = self.status.lock() {
            s.retain(|id, _| wanted.contains_key(id));
        }
        for (id, scope) in wanted {
            if self.active.contains_key(&id) {
                continue;
            }
            let available = scope.available;
            self.active.insert(
                id.clone(),
                Active {
                    scope,
                    watcher: None,
                    planner: WatchPlanner::default(),
                    lost: Arc::new(AtomicBool::new(false)),
                },
            );
            if available {
                self.start(&id);
            } else {
                self.report(&id, WatchState::Paused, Some("Source is offline".into()));
            }
        }
    }
    fn start(&mut self, id: &str) {
        let Some(a) = self.active.get_mut(id) else {
            return;
        };
        let (sender, lost, scope) = (self.sender.clone(), a.lost.clone(), a.scope.clone());
        let callback = move |result: notify::Result<Event>| {
            let send = |signal| match sender.try_send((scope.source_id.clone(), scope.generation, signal)) {
                Ok(()) => {}
                // Never block an OS thread; a lost event forces a full refresh.
                Err(TrySendError::Full(_)) => lost.store(true, Ordering::Relaxed),
                Err(TrySendError::Disconnected(_)) => {}
            };
            match result {
                Ok(event) if event.need_rescan() => send(Signal::Rescan),
                Ok(event) => {
                    for path in &event.paths {
                        if let Some(relative) = scope.relative(path) {
                            send(Signal::Path(relative));
                        }
                    }
                }
                Err(_) => send(Signal::Rescan),
            }
        };
        let mode = if a.scope.recursive {
            RecursiveMode::Recursive
        } else {
            // Exact files: watch their folder shallowly; siblings are filtered.
            RecursiveMode::NonRecursive
        };
        let started = RecommendedWatcher::new(callback, Config::default().with_follow_symlinks(false))
            .and_then(|mut w| w.watch(&a.scope.root, mode).map(|()| w));
        match started {
            Ok(watcher) => {
                a.watcher = Some(watcher);
                // Nothing observed changes made before this watcher existed (app
                // closed, watching just enabled, drive away): one quick check.
                a.planner.rescan(Instant::now());
                self.report(id, WatchState::Watching, None);
            }
            Err(error) => {
                a.watcher = None;
                // e.g. Linux inotify watch limits; Refresh remains available.
                self.report(id, WatchState::Failed, Some(error.to_string()));
            }
        }
    }
    /// Detect disconnects the OS backend may not report, and reconnects it cannot.
    fn probe(&mut self) {
        let ids: Vec<String> = self.active.keys().cloned().collect();
        for id in ids {
            let Ok(probe) = watch_probe(&self.catalog, &id) else {
                continue;
            };
            let Some(a) = self.active.get_mut(&id) else {
                continue;
            };
            if probe.root_state == RootState::Online {
                // A remount between probes can leave the old OS stream silent.
                if probe.reconnected || a.watcher.is_none() {
                    a.watcher = None;
                    self.start(&id);
                }
            } else {
                a.watcher = None;
                let reason = match probe.root_state {
                    RootState::Inaccessible => "Folder access denied",
                    RootState::Changed => "Folder identity changed; relink to verify",
                    _ => "Source is offline",
                };
                self.report(&id, WatchState::Paused, Some(reason.into()));
            }
        }
    }
    fn drain(&mut self) {
        let now = Instant::now();
        for _ in 0..EVENT_QUEUE {
            let Ok((id, generation, signal)) = self.events.try_recv() else {
                break;
            };
            if let Some(a) = self.active.get_mut(&id).filter(|a| a.scope.generation == generation) {
                match signal {
                    Signal::Path(relative) => a.planner.changed(relative, now),
                    Signal::Rescan => a.planner.rescan(now),
                }
            }
        }
        for a in self.active.values_mut() {
            if a.lost.swap(false, Ordering::Relaxed) {
                a.planner.rescan(now);
            }
        }
    }
    fn plan(&mut self) {
        let now = Instant::now();
        let mut resync = false;
        for (id, a) in &mut self.active {
            if a.watcher.is_none() || (a.planner.pending() == 0 && !a.planner.overflowed()) {
                continue;
            }
            // One job per source: leave changes pending while it is busy.
            if self.catalog.lock().ok().and_then(|c| c.image_source_active(id).ok()) != Some(false) {
                continue;
            }
            let root = a.scope.root.clone();
            let Some(batch) = a.planner.poll(now, |relative| observe(&root, relative)) else {
                continue;
            };
            let queued = self.catalog.lock().map_err(|_| E::Internal).and_then(|c| c.enqueue_watch_batch(&a.scope, &batch));
            match queued {
                Ok(Some(job)) => {
                    if let Some(sink) = &self.sink {
                        sink(job);
                    }
                }
                Ok(None) => {}
                Err(E::SourceConflict | E::QueueFull | E::Busy | E::LimitExceeded) => a.planner.requeue(batch, now),
                Err(_) => resync = true,
            }
        }
        if resync {
            self.sync();
        }
    }
}

#[cfg(all(test, feature = "image-development"))]
mod tests {
    use super::*;
    use soundshelf_core::{
        image_import_preview::{MediaImportPreviews, NativeSelectionKind},
        image_jobs::{ImportMode, JobOrigin, ScanKind},
    };
    use std::{fs, path::PathBuf};
    struct Fixture {
        _temp: tempfile::TempDir,
        db: Arc<Mutex<Catalog>>,
        root: PathBuf,
    }
    fn fixture(files: &[&str], kind: NativeSelectionKind) -> (Fixture, String) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("media");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        for f in files {
            fs::write(root.join(f), b"\x89PNG\r\n\x1a\nnot decoded here").unwrap();
        }
        let db = Arc::new(Mutex::new(Catalog::open(&temp.path().join("db.sqlite")).unwrap()));
        let previews = MediaImportPreviews::default();
        let selected = if matches!(kind, NativeSelectionKind::Folders) { vec![root.clone()] } else { vec![root.join(files[0])] };
        let raw = previews.approve_native(&selected, kind).unwrap();
        let preview = previews.prepare(&db, &uuid::Uuid::new_v4().to_string(), raw, ImportMode::Images).unwrap();
        let job = previews.confirm(&db, &preview.id).unwrap().remove(0);
        // No import worker here: settle the job so the watcher can queue its own.
        db.lock().unwrap().cancel_image_import(&job.id, 0).unwrap();
        db.lock().unwrap().set_image_source_watch(&job.source_id, true).unwrap();
        (Fixture { _temp: temp, db, root }, job.source_id)
    }
    /// Wait for the watcher and its start-up quick check, then clear that job.
    fn start(f: &Fixture, source: &str, scan: ScanKind) -> ImageWatchers {
        let watchers = ImageWatchers::new(f.db.clone(), true, None);
        let jobs = wait_for(&f.db, |jobs| !jobs.is_empty());
        assert_eq!(watchers.status(source).map(|s| s.state), Some(WatchState::Watching));
        assert_eq!((jobs.len(), jobs[0].scan, jobs[0].verify), (1, scan, false), "{jobs:?}");
        f.db.lock().unwrap().cancel_image_import(&jobs[0].id, 0).unwrap();
        thread::sleep(Duration::from_millis(300));
        watchers
    }
    fn watch_jobs(db: &Arc<Mutex<Catalog>>) -> Vec<ImageImportJob> {
        db.lock().unwrap().image_import_jobs(None, 50).unwrap().into_iter().filter(|j| j.origin == JobOrigin::Watch).collect()
    }
    fn wait_for(db: &Arc<Mutex<Catalog>>, mut done: impl FnMut(&[ImageImportJob]) -> bool) -> Vec<ImageImportJob> {
        let start = Instant::now();
        loop {
            let jobs = watch_jobs(db);
            if done(&jobs) {
                return jobs;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "watcher did not queue work: {jobs:?}");
            thread::sleep(Duration::from_millis(50));
        }
    }
    #[test]
    fn image_watch_real_os_events_settle_into_one_scoped_quick_job() {
        let _serial = crate::service::NATIVE_IMAGE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let (f, source) = fixture(&["a.png"], NativeSelectionKind::Folders);
        let watchers = start(&f, &source, ScanKind::Discover);
        // Rapid saves, an editor temp file renamed into place, app-owned noise.
        for i in 0..5 {
            fs::write(f.root.join("b.png.tmp"), vec![i; 64]).unwrap();
        }
        fs::rename(f.root.join("b.png.tmp"), f.root.join("b.png")).unwrap();
        fs::create_dir(f.root.join(".creativeshelf")).unwrap();
        fs::write(f.root.join(".creativeshelf/cache.png"), b"x").unwrap();
        wait_for(&f.db, |jobs| jobs.iter().any(|j| !j.terminal()));
        thread::sleep(Duration::from_secs(3));
        let jobs: Vec<_> = watch_jobs(&f.db).into_iter().filter(|j| !j.terminal()).collect();
        assert_eq!(jobs.len(), 1, "one coalesced job: {jobs:?}");
        let job = &jobs[0];
        assert_eq!((job.scan, job.verify), (ScanKind::Discover, false));
        let c = f.db.lock().unwrap();
        // The selection is exactly the settled final name, never the temp or sidecar.
        let selected: (Option<String>, String) = c.db_connection().query_row("SELECT paths,folders FROM image_import_jobs WHERE id=?1", [&job.id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(selected, (Some(r#"["b.png"]"#.into()), "[]".into()));
        drop(c);
        watchers.shutdown();
        assert!(watchers.thread.lock().unwrap().is_none());
    }
    #[test]
    fn image_watch_exact_files_ignore_siblings_and_disconnect_pauses_then_refreshes() {
        let _serial = crate::service::NATIVE_IMAGE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let (f, source) = fixture(&["only.png", "sibling.png"], NativeSelectionKind::Files);
        let watchers = start(&f, &source, ScanKind::Listed);
        fs::write(f.root.join("sibling.png"), b"changed sibling").unwrap();
        fs::write(f.root.join("new-sibling.png"), b"new").unwrap();
        thread::sleep(Duration::from_secs(4));
        assert!(watch_jobs(&f.db).iter().all(|j| j.terminal()), "sibling changes are invisible to an exact-file watcher");
        fs::write(f.root.join("only.png"), b"changed member").unwrap();
        let jobs = wait_for(&f.db, |jobs| jobs.iter().any(|j| !j.terminal()));
        let job = jobs.iter().find(|j| !j.terminal()).unwrap();
        let paths: Option<String> = f.db.lock().unwrap().db_connection().query_row("SELECT paths FROM image_import_jobs WHERE id=?1", [&job.id], |r| r.get(0)).unwrap();
        assert_eq!(paths.as_deref(), Some(r#"["only.png"]"#));
        f.db.lock().unwrap().cancel_image_import(&job.id, 0).unwrap();

        // Disconnect: the probe pauses watching; reconnecting forces one full check.
        let parked = f.root.with_file_name("parked");
        fs::rename(&f.root, &parked).unwrap();
        let start = Instant::now();
        while watchers.status(&source).map(|s| s.state) != Some(WatchState::Paused) {
            assert!(start.elapsed() < Duration::from_secs(15), "{:?}", watchers.status(&source));
            thread::sleep(Duration::from_millis(50));
        }
        fs::rename(&parked, &f.root).unwrap();
        let jobs = wait_for(&f.db, |jobs| jobs.iter().any(|j| !j.terminal()));
        assert!(jobs.iter().any(|j| !j.terminal() && j.scan == ScanKind::Listed && !j.verify), "{jobs:?}");
        assert_eq!(watchers.status(&source).map(|s| s.state), Some(WatchState::Watching));
        // Disabling stops the watcher and forgets its status.
        f.db.lock().unwrap().set_image_source_watch(&source, false).unwrap();
        watchers.nudge();
        let start = Instant::now();
        while watchers.status(&source).is_some() {
            assert!(start.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(20));
        }
    }
}
