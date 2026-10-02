-- Derived metadata for search. Full profiles and source sidecars remain authoritative.
CREATE TABLE search_profiles (
 content_hash TEXT NOT NULL, analyzer TEXT NOT NULL, profile TEXT NOT NULL,
 PRIMARY KEY(content_hash,analyzer),
 FOREIGN KEY(content_hash,analyzer) REFERENCES analyses(content_hash,analyzer)
 ON DELETE CASCADE ON UPDATE CASCADE
);
INSERT INTO search_profiles SELECT content_hash,analyzer,
 CASE WHEN json_valid(profile) THEN json_set(profile,'$.waveform',json('[]')) ELSE profile END FROM analyses;
CREATE TRIGGER analyses_search_insert AFTER INSERT ON analyses BEGIN
 INSERT INTO search_profiles VALUES(NEW.content_hash,NEW.analyzer,
 CASE WHEN json_valid(NEW.profile) THEN json_set(NEW.profile,'$.waveform',json('[]')) ELSE NEW.profile END)
 ON CONFLICT(content_hash,analyzer) DO UPDATE SET profile=excluded.profile;
END;
CREATE TRIGGER analyses_search_update AFTER UPDATE OF profile,content_hash,analyzer ON analyses BEGIN
 INSERT INTO search_profiles VALUES(NEW.content_hash,NEW.analyzer,
 CASE WHEN json_valid(NEW.profile) THEN json_set(NEW.profile,'$.waveform',json('[]')) ELSE NEW.profile END)
 ON CONFLICT(content_hash,analyzer) DO UPDATE SET profile=excluded.profile;
END;
