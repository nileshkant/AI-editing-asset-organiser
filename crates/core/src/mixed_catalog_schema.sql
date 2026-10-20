-- v15 (SS-048): mixed portable and folder catalogs replace the SS-041 guard.
-- Image writes still mark a folder snapshot dirty, but no longer record the
-- "requires mixed catalog support" error that blocked audio-only snapshots.
DROP TRIGGER image_dirty_insert;
DROP TRIGGER image_dirty_update;
DROP TRIGGER image_import_protect_snapshot;
CREATE TRIGGER image_dirty_insert AFTER INSERT ON image_assets BEGIN
 INSERT INTO source_catalog_state(source_id,dirty)
 SELECT source_id,1 FROM asset_registry WHERE id=NEW.id
 ON CONFLICT(source_id) DO UPDATE SET dirty=dirty+1;
END;
CREATE TRIGGER image_dirty_update AFTER UPDATE ON image_assets BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1
 WHERE source_id=(SELECT source_id FROM asset_registry WHERE id=NEW.id);
END;
CREATE TRIGGER image_import_protect_snapshot AFTER INSERT ON image_import_jobs BEGIN
 INSERT INTO source_catalog_state(source_id,dirty) VALUES(NEW.source_id,1)
 ON CONFLICT(source_id) DO UPDATE SET dirty=dirty+1;
END;
UPDATE source_catalog_state SET last_error='' WHERE last_error IN (
 'Image metadata requires mixed catalog support; audio-only snapshot is protected',
 'Image import requires mixed catalog support; audio-only snapshot is protected');
