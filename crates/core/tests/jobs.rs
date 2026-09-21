use soundshelf_core::{catalog::Catalog, jobs::{now_secs, Job, LEASE_SECS}, library::Progress};
use rusqlite::Connection;
use std::path::Path;
use tempfile::tempdir;

fn profile() -> soundshelf_core::catalog::Profile {
    soundshelf_core::catalog::Profile {duration:1.0,sample_rate:48000,channels:1,frames:48000,peak:0.8,rms:0.2,description:"Measured".into(),tags:vec!["short".into()],waveform:vec![[-0.8,0.8]]}
}

#[test]
fn expired_running_job_returns_to_queue_and_pending_stays_unready() {
    let dir = tempdir().unwrap();
    let media = tempdir().unwrap();
    std::fs::write(media.path().join("tone.wav"), b"audio").unwrap();
    let path = dir.path().join("library.sqlite");
    let mut catalog = Catalog::open(&path).unwrap();
    let source = catalog.add_source(media.path()).unwrap();
    let hash = soundshelf_core::catalog::hash_file(&media.path().join("tone.wav")).unwrap();
    let sound_id = catalog.register(&source, "tone.wav", &hash).unwrap();
    let job = catalog.enqueue_scan(&source.id).unwrap();
    assert_eq!(job.state, "queued");
    let claimed = catalog.claim_job(&job.id, "worker-a", now_secs()).unwrap().unwrap();
    catalog.persist_progress("worker-a", &Progress {
        job_id: claimed.id.clone(),
        source_id: source.id.clone(),
        status: "analyzing".into(),
        completed: 0,
        total: 1,
        ..Default::default()
    }).unwrap();
    assert_eq!(catalog.sound(&sound_id).unwrap().status, "pending");
    drop(catalog);

    let catalog = Catalog::open(&path).unwrap();
    let recovered = catalog.recover_jobs(now_secs() + LEASE_SECS + 1).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, job.id);
    assert_eq!(recovered[0].state, "queued");
    assert_eq!(catalog.sound(&sound_id).unwrap().status, "pending");
    assert!(catalog.sound(&sound_id).unwrap().profile.is_none());
}

#[test]
fn live_lease_is_not_stolen() {
    let dir = tempdir().unwrap();
    let media = tempdir().unwrap();
    std::fs::write(media.path().join("tone.wav"), b"audio").unwrap();
    let catalog = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
    let source = catalog.add_source(media.path()).unwrap();
    let job = catalog.enqueue_scan(&source.id).unwrap();
    let now = now_secs();
    assert!(catalog.claim_job(&job.id, "worker-a", now).unwrap().is_some());
    assert!(catalog.claim_job(&job.id, "worker-b", now).unwrap().is_none());
    assert!(catalog.recover_jobs(now).unwrap().is_empty());
}

#[test]
fn graceful_checkpoint_requeues_running_work() {
    let dir = tempdir().unwrap();
    let media = tempdir().unwrap();
    std::fs::write(media.path().join("tone.wav"), b"audio").unwrap();
    let catalog = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
    let source = catalog.add_source(media.path()).unwrap();
    let job = catalog.enqueue_scan(&source.id).unwrap();
    catalog.claim_job(&job.id, "worker-a", now_secs()).unwrap();
    catalog.checkpoint_running().unwrap();
    let recovered = catalog.recover_jobs(now_secs()).unwrap();
    assert_eq!(recovered[0].id, job.id);
    assert_eq!(recovered[0].state, "queued");
}

#[test]
fn v1_catalogs_gain_jobs_table_transactionally() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("legacy.sqlite");
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE sources (id TEXT PRIMARY KEY, name TEXT NOT NULL, root TEXT NOT NULL UNIQUE, generation INTEGER NOT NULL DEFAULT 0, available INTEGER NOT NULL DEFAULT 1);
CREATE TABLE sounds (id TEXT PRIMARY KEY, source_id TEXT NOT NULL REFERENCES sources(id), relative_path TEXT NOT NULL, title TEXT NOT NULL, content_hash TEXT NOT NULL, status TEXT NOT NULL, UNIQUE(source_id,relative_path));
CREATE TABLE analyses (content_hash TEXT NOT NULL, analyzer TEXT NOT NULL, profile TEXT NOT NULL, PRIMARY KEY(content_hash,analyzer));
CREATE TABLE annotations (sound_id TEXT PRIMARY KEY REFERENCES sounds(id), tags TEXT NOT NULL DEFAULT '[]', comment TEXT NOT NULL DEFAULT '', favorite INTEGER NOT NULL DEFAULT 0);").unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
    }
    let catalog = Catalog::open(&path).unwrap();
    let version: u32 = {
        // reopen to read pragma through rusqlite on the same file
        let db = Connection::open(&path).unwrap();
        db.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap()
    };
    assert_eq!(version, soundshelf_core::catalog::SCHEMA_VERSION);
    let media = tempdir().unwrap();
    std::fs::write(media.path().join("tone.wav"), b"audio").unwrap();
    let source = catalog.add_source(media.path()).unwrap();
    let job = catalog.enqueue_scan(&source.id).unwrap();
    assert_eq!(job.kind, "scan");
    let _ = Path::new(".");
}

#[test]
fn finished_scan_is_durable_and_retryable() {
    let dir = tempdir().unwrap();
    let media = tempdir().unwrap();
    std::fs::write(media.path().join("tone.wav"), b"audio").unwrap();
    let mut catalog = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
    let source = catalog.add_source(media.path()).unwrap();
    let hash = soundshelf_core::catalog::hash_file(&media.path().join("tone.wav")).unwrap();
    let id = catalog.register(&source, "tone.wav", &hash).unwrap();
    catalog.publish(&source, &id, &hash, &profile()).unwrap();
    let job = catalog.enqueue_scan(&source.id).unwrap();
    catalog.claim_job(&job.id, "worker-a", now_secs()).unwrap();
    let done = Progress { job_id: job.id.clone(), source_id: source.id.clone(), status: "complete".into(), completed: 1, total: 1, reused: 1, ..Default::default() };
    catalog.persist_progress("worker-a", &done).unwrap();
    let finished = catalog.finish_job(&job.id, "worker-a", &done, false).unwrap();
    assert_eq!(finished.state, "complete");
    let retried = catalog.enqueue_scan(&source.id).unwrap();
    assert_eq!(retried.id, job.id);
    assert_eq!(retried.state, "queued");
}

fn _assert_job(_: &Job) {}
