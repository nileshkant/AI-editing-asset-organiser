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
