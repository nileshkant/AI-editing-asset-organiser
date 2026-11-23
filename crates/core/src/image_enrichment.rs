//! SS-052 consented image enrichment: durable jobs, per-source consent, usage
//! reservations and fenced publication for optional image AI.
//!
//! Nothing here contacts a provider unless a source has explicit consent that
//! names the provider revision, and every step re-checks it. The runner takes
//! the catalog lock only for short database steps; preparing pixels and the
//! provider request run without it. A result is attached to an asset only by a
//! publication that re-checks, in one transaction, that the job is live, the
//! consent and provider configuration are unchanged, the source generation is
//! the one the job was created for, and the asset still has the same content.
//!
//! Billing is never guessed. A request that ends without an answer after it was
//! sent (timeout, connection loss, 5xx, a crash) is `unknown_outcome`: its
//! reservation stays counted, it is not retried automatically, and the user may
//! retry it knowingly with the same operation identity. No exactly-once billing
//! is promised.
use crate::{
    ai_provider::{CapabilityState, ProviderError, ProviderKind, ProviderProfile},
    catalog::Catalog,
    image_ai::{Described, DescribeError, Insight, PreparedImage, INSIGHT_OUTPUT_TOKENS, VISUAL_SCHEMA_ID as INSIGHT_SCHEMA_ID},
    jobs::now_secs,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// What is sent, pinned: the cached 512 px preview, re-encoded as PNG with no
/// metadata. Results are keyed by this recipe and never mixed with another.
pub const PREPROCESS_ID: &str = "srgb-oriented-preview512-png/v1";
pub const PREVIEW_EDGE: u32 = 512;
/// Labeled estimate of one request: image and instructions in, answer cap out.
pub const EST_INPUT_UNITS: i64 = 2000;
pub const EST_UNITS: i64 = EST_INPUT_UNITS + INSIGHT_OUTPUT_TOKENS as i64;
pub const MAX_JOB_ITEMS: usize = 20_000;
pub const MAX_ACTIVE_JOBS: i64 = 8;
pub const MAX_FINISHED_JOBS: i64 = 100;
pub const LEASE_SECONDS: i64 = 180;
/// Automatic retries of a rate-limited request before it is reported as failed.
pub const MAX_RATE_ATTEMPTS: i64 = 3;
/// Ceiling for jobs started by "analyze new imports".
pub const AUTO_MAX_REQUESTS: i64 = 200;
const MAX_CLAIM_SKIPS: usize = 16;

pub type Result<T> = std::result::Result<T, EnrichError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EnrichError {
    #[error("Invalid enrichment request")]
    InvalidArgument,
    #[error("Allow image analysis for this source and provider first")]
    ConsentRequired,
    #[error("The provider settings changed; review the provider and allow analysis again")]
    ProviderChanged,
    #[error("This provider has not passed the image and structured-output checks")]
    ProviderNotReady,
    #[error("Source not found")]
    SourceNotFound,
    #[error("The source is offline")]
    SourceOffline,
    #[error("No eligible images to analyze")]
    NothingToDo,
    #[error("The batch ceiling would be exceeded")]
    BudgetExceeded,
    #[error("Too many analysis batches are active; wait for one to finish")]
    QueueFull,
    #[error("Analysis batch or image not found")]
    NotFound,
    #[error("The record changed; refresh and try again")]
    Conflict,
    #[error("The analysis was cancelled")]
    Cancelled,
    #[error("Image analysis is unavailable in this build")]
    Unavailable,
    #[error("Image analysis failed internally")]
    Internal,
}
impl From<rusqlite::Error> for EnrichError {
    fn from(_: rusqlite::Error) -> Self {
        EnrichError::Internal
    }
}

// ----------------------------------------------------------------- types

/// Exactly what a consent allows to leave this computer. Shown before the
/// user agrees, and stored as the fixed values `purpose` and `payload`.
#[derive(Debug, Clone, Serialize)]
pub struct Disclosure {
    pub sends: &'static str,
    pub never_sends: &'static [&'static str],
    pub purpose: &'static str,
}
pub fn disclosure() -> Disclosure {
    Disclosure {
        sends: "A re-encoded copy of each image you analyze, with its longest side at most 512 pixels, plus fixed instructions asking for a caption and tags.",
        never_sends: &[
            "file names or folder paths",
            "your tags, notes, ratings or prompts",
            "EXIF, location, camera or other embedded metadata",
            "the original file",
        ],
        purpose: "Describe images so they can be searched and tagged",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consent {
    pub source_id: String,
    pub provider_id: String,
    pub provider_revision: u64,
    pub model: String,
    pub analyze_new: bool,
    pub revision: u64,
    pub granted_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    NewImports,
    Selected,
    Backfill,
}
impl Kind {
    fn text(self) -> &'static str {
        match self {
            Kind::NewImports => "new_imports",
            Kind::Selected => "selected",
            Kind::Backfill => "backfill",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentRequest {
    pub source_id: String,
    pub provider_id: String,
    pub provider_revision: u64,
    pub analyze_new: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EstimateRequest {
    pub source_id: String,
    pub provider_id: String,
    pub provider_revision: u64,
    pub kind: Kind,
    pub asset_ids: Option<Vec<String>>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub source_id: String,
    pub provider_id: String,
    pub provider_revision: u64,
    pub kind: Kind,
    pub asset_ids: Option<Vec<String>>,
    pub max_requests: i64,
    pub max_units: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Estimate {
    /// Images that would be sent now, after skipping reusable and finished ones.
    pub to_send: i64,
    /// Images whose identical content already has a result for this model and
    /// recipe; they are attached without any request.
    pub reusable: i64,
    /// Images already analyzed with this model and recipe.
    pub already_done: i64,
    /// Images already waiting in another batch.
    pub already_queued: i64,
    /// Images skipped because they are not technically ready or are offline.
    pub not_ready: i64,
    /// An estimate, not a quote: `to_send` times a fixed per-request size.
    pub estimated_units: i64,
    pub units_per_request: i64,
    pub unit: &'static str,
    /// Prices are not known to the app, so cost is never invented.
    pub cost: &'static str,
    pub suggested_max_requests: i64,
    pub suggested_max_units: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub pending: i64,
    pub running: i64,
    pub done: i64,
    pub reused: i64,
    pub failed: i64,
    pub unknown_outcome: i64,
    pub stale: i64,
    pub revoked: i64,
}
impl Counts {
    pub fn total(&self) -> i64 {
        self.pending + self.running + self.done + self.reused + self.failed + self.unknown_outcome + self.stale + self.revoked
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageSummary {
    pub requests: i64,
    /// Settled units: provider-reported where available, else the estimate.
    pub used_units: i64,
    pub reserved_units: i64,
    /// Units of requests whose outcome is unknown; they may or may not be billed.
    pub unknown_units: i64,
    pub estimated_entries: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobView {
    pub id: String,
    pub source_id: String,
    pub kind: Kind,
    pub provider_id: String,
    pub model: String,
    pub state: String,
    pub error_code: Option<String>,
    pub max_requests: i64,
    pub max_units: i64,
    pub counts: Counts,
    pub usage: UsageSummary,
    pub created_at: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemView {
    pub asset_id: String,
    pub state: String,
    pub error_code: Option<String>,
    pub attempts: i64,
    pub operation_id: String,
}

// -------------------------------------------------------------- providers

fn ready(profile: &ProviderProfile) -> bool {
    let c = &profile.capabilities;
    c.tested_revision == Some(profile.revision)
        && c.image_input == CapabilityState::Supported
        && c.structured_output == CapabilityState::Supported
}
fn kind_text(kind: ProviderKind) -> String {
    serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}
/// Reuse only across the same provider configuration and model.
pub fn model_key(profile: &ProviderProfile) -> String {
    let scope = format!("{}\0{}\0{}\0{}", kind_text(profile.kind), profile.id, profile.revision, profile.model);
    blake3::hash(scope.as_bytes()).to_hex().to_string()
}

fn current_profile(db: &Connection, id: &str) -> Result<Option<ProviderProfile>> {
    Ok(crate::ai_provider::read_profiles(db).map_err(|_| EnrichError::Internal)?.into_iter().find(|p| p.id == id))
}
/// A provider exactly at the revision the user chose, tested and image capable.
fn exact_ready_profile(db: &Connection, id: &str, revision: u64) -> Result<ProviderProfile> {
    let profile = current_profile(db, id)?.ok_or(EnrichError::ProviderChanged)?;
    if profile.revision != revision {
        return Err(EnrichError::ProviderChanged);
    }
    if !ready(&profile) {
        return Err(EnrichError::ProviderNotReady);
    }
    Ok(profile)
}

// -------------------------------------------------------------- consent

fn read_consent(db: &Connection, source_id: &str) -> Result<Option<Consent>> {
    Ok(db
        .query_row(
            "SELECT source_id,provider_id,provider_revision,model,analyze_new,revision,granted_at FROM image_enrichment_consent WHERE source_id=?1",
            [source_id],
            |r| {
                Ok(Consent {
                    source_id: r.get(0)?,
                    provider_id: r.get(1)?,
                    provider_revision: r.get::<_, i64>(2)? as u64,
                    model: r.get(3)?,
                    analyze_new: r.get::<_, i64>(4)? != 0,
                    revision: r.get::<_, i64>(5)? as u64,
                    granted_at: r.get(6)?,
                })
            },
        )
        .optional()?)
}
fn source_state(db: &Connection, source_id: &str) -> Result<(i64, bool)> {
    db.query_row("SELECT generation,available FROM sources WHERE id=?1", [source_id], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)))
        .optional()?
        .ok_or(EnrichError::SourceNotFound)
}
fn next_order(db: &Connection) -> Result<i64> {
    Ok(db.query_row("SELECT COALESCE(MAX(queue_order),0)+1 FROM image_enrichment_jobs", [], |r| r.get(0))?)
}

/// Stops a source's active work: pending items are revoked and the job ends.
/// An item already sent keeps `dispatching`; its answer is discarded when it
/// arrives. Data already transmitted cannot be recalled.
fn stop_source_jobs(tx: &Transaction, source_id: &str, code: &str, item_state: &str) -> Result<()> {
    let now = now_secs();
    tx.execute(
        "UPDATE image_enrichment_items SET state=?3,lease_owner=NULL,lease_until=NULL,error_code=?2,updated_at=?4 WHERE state IN ('pending','running') AND job_id IN (SELECT id FROM image_enrichment_jobs WHERE source_id=?1 AND state IN ('queued','running','paused'))",
        params![source_id, code, item_state, now],
    )?;
    tx.execute(
        "UPDATE image_enrichment_jobs SET state='cancelled',error_code=?2,updated_at=?3 WHERE source_id=?1 AND state IN ('queued','running','paused')",
        params![source_id, code, now],
    )?;
    Ok(())
}

impl Catalog {
    pub fn enrichment_consents(&self) -> Result<Vec<Consent>> {
        let ids: Vec<String> = {
            let mut q = self.db.prepare("SELECT source_id FROM image_enrichment_consent ORDER BY source_id")?;
            let rows = q.query_map([], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        ids.iter().filter_map(|id| read_consent(&self.db, id).transpose()).collect()
    }

    /// The user's explicit agreement that one source's images may be analyzed
    /// by one provider at one configuration revision. A different provider than
    /// before ends the source's active work first.
    pub fn grant_enrichment_consent(&mut self, request: &ConsentRequest) -> Result<Consent> {
        let profile = exact_ready_profile(&self.db, &request.provider_id, request.provider_revision)?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        source_state(&tx, &request.source_id)?;
        let existing = read_consent(&tx, &request.source_id)?;
        let now = now_secs();
        let changed = existing.as_ref().is_some_and(|c| c.provider_id != profile.id || c.provider_revision != profile.revision);
        if changed {
            stop_source_jobs(&tx, &request.source_id, "CONSENT_CHANGED", "revoked")?;
        }
        tx.execute(
            "INSERT INTO image_enrichment_consent(source_id,provider_id,provider_revision,model,purpose,payload,analyze_new,revision,granted_at) VALUES(?1,?2,?3,?4,'description','preview_image',?5,1,?6)
             ON CONFLICT(source_id) DO UPDATE SET provider_id=excluded.provider_id,provider_revision=excluded.provider_revision,model=excluded.model,analyze_new=excluded.analyze_new,revision=revision+1,granted_at=excluded.granted_at",
            params![request.source_id, profile.id, profile.revision as i64, profile.model, request.analyze_new as i64, now],
        )?;
        let consent = read_consent(&tx, &request.source_id)?.ok_or(EnrichError::Internal)?;
        tx.commit()?;
        Ok(consent)
    }

    /// Withdraws consent. New dispatch and publication stop at once; requests
    /// already sent cannot be recalled and their answers are discarded.
    pub fn revoke_enrichment_consent(&mut self, source_id: &str) -> Result<()> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM image_enrichment_consent WHERE source_id=?1", [source_id])?;
        stop_source_jobs(&tx, source_id, "CONSENT_REVOKED", "revoked")?;
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------- estimate / start

    fn candidates(&self, source_id: &str, key: &str, kind: Kind, selected: &Option<Vec<String>>) -> Result<(Estimate, Vec<(String, String)>)> {
        let (_, online) = source_state(&self.db, source_id)?;
        let wanted: Option<std::collections::HashSet<&str>> = match (kind, selected) {
            (Kind::Backfill, None) => None,
            (Kind::Backfill, Some(_)) => return Err(EnrichError::InvalidArgument),
            (_, Some(ids)) if !ids.is_empty() && ids.len() <= MAX_JOB_ITEMS => Some(ids.iter().map(String::as_str).collect()),
            _ => return Err(EnrichError::InvalidArgument),
        };
        let mut query = self.db.prepare(
            "SELECT a.id,a.content_hash,a.technical_state,a.availability,
              EXISTS(SELECT 1 FROM image_enrichment_outputs o WHERE o.asset_id=a.id AND o.content_hash=a.content_hash AND o.model_key=?2 AND o.preprocess=?3 AND o.schema_id=?4),
              EXISTS(SELECT 1 FROM image_enrichment_items i JOIN image_enrichment_jobs j ON j.id=i.job_id WHERE i.asset_id=a.id AND j.schema_id=?4 AND i.state IN ('pending','running','dispatching','unknown_outcome') AND j.state IN ('queued','running','paused')),
              EXISTS(SELECT 1 FROM image_enrichment_results x WHERE x.content_hash=a.content_hash AND x.model_key=?2 AND x.preprocess=?3 AND x.schema_id=?4)
             FROM image_assets a JOIN asset_registry r ON r.id=a.id WHERE r.source_id=?1 AND r.kind='image' ORDER BY a.id",
        )?;
        let mut rows = query.query(params![source_id, key, PREPROCESS_ID, INSIGHT_SCHEMA_ID])?;
        let mut estimate = Estimate {
            to_send: 0,
            reusable: 0,
            already_done: 0,
            already_queued: 0,
            not_ready: 0,
            estimated_units: 0,
            units_per_request: EST_UNITS,
            unit: "tokens (estimate)",
            cost: "unknown: prices are not known to CreativeShelf",
            suggested_max_requests: 0,
            suggested_max_units: 0,
        };
        let mut items = vec![];
        let mut seen = 0usize;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            if wanted.as_ref().is_some_and(|w| !w.contains(id.as_str())) {
                continue;
            }
            seen += 1;
            let hash: Option<String> = row.get(1)?;
            let technical: String = row.get(2)?;
            let availability: String = row.get(3)?;
            let (done, queued, cached): (bool, bool, bool) = (row.get(4)?, row.get(5)?, row.get(6)?);
            let Some(hash) = hash.filter(|_| technical == "ready" && availability == "available" && online) else {
                estimate.not_ready += 1;
                continue;
            };
            if done {
                estimate.already_done += 1;
            } else if queued {
                estimate.already_queued += 1;
            } else {
                if cached {
                    estimate.reusable += 1;
                } else {
                    estimate.to_send += 1;
                }
                items.push((id, hash));
            }
        }
        // A selection that names an image of another source is a bad request.
        if let Some(w) = &wanted {
            if seen != w.len() {
                return Err(EnrichError::InvalidArgument);
            }
        }
        estimate.estimated_units = estimate.to_send * EST_UNITS;
        estimate.suggested_max_requests = estimate.to_send.max(1);
        estimate.suggested_max_units = (estimate.to_send * EST_UNITS).max(EST_UNITS);
        Ok((estimate, items))
    }

    /// Counts and estimated units before anything is sent. Needs no consent.
    pub fn estimate_enrichment(&self, request: &EstimateRequest) -> Result<Estimate> {
        let profile = exact_ready_profile(&self.db, &request.provider_id, request.provider_revision)?;
        Ok(self.candidates(&request.source_id, &model_key(&profile), request.kind, &request.asset_ids)?.0)
    }

    pub fn start_enrichment(&mut self, request: &StartRequest) -> Result<JobView> {
        if request.max_requests < 1 || request.max_requests > 100_000 || request.max_units < EST_UNITS || request.max_units > 1_000_000_000 {
            return Err(EnrichError::InvalidArgument);
        }
        let profile = exact_ready_profile(&self.db, &request.provider_id, request.provider_revision)?;
        let key = model_key(&profile);
        let (_, items) = self.candidates(&request.source_id, &key, request.kind, &request.asset_ids)?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (generation, online) = source_state(&tx, &request.source_id)?;
        if !online {
            return Err(EnrichError::SourceOffline);
        }
        let consent = read_consent(&tx, &request.source_id)?.ok_or(EnrichError::ConsentRequired)?;
        if consent.provider_id != profile.id || consent.provider_revision != profile.revision {
            return Err(EnrichError::ConsentRequired);
        }
        if items.is_empty() {
            return Err(EnrichError::NothingToDo);
        }
        if items.len() > MAX_JOB_ITEMS {
            return Err(EnrichError::InvalidArgument);
        }
        let active: i64 = tx.query_row("SELECT count(*) FROM image_enrichment_jobs WHERE state IN ('queued','running','paused')", [], |r| r.get(0))?;
        if active >= MAX_ACTIVE_JOBS {
            return Err(EnrichError::QueueFull);
        }
        let id = Uuid::new_v4().to_string();
        let now = now_secs();
        tx.execute(
            "INSERT INTO image_enrichment_jobs(id,source_id,source_generation,kind,provider_id,provider_revision,model,model_key,preprocess,schema_id,state,max_requests,max_units,queue_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'queued',?11,?12,?13,?14,?14)",
            params![id, request.source_id, generation, request.kind.text(), profile.id, profile.revision as i64, profile.model, key, PREPROCESS_ID, INSIGHT_SCHEMA_ID, request.max_requests, request.max_units, next_order(&tx)?, now],
        )?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO image_enrichment_items(job_id,asset_id,content_hash,state,operation_id,updated_at) VALUES(?1,?2,?3,'pending',?4,?5)",
            )?;
            for (asset, hash) in &items {
                insert.execute(params![id, asset, hash, Uuid::new_v4().to_string(), now])?;
            }
        }
        tx.commit()?;
        self.enrichment_job(&id)
    }

    /// Queues the images of a finished import when the source allowed analysis
    /// of new imports. Without that consent, or when the provider is not
    /// currently ready, nothing is queued and nothing is sent.
    pub fn enqueue_new_imports(&mut self, source_id: &str, asset_ids: &[String]) -> Result<Option<JobView>> {
        let Some(consent) = read_consent(&self.db, source_id)?.filter(|c| c.analyze_new) else { return Ok(None) };
        if asset_ids.is_empty() {
            return Ok(None);
        }
        let ids: Vec<String> = asset_ids.iter().take(MAX_JOB_ITEMS).cloned().collect();
        let request = StartRequest {
            source_id: source_id.to_owned(),
            provider_id: consent.provider_id,
            provider_revision: consent.provider_revision,
            kind: Kind::NewImports,
            max_requests: (ids.len() as i64).min(AUTO_MAX_REQUESTS),
            max_units: (ids.len() as i64).min(AUTO_MAX_REQUESTS) * EST_UNITS,
            asset_ids: Some(ids),
        };
        match self.start_enrichment(&request) {
            Ok(job) => Ok(Some(job)),
            Err(EnrichError::NothingToDo | EnrichError::ProviderChanged | EnrichError::ProviderNotReady | EnrichError::QueueFull | EnrichError::SourceOffline) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Queues the images a finished import made ready (see `enqueue_new_imports`).
    /// Unchanged files that were analyzed before are skipped by the estimate.
    pub fn enqueue_new_from_import(&mut self, import_job_id: &str) -> Result<Option<JobView>> {
        let (source_id, ids) = {
            let source: Option<String> = self
                .db
                .query_row("SELECT source_id FROM image_import_jobs WHERE id=?1 AND state='complete'", [import_job_id], |r| r.get(0))
                .optional()?;
            let Some(source) = source else { return Ok(None) };
            let mut q = self.db.prepare(
                "SELECT asset_id FROM image_import_items WHERE job_id=?1 AND state='ready' AND media_kind='image' AND asset_id IS NOT NULL ORDER BY relative_path LIMIT ?2",
            )?;
            let ids = q.query_map(params![import_job_id, MAX_JOB_ITEMS as i64], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            (source, ids)
        };
        self.enqueue_new_imports(&source_id, &ids)
    }
    /// True while an image import is queued or running; analysis yields to it.
    pub fn image_imports_active(&self) -> Result<bool> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM image_import_jobs WHERE state IN ('queued','running'))", [], |r| r.get(0))?)
    }

    // -------------------------------------------------------------- views

    fn counts(&self, job_id: &str) -> Result<Counts> {
        let mut counts = Counts::default();
        let mut q = self.db.prepare("SELECT state,count(*) FROM image_enrichment_items WHERE job_id=?1 GROUP BY state")?;
        let rows = q.query_map([job_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (state, n) = row?;
            match state.as_str() {
                "pending" => counts.pending += n,
                "running" | "dispatching" => counts.running += n,
                "done" => counts.done += n,
                "reused" => counts.reused += n,
                "failed" => counts.failed += n,
                "unknown_outcome" => counts.unknown_outcome += n,
                "stale" => counts.stale += n,
                "revoked" => counts.revoked += n,
                _ => {}
            }
        }
        Ok(counts)
    }
    fn usage(db: &Connection, job_id: &str) -> Result<UsageSummary> {
        Ok(db.query_row(
            "SELECT count(*) FILTER (WHERE status!='released'),
                    COALESCE(SUM(CASE WHEN status='settled' THEN used_units END),0),
                    COALESCE(SUM(CASE WHEN status='reserved' THEN reserved_units END),0),
                    COALESCE(SUM(CASE WHEN status='unknown' THEN reserved_units END),0),
                    count(*) FILTER (WHERE status='settled' AND used_is_estimate=1)
             FROM image_enrichment_usage WHERE job_id=?1",
            [job_id],
            |r| Ok(UsageSummary { requests: r.get(0)?, used_units: r.get(1)?, reserved_units: r.get(2)?, unknown_units: r.get(3)?, estimated_entries: r.get(4)? }),
        )?)
    }
    pub fn enrichment_job(&self, id: &str) -> Result<JobView> {
        let row = self
            .db
            .query_row(
                "SELECT id,source_id,kind,provider_id,model,state,error_code,max_requests,max_units,created_at FROM image_enrichment_jobs WHERE id=?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, i64>(7)?, r.get::<_, i64>(8)?, r.get::<_, i64>(9)?)),
            )
            .optional()?
            .ok_or(EnrichError::NotFound)?;
        let kind = match row.2.as_str() {
            "new_imports" => Kind::NewImports,
            "selected" => Kind::Selected,
            _ => Kind::Backfill,
        };
        Ok(JobView {
            id: row.0,
            source_id: row.1,
            kind,
            provider_id: row.3,
            model: row.4,
            state: row.5,
            error_code: row.6,
            max_requests: row.7,
            max_units: row.8,
            counts: self.counts(id)?,
            usage: Self::usage(&self.db, id)?,
            created_at: row.9,
        })
    }
    pub fn enrichment_jobs(&self) -> Result<Vec<JobView>> {
        let ids: Vec<String> = {
            let mut q = self.db.prepare("SELECT id FROM image_enrichment_jobs ORDER BY queue_order DESC,id LIMIT 200")?;
            let rows = q.query_map([], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        ids.iter().map(|id| self.enrichment_job(id)).collect()
    }
    pub fn enrichment_items(&self, job_id: &str, offset: i64, limit: i64) -> Result<Vec<ItemView>> {
        let limit = limit.clamp(1, 500);
        let mut q = self.db.prepare(
            "SELECT asset_id,state,error_code,attempts,operation_id FROM image_enrichment_items WHERE job_id=?1 ORDER BY CASE state WHEN 'unknown_outcome' THEN 0 WHEN 'failed' THEN 1 ELSE 2 END,asset_id LIMIT ?2 OFFSET ?3",
        )?;
        let rows = q.query_map(params![job_id, limit, offset.max(0)], |r| {
            Ok(ItemView { asset_id: r.get(0)?, state: r.get(1)?, error_code: r.get(2)?, attempts: r.get(3)?, operation_id: r.get(4)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    pub fn enrichment_has_work(&self) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM image_enrichment_items i JOIN image_enrichment_jobs j ON j.id=i.job_id WHERE j.state IN ('queued','running') AND i.state IN ('pending','running','dispatching'))",
            [],
            |r| r.get(0),
        )?)
    }
    /// Assets with a published result for the current model and recipe: what
    /// SS-053 turns into persisted, reviewable suggestions.
    pub fn enrichment_output(&self, asset_id: &str, model_key: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .db
            .query_row(
                "SELECT r.insight,o.content_hash FROM image_enrichment_outputs o JOIN image_enrichment_results r ON r.content_hash=o.content_hash AND r.model_key=o.model_key AND r.preprocess=o.preprocess AND r.schema_id=o.schema_id WHERE o.asset_id=?1 AND o.model_key=?2 AND o.preprocess=?3 AND o.schema_id=?4",
                params![asset_id, model_key, PREPROCESS_ID, INSIGHT_SCHEMA_ID],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    // ------------------------------------------------------------ controls

    pub fn cancel_enrichment_job(&mut self, id: &str) -> Result<()> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = now_secs();
        let changed = tx.execute(
            "UPDATE image_enrichment_jobs SET state='cancelled',error_code='CANCELLED',updated_at=?2 WHERE id=?1 AND state IN ('queued','running','paused')",
            params![id, now],
        )?;
        if changed == 0 {
            return if tx.query_row("SELECT EXISTS(SELECT 1 FROM image_enrichment_jobs WHERE id=?1)", [id], |r| r.get::<_, bool>(0))? {
                Err(EnrichError::Conflict)
            } else {
                Err(EnrichError::NotFound)
            };
        }
        tx.execute(
            "UPDATE image_enrichment_items SET state='revoked',lease_owner=NULL,lease_until=NULL,error_code='CANCELLED',updated_at=?2 WHERE job_id=?1 AND state IN ('pending','running')",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Resumes a paused batch after the user fixed the provider, quota or
    /// source. Everything is re-validated at the next claim.
    pub fn resume_enrichment_job(&mut self, id: &str) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE image_enrichment_jobs SET state='queued',error_code=NULL,updated_at=?2 WHERE id=?1 AND state='paused'",
            params![id, now_secs()],
        )?;
        if changed == 0 {
            return Err(EnrichError::NotFound);
        }
        Ok(())
    }

    /// Puts a failed image back in the queue. An unknown-outcome image needs
    /// `accept_possible_charge`: the earlier request may already have been billed.
    pub fn retry_enrichment_item(&mut self, job_id: &str, asset_id: &str, accept_possible_charge: bool) -> Result<()> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (state, attempts, content_hash): (String, i64, String) = tx
            .query_row("SELECT state,attempts,content_hash FROM image_enrichment_items WHERE job_id=?1 AND asset_id=?2",
                params![job_id, asset_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?
            .ok_or(EnrichError::NotFound)?;
        match state.as_str() {
            "failed" => {}
            "unknown_outcome" if accept_possible_charge => {}
            "unknown_outcome" => return Err(EnrichError::InvalidArgument),
            _ => return Err(EnrichError::Conflict),
        }
        if attempts >= 20 {
            return Err(EnrichError::Conflict);
        }
        if !content_current(&tx, asset_id, &content_hash)? {
            return Err(EnrichError::Conflict);
        }
        let (job, schema): (String, String) = tx.query_row(
            "SELECT state,schema_id FROM image_enrichment_jobs WHERE id=?1", [job_id], |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if schema != INSIGHT_SCHEMA_ID {
            return Err(EnrichError::Conflict);
        }
        let now = now_secs();
        tx.execute(
            "UPDATE image_enrichment_items SET state='pending',error_code=NULL,not_before=0,updated_at=?3 WHERE job_id=?1 AND asset_id=?2",
            params![job_id, asset_id, now],
        )?;
        if matches!(job.as_str(), "complete" | "failed") {
            tx.execute("UPDATE image_enrichment_jobs SET state='queued',error_code=NULL,updated_at=?2 WHERE id=?1", params![job_id, now])?;
        } else if job == "cancelled" {
            return Err(EnrichError::Conflict);
        }
        tx.commit()?;
        Ok(())
    }

    // ----------------------------------------------------------- recovery

    /// Restart and lease recovery. An image that was only claimed goes back to
    /// pending. An image whose request may have been sent becomes
    /// `unknown_outcome` with its reservation kept: after a crash the app cannot
    /// know whether the provider answered or billed. Pass `all_leases` at start.
    pub fn recover_enrichment(&mut self, all_leases: bool) -> Result<usize> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = now_secs();
        let cutoff = if all_leases { i64::MAX } else { now };
        let unknown = tx.execute(
            "UPDATE image_enrichment_items SET state='unknown_outcome',lease_owner=NULL,lease_until=NULL,error_code='OUTCOME_UNKNOWN',updated_at=?1 WHERE state='dispatching' AND lease_until<?2",
            params![now, cutoff],
        )?;
        tx.execute(
            "UPDATE image_enrichment_usage SET status='unknown',updated_at=?1 WHERE status='reserved' AND (operation_id,attempt) IN (SELECT operation_id,attempts FROM image_enrichment_items WHERE state='unknown_outcome')",
            params![now],
        )?;
        let requeued = tx.execute(
            "UPDATE image_enrichment_items SET state='pending',lease_owner=NULL,lease_until=NULL,updated_at=?1 WHERE state='running' AND lease_until<?2",
            params![now, cutoff],
        )?;
        tx.execute(
            "UPDATE image_enrichment_jobs SET state='queued',updated_at=?1 WHERE state='running' AND NOT EXISTS(SELECT 1 FROM image_enrichment_items i WHERE i.job_id=image_enrichment_jobs.id AND i.state IN ('running','dispatching'))",
            params![now],
        )?;
        tx.commit()?;
        Ok(unknown + requeued)
    }

    /// Database restore: consent belongs to the provider settings of the
    /// moment, so a restored backup must not silently re-grant it, and queued
    /// work from another time is cancelled. Results already stored stay.
    pub(crate) fn reset_enrichment_after_restore(&mut self) -> rusqlite::Result<()> {
        let now = now_secs();
        self.db.execute("DELETE FROM image_enrichment_consent", [])?;
        self.db.execute("UPDATE image_enrichment_items SET state='revoked',lease_owner=NULL,lease_until=NULL,error_code='CANCELLED',updated_at=?1 WHERE state IN ('pending','running')", [now])?;
        self.db.execute("UPDATE image_enrichment_items SET state='unknown_outcome',lease_owner=NULL,lease_until=NULL,error_code='OUTCOME_UNKNOWN',updated_at=?1 WHERE state='dispatching'", [now])?;
        self.db.execute("UPDATE image_enrichment_usage SET status='unknown',updated_at=?1 WHERE status='reserved'", [now])?;
        self.db.execute("UPDATE image_enrichment_jobs SET state='cancelled',error_code='CANCELLED',updated_at=?1 WHERE state IN ('queued','running','paused')", [now])?;
        Ok(())
    }
}

// ------------------------------------------------------------ worker steps

/// One image claimed for processing. Holding it grants nothing: every later
/// step re-checks consent, configuration, generation and content.
#[derive(Debug, Clone)]
pub struct Claim {
    pub job_id: String,
    pub asset_id: String,
    pub content_hash: String,
    pub operation_id: String,
    pub attempts: i64,
    pub provider_id: String,
    pub provider_revision: u64,
    pub model_key: String,
    pub owner: String,
}

#[derive(Debug)]
pub enum Claimed {
    None,
    /// State changed under the picked image (stale, paused job); claim again.
    Skipped,
    Item(Claim),
}

#[allow(dead_code)]
struct JobRow {
    id: String,
    state: String,
    source_id: String,
    generation: i64,
    provider_id: String,
    revision: i64,
    model: String,
    model_key: String,
    max_requests: i64,
    max_units: i64,
    schema_id: String,
}
fn job_row(db: &Connection, id: &str) -> Result<JobRow> {
    db.query_row(
        "SELECT id,state,source_id,source_generation,provider_id,provider_revision,model,model_key,max_requests,max_units,schema_id FROM image_enrichment_jobs WHERE id=?1",
        [id],
        |r| Ok(JobRow { id: r.get(0)?, state: r.get(1)?, source_id: r.get(2)?, generation: r.get(3)?, provider_id: r.get(4)?, revision: r.get(5)?, model: r.get(6)?, model_key: r.get(7)?, max_requests: r.get(8)?, max_units: r.get(9)?, schema_id: r.get(10)? }),
    )
    .optional()?
    .ok_or(EnrichError::NotFound)
}

/// Why a job may not publish or dispatch right now.
#[derive(Debug, PartialEq, Eq)]
enum Fence {
    Open,
    /// Pause the batch and keep its images queued for the user to resume.
    Pause(&'static str),
    /// The batch can never be valid again.
    Revoked(&'static str),
    Stale,
}

fn fence(db: &Connection, job: &JobRow) -> Result<Fence> {
    if !matches!(job.state.as_str(), "queued" | "running") {
        return Ok(Fence::Revoked("CANCELLED"));
    }
    if job.schema_id != INSIGHT_SCHEMA_ID {
        return Ok(Fence::Revoked("ANALYSIS_SCHEMA_CHANGED"));
    }
    let Some(consent) = read_consent(db, &job.source_id)? else { return Ok(Fence::Revoked("CONSENT_REVOKED")) };
    if consent.provider_id != job.provider_id || consent.provider_revision as i64 != job.revision || consent.model != job.model {
        return Ok(Fence::Revoked("CONSENT_CHANGED"));
    }
    match current_profile(db, &job.provider_id)? {
        None => return Ok(Fence::Pause("PROVIDER_REMOVED")),
        Some(p) if p.revision as i64 != job.revision => return Ok(Fence::Pause("PROVIDER_CHANGED")),
        Some(p) if !ready(&p) => return Ok(Fence::Pause("PROVIDER_NOT_READY")),
        Some(_) => {}
    }
    let (generation, online) = match source_state(db, &job.source_id) {
        Ok(state) => state,
        Err(_) => return Ok(Fence::Revoked("SOURCE_CHANGED")),
    };
    if generation != job.generation {
        return Ok(Fence::Stale);
    }
    if !online {
        return Ok(Fence::Pause("SOURCE_OFFLINE"));
    }
    Ok(Fence::Open)
}
fn content_current(db: &Connection, asset_id: &str, hash: &str) -> Result<bool> {
    Ok(db
        .query_row(
            "SELECT content_hash=?2 AND technical_state='ready' AND availability='available' FROM image_assets WHERE id=?1",
            params![asset_id, hash],
            |r| r.get::<_, Option<bool>>(0),
        )
        .optional()?
        .flatten()
        .unwrap_or(false))
}
fn apply_fence(tx: &Transaction, job_id: &str, result: &Fence) -> Result<()> {
    let now = now_secs();
    match result {
        Fence::Open => {}
        Fence::Pause(code) => {
            tx.execute("UPDATE image_enrichment_jobs SET state='paused',error_code=?2,updated_at=?3 WHERE id=?1 AND state IN ('queued','running')", params![job_id, code, now])?;
        }
        Fence::Revoked(code) => {
            tx.execute("UPDATE image_enrichment_jobs SET state='cancelled',error_code=COALESCE(error_code,?2),updated_at=?3 WHERE id=?1 AND state IN ('queued','running','paused')", params![job_id, code, now])?;
            tx.execute("UPDATE image_enrichment_items SET state='revoked',lease_owner=NULL,lease_until=NULL,error_code=?2,updated_at=?3 WHERE job_id=?1 AND state IN ('pending','running')", params![job_id, code, now])?;
        }
        Fence::Stale => {
            tx.execute("UPDATE image_enrichment_jobs SET state='cancelled',error_code='SOURCE_CHANGED',updated_at=?2 WHERE id=?1 AND state IN ('queued','running','paused')", params![job_id, now])?;
            tx.execute("UPDATE image_enrichment_items SET state='stale',lease_owner=NULL,lease_until=NULL,error_code='SOURCE_CHANGED',updated_at=?2 WHERE job_id=?1 AND state IN ('pending','running')", params![job_id, now])?;
        }
    }
    Ok(())
}

/// Ends a job whose images are all settled. Keeps a bounded history.
fn finalize_if_done(tx: &Transaction, job_id: &str) -> Result<()> {
    let open: i64 = tx.query_row("SELECT count(*) FROM image_enrichment_items WHERE job_id=?1 AND state IN ('pending','running','dispatching')", [job_id], |r| r.get(0))?;
    if open != 0 {
        return Ok(());
    }
    let good: i64 = tx.query_row("SELECT count(*) FROM image_enrichment_items WHERE job_id=?1 AND state IN ('done','reused')", [job_id], |r| r.get(0))?;
    let bad: i64 = tx.query_row("SELECT count(*) FROM image_enrichment_items WHERE job_id=?1 AND state IN ('failed','unknown_outcome')", [job_id], |r| r.get(0))?;
    let state = if good > 0 || bad == 0 { "complete" } else { "failed" };
    tx.execute("UPDATE image_enrichment_jobs SET state=?2,updated_at=?3 WHERE id=?1 AND state IN ('queued','running')", params![job_id, state, now_secs()])?;
    tx.execute(
        "DELETE FROM image_enrichment_jobs WHERE state IN ('complete','failed','cancelled') AND id NOT IN (SELECT id FROM image_enrichment_jobs WHERE state IN ('complete','failed','cancelled') ORDER BY updated_at DESC,id LIMIT ?1)",
        [MAX_FINISHED_JOBS],
    )?;
    Ok(())
}

/// Attaches an existing result to an asset. The caller holds the fence.
fn attach(tx: &Transaction, claim: &Claim, job: &JobRow) -> Result<bool> {
    use crate::image_contracts::{Coverage, EnrichmentState, VisualSuggestions, MAX_EVIDENCE};
    let mut asset = crate::image_catalog::load(tx, &claim.asset_id).map_err(|_| EnrichError::Internal)?;
    if asset.evidence.len() >= MAX_EVIDENCE {
        return Ok(false);
    }
    let (raw, model_version, coverage, created_at): (String, Option<String>, String, i64) = tx.query_row(
        "SELECT insight,model_version,coverage,created_at FROM image_enrichment_results
         WHERE content_hash=?1 AND model_key=?2 AND preprocess=?3 AND schema_id=?4",
        params![claim.content_hash, job.model_key, PREPROCESS_ID, INSIGHT_SCHEMA_ID],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let insight: Insight = serde_json::from_str(&raw).map_err(|_| EnrichError::Internal)?;
    let profile = exact_ready_profile(tx, &job.provider_id, claim.provider_revision)?;
    let mut model = crate::image_ai::model_ref(profile.kind, &job.model, model_version.as_deref());
    model.preprocessing = PREPROCESS_ID.into();
    model.output_schema = INSIGHT_SCHEMA_ID.into();
    let coverage = if coverage == "full_image" { Coverage::FullImage } else { Coverage::Preview };
    let mut evidence = crate::image_ai::evidence(
        &insight, &claim.content_hash, coverage, model, (created_at.max(0) as u64).saturating_mul(1000),
    );
    evidence.visual = Some(VisualSuggestions {
        tags: insight.visual_tags,
        uncertain: insight.uncertain,
        provider_revision: claim.provider_revision,
    });
    asset.evidence.push(evidence);
    asset.header.enrichment_state = EnrichmentState::Complete;
    if serde_json::to_vec(&asset).map_err(|_| EnrichError::Internal)?.len() > crate::image_contracts::MAX_BUNDLE_BYTES {
        return Ok(false);
    }
    crate::image_catalog::validate(&asset).map_err(|_| EnrichError::Internal)?;
    let evidence = serde_json::to_string(&asset.evidence).map_err(|_| EnrichError::Internal)?;
    tx.execute(
        "UPDATE image_assets SET evidence=?2,enrichment_state='complete' WHERE id=?1",
        params![claim.asset_id, evidence],
    )?;
    tx.execute(
        "INSERT INTO image_enrichment_outputs(asset_id,content_hash,model_key,preprocess,schema_id,job_id,source_generation,published_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(asset_id,model_key,preprocess,schema_id) DO UPDATE SET content_hash=excluded.content_hash,job_id=excluded.job_id,source_generation=excluded.source_generation,published_at=excluded.published_at",
        params![claim.asset_id, claim.content_hash, job.model_key, PREPROCESS_ID, INSIGHT_SCHEMA_ID, claim.job_id, job.generation, now_secs()],
    )?;
    Ok(true)
}

impl Catalog {
    /// Picks the next pending image and takes a lease on it. A batch that fails
    /// its fence is paused or ended here, before anything is prepared or sent.
    pub fn claim_enrichment(&mut self, owner: &str) -> Result<Claimed> {
        let now = now_secs();
        let picked: Option<(String, String, String, String, i64)> = self
            .db
            .query_row(
                "SELECT i.job_id,i.asset_id,i.content_hash,i.operation_id,i.attempts FROM image_enrichment_items i JOIN image_enrichment_jobs j ON j.id=i.job_id
                 WHERE j.state IN ('queued','running') AND i.state='pending' AND i.not_before<=?1
                   AND NOT EXISTS(SELECT 1 FROM image_enrichment_items x WHERE x.content_hash=i.content_hash AND x.state IN ('running','dispatching') AND NOT (x.job_id=i.job_id AND x.asset_id=i.asset_id))
                 ORDER BY j.queue_order,i.asset_id LIMIT 1",
                [now],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((job_id, asset_id, hash, operation_id, attempts)) = picked else { return Ok(Claimed::None) };
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = job_row(&tx, &job_id)?;
        let verdict = fence(&tx, &job)?;
        if verdict != Fence::Open {
            apply_fence(&tx, &job_id, &verdict)?;
            tx.commit()?;
            return Ok(Claimed::Skipped);
        }
        if !content_current(&tx, &asset_id, &hash)? {
            tx.execute("UPDATE image_enrichment_items SET state='stale',error_code='CONTENT_CHANGED',updated_at=?3 WHERE job_id=?1 AND asset_id=?2 AND state='pending'", params![job_id, asset_id, now])?;
            finalize_if_done(&tx, &job_id)?;
            tx.commit()?;
            return Ok(Claimed::Skipped);
        }
        let claimed = tx.execute(
            "UPDATE image_enrichment_items SET state='running',lease_owner=?3,lease_until=?4,updated_at=?5 WHERE job_id=?1 AND asset_id=?2 AND state='pending'",
            params![job_id, asset_id, owner, now + LEASE_SECONDS, now],
        )?;
        if claimed == 0 {
            tx.commit()?;
            return Ok(Claimed::Skipped);
        }
        tx.execute("UPDATE image_enrichment_jobs SET state='running',updated_at=?2 WHERE id=?1 AND state='queued'", params![job_id, now])?;
        tx.commit()?;
        Ok(Claimed::Item(Claim {
            job_id,
            asset_id,
            content_hash: hash,
            operation_id,
            attempts,
            provider_id: job.provider_id,
            provider_revision: job.revision as u64,
            model_key: job.model_key,
            owner: owner.to_owned(),
        }))
    }

    /// Reuses a stored result for identical content, model, recipe and schema.
    /// Reuse still needs this source's live consent for the same provider, so a
    /// source that never agreed cannot receive a result that another produced.
    pub fn reuse_enrichment(&mut self, claim: &Claim) -> Result<bool> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = job_row(&tx, &claim.job_id)?;
        let verdict = fence(&tx, &job)?;
        if verdict != Fence::Open {
            apply_fence(&tx, &claim.job_id, &verdict)?;
            Self::release_claim(&tx, claim, "revoked", verdict_code(&verdict))?;
            finalize_if_done(&tx, &claim.job_id)?;
            tx.commit()?;
            return Err(EnrichError::Cancelled);
        }
        let hit: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM image_enrichment_results WHERE content_hash=?1 AND model_key=?2 AND preprocess=?3 AND schema_id=?4)",
            params![claim.content_hash, job.model_key, PREPROCESS_ID, INSIGHT_SCHEMA_ID],
            |r| r.get(0),
        )?;
        let leased: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM image_enrichment_items WHERE job_id=?1 AND asset_id=?2 AND state='running' AND lease_owner=?3)",
            params![claim.job_id, claim.asset_id, claim.owner], |r| r.get(0),
        )?;
        if !hit || !leased || !content_current(&tx, &claim.asset_id, &claim.content_hash)? {
            tx.commit()?;
            return Ok(false);
        }
        if attach(&tx, claim, &job)? {
            Self::release_claim(&tx, claim, "reused", "")?;
        } else {
            Self::release_claim(&tx, claim, "failed", "EVIDENCE_LIMIT")?;
        }
        finalize_if_done(&tx, &claim.job_id)?;
        tx.commit()?;
        Ok(true)
    }

    fn release_claim(tx: &Transaction, claim: &Claim, state: &str, code: &str) -> Result<()> {
        tx.execute(
            "UPDATE image_enrichment_items SET state=?3,lease_owner=NULL,lease_until=NULL,error_code=NULLIF(?4,''),updated_at=?5 WHERE job_id=?1 AND asset_id=?2 AND state IN ('running','dispatching')",
            params![claim.job_id, claim.asset_id, state, code, now_secs()],
        )?;
        Ok(())
    }

    /// Ends an image without any request having been made.
    pub fn fail_enrichment_item(&mut self, claim: &Claim, state: &str, code: &str) -> Result<()> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::release_claim(&tx, claim, state, code)?;
        finalize_if_done(&tx, &claim.job_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Reserves budget atomically and marks the image as about to be sent. The
    /// consent and configuration are checked once more in the same transaction,
    /// so nothing is dispatched after a revocation that already committed.
    pub fn reserve_enrichment(&mut self, claim: &Claim) -> Result<()> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = job_row(&tx, &claim.job_id)?;
        let verdict = fence(&tx, &job)?;
        let item_live: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM image_enrichment_items WHERE job_id=?1 AND asset_id=?2 AND state='running' AND lease_owner=?3)",
            params![claim.job_id, claim.asset_id, claim.owner],
            |r| r.get(0),
        )?;
        if !item_live {
            tx.commit()?;
            return Err(EnrichError::Cancelled);
        }
        if verdict != Fence::Open {
            apply_fence(&tx, &claim.job_id, &verdict)?;
            if let Fence::Pause(code) = verdict {
                // Keep the image queued for when the batch is resumed.
                Self::release_claim(&tx, claim, "pending", code)?;
            } else {
                Self::release_claim(&tx, claim, if verdict == Fence::Stale { "stale" } else { "revoked" }, verdict_code(&verdict))?;
            }
            finalize_if_done(&tx, &claim.job_id)?;
            tx.commit()?;
            return Err(EnrichError::Cancelled);
        }
        if !content_current(&tx, &claim.asset_id, &claim.content_hash)? {
            Self::release_claim(&tx, claim, "stale", "CONTENT_CHANGED")?;
            finalize_if_done(&tx, &claim.job_id)?;
            tx.commit()?;
            return Err(EnrichError::Cancelled);
        }
        let evidence_count: i64 = tx.query_row(
            "SELECT json_array_length(evidence) FROM image_assets WHERE id=?1",
            [&claim.asset_id], |row| row.get(0),
        )?;
        if evidence_count >= crate::image_contracts::MAX_EVIDENCE as i64 {
            Self::release_claim(&tx, claim, "failed", "EVIDENCE_LIMIT")?;
            finalize_if_done(&tx, &claim.job_id)?;
            tx.commit()?;
            return Err(EnrichError::Cancelled);
        }
        let (requests, units): (i64, i64) = tx.query_row(
            "SELECT count(*) FILTER (WHERE status!='released'),
                    COALESCE(SUM(CASE status WHEN 'reserved' THEN reserved_units WHEN 'unknown' THEN reserved_units WHEN 'settled' THEN used_units ELSE 0 END),0)
             FROM image_enrichment_usage WHERE job_id=?1",
            [&claim.job_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let ceiling = if requests + 1 > job.max_requests {
            Some("REQUEST_CEILING")
        } else if units + EST_UNITS > job.max_units {
            Some("UNIT_CEILING")
        } else {
            None
        };
        if let Some(code) = ceiling {
            apply_fence(&tx, &claim.job_id, &Fence::Pause(code))?;
            Self::release_claim(&tx, claim, "pending", code)?;
            tx.commit()?;
            return Err(EnrichError::BudgetExceeded);
        }
        let now = now_secs();
        let attempt = claim.attempts + 1;
        tx.execute(
            "INSERT INTO image_enrichment_usage(operation_id,attempt,job_id,asset_id,status,reserved_units,created_at,updated_at) VALUES(?1,?2,?3,?4,'reserved',?5,?6,?6)",
            params![claim.operation_id, attempt, claim.job_id, claim.asset_id, EST_UNITS, now],
        )?;
        tx.execute(
            "UPDATE image_enrichment_items SET state='dispatching',attempts=?3,lease_until=?4,updated_at=?5 WHERE job_id=?1 AND asset_id=?2",
            params![claim.job_id, claim.asset_id, attempt, now + LEASE_SECONDS, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Records the end of a request and publishes the answer only if the fence
    /// still holds. A late answer for a cancelled, revoked or changed image is
    /// discarded, though its units were spent and stay counted.
    pub fn settle_enrichment(&mut self, claim: &Claim, result: std::result::Result<(Described, crate::image_contracts::Coverage), DescribeError>, original_verified: bool) -> Result<Settled> {
        let attempt = claim.attempts + 1;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job = job_row(&tx, &claim.job_id)?;
        let verdict = fence(&tx, &job)?;
        let now = now_secs();
        let usage = |tx: &Transaction, status: &str, used: Option<(i64, bool)>| -> Result<()> {
            tx.execute(
                "UPDATE image_enrichment_usage SET status=?3,used_units=?4,used_is_estimate=?5,updated_at=?6 WHERE operation_id=?1 AND attempt=?2",
                params![claim.operation_id, attempt, status, used.map(|u| u.0), used.map(|u| u.1 as i64), now],
            )?;
            Ok(())
        };
        let outcome = match result {
            Ok((described, coverage)) => {
                let reported = match (described.usage.input_tokens, described.usage.output_tokens) {
                    (Some(i), Some(o)) => Some(i.saturating_add(o).min(100_000_000) as i64),
                    _ => None,
                };
                usage(&tx, "settled", Some((reported.unwrap_or(EST_UNITS), reported.is_none())))?;
                let item_live: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM image_enrichment_items WHERE job_id=?1 AND asset_id=?2 AND state='dispatching' AND lease_owner=?3)",
                    params![claim.job_id, claim.asset_id, claim.owner],
                    |r| r.get(0),
                )?;
                // Read inside this write transaction: a revocation that committed
                // while the request was in flight is seen here.
                let verdict_now = if item_live { verdict } else { Fence::Revoked("CANCELLED") };
                if verdict_now == Fence::Open && original_verified && content_current(&tx, &claim.asset_id, &claim.content_hash)? {
                    let insight = serde_json::to_string(&described.insight).map_err(|_| EnrichError::Internal)?;
                    tx.execute(
                        "INSERT INTO image_enrichment_results(content_hash,model_key,preprocess,schema_id,insight,model_version,coverage,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                         ON CONFLICT(content_hash,model_key,preprocess,schema_id) DO NOTHING",
                        params![claim.content_hash, job.model_key, PREPROCESS_ID, INSIGHT_SCHEMA_ID, insight, described.model_version, if coverage == crate::image_contracts::Coverage::FullImage { "full_image" } else { "preview" }, now],
                    )?;
                    if attach(&tx, claim, &job)? {
                        Self::release_claim(&tx, claim, "done", "")?;
                        Settled::Published
                    } else {
                        Self::release_claim(&tx, claim, "failed", "EVIDENCE_LIMIT")?;
                        Settled::Failed
                    }
                } else {
                    let (state, code) = match &verdict_now {
                        Fence::Stale => ("stale", "SOURCE_CHANGED"),
                        Fence::Open => ("stale", "CONTENT_CHANGED"),
                        other => ("revoked", verdict_code(other)),
                    };
                    if verdict_now != Fence::Open {
                        apply_fence(&tx, &claim.job_id, &verdict_now)?;
                    }
                    Self::release_claim(&tx, claim, state, code)?;
                    Settled::Discarded
                }
            }
            Err(error) => match disposition(&error) {
                Disposition::Billed => {
                    usage(&tx, "settled", Some((EST_UNITS, true)))?;
                    Self::release_claim(&tx, claim, "failed", error.code())?;
                    Settled::Failed
                }
                Disposition::Unknown => {
                    // Keep the reservation counted; never retried without the user.
                    usage(&tx, "unknown", None)?;
                    tx.execute(
                        "UPDATE image_enrichment_items SET state='unknown_outcome',lease_owner=NULL,lease_until=NULL,error_code='OUTCOME_UNKNOWN',updated_at=?3 WHERE job_id=?1 AND asset_id=?2 AND state='dispatching'",
                        params![claim.job_id, claim.asset_id, now],
                    )?;
                    Settled::Unknown
                }
                Disposition::Retry => {
                    usage(&tx, "released", None)?;
                    if attempt >= MAX_RATE_ATTEMPTS {
                        Self::release_claim(&tx, claim, "failed", "RATE_LIMITED")?;
                        Settled::Failed
                    } else {
                        let wait = (15i64 << (attempt - 1)).min(900);
                        tx.execute(
                            "UPDATE image_enrichment_items SET state='pending',lease_owner=NULL,lease_until=NULL,not_before=?3,error_code='RATE_LIMITED',updated_at=?4 WHERE job_id=?1 AND asset_id=?2",
                            params![claim.job_id, claim.asset_id, now + wait, now],
                        )?;
                        Settled::Retrying
                    }
                }
                Disposition::Pause(code) => {
                    usage(&tx, "released", None)?;
                    Self::release_claim(&tx, claim, "pending", code)?;
                    apply_fence(&tx, &claim.job_id, &Fence::Pause(code))?;
                    Settled::Paused
                }
                Disposition::NotBilled => {
                    usage(&tx, "released", None)?;
                    Self::release_claim(&tx, claim, "failed", error.code())?;
                    Settled::Failed
                }
            },
        };
        finalize_if_done(&tx, &claim.job_id)?;
        tx.commit()?;
        Ok(outcome)
    }
}

fn verdict_code(fence: &Fence) -> &'static str {
    match fence {
        Fence::Pause(c) | Fence::Revoked(c) => c,
        Fence::Stale => "SOURCE_CHANGED",
        Fence::Open => "",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    Published,
    /// Answer received and paid for, but not stored: the fence no longer held.
    Discarded,
    Failed,
    Unknown,
    Retrying,
    Paused,
}

#[derive(Debug, PartialEq, Eq)]
enum Disposition {
    /// A valid HTTP answer arrived but its content was unusable: billed.
    Billed,
    /// Sent, but no answer: the provider may have billed.
    Unknown,
    /// Rate limited before processing: back off and retry a few times.
    Retry,
    /// The batch cannot continue until the user fixes something.
    Pause(&'static str),
    /// Refused before processing: not billed, this image failed.
    NotBilled,
}
fn disposition(error: &DescribeError) -> Disposition {
    use ProviderError as P;
    match error {
        DescribeError::Output(_) => Disposition::Billed,
        DescribeError::Provider(p) => match p {
            P::Timeout | P::ProviderUnavailable(_) | P::ResponseTooLarge | P::MalformedResponse => Disposition::Unknown,
            P::RateLimited => Disposition::Retry,
            P::QuotaExceeded => Disposition::Pause("QUOTA_EXCEEDED"),
            P::Authentication | P::KeyMissing | P::KeychainDenied | P::KeychainUnavailable(_) => Disposition::Pause("PROVIDER_AUTH"),
            P::ModelNotFound => Disposition::Pause("MODEL_NOT_FOUND"),
            P::ImageUnsupported | P::SchemaIncompatible | P::Unsupported => Disposition::Pause("PROVIDER_NOT_READY"),
            P::InvalidSettings(_) | P::InsecureEndpoint | P::LocalNotPermitted | P::AddressNotAllowed | P::Conflict | P::NotFound => Disposition::Pause("PROVIDER_CHANGED"),
            P::Redirected | P::Tls | P::Unreachable | P::RequestTooLarge | P::Rejected { .. } => Disposition::NotBilled,
        },
    }
}

// ----------------------------------------------------------------- runner

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelError {
    /// The original or its preview cannot be read right now.
    Unavailable,
    /// The content is no longer the version that was queued.
    Stale,
    Failed,
}
/// Supplies the pixels to send: the oriented, bounded preview of the exact
/// content version, decoded outside the long-lived database lock.
pub trait PixelSource: Send + Sync {
    fn prepare(&self, asset_id: &str, content_hash: &str) -> std::result::Result<PreparedImage, PixelError>;
    fn verify(&self, _asset_id: &str, _content_hash: &str) -> std::result::Result<(), PixelError> {
        Ok(())
    }
}
/// The production pixel source: the Large (512 px) preview of the exact content
/// version, produced by the isolated image worker and re-encoded without any
/// metadata. Coverage is judged against the original's real dimensions.
pub struct PreviewPixels {
    pub previews: Arc<crate::image_preview::PreviewService>,
}
impl PixelSource for PreviewPixels {
    fn verify(&self, asset_id: &str, content_hash: &str) -> std::result::Result<(), PixelError> {
        use crate::image_preview::PreviewError as E;
        self.previews.verify_original(asset_id, content_hash).map_err(|error| match error {
            E::StaleVersion | E::SourceChanged | E::NotFound => PixelError::Stale,
            _ => PixelError::Unavailable,
        })
    }
    fn prepare(&self, asset_id: &str, content_hash: &str) -> std::result::Result<PreparedImage, PixelError> {
        use crate::image_preview::{PreviewError as E, PreviewRequest, PreviewTier};
        let request = PreviewRequest {
            request_id: format!("enrich-{}", uuid::Uuid::new_v4()),
            asset_id: asset_id.to_owned(),
            version: content_hash.to_owned(),
            tier: PreviewTier::Large,
            tile: None,
        };
        let preview = self.previews.preview(&request).map_err(|e| match e {
            E::StaleVersion | E::SourceChanged | E::NotFound => PixelError::Stale,
            E::OriginalUnavailable | E::Busy | E::TimedOut | E::Cancelled | E::WorkerUnavailable => PixelError::Unavailable,
            _ => PixelError::Failed,
        })?;
        prepare_preview(&preview.bytes, preview.meta.source_width, preview.meta.source_height)
    }
}

pub(crate) fn prepare_preview(bytes: &[u8], source_width: u32, source_height: u32) -> std::result::Result<PreparedImage, PixelError> {
    let decoded = image::load_from_memory(bytes).map_err(|_| PixelError::Failed)?.to_rgba8();
    let mut prepared = crate::image_ai::prepare_pixels(&decoded).map_err(|_| PixelError::Failed)?;
    prepared.source_width = source_width;
    prepared.source_height = source_height;
    Ok(prepared)
}
/// Performs the one provider request. Never retries.
pub trait Dispatcher: Send + Sync {
    fn describe(&self, provider_id: &str, revision: u64, image: &PreparedImage) -> std::result::Result<Described, DescribeError>;
}

/// Real dispatch through the configured provider at the exact revision.
pub struct ProviderDispatcher {
    pub providers: Arc<crate::ai_provider::Providers>,
}
impl Dispatcher for ProviderDispatcher {
    fn describe(&self, provider_id: &str, revision: u64, image: &PreparedImage) -> std::result::Result<Described, DescribeError> {
        let adapter = self.providers.adapter(provider_id, revision)?;
        crate::image_ai::describe_visual(adapter.as_ref(), image)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Idle,
    Worked,
    /// Interactive work (imports, previews, playback) has priority right now.
    Yielded,
}

pub struct Runner {
    pub catalog: Arc<Mutex<Catalog>>,
    pub pixels: Arc<dyn PixelSource>,
    pub dispatcher: Arc<dyn Dispatcher>,
    /// True while interactive work should keep the machine to itself.
    pub busy: Arc<dyn Fn() -> bool + Send + Sync>,
    pub owner: String,
}
impl Runner {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Catalog>> {
        self.catalog.lock().map_err(|_| EnrichError::Internal)
    }

    /// Processes at most one image. One request is in flight at a time and one
    /// prepared payload is held, which bounds memory and spend rate.
    pub fn step(&self) -> Result<Step> {
        if (self.busy)() {
            return Ok(Step::Yielded);
        }
        let mut claim = None;
        for _ in 0..MAX_CLAIM_SKIPS {
            match self.lock()?.claim_enrichment(&self.owner)? {
                Claimed::None => return Ok(Step::Idle),
                Claimed::Skipped => continue,
                Claimed::Item(c) => {
                    claim = Some(c);
                    break;
                }
            }
        }
        let Some(claim) = claim else { return Ok(Step::Worked) };
        if let Err(error) = self.pixels.verify(&claim.asset_id, &claim.content_hash) {
            let (state, code) = match error {
                PixelError::Stale => ("stale", "CONTENT_CHANGED"),
                _ => ("failed", "ORIGINAL_UNAVAILABLE"),
            };
            self.lock()?.fail_enrichment_item(&claim, state, code)?;
            return Ok(Step::Worked);
        }
        match self.lock()?.reuse_enrichment(&claim) {
            Ok(true) => return Ok(Step::Worked),
            Ok(false) => {}
            Err(EnrichError::Cancelled) => return Ok(Step::Worked),
            Err(e) => return Err(e),
        }
        let prepared = match self.pixels.prepare(&claim.asset_id, &claim.content_hash) {
            Ok(p) => p,
            Err(error) => {
                let (state, code) = match error {
                    PixelError::Stale => ("stale", "CONTENT_CHANGED"),
                    PixelError::Unavailable => ("failed", "ORIGINAL_UNAVAILABLE"),
                    PixelError::Failed => ("failed", "DECODE_FAILED"),
                };
                self.lock()?.fail_enrichment_item(&claim, state, code)?;
                return Ok(Step::Worked);
            }
        };
        match self.lock()?.reserve_enrichment(&claim) {
            Ok(()) => {}
            Err(EnrichError::Cancelled | EnrichError::BudgetExceeded) => return Ok(Step::Worked),
            Err(e) => return Err(e),
        }
        let coverage = prepared.coverage();
        let answer = self.dispatcher.describe(&claim.provider_id, claim.provider_revision, &prepared);
        drop(prepared);
        let verified = answer.is_err() || self.pixels.verify(&claim.asset_id, &claim.content_hash).is_ok();
        self.lock()?.settle_enrichment(&claim, answer.map(|d| (d, coverage)), verified)?;
        Ok(Step::Worked)
    }
}
