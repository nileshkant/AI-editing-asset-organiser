# Recovery and resource settings

Settings → Audio and resources selects the system-default or a named output.
Changing output stops playback; choose Play again. An unavailable device reports
an error, never inaudible success. Device names are persisted, not hardware IDs;
duplicate OS labels share a choice. Physical switching, unplug/recovery and
latency still require installed audio qualification.

Imports run one job at a time with one decoder thread. Pause queued imports
persists across restart and takes effect after the current job. Cancel imports
before restoring a backup; resume or rescan deliberately afterward.

## Database backup and restore

Back up database creates a consistent SQLite `VACUUM INTO` snapshot of committed
WAL state under the catalog lock. It includes source references, annotations,
profiles, clip revision history, jobs, saved searches and preferences. It excludes
original media, source-folder JSON files, waveform caches, materialized exports
and temporary MCP credentials. Backup is staged/synced and published without
replacement; existing files and user recordings are never overwritten.

Restore requires explicit confirmation and native file selection, matching
schema/version, integrity and foreign-key checks. Additional schema objects are
rejected. Backups from incompatible schema versions/layouts require a matching
app or portable JSON import. Active imports/exports prevent restore. Playback
and MCP stop. A new rollback SQLite snapshot is saved in the app data directory
before restoration; it can be selected for the same restore procedure.

The live connection is restored using SQLite's backup API. Incomplete jobs from
the selected backup are cancelled. Restart the app after restoration to refresh
all views and pair MCP clients again. Rescan incomplete sources. Saved source
catalog revisions remain intact: a newer source snapshot causes a visible
conflict instead of silently replacing its annotations. Keep both copies and
review metadata before choosing an explicit catalog re-import or rebuild. Source
folder rebuild saves its previous JSON first. Referenced source media is intact.

## Cache and diagnostics

Clear waveform cache requires confirmation, serializes against waveform builds
and removes only generated hash-named `.sswf` regular files. Waveforms rebuild
on demand. It does not reset measured profiles, annotations or original media;
playback decodes directly and does not depend on this disk cache.

Export redacted diagnostics writes an allowlisted JSON report of versions,
platform/architecture, engine availability and counts. It contains no filenames,
paths, recordings, titles, tags, comments, logs or credentials. Nothing uploads
automatically. The user can attach that report to feedback deliberately.

No recognition model ships in the baseline. AI consent/provider controls belong
to the optional provider/model tickets before recognition can be enabled.

## Additive image storage (schema v10)

SS-041 adds internal image metadata without enabling image UI/import. Full
SQLite backups include those records and restore validates their typed fields,
registry membership, version history and lineage before changing live data.
Migration creates a pre-v10 snapshot for v5–v9 databases; restore that snapshot
with the matching old binary for downgrade, retaining the newer database
separately. Never edit `user_version`. Mixed-library portable JSON and image
source snapshots are supported from SS-048 (schema v15); see
[Mixed catalogs](MIXED_CATALOGS.md). See [Image storage](IMAGE_STORAGE.md)
for migration failure behavior, naming and compatibility limits.

## Restore preview and mixed catalogs (SS-048, schema v15)

Choosing **Restore database** now opens a preview first. The backup is fully
validated without changing anything, and its counts (sources, sounds, images,
clips, tag aliases, folder catalogs) are shown. **Restore this backup** applies
exactly the previewed file: the preview is bound to a BLAKE3 digest, so a file
edited after preview is refused before any rollback copy is written.
**Cancel restore** discards the preview. A backup written by a newer app
reports that it needs the newer version. After restore, folder catalogs are
marked for review and a newer folder snapshot is never overwritten silently.
