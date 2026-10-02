use soundshelf_core::{
    catalog::{Catalog, Source},
    export::ExportService,
    jobs::{now_secs, Job},
    library::{scan_paths, Progress},
    media::MediaTools,
    playback::Player,
    waveform::WaveformService,
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, RecvTimeoutError, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use uuid::Uuid;

struct Work {
    source_id: String,
    job_id: String,
}

pub struct AppState {
    pub agent: Arc<soundshelf_agent::Agent>,
    pub data_directory: PathBuf,
    pub catalog_imports: Arc<Mutex<std::collections::HashMap<String, (String, bool)>>>,
    pub catalog_roots: Arc<Mutex<std::collections::HashMap<(String, String), String>>>,
    pub catalog: Arc<Mutex<Catalog>>,
    pub tools: Option<MediaTools>,
    pub player: Arc<Player>,
    pub progress: Arc<Mutex<Vec<Progress>>>,
    pub waveforms: Arc<WaveformService>,
    pub exports: Arc<ExportService>,
    pub cancel: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    sender: SyncSender<Work>,
    scheduled: Arc<Mutex<HashSet<String>>>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}
impl AppState {
    pub fn new(
        data_directory: PathBuf,
        resources: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&data_directory)?;
        let cache_dir = data_directory.join("cache").join("waveforms");
        std::fs::create_dir_all(&cache_dir)?;
        let waveforms = Arc::new(WaveformService::new(cache_dir));
        let catalog = Arc::new(Mutex::new(Catalog::open(
            &data_directory.join("library.sqlite"),
        )?));
        let mut tools = MediaTools {
            ffmpeg: resources.join("media").join(if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            }),
            ffprobe: resources.join("media").join(if cfg!(windows) {
                "ffprobe.exe"
            } else {
                "ffprobe"
            }),
        };
        if tools.validate().is_err() {
            if let Some(discovered) = MediaTools::discover() {
                tools = discovered;
            }
        }
        let tools = tools.validate().ok().map(|_| tools);
        let progress = Arc::new(Mutex::new(Vec::<Progress>::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let owner = Uuid::new_v4().to_string();
        let (tx, rx) = sync_channel::<Work>(32);
        let db = catalog.clone();
        let updates = progress.clone();
        let flag = cancel.clone();
        let quit = stop.clone();
        let media = tools.clone();
        let worker_owner = owner.clone();
        let scheduled = Arc::new(Mutex::new(HashSet::new()));
        let scheduled_worker = scheduled.clone();
        let worker = thread::spawn(move || loop {
            if quit.load(Ordering::Relaxed) {
                break;
            }
            let work = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(s) => s,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            let id = work.source_id.clone();
            let job_id = work.job_id.clone();
            let claimed = {
                match db.lock() {
                    Ok(catalog) => {
                        flag.store(false, Ordering::Relaxed);
                        let job = catalog
                            .claim_job(&job_id, &worker_owner, now_secs())
                            .ok()
                            .flatten();
                        if let Ok(mut pending) = scheduled_worker.lock() {
                            pending.remove(&job_id);
                        }
                        job
                    }
                    Err(_) => None,
                }
            };
            let Some(job) = claimed else {
                continue;
            };
            let source = match db.lock().ok().and_then(|c| c.source(&id).ok()) {
                Some(s) => s,
                None => continue,
            };
            if let Some(tools) = &media {
                let persist = db.clone();
                let persist_owner = worker_owner.clone();
                let result = scan_paths(
                    db.clone(),
                    source,
                    tools,
                    flag.clone(),
                    job_id.clone(),
                    job.paths,
                    |p| {
                        if let Ok(catalog) = persist.lock() {
                            let _ = catalog.persist_progress(&persist_owner, &p);
                        }
                        if let Ok(mut list) = updates.lock() {
                            if let Some(old) = list.iter_mut().find(|s| s.job_id == p.job_id) {
                                *old = p;
                            } else {
                                list.push(p);
                            }
                        }
                    },
                );
                match result {
                    Ok(p) => {
                        if let Ok(catalog) = db.lock() {
                            let _ = catalog.finish_job(&job_id, &worker_owner, &p, false);
                        }
                        if let Ok(mut list) = updates.lock() {
                            if let Some(old) = list.iter_mut().find(|s| s.job_id == p.job_id) {
                                *old = p;
                            } else {
                                list.push(p);
                            }
                        }
                    }
                    Err(error) => {
                        let cancelled = flag.load(Ordering::Relaxed);
                        let mut failed = Progress {
                            job_id: job_id.clone(),
                            source_id: id.clone(),
                            status: if cancelled { "cancelled" } else { "failed" }.into(),
                            errors: vec![error.to_string()],
                            ..Default::default()
                        };
                        if let Ok(list) = updates.lock() {
                            if let Some(p) = list.iter().find(|s| s.job_id == job_id) {
                                failed.completed = p.completed;
                                failed.total = p.total;
                                failed.reused = p.reused;
                                failed.failed = p.failed;
                                failed.current = p.current.clone();
                                failed.errors.extend(p.errors.clone());
                            }
                        }
                        if let Ok(catalog) = db.lock() {
                            let _ = catalog.finish_job(&job_id, &worker_owner, &failed, cancelled);
                        }
                        if let Ok(mut list) = updates.lock() {
                            if let Some(p) = list.iter_mut().find(|s| s.job_id == job_id) {
                                *p = failed;
                            }
                        }
                    }
                }
            }
        });
        let player = Arc::new(Player::new(tools.clone()));
        let exports = Arc::new(ExportService::new(data_directory.join("export-journal"))?);
        let state = Self {
            agent: Arc::new(soundshelf_agent::Agent::with_discovery(data_directory.join("runtime/mcp.json"))),
            data_directory,
            catalog_imports: Arc::new(Mutex::new(std::collections::HashMap::new())),
            catalog_roots: Arc::new(Mutex::new(std::collections::HashMap::new())),
            catalog,
            tools,
            player,
            progress,
            waveforms,
            exports,
            cancel,
            stop,
            sender: tx,
            scheduled,
            worker: Mutex::new(Some(worker)),
        };
        state.resume_persisted()?;
        Ok(state)
    }
    fn remember(&self, job: &Job) -> Result<(), String> {
        let mut list = self.progress.lock().map_err(|e| e.to_string())?;
        list.retain(|p| p.job_id != job.id);
        list.push(job.progress());
        Ok(())
    }
    fn resume_persisted(&self) -> Result<(), String> {
        let jobs = {
            let catalog = self.catalog.lock().map_err(|e| e.to_string())?;
            catalog
                .recover_jobs(now_secs())
                .map_err(|e| e.to_string())?
        };
        for job in jobs {
            self.remember(&job)?;
            let source = self
                .catalog
                .lock()
                .map_err(|e| e.to_string())?
                .source(&job.source_id)
                .map_err(|e| e.to_string())?;
            self.schedule(source, &job)?;
        }
        let history = {
            let catalog = self.catalog.lock().map_err(|e| e.to_string())?;
            catalog.active_jobs().map_err(|e| e.to_string())?
        };
        let mut list = self.progress.lock().map_err(|e| e.to_string())?;
        for job in history {
            if !list.iter().any(|p| p.job_id == job.id) {
                list.push(job.progress());
            }
        }
        Ok(())
    }
    pub fn enqueue(&self, source: Source) -> Result<(), String> {
        self.enqueue_paths(source, None)
    }
    pub fn enqueue_paths(&self, source: Source, paths: Option<Vec<String>>) -> Result<(), String> {
        if self.tools.is_none() {
            return Err("Media binaries are not included in this development package".into());
        }
        let catalog = self.catalog.lock().map_err(|e| e.to_string())?;
        let job = catalog
            .enqueue_paths(&source.id, paths)
            .map_err(|e| e.to_string())?;
        self.remember(&job)?;
        if job.state != "running" {
            self.schedule(source, &job)?;
        }
        Ok(())
    }
    fn schedule(&self, source: Source, job: &Job) -> Result<(), String> {
        let mut pending = self.scheduled.lock().map_err(|e| e.to_string())?;
        if !pending.insert(job.id.clone()) {
            return Ok(());
        }
        if self
            .sender
            .try_send(Work {
                source_id: source.id,
                job_id: job.id.clone(),
            })
            .is_err()
        {
            pending.remove(&job.id);
            return Err("Import queue is full; retry after current imports finish".into());
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        self.agent.stop();
        self.exports.shutdown();
        self.player.stop();
        self.stop.store(true, Ordering::Relaxed);
        self.cancel.store(true, Ordering::Relaxed);
        if let Ok(catalog) = self.catalog.lock() {
            let _ = catalog.checkpoint_running();
        }
        if let Ok(mut handle) = self.worker.lock() {
            if let Some(worker) = handle.take() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command, time::Instant};
    struct Fixture {
        state: Option<AppState>,
        root: PathBuf,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(state) = self.state.take() {
                state.shutdown();
                drop(state);
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!("soundshelf-worker-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("media")).unwrap();
        let state = AppState::new(root.join("app"), root.join("resources")).unwrap();
        assert!(state.tools.is_some());
        Fixture {
            state: Some(state),
            root,
        }
    }
    #[test]
    fn desktop_shutdown_closes_mcp_and_clears_pairings() {
        let root=std::env::temp_dir().join(format!("soundshelf-mcp-shutdown-{}",Uuid::new_v4()));
        let state=AppState::new(root.join("app"),root.join("resources")).unwrap();
        assert!(state.agent.status().unwrap().endpoint.is_none());
        let endpoint=state.agent.start(0).unwrap().endpoint.unwrap();
        state.agent.pair("test".into()).unwrap();
        assert!(root.join("app/runtime/mcp.json").exists());
        state.shutdown();
        assert!(state.agent.status().unwrap().clients.is_empty());
        assert!(!root.join("app/runtime/mcp.json").exists());
        let address=endpoint.strip_prefix("http://").unwrap().strip_suffix("/mcp").unwrap();
        assert!(std::net::TcpStream::connect(address).is_err());
        drop(state);fs::remove_dir_all(root).unwrap();
    }
    fn media(f: &Fixture) {
        let t = f.state.as_ref().unwrap().tools.as_ref().unwrap();
        let a = f.root.join("media/a.wav");
        assert!(Command::new(&t.ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i", "sine=duration=0.05"])
            .arg(&a)
            .status()
            .unwrap()
            .success());
        fs::copy(&a, f.root.join("media/b.wav")).unwrap();
        fs::copy(&a, f.root.join("media/sibling.wav")).unwrap();
    }
    fn wait(state: &AppState) {
        let start = Instant::now();
        loop {
            let jobs = state.catalog.lock().unwrap().active_jobs().unwrap();
            if !jobs.is_empty() && jobs.iter().all(|j| j.state == "complete") {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "Worker did not complete jobs: {jobs:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    #[ignore = "requires explicit FFmpeg fixture tools"]
    fn duplicate_submissions_share_work_and_same_parent_files_keep_distinct_progress() {
        let f = fixture();
        media(&f);
        let state = f.state.as_ref().unwrap();
        for name in ["a.wav", "b.wav"] {
            let (source, path) = state
                .catalog
                .lock()
                .unwrap()
                .select_file(&f.root.join("media").join(name))
                .unwrap();
            for _ in 0..10 {
                state
                    .enqueue_paths(source.clone(), Some(vec![path.clone()]))
                    .unwrap();
            }
        }
        wait(state);
        let c = state.catalog.lock().unwrap();
        assert_eq!(c.all_sounds().unwrap().len(), 2);
        let jobs = c.active_jobs().unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().all(|j| j.total == 1));
        assert!(jobs.iter().map(|j| j.reused).sum::<usize>() >= 1);
        drop(c);
        assert_eq!(state.progress.lock().unwrap().len(), 2);
        assert!(state.scheduled.lock().unwrap().is_empty());
    }
    #[test]
    #[ignore = "requires explicit FFmpeg fixture tools"]
    fn worker_restart_resumes_only_persisted_targets() {
        let mut f = fixture();
        media(&f);
        let old = f.state.take().unwrap();
        old.shutdown();
        drop(old);
        let db = Catalog::open(&f.root.join("app/library.sqlite")).unwrap();
        let (source, a) = db.select_file(&f.root.join("media/a.wav")).unwrap();
        db.select_file(&f.root.join("media/b.wav")).unwrap();
        db.enqueue_paths(&source.id, Some(vec![a])).unwrap();
        drop(db);
        f.state = Some(AppState::new(f.root.join("app"), f.root.join("resources")).unwrap());
        let state = f.state.as_ref().unwrap();
        wait(state);
        let c = state.catalog.lock().unwrap();
        let sounds = c.all_sounds().unwrap();
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].relative_path, "a.wav");
        assert_eq!(c.source(&source.id).unwrap().files.len(), 2);
    }
}
