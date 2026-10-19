-- Additive v13: creator-owned image tag aliases and reversible annotation history (SS-046).
-- Aliases are library-wide user data; history is local, bounded undo state.
CREATE TABLE image_tag_aliases (
 alias_key TEXT PRIMARY KEY CHECK(length(CAST(alias_key AS BLOB)) BETWEEN 1 AND 256),
 alias TEXT NOT NULL CHECK(length(CAST(alias AS BLOB)) BETWEEN 1 AND 256),
 canonical TEXT NOT NULL CHECK(length(CAST(canonical AS BLOB)) BETWEEN 1 AND 256),
 created_at INTEGER NOT NULL
);
CREATE TABLE image_annotation_actions (
 seq INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT NOT NULL UNIQUE CHECK(length(id)=36),
 kind TEXT NOT NULL CHECK(kind IN ('edit','bulk','rename')),
 label TEXT NOT NULL CHECK(length(CAST(label AS BLOB)) BETWEEN 1 AND 1024),
 state TEXT NOT NULL CHECK(state IN ('applied','undone')),
 created_at INTEGER NOT NULL,
 items INTEGER NOT NULL CHECK(items BETWEEN 1 AND 10000),
 entries TEXT NOT NULL CHECK(json_valid(entries) AND json_type(entries)='array'
   AND length(CAST(entries AS BLOB))<=8388608),
 alias TEXT CHECK(alias IS NULL OR (json_valid(alias) AND json_type(alias)='object'))
);
