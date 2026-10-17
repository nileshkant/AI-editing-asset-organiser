-- Additive v10 image storage. Legacy audio payloads/tables keep their identities.
CREATE TABLE asset_registry (
 id TEXT PRIMARY KEY,
 kind TEXT NOT NULL CHECK(kind IN ('audio','image','element')),
 source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
 relative_path TEXT NOT NULL,
 CHECK(kind='audio' OR (length(id)=36 AND id=lower(id) AND
   substr(id,9,1)='-' AND substr(id,14,1)='-' AND substr(id,19,1)='-' AND substr(id,24,1)='-' AND
   length(replace(id,'-',''))=32 AND replace(id,'-','') NOT GLOB '*[^0-9a-f]*' AND
   id!='00000000-0000-0000-0000-000000000000' AND
   length(CAST(source_id AS BLOB)) BETWEEN 1 AND 256 AND
   length(CAST(relative_path AS BLOB)) BETWEEN 1 AND 32768)),
 UNIQUE(source_id,relative_path)
);
INSERT INTO asset_registry(id,kind,source_id,relative_path)
 SELECT id,'audio',source_id,relative_path FROM sounds;
CREATE INDEX asset_registry_source_kind ON asset_registry(source_id,kind,id);
CREATE INDEX asset_registry_kind ON asset_registry(kind,id);

-- Audio writers participate without rewriting sounds or changing its wire shape.
CREATE TRIGGER registry_sound_insert AFTER INSERT ON sounds BEGIN
 INSERT INTO asset_registry VALUES(NEW.id,'audio',NEW.source_id,NEW.relative_path);
END;
CREATE TRIGGER registry_sound_update AFTER UPDATE OF id,source_id,relative_path ON sounds BEGIN
 UPDATE asset_registry SET id=NEW.id,source_id=NEW.source_id,relative_path=NEW.relative_path
 WHERE id=OLD.id AND kind='audio';
 SELECT CASE WHEN changes()!=1 THEN RAISE(ABORT,'Audio registry entry missing') END;
END;
CREATE TRIGGER registry_sound_delete AFTER DELETE ON sounds BEGIN
 DELETE FROM asset_registry WHERE id=OLD.id AND kind='audio';
END;
CREATE TRIGGER registry_audio_delete_guard BEFORE DELETE ON asset_registry
 WHEN OLD.kind='audio' AND EXISTS(SELECT 1 FROM sounds WHERE id=OLD.id) BEGIN
 SELECT RAISE(ABORT,'Delete audio through its catalog record');
END;
CREATE TRIGGER registry_audio_kind_guard BEFORE UPDATE OF kind ON asset_registry
 WHEN OLD.kind!=NEW.kind BEGIN
 SELECT RAISE(ABORT,'Asset kind is immutable');
END;

CREATE TABLE image_contents (
 content_hash TEXT PRIMARY KEY CHECK(length(content_hash)=64 AND content_hash NOT GLOB '*[^0-9a-f]*')
);
CREATE TABLE image_analyses (
 content_hash TEXT NOT NULL REFERENCES image_contents(content_hash),
 analyzer_version TEXT NOT NULL CHECK(length(CAST(analyzer_version AS BLOB)) BETWEEN 1 AND 256),
 profile TEXT NOT NULL CHECK(json_valid(profile) AND length(CAST(profile AS BLOB))<=16384
   AND json_extract(profile,'$.analyzer_version')=analyzer_version),
 PRIMARY KEY(content_hash,analyzer_version)
);
CREATE TRIGGER image_analysis_immutable BEFORE UPDATE ON image_analyses BEGIN
 SELECT RAISE(ABORT,'Image analysis versions are immutable');
END;

CREATE TABLE image_assets (
 id TEXT PRIMARY KEY REFERENCES asset_registry(id) ON DELETE CASCADE,
 content_hash TEXT REFERENCES image_contents(content_hash),
 analyzer_version TEXT,
 technical_state TEXT NOT NULL CHECK(technical_state IN ('pending','ready','failed')),
 availability TEXT NOT NULL CHECK(availability IN ('available','offline','missing','inaccessible')),
 enrichment_state TEXT NOT NULL CHECK(enrichment_state IN ('disabled','queued','running','complete','failed','cancelled','stale')),
 annotation_revision INTEGER NOT NULL CHECK(annotation_revision BETWEEN 1 AND 4294967295),
 annotations TEXT NOT NULL CHECK(json_valid(annotations) AND json_type(annotations)='object'
   AND json_extract(annotations,'$.revision') IS annotation_revision),
 evidence TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(evidence) AND json_type(evidence)='array'),
 recipes TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(recipes) AND json_type(recipes)='array'),
 lineage TEXT CHECK(lineage IS NULL OR (json_valid(lineage) AND json_type(lineage)='object'
   AND json_extract(lineage,'$.parent_id')!=id)),
 CHECK(length(CAST(annotations AS BLOB))+length(CAST(evidence AS BLOB))+
       length(CAST(recipes AS BLOB))+COALESCE(length(CAST(lineage AS BLOB)),0)<=2097152),
 CHECK(technical_state!='ready' OR (content_hash IS NOT NULL AND analyzer_version IS NOT NULL)),
 FOREIGN KEY(content_hash,analyzer_version) REFERENCES image_analyses(content_hash,analyzer_version)
);
CREATE INDEX image_assets_version ON image_assets(content_hash,analyzer_version);
CREATE TABLE image_asset_versions (
 asset_id TEXT NOT NULL REFERENCES image_assets(id) ON DELETE CASCADE,
 content_hash TEXT NOT NULL REFERENCES image_contents(content_hash),
 first_seen INTEGER NOT NULL,
 PRIMARY KEY(asset_id,content_hash)
);
CREATE TRIGGER image_asset_insert_guard BEFORE INSERT ON image_assets BEGIN
 SELECT CASE WHEN NOT EXISTS(
   SELECT 1 FROM asset_registry r JOIN sources s ON s.id=r.source_id
   WHERE r.id=NEW.id AND r.kind IN ('image','element')
   AND ((r.kind='image' AND NEW.lineage IS NULL) OR (r.kind='element' AND NEW.lineage IS NOT NULL))
   AND (s.scope='folder' OR EXISTS(
     SELECT 1 FROM source_files f WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path))
 ) THEN RAISE(ABORT,'Invalid image identity, kind or selected-file membership') END;
END;
CREATE TRIGGER image_asset_update_guard BEFORE UPDATE ON image_assets BEGIN
 SELECT CASE WHEN NOT EXISTS(
   SELECT 1 FROM asset_registry r WHERE r.id=NEW.id
   AND ((r.kind='image' AND NEW.lineage IS NULL) OR (r.kind='element' AND NEW.lineage IS NOT NULL))
 ) THEN RAISE(ABORT,'Invalid image identity or lineage kind') END;
END;
CREATE TRIGGER image_dirty_insert AFTER INSERT ON image_assets BEGIN
 INSERT INTO source_catalog_state(source_id,dirty,last_error)
 SELECT source_id,1,'Image metadata requires mixed catalog support; audio-only snapshot is protected'
 FROM asset_registry WHERE id=NEW.id
 ON CONFLICT(source_id) DO UPDATE SET dirty=dirty+1,
 last_error='Image metadata requires mixed catalog support; audio-only snapshot is protected';
END;
CREATE TRIGGER image_dirty_update AFTER UPDATE ON image_assets BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1,
 last_error='Image metadata requires mixed catalog support; audio-only snapshot is protected'
 WHERE source_id=(SELECT source_id FROM asset_registry WHERE id=NEW.id);
END;
CREATE TRIGGER image_dirty_delete BEFORE DELETE ON image_assets BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1
 WHERE source_id=(SELECT source_id FROM asset_registry WHERE id=OLD.id);
END;
