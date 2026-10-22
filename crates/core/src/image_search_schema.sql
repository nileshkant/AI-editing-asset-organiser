-- Additive v18 (SS-058): exact image search index and saved image searches.
-- The index is derived data. Writers only queue identities in image_search_dirty
-- (same transaction as the change); image_search::flush rebuilds those documents
-- in Rust before any search reads, so results always reflect committed edits.
CREATE TABLE image_search_docs (
 doc INTEGER PRIMARY KEY AUTOINCREMENT,
 asset_id TEXT NOT NULL UNIQUE,
 kind TEXT NOT NULL,
 favorite INTEGER NOT NULL,
 rating INTEGER,
 width INTEGER,
 height INTEGER,
 orientation TEXT,
 alpha INTEGER,
 transparency TEXT,
 format TEXT,
 technical TEXT NOT NULL,
 availability TEXT NOT NULL,
 enrichment TEXT NOT NULL,
 suggestions INTEGER NOT NULL,
 origins TEXT NOT NULL,
 added_at INTEGER
);
CREATE INDEX image_search_docs_rating ON image_search_docs(rating);
CREATE INDEX image_search_docs_favorite ON image_search_docs(favorite) WHERE favorite=1;
-- Confirmed and suggested tags by their case-folded NFC key (exact tag filters, facets).
CREATE TABLE image_search_tags (
 doc INTEGER NOT NULL REFERENCES image_search_docs(doc) ON DELETE CASCADE,
 key TEXT NOT NULL,
 tag TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('confirmed','suggested')),
 PRIMARY KEY(key,state,doc)
) WITHOUT ROWID;
CREATE INDEX image_search_tags_doc ON image_search_tags(doc);
-- Token search. Text is NFC-folded with CJK characters spaced so each is a token;
-- U+E000 separates tags so a phrase never spans two tags.
CREATE VIRTUAL TABLE image_search_fts USING fts5(
 name, tags, suggested, caption, notes,
 tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TABLE image_search_dirty (asset_id TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO image_search_dirty SELECT id FROM image_assets;
CREATE TRIGGER image_search_asset_insert AFTER INSERT ON image_assets BEGIN
 INSERT OR IGNORE INTO image_search_dirty VALUES(NEW.id);
END;
CREATE TRIGGER image_search_asset_update AFTER UPDATE ON image_assets BEGIN
 INSERT OR IGNORE INTO image_search_dirty VALUES(NEW.id);
END;
CREATE TRIGGER image_search_asset_delete AFTER DELETE ON image_assets BEGIN
 INSERT OR IGNORE INTO image_search_dirty VALUES(OLD.id);
END;
CREATE TRIGGER image_search_path_update AFTER UPDATE OF source_id,relative_path ON asset_registry
 WHEN NEW.kind!='audio' BEGIN
 INSERT OR IGNORE INTO image_search_dirty VALUES(NEW.id);
END;
CREATE TRIGGER image_search_version_insert AFTER INSERT ON image_asset_versions BEGIN
 INSERT OR IGNORE INTO image_search_dirty VALUES(NEW.asset_id);
END;
CREATE TRIGGER image_search_version_delete AFTER DELETE ON image_asset_versions BEGIN
 INSERT OR IGNORE INTO image_search_dirty VALUES(OLD.asset_id);
END;

-- Library-wide named image queries (portable catalog v2; never in source snapshots).
CREATE TABLE saved_image_searches (
 id TEXT PRIMARY KEY CHECK(length(id)=36),
 name TEXT NOT NULL CHECK(length(CAST(name AS BLOB)) BETWEEN 1 AND 256),
 name_key TEXT NOT NULL UNIQUE,
 query TEXT NOT NULL CHECK(json_valid(query) AND json_type(query)='object'
   AND length(CAST(query AS BLOB))<=16384),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
