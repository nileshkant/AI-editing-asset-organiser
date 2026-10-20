-- Additive v14 (SS-047): refresh/check scans, the file-stat fast path, and opt-in
-- per-source monitoring. Existing jobs keep exact-byte import semantics.
ALTER TABLE image_import_jobs ADD COLUMN scan_kind TEXT NOT NULL DEFAULT 'discover'
 CHECK(scan_kind IN ('discover','listed'));
ALTER TABLE image_import_jobs ADD COLUMN verify INTEGER NOT NULL DEFAULT 1 CHECK(verify IN (0,1));
ALTER TABLE image_import_jobs ADD COLUMN origin TEXT NOT NULL DEFAULT 'import'
 CHECK(origin IN ('import','refresh','watch','relink'));

-- Machine-local evidence that a member is unchanged since its bytes were hashed.
-- Never identity: a mismatch only causes a rehash. Deleted with the asset.
CREATE TABLE asset_file_stats (
 asset_id TEXT PRIMARY KEY REFERENCES asset_registry(id) ON DELETE CASCADE,
 content_hash TEXT NOT NULL CHECK(length(content_hash)=64 AND content_hash NOT GLOB '*[^0-9a-f]*'),
 volume TEXT NOT NULL CHECK(length(CAST(volume AS BLOB)) BETWEEN 1 AND 128),
 file_id TEXT NOT NULL CHECK(length(file_id) BETWEEN 1 AND 20 AND file_id NOT GLOB '*[^0-9]*'),
 size INTEGER NOT NULL CHECK(size>=0),
 modified INTEGER NOT NULL,
 changed INTEGER,
 recorded INTEGER NOT NULL
);
CREATE INDEX asset_file_stats_identity ON asset_file_stats(volume,file_id);

-- Watching is opt-in per source and never implied by an import.
CREATE TABLE image_source_monitor (
 source_id TEXT PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
 watch INTEGER NOT NULL DEFAULT 0 CHECK(watch IN (0,1)),
 mode TEXT NOT NULL DEFAULT 'images' CHECK(mode IN ('images','mixed')),
 root_state TEXT NOT NULL DEFAULT 'unknown'
  CHECK(root_state IN ('unknown','online','offline','inaccessible','changed')),
 checked_at INTEGER NOT NULL DEFAULT 0 CHECK(checked_at>=0)
);
INSERT INTO image_source_monitor(source_id,mode)
 SELECT source_id,CASE WHEN max(mode='mixed') THEN 'mixed' ELSE 'images' END
 FROM image_import_jobs GROUP BY source_id;
INSERT OR IGNORE INTO image_source_monitor(source_id)
 SELECT DISTINCT source_id FROM asset_registry WHERE kind IN ('image','element')
 UNION SELECT source_id FROM image_source_identity;
-- Remember the widest media mode a source was imported with.
CREATE TRIGGER image_monitor_mode AFTER INSERT ON image_import_jobs BEGIN
 INSERT INTO image_source_monitor(source_id,mode) VALUES(NEW.source_id,NEW.mode)
 ON CONFLICT(source_id) DO UPDATE SET mode=CASE WHEN excluded.mode='mixed' THEN 'mixed' ELSE mode END;
END;
