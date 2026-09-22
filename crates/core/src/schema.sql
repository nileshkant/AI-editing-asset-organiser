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
CREATE TABLE jobs (
 id TEXT PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES sources(id),
 kind TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN('queued','running','complete','failed','cancelled')),
 status TEXT NOT NULL,
 lease_owner TEXT,
 lease_until INTEGER,
 completed INTEGER NOT NULL DEFAULT 0,
 total INTEGER NOT NULL DEFAULT 0,
 reused INTEGER NOT NULL DEFAULT 0,
 failed INTEGER NOT NULL DEFAULT 0,
 current TEXT NOT NULL DEFAULT '',
 errors TEXT NOT NULL DEFAULT '[]',
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 UNIQUE(source_id, kind)
);
CREATE TABLE saved_searches (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  query TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
