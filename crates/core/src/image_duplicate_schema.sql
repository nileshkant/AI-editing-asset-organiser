-- Additive v16 (SS-062): creator decisions on exact-duplicate stacks. Membership
-- is derived on read from verified content digests; only explicit choices persist.
-- Grouping never deletes files or merges per-file annotations.
CREATE TABLE image_duplicate_groups (
 content_hash TEXT PRIMARY KEY REFERENCES image_contents(content_hash),
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 4294967295),
 preferred_id TEXT REFERENCES asset_registry(id) ON DELETE SET NULL,
 dismissed INTEGER NOT NULL DEFAULT 0 CHECK(dismissed IN (0,1)),
 updated_at INTEGER NOT NULL
);
-- 'separated': the creator keeps this copy out of the stack.
-- 'reviewed': the copy was present when the stack was dismissed; a copy that
-- arrives later resurfaces the stack.
CREATE TABLE image_duplicate_marks (
 content_hash TEXT NOT NULL REFERENCES image_duplicate_groups(content_hash) ON DELETE CASCADE,
 asset_id TEXT NOT NULL REFERENCES asset_registry(id) ON DELETE CASCADE,
 mark TEXT NOT NULL CHECK(mark IN ('separated','reviewed')),
 PRIMARY KEY(content_hash,asset_id)
);
CREATE INDEX image_duplicate_marks_asset ON image_duplicate_marks(asset_id);
CREATE TRIGGER image_duplicate_preferred_kind BEFORE INSERT ON image_duplicate_groups
 WHEN NEW.preferred_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM asset_registry WHERE id=NEW.preferred_id AND kind='image') BEGIN
 SELECT RAISE(ABORT,'Duplicate stacks contain images only');
END;
CREATE TRIGGER image_duplicate_preferred_kind_update BEFORE UPDATE OF preferred_id ON image_duplicate_groups
 WHEN NEW.preferred_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM asset_registry WHERE id=NEW.preferred_id AND kind='image') BEGIN
 SELECT RAISE(ABORT,'Duplicate stacks contain images only');
END;
CREATE TRIGGER image_duplicate_mark_kind BEFORE INSERT ON image_duplicate_marks
 WHEN NOT EXISTS(SELECT 1 FROM asset_registry WHERE id=NEW.asset_id AND kind='image') BEGIN
 SELECT RAISE(ABORT,'Duplicate stacks contain images only');
END;
-- Every decision write inserts or bumps the group row, so these two triggers mark
-- each folder holding a copy dirty for its next snapshot.
CREATE TRIGGER image_duplicate_dirty_insert AFTER INSERT ON image_duplicate_groups BEGIN
 INSERT INTO source_catalog_state(source_id,dirty)
 SELECT DISTINCT r.source_id,1 FROM image_assets i JOIN asset_registry r ON r.id=i.id
 WHERE i.content_hash=NEW.content_hash
 ON CONFLICT(source_id) DO UPDATE SET dirty=dirty+1;
END;
CREATE TRIGGER image_duplicate_dirty_update AFTER UPDATE ON image_duplicate_groups BEGIN
 INSERT INTO source_catalog_state(source_id,dirty)
 SELECT DISTINCT r.source_id,1 FROM image_assets i JOIN asset_registry r ON r.id=i.id
 WHERE i.content_hash=NEW.content_hash
 ON CONFLICT(source_id) DO UPDATE SET dirty=dirty+1;
END;
