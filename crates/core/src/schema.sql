CREATE TABLE sources (
 id TEXT PRIMARY KEY, name TEXT NOT NULL, root TEXT NOT NULL UNIQUE,
 generation INTEGER NOT NULL DEFAULT 0, available INTEGER NOT NULL DEFAULT 1 CHECK(available IN(0,1))
);
CREATE TABLE sounds (
 id TEXT PRIMARY KEY, source_id TEXT NOT NULL REFERENCES sources(id),
 relative_path TEXT NOT NULL, title TEXT NOT NULL, content_hash TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN('pending','ready','failed','missing')),
 UNIQUE(source_id,relative_path)
);
CREATE INDEX sounds_hash ON sounds(content_hash);
CREATE INDEX sounds_state ON sounds(source_id,status);
CREATE TABLE analyses (
 content_hash TEXT NOT NULL, analyzer TEXT NOT NULL, profile TEXT NOT NULL,
 PRIMARY KEY(content_hash,analyzer)
);
CREATE TABLE annotations (
 sound_id TEXT PRIMARY KEY REFERENCES sounds(id), tags TEXT NOT NULL DEFAULT '[]',
 comment TEXT NOT NULL DEFAULT '', favorite INTEGER NOT NULL DEFAULT 0 CHECK(favorite IN(0,1))
);
