# Portable catalog and legacy migration

Settings → Data exports JSON or a readable Markdown companion and imports JSON metadata. No media is copied. Use a new output filename: exports deliberately refuse overwrites. Output is staged and published atomically; destinations must support hard links (such as APFS, NTFS or ext4). An unsupported destination reports an error and leaves existing files intact.

## Portable JSON v1

The envelope uses `schema: "soundshelf-catalog/v1"`. Sources carry stable IDs, display names, folder/files scope and normalized relative file members. Sounds retain identity, content digest, measured profile, tags, comments and favorite state. Current clip recipes/revisions, saved searches and legacy provenance are retained. This is a metadata transfer, not a full database backup: old clip revision history, jobs, caches, settings, credentials and machine file grants are excluded. Markdown is a readable companion, not a restore format.

Source roots and application data paths are not exported. User-authored comments, tags, names and saved query text are included; review their content before sharing a catalog.

Choose Import catalog, inspect the counts, map sources to local folders with the native chooser and confirm. Stored analysis is reused without reading or decoding audio, even when files are absent. Unmapped sources remain offline with their metadata preserved; use Sources → Relink folder to restore them after their folders are available. Relink verifies content. Import mapping trusts the explicitly selected catalog's stored analysis; use Rescan if the mapped files may have changed.

Imports append atomically and never replace an existing source or sound. Identity collisions, conflicting cached profiles, overlapping roots, invalid scope membership, traversal, unsupported schemas and malformed profiles fail without catalog changes. Source-relative paths use `/`, retain Unicode and reject absolute paths, drive prefixes, `.`/`..` and backslashes. Root mappings must use existing absolute folders. JSON input and output are limited to 64 MiB; import supports 100,000 sounds and 10,000 sources.

## Legacy v3

Import legacy catalog explicitly opts into `portable-sound-effects-catalog/v3`, the format produced by the original builder. The native picker creates an immutable preview; disk edits cannot change what is approved. Every legacy source requires a user-chosen root. Embedded local/configured paths and AI instructions are ignored.

The declared sound count, duplicate identities/paths and source references are validated. Media is hashed with BLAKE3 in bounded chunks outside the catalog lock. Legacy path-derived IDs are not treated as content hashes; an optional `content_hash` must match the actual file. Missing media, traversal or symlink escape abort before any metadata is inserted. The import creates explicit file scopes containing only catalog members, so background analysis never discovers siblings.

Descriptions are retained separately as `legacy_filename_inference`, not as user annotations or measured facts. Legacy evidence records coverage as `first_up_to_20_seconds_12khz_mono`, with any reported coverage limited to 20 seconds. Old loudness, pitch proxies, waveform and channel metrics never become a native measured profile. Imported sounds start pending, keeping user annotations; full-file analysis uses the existing durable worker and analysis cache. Queue failures are reported after successful metadata import with a Settings Rescan recovery action. Review legacy evidence in Data or export JSON to retain all records.

## Database recovery

Database schema v6 adds `legacy_evidence`, with an indexed sound identity and foreign-key deletion cleanup. Upgrades run in a transaction and create a consistent `VACUUM INTO` pre-upgrade backup first. A v5 upgrade creates `*.pre-v7-<uuid>.sqlite`; older databases preserve the existing `pre-v5` naming convention. Before running an older binary, close CreativeShelf and restore the pre-upgrade backup to the data directory. Never downgrade by editing `user_version`. Schema initialization and failed upgrades are covered by disposable fixtures; tests never open the user's live database.

## Source-folder catalogs (SS-029)

Folder import now checks `.creativeshelf/catalog.json` first. A compatible
metadata catalog supplies paths, measured profiles, user annotations and clip
revision history. Unchanged listed files use only existence/stat checks: no
directory traversal, content hashing or decoding. Rescan discovers unlisted
files and hashes content; ordinary reimport checks listed members only.

A size/timestamp change analyzes only affected members. A same-size/same-timestamp
replacement requires explicit Rescan; cheap stat checks do not prove byte equality.
The waveform profile is portable; detailed waveform tiles remain an app cache
and rebuild on demand when needed for audition. Original audio is never copied.

Confirmed missing members are pruned from the snapshot, while database identity,
annotations and stale recipes remain for recovery. Offline drives, access errors,
partial scans and failed/cancelled work preserve the prior snapshot.

Existing library annotations/recipes win on explicit reimport. Concurrent
snapshots use an OS file lock and revision comparison. Failed writes preserve the
previous JSON and persist a retry warning. Settings > Folder metadata catalogs
can save again or explicitly rebuild; rebuild keeps the previous catalog as a
backup before scanning again. Recover a folder catalog handles a malformed
sidecar before the folder has been imported. Read-only sources remain usable;
their metadata-save warning is visible and they cannot carry an updated snapshot.

Schema v7 adds persistent snapshot revision/dirty/error state and mutation
triggers. A pre-v7 SQLite backup is made before migration. Sidecars include one
folder only and omit credentials, absolute roots, jobs and global saved searches.
Changed local annotations/recipes save in the background when no incomplete or
failed source jobs remain; failures stay visible until retried. AI event labels
will require a future versioned extension under SS-016/017.

Database backup/restore and preference migration use SQLite v8; see [Recovery](RECOVERY.md). The portable JSON and source-folder schema versions remain unchanged.
