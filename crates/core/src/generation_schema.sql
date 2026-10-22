-- v17 (SS-055): generation metadata claims and user-authored prompt provenance.
-- Embedded claims are content-bound and parser-versioned, like analyses.
-- Sidecar claims and user provenance belong to one asset and survive content
-- changes. Every write marks the folder snapshot dirty.
CREATE TABLE image_generation_extractions (
 content_hash TEXT NOT NULL REFERENCES image_contents(content_hash),
 parser_version TEXT NOT NULL CHECK(length(CAST(parser_version AS BLOB)) BETWEEN 1 AND 256),
 extraction TEXT NOT NULL CHECK(json_valid(extraction) AND json_type(extraction)='object'
   AND length(CAST(extraction AS BLOB))<=524288 AND json_extract(extraction,'$.parser')=parser_version),
 PRIMARY KEY(content_hash,parser_version)
) WITHOUT ROWID;
CREATE TABLE image_generation_sidecars (
 asset_id TEXT PRIMARY KEY REFERENCES image_assets(id) ON DELETE CASCADE,
 relative_path TEXT NOT NULL CHECK(length(CAST(relative_path AS BLOB)) BETWEEN 1 AND 32768),
 present INTEGER NOT NULL CHECK(present IN (0,1)),
 -- NULL while present: the sidecar exceeded the read limit and was not hashed.
 sidecar_hash TEXT CHECK(sidecar_hash IS NULL OR (length(sidecar_hash)=64 AND sidecar_hash NOT GLOB '*[^0-9a-f]*')),
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),
 extraction TEXT NOT NULL CHECK(json_valid(extraction) AND json_type(extraction)='object'
   AND length(CAST(extraction AS BLOB))<=524288),
 observed_at INTEGER NOT NULL,
 size INTEGER,
 modified INTEGER
);
CREATE TABLE image_generation_provenance (
 asset_id TEXT PRIMARY KEY REFERENCES image_assets(id) ON DELETE CASCADE,
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),
 record TEXT NOT NULL CHECK(json_valid(record) AND json_type(record)='object'
   AND json_extract(record,'$.revision') IS revision AND length(CAST(record AS BLOB))<=262144),
 updated_at INTEGER NOT NULL
);
CREATE TRIGGER generation_sidecar_dirty_insert AFTER INSERT ON image_generation_sidecars BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1
 WHERE source_id=(SELECT source_id FROM asset_registry WHERE id=NEW.asset_id);
END;
CREATE TRIGGER generation_sidecar_dirty_update AFTER UPDATE OF present,sidecar_hash,revision,extraction ON image_generation_sidecars BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1
 WHERE source_id=(SELECT source_id FROM asset_registry WHERE id=NEW.asset_id);
END;
CREATE TRIGGER generation_provenance_dirty_insert AFTER INSERT ON image_generation_provenance BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1
 WHERE source_id=(SELECT source_id FROM asset_registry WHERE id=NEW.asset_id);
END;
CREATE TRIGGER generation_provenance_dirty_update AFTER UPDATE ON image_generation_provenance BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1
 WHERE source_id=(SELECT source_id FROM asset_registry WHERE id=NEW.asset_id);
END;
CREATE TRIGGER generation_extraction_dirty AFTER INSERT ON image_generation_extractions BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1
 WHERE source_id IN (SELECT r.source_id FROM image_assets i JOIN asset_registry r ON r.id=i.id
   WHERE i.content_hash=NEW.content_hash);
END;
