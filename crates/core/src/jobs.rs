use crate::{catalog::Catalog, invalid, library::Progress, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const SCAN: &str = "scan";
pub const LEASE_SECS: i64 = 30;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub id: String,
    pub source_id: String,
    pub kind: String,
    pub state: String,
    pub status: String,
    pub lease_owner: Option<String>,
    pub lease_until: Option<i64>,
    pub completed: usize,
    pub total: usize,
    pub reused: usize,
    pub failed: usize,
    pub current: String,
    pub errors: Vec<String>,
}

impl Job {
    pub fn progress(&self) -> Progress {
        Progress {
            job_id: self.id.clone(),
            source_id: self.source_id.clone(),
            status: self.status.clone(),
            completed: self.completed,
            total: self.total,
            reused: self.reused,
            failed: self.failed,
            current: self.current.clone(),
            errors: self.errors.clone(),
        }
    }
}

pub fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Catalog {
    pub fn enqueue_scan(&self, source_id: &str) -> Result<Job> {
        let _ = self.source(source_id)?;
        let now = now_secs();
        if let Some(existing) = self.job_for_source(source_id, SCAN)? {
            if existing.state == "running" && existing.lease_until.unwrap_or(0) > now {
                return Ok(existing);
            }
            if existing.state == "queued" {
                return Ok(existing);
            }
            self.db.execute(
                "UPDATE jobs SET state='queued', status='queued', lease_owner=NULL, lease_until=NULL, completed=0, total=0, reused=0, failed=0, current='', errors='[]', updated_at=?2 WHERE id=?1",
                params![existing.id, now],
            )?;
            return self.job(&existing.id);
        }
        let id = Uuid::new_v4().to_string();
        self.db.execute(
            "INSERT INTO jobs(id,source_id,kind,state,status,created_at,updated_at) VALUES(?1,?2,?3,'queued','queued',?4,?4)",
            params![id, source_id, SCAN, now],
        )?;
        self.job(&id)
    }

    pub fn recover_jobs(&self, now: i64) -> Result<Vec<Job>> {
        self.db.execute(
            "UPDATE jobs SET state='queued', status='queued', lease_owner=NULL, lease_until=NULL, updated_at=?1 WHERE state='running' AND (lease_until IS NULL OR lease_until<=?1)",
            params![now],
        )?;
        self.jobs_in_states(&["queued"])
    }

    pub fn checkpoint_running(&self) -> Result<()> {
        self.db.execute(
            "UPDATE jobs SET state='queued', status='queued', lease_owner=NULL, lease_until=NULL, updated_at=?1 WHERE state='running'",
            params![now_secs()],
        )?;
        Ok(())
    }

    pub fn claim_job(&self, id: &str, owner: &str, now: i64) -> Result<Option<Job>> {
        let changed = self.db.execute(
            "UPDATE jobs SET state='running', status=CASE WHEN status IN ('queued','cancelled') THEN 'queued' ELSE status END, lease_owner=?2, lease_until=?4, updated_at=?3 WHERE id=?1 AND (state='queued' OR (state='running' AND (lease_until IS NULL OR lease_until<=?3)))",
            params![id, owner, now, now + LEASE_SECS],
        )?;
        if changed == 0 { return Ok(None); }
        Ok(Some(self.job(id)?))
    }

    pub fn persist_progress(&self, owner: &str, progress: &Progress) -> Result<()> {
        if progress.job_id.is_empty() { return Err(invalid("Job id required")); }
        let now = now_secs();
        let changed = self.db.execute(
            "UPDATE jobs SET status=?3, completed=?4, total=?5, reused=?6, failed=?7, current=?8, errors=?9, lease_owner=?2, lease_until=?10, updated_at=?11 WHERE id=?1 AND lease_owner=?2 AND state='running'",
            params![
                progress.job_id,
                owner,
                progress.status,
                progress.completed as i64,
                progress.total as i64,
                progress.reused as i64,
                progress.failed as i64,
                progress.current,
                serde_json::to_string(&progress.errors)?,
                now + LEASE_SECS,
                now
            ],
        )?;
        if changed == 0 { return Err(invalid("Job lease is no longer valid")); }
        Ok(())
    }

    pub fn finish_job(&self, id: &str, owner: &str, progress: &Progress, cancelled: bool) -> Result<Job> {
        let now = now_secs();
        let (state, status) = if cancelled {
            ("cancelled", "cancelled")
        } else if progress.status == "failed" || (progress.failed > 0 && progress.status != "complete" && !progress.status.starts_with("completed")) {
            ("failed", progress.status.as_str())
        } else {
            ("complete", progress.status.as_str())
        };
        let changed = self.db.execute(
            "UPDATE jobs SET state=?3, status=?4, completed=?5, total=?6, reused=?7, failed=?8, current=?9, errors=?10, lease_owner=NULL, lease_until=NULL, updated_at=?11 WHERE id=?1 AND lease_owner=?2 AND state='running'",
            params![
                id,
                owner,
                state,
                status,
                progress.completed as i64,
                progress.total as i64,
                progress.reused as i64,
                progress.failed as i64,
                progress.current,
                serde_json::to_string(&progress.errors)?,
                now
            ],
        )?;
        if changed == 0 { return Err(invalid("Job lease is no longer valid")); }
        self.job(id)
    }

    pub fn active_jobs(&self) -> Result<Vec<Job>> {
        self.jobs_in_states(&["queued", "running", "failed", "cancelled", "complete"])
    }

    pub fn job(&self, id: &str) -> Result<Job> {
        self.read_job("SELECT id,source_id,kind,state,status,lease_owner,lease_until,completed,total,reused,failed,current,errors FROM jobs WHERE id=?1", params![id])?
            .ok_or_else(|| invalid("Job not found"))
    }

    fn job_for_source(&self, source_id: &str, kind: &str) -> Result<Option<Job>> {
        self.read_job(
            "SELECT id,source_id,kind,state,status,lease_owner,lease_until,completed,total,reused,failed,current,errors FROM jobs WHERE source_id=?1 AND kind=?2",
            params![source_id, kind],
        )
    }

    fn jobs_in_states(&self, states: &[&str]) -> Result<Vec<Job>> {
        let placeholders = states.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT id,source_id,kind,state,status,lease_owner,lease_until,completed,total,reused,failed,current,errors FROM jobs WHERE state IN ({placeholders}) ORDER BY created_at,id");
        let mut query = self.db.prepare(&sql)?;
        let rows = query.query_map(rusqlite::params_from_iter(states.iter()), map_job)?;
        let mut jobs = vec![];
        for row in rows { jobs.push(row_job(row?)?); }
        Ok(jobs)
    }

    fn read_job(&self, sql: &str, params: impl rusqlite::Params) -> Result<Option<Job>> {
        self.db.query_row(sql, params, map_job).optional()?.map(row_job).transpose()
    }
}

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawJob> {
    Ok(RawJob {
        id: row.get(0)?,
        source_id: row.get(1)?,
        kind: row.get(2)?,
        state: row.get(3)?,
        status: row.get(4)?,
        lease_owner: row.get(5)?,
        lease_until: row.get(6)?,
        completed: row.get(7)?,
        total: row.get(8)?,
        reused: row.get(9)?,
        failed: row.get(10)?,
        current: row.get(11)?,
        errors: row.get(12)?,
    })
}

struct RawJob {
    id: String,
    source_id: String,
    kind: String,
    state: String,
    status: String,
    lease_owner: Option<String>,
    lease_until: Option<i64>,
    completed: i64,
    total: i64,
    reused: i64,
    failed: i64,
    current: String,
    errors: String,
}

fn row_job(raw: RawJob) -> Result<Job> {
    Ok(Job {
        id: raw.id,
        source_id: raw.source_id,
        kind: raw.kind,
        state: raw.state,
        status: raw.status,
        lease_owner: raw.lease_owner,
        lease_until: raw.lease_until,
        completed: raw.completed as usize,
        total: raw.total as usize,
        reused: raw.reused as usize,
        failed: raw.failed as usize,
        current: raw.current,
        errors: serde_json::from_str(&raw.errors)?,
    })
}
