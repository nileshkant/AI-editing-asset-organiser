-- Additive v11: independent typed image/mixed import lane. Legacy audio jobs and
-- their MCP wire shape are unchanged and cannot expose these records.
CREATE TABLE image_import_jobs (
 id TEXT PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
 source_generation INTEGER NOT NULL CHECK(source_generation>=0),
 selection_key TEXT NOT NULL CHECK(length(selection_key)<=128),
 mode TEXT NOT NULL CHECK(mode IN ('images','mixed')),
 discovery_epoch INTEGER NOT NULL DEFAULT 1 CHECK(discovery_epoch>0),
 root_stamp TEXT CHECK(root_stamp IS NULL OR (json_valid(root_stamp) AND length(CAST(root_stamp AS BLOB))<=4096)),
 folder_stamps TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(folder_stamps) AND json_type(folder_stamps)='object' AND length(CAST(folder_stamps AS BLOB))<=131072),
 folders TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(folders) AND json_type(folders)='array' AND length(CAST(folders AS BLOB))<=1048576),
 paths TEXT CHECK(paths IS NULL OR (json_valid(paths) AND json_type(paths)='array' AND length(CAST(paths AS BLOB))<=1048576)),
 state TEXT NOT NULL CHECK(state IN ('queued','running','complete','failed','cancelled')),
 phase TEXT NOT NULL CHECK(phase IN ('discovery','analysis','complete')),
 history_truncated INTEGER NOT NULL DEFAULT 0 CHECK(history_truncated IN (0,1)),
 discovery_complete INTEGER NOT NULL DEFAULT 0 CHECK(discovery_complete IN (0,1)),
 lease_owner TEXT,
 lease_until INTEGER,
 sequence INTEGER NOT NULL DEFAULT 1 CHECK(sequence BETWEEN 1 AND 9007199254740991),
 queue_order INTEGER NOT NULL CHECK(queue_order BETWEEN 1 AND 9007199254740991),
 total INTEGER NOT NULL DEFAULT 0 CHECK(total BETWEEN 0 AND 100000),
 completed INTEGER NOT NULL DEFAULT 0 CHECK(completed BETWEEN 0 AND total),
 ready INTEGER NOT NULL DEFAULT 0 CHECK(ready BETWEEN 0 AND completed),
 reused INTEGER NOT NULL DEFAULT 0 CHECK(reused BETWEEN 0 AND ready),
 failed INTEGER NOT NULL DEFAULT 0 CHECK(failed BETWEEN 0 AND completed),
 unsupported INTEGER NOT NULL DEFAULT 0 CHECK(unsupported BETWEEN 0 AND completed),
 current TEXT NOT NULL DEFAULT '' CHECK(length(CAST(current AS BLOB))<=32768),
 error_code TEXT CHECK(error_code IS NULL OR length(error_code)<=64),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 CHECK(completed=ready+failed+unsupported),
 CHECK((state='running' AND lease_owner IS NOT NULL AND lease_until IS NOT NULL) OR
       (state!='running' AND lease_owner IS NULL AND lease_until IS NULL))
);
CREATE UNIQUE INDEX image_import_active_selection ON image_import_jobs(source_id,source_generation)
 WHERE state IN ('queued','running');
CREATE INDEX image_import_schedule ON image_import_jobs(state,queue_order,id);
CREATE INDEX image_import_source_history ON image_import_jobs(source_id,created_at,id);
CREATE TABLE image_import_items (
 job_id TEXT NOT NULL REFERENCES image_import_jobs(id) ON DELETE CASCADE,
 relative_path TEXT NOT NULL CHECK(length(CAST(relative_path AS BLOB)) BETWEEN 1 AND 32768),
 media_kind TEXT NOT NULL CHECK(media_kind IN ('image','audio','unsupported')),
 state TEXT NOT NULL CHECK(state IN ('pending','running','ready','failed','unsupported')),
 asset_id TEXT CHECK(asset_id IS NULL OR length(CAST(asset_id AS BLOB))<=256) REFERENCES asset_registry(id) ON DELETE SET NULL,
 content_hash TEXT CHECK(content_hash IS NULL OR (length(content_hash)=64 AND content_hash NOT GLOB '*[^0-9a-f]*')),
 warnings TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(warnings) AND json_type(warnings)='array' AND length(CAST(warnings AS BLOB))<=512),
 error_code TEXT CHECK(error_code IS NULL OR length(error_code)<=64),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 100),
 discovered_epoch INTEGER NOT NULL DEFAULT 1 CHECK(discovered_epoch>0),
 CHECK(media_kind!='unsupported' OR state!='ready'),
 PRIMARY KEY(job_id,relative_path)
);
CREATE INDEX image_import_pending_items ON image_import_items(job_id,state,relative_path);
CREATE TRIGGER image_import_protect_snapshot AFTER INSERT ON image_import_jobs BEGIN
 INSERT INTO source_catalog_state(source_id,dirty,last_error) VALUES(NEW.source_id,1,
 'Image import requires mixed catalog support; audio-only snapshot is protected')
 ON CONFLICT(source_id) DO UPDATE SET dirty=dirty+1,
 last_error='Image import requires mixed catalog support; audio-only snapshot is protected';
END;

-- Logical job metadata, not user asset annotations, has a bounded retention budget.
CREATE TABLE image_import_budget (
 id INTEGER PRIMARY KEY CHECK(id=1),
 rows INTEGER NOT NULL CHECK(rows BETWEEN 0 AND 100000),
 bytes INTEGER NOT NULL CHECK(bytes BETWEEN 0 AND 134217728)
);
INSERT INTO image_import_budget VALUES(1,0,0);
CREATE TRIGGER image_import_item_budget_insert AFTER INSERT ON image_import_items BEGIN
 UPDATE image_import_budget SET rows=rows+1,bytes=bytes+1024+length(CAST(NEW.relative_path AS BLOB)) WHERE id=1;
END;
CREATE TRIGGER image_import_item_budget_delete AFTER DELETE ON image_import_items BEGIN
 UPDATE image_import_budget SET rows=rows-1,bytes=bytes-1024-length(CAST(OLD.relative_path AS BLOB)) WHERE id=1;
END;
CREATE TRIGGER image_import_item_budget_update AFTER UPDATE OF relative_path ON image_import_items BEGIN
 UPDATE image_import_budget SET bytes=bytes+length(CAST(NEW.relative_path AS BLOB))-length(CAST(OLD.relative_path AS BLOB)) WHERE id=1;
END;

CREATE INDEX image_import_history_order ON image_import_jobs(state,created_at,id);
CREATE TRIGGER image_import_job_budget_insert AFTER INSERT ON image_import_jobs BEGIN
 UPDATE image_import_budget SET bytes=bytes+40960+COALESCE(length(CAST(NEW.paths AS BLOB)),0)+length(CAST(NEW.folders AS BLOB))+length(CAST(NEW.folder_stamps AS BLOB)) WHERE id=1;
END;
CREATE TRIGGER image_import_job_budget_delete AFTER DELETE ON image_import_jobs BEGIN
 UPDATE image_import_budget SET bytes=bytes-40960-COALESCE(length(CAST(OLD.paths AS BLOB)),0)-length(CAST(OLD.folders AS BLOB))-length(CAST(OLD.folder_stamps AS BLOB)) WHERE id=1;
END;
CREATE TRIGGER image_import_job_budget_update AFTER UPDATE OF paths,folders,folder_stamps ON image_import_jobs BEGIN
 UPDATE image_import_budget SET bytes=bytes+COALESCE(length(CAST(NEW.paths AS BLOB)),0)+length(CAST(NEW.folders AS BLOB))+length(CAST(NEW.folder_stamps AS BLOB))-COALESCE(length(CAST(OLD.paths AS BLOB)),0)-length(CAST(OLD.folders AS BLOB))-length(CAST(OLD.folder_stamps AS BLOB)) WHERE id=1;
END;

-- Machine-local source binding is durable, separate from evictable job history.
CREATE TABLE image_source_identity (
 source_id TEXT PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
 source_generation INTEGER NOT NULL CHECK(source_generation>=0),
 root_stamp TEXT NOT NULL CHECK(json_valid(root_stamp) AND length(CAST(root_stamp AS BLOB))<=4096)
);

-- Content-bound technical warnings are cached independently of file-name hints.
CREATE TABLE image_analysis_warnings (
 content_hash TEXT NOT NULL,
 analyzer_version TEXT NOT NULL,
 warnings TEXT NOT NULL CHECK(json_valid(warnings) AND json_type(warnings)='array' AND length(CAST(warnings AS BLOB))<=512),
 PRIMARY KEY(content_hash,analyzer_version),
 FOREIGN KEY(content_hash,analyzer_version) REFERENCES image_analyses(content_hash,analyzer_version)
);
CREATE TRIGGER image_analysis_warnings_immutable BEFORE UPDATE ON image_analysis_warnings BEGIN
 SELECT RAISE(ABORT,'Technical warning versions are immutable');
END;

-- Serialize different import lanes for one source while allowing other sources
-- and playback to progress. Triggers cover independent SQLite connections too.
CREATE TRIGGER image_import_audio_conflict BEFORE INSERT ON image_import_jobs
 WHEN EXISTS(SELECT 1 FROM jobs WHERE source_id=NEW.source_id AND state IN ('queued','running')) BEGIN
 SELECT RAISE(ABORT,'Another import owns this source');
END;
CREATE TRIGGER audio_import_image_conflict_insert BEFORE INSERT ON jobs
 WHEN NEW.state IN ('queued','running') AND EXISTS(SELECT 1 FROM image_import_jobs WHERE source_id=NEW.source_id AND state IN ('queued','running')) BEGIN
 SELECT RAISE(ABORT,'Image import owns this source');
END;
CREATE TRIGGER audio_import_image_conflict_update BEFORE UPDATE OF state ON jobs
 WHEN NEW.state IN ('queued','running') AND EXISTS(SELECT 1 FROM image_import_jobs WHERE source_id=NEW.source_id AND state IN ('queued','running')) BEGIN
 SELECT RAISE(ABORT,'Image import owns this source');
END;
