CREATE TABLE source_catalog_state (
 source_id TEXT PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
 revision TEXT, dirty INTEGER NOT NULL DEFAULT 1,
 last_error TEXT NOT NULL DEFAULT ''
);
INSERT INTO source_catalog_state(source_id,dirty) SELECT id,1 FROM sources WHERE scope='folder';
CREATE TRIGGER catalog_sound_insert AFTER INSERT ON sounds BEGIN
 INSERT INTO source_catalog_state(source_id,dirty) VALUES(NEW.source_id,1) ON CONFLICT(source_id) DO UPDATE SET dirty=dirty+1;
END;
CREATE TRIGGER catalog_sound_update AFTER UPDATE ON sounds BEGIN
 INSERT INTO source_catalog_state(source_id,dirty) VALUES(NEW.source_id,1) ON CONFLICT(source_id) DO UPDATE SET dirty=dirty+1;
END;
CREATE TRIGGER catalog_annotation_insert AFTER INSERT ON annotations BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1 WHERE source_id=(SELECT source_id FROM sounds WHERE id=NEW.sound_id);
END;
CREATE TRIGGER catalog_annotation_update AFTER UPDATE ON annotations BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1 WHERE source_id=(SELECT source_id FROM sounds WHERE id=NEW.sound_id);
END;
CREATE TRIGGER catalog_clip_insert AFTER INSERT ON clips BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1 WHERE source_id=(SELECT source_id FROM sounds WHERE id=NEW.sound_id);
END;
CREATE TRIGGER catalog_clip_update AFTER UPDATE ON clips BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1 WHERE source_id=(SELECT source_id FROM sounds WHERE id=NEW.sound_id);
END;
CREATE TRIGGER catalog_clip_delete AFTER DELETE ON clips BEGIN
 UPDATE source_catalog_state SET dirty=dirty+1 WHERE source_id=(SELECT source_id FROM sounds WHERE id=OLD.sound_id);
END;
