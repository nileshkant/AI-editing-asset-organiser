-- v19 (SS-052): consented image enrichment. Everything here is machine-local:
-- consent names a provider configured on this computer, and reservations,
-- leases and billing evidence describe requests made from it. None of it is in
-- portable catalogs or source snapshots; accepted AI evidence becomes
-- portable only when SS-053 persists it on the asset.
CREATE TABLE image_enrichment_consent (
 source_id TEXT PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
 provider_id TEXT NOT NULL CHECK(length(provider_id) BETWEEN 1 AND 64),
 provider_revision INTEGER NOT NULL CHECK(provider_revision BETWEEN 1 AND 9007199254740991),
 model TEXT NOT NULL CHECK(length(model) BETWEEN 1 AND 200),
 purpose TEXT NOT NULL CHECK(purpose='description'),
 -- What leaves the device. Only a re-encoded preview, never metadata or text.
 payload TEXT NOT NULL CHECK(payload='preview_image'),
 analyze_new INTEGER NOT NULL DEFAULT 0 CHECK(analyze_new IN (0,1)),
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>=1),
 granted_at INTEGER NOT NULL
);
CREATE TABLE image_enrichment_jobs (
 id TEXT PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
 source_generation INTEGER NOT NULL CHECK(source_generation>=0),
 kind TEXT NOT NULL CHECK(kind IN ('new_imports','selected','backfill')),
 provider_id TEXT NOT NULL CHECK(length(provider_id) BETWEEN 1 AND 64),
 provider_revision INTEGER NOT NULL CHECK(provider_revision>=1),
 model TEXT NOT NULL CHECK(length(model) BETWEEN 1 AND 200),
 model_key TEXT NOT NULL CHECK(length(model_key) BETWEEN 1 AND 240),
 preprocess TEXT NOT NULL CHECK(length(preprocess) BETWEEN 1 AND 64),
 schema_id TEXT NOT NULL CHECK(length(schema_id) BETWEEN 1 AND 64),
 state TEXT NOT NULL CHECK(state IN ('queued','running','paused','complete','failed','cancelled')),
 error_code TEXT CHECK(error_code IS NULL OR length(error_code)<=64),
 max_requests INTEGER NOT NULL CHECK(max_requests BETWEEN 1 AND 100000),
 max_units INTEGER NOT NULL CHECK(max_units BETWEEN 1 AND 1000000000),
 queue_order INTEGER NOT NULL CHECK(queue_order BETWEEN 1 AND 9007199254740991),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE INDEX image_enrichment_job_schedule ON image_enrichment_jobs(state,queue_order,id);
CREATE INDEX image_enrichment_job_source ON image_enrichment_jobs(source_id,created_at,id);
CREATE TABLE image_enrichment_items (
 job_id TEXT NOT NULL REFERENCES image_enrichment_jobs(id) ON DELETE CASCADE,
 asset_id TEXT NOT NULL REFERENCES asset_registry(id) ON DELETE CASCADE,
 content_hash TEXT NOT NULL CHECK(length(content_hash)=64 AND content_hash NOT GLOB '*[^0-9a-f]*'),
 state TEXT NOT NULL CHECK(state IN ('pending','running','dispatching','done','reused','failed','unknown_outcome','stale','revoked')),
 -- Stable identity of this unit of billable work across retries and restarts.
 operation_id TEXT NOT NULL CHECK(length(operation_id)=36),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 20),
 not_before INTEGER NOT NULL DEFAULT 0,
 lease_owner TEXT,
 lease_until INTEGER,
 error_code TEXT CHECK(error_code IS NULL OR length(error_code)<=64),
 updated_at INTEGER NOT NULL,
 PRIMARY KEY(job_id,asset_id),
 CHECK((state IN ('running','dispatching')) = (lease_owner IS NOT NULL AND lease_until IS NOT NULL))
);
CREATE INDEX image_enrichment_pending ON image_enrichment_items(job_id,state,not_before,asset_id);
CREATE INDEX image_enrichment_item_asset ON image_enrichment_items(asset_id,state);
CREATE INDEX image_enrichment_item_content ON image_enrichment_items(content_hash,state);
-- Reservations and billing evidence. A row exists per dispatch attempt; the
-- operation id repeats across attempts of the same item.
CREATE TABLE image_enrichment_usage (
 operation_id TEXT NOT NULL CHECK(length(operation_id)=36),
 attempt INTEGER NOT NULL CHECK(attempt BETWEEN 1 AND 20),
 job_id TEXT NOT NULL REFERENCES image_enrichment_jobs(id) ON DELETE CASCADE,
 asset_id TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('reserved','settled','unknown','released')),
 reserved_units INTEGER NOT NULL CHECK(reserved_units BETWEEN 0 AND 1000000),
 used_units INTEGER CHECK(used_units IS NULL OR used_units BETWEEN 0 AND 100000000),
 used_is_estimate INTEGER CHECK(used_is_estimate IS NULL OR used_is_estimate IN (0,1)),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 PRIMARY KEY(operation_id,attempt)
);
CREATE INDEX image_enrichment_usage_job ON image_enrichment_usage(job_id,status);
-- Paid results, shared by content, model, preprocessing and schema version.
-- A result is attached to an asset only by a fenced publication.
CREATE TABLE image_enrichment_results (
 content_hash TEXT NOT NULL CHECK(length(content_hash)=64 AND content_hash NOT GLOB '*[^0-9a-f]*'),
 model_key TEXT NOT NULL,
 preprocess TEXT NOT NULL,
 schema_id TEXT NOT NULL,
 insight TEXT NOT NULL CHECK(json_valid(insight) AND length(CAST(insight AS BLOB))<=16384),
 model_version TEXT CHECK(model_version IS NULL OR length(model_version)<=200),
 coverage TEXT NOT NULL CHECK(coverage IN ('full_image','preview')),
 created_at INTEGER NOT NULL,
 PRIMARY KEY(content_hash,model_key,preprocess,schema_id)
);
CREATE TABLE image_enrichment_outputs (
 asset_id TEXT NOT NULL REFERENCES asset_registry(id) ON DELETE CASCADE,
 content_hash TEXT NOT NULL,
 model_key TEXT NOT NULL,
 preprocess TEXT NOT NULL,
 schema_id TEXT NOT NULL,
 job_id TEXT,
 source_generation INTEGER NOT NULL,
 published_at INTEGER NOT NULL,
 PRIMARY KEY(asset_id,model_key,preprocess,schema_id),
 FOREIGN KEY(content_hash,model_key,preprocess,schema_id)
  REFERENCES image_enrichment_results(content_hash,model_key,preprocess,schema_id) ON DELETE CASCADE
);
