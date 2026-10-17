# Scoped image/mixed imports — SS-043

Production image capability remains disabled until SS-048. This ticket adds an
explicit `image-development` Cargo feature to the desktop package for disposable
internal libraries. It does not enable image MCP, uploads, models, a gallery,
previews, export, or filesystem watching. Technically ready image metadata is
browsable under Imports without any AI configuration. The source-scoped metadata
list is independent of expiring job history and remains available for retained
image records when new development imports are disabled.

## Development workflow

Build the trusted helper with `node scripts/cargo.mjs build -p soundshelf-core
--bin creativeshelf-image-worker --locked`, then run the desktop with the
`image-development` feature and a disposable `SOUNDSHELF_DATA_DIR`. Keep the
normal build's feature disabled. A missing helper disables new image imports;
existing image-job metadata and cancellation remain available for recovery.

The helper is selected only from `resources/images/creativeshelf-image-worker`
(with `.exe` on Windows), or the sibling build output in debug builds. There is
no PATH/source-folder executable discovery. `CREATIVESHELF_IMAGE_DEVELOPMENT=1
npm run desktop:local` also builds/stages the helper and complete retained image
notices. Signed production packaging rejects that flag until mixed persistence
ships. This is a packaging path, not installed/signing qualification.

Choose native files/folders or drop OS-approved paths. Review the media mode,
counts, exact files or recursive folders, and reference-in-place behavior before
confirming. Preview alone creates no sources. Header counts are estimates; full
analysis may reject corrupt files. An unknown folder's existing
`.creativeshelf/catalog.json` blocks the development mixed flow; use the existing
audio restore/import flow or a disposable folder. It is never overwritten.

Native grants are exact, expiring, and typed as files/folders before preview.
A file changed to a directory cannot become recursive. Duplicate/ancestor input
selections collapse; existing recursive sources can be reused with a selected
subfolder prefix without scanning siblings. Overlapping files-only source scopes
fail explicitly. Preview IDs are opaque, single-use, bounded, expire after five
minutes, and are discarded/cancelled when closed or superseded.

## Durable execution and publication

SQLite v11 adds separate image/mixed jobs, items, retention budget, machine-local
source bindings and content-bound warning records. Genuine v10 migrations back
up and upgrade transactionally. Existing audio `Job`/`Progress` and MCP payloads
are unchanged. Cross-lane SQL triggers serialize jobs for the same source;
different sources and playback remain independent.

The worker claims a durable lease, performs discovery in batches of at most 32,
then processes one file per scheduling quantum and returns the source to the end
of the queue. Each file hashes outside the catalog mutex. Images use SS-042's
full supervised decoder; audio uses existing media analysis/cache/publication.
Root/selected-folder identities, containment, source generation, current bytes,
lease ownership and cancellation fence publication. Asset facts and per-item
progress commit in the same SQLite transaction. No worker error/stderr or
arbitrary OS details become a per-file IPC error: errors are named categories.

Ready assets publish independently of failed siblings. Identical content shares
versioned technical facts and content-bound warnings; filename mismatch is
recomputed per path. Reimport preserves stable identity and user annotations.
Version changes stale content-derived evidence/recipes. Failed/removed originals
retain explicit technical/availability state and retryable import records.

Cancellation persists first and then the watchdog observes it. Shutdown
checkpoints the owned job; expired crash leases rediscover and rehash bytes on
recovery, retaining committed assets. Lease expiry is 180 seconds, not proof that
a different process has stopped. Database fences reject an old owner regardless.
Retries create a fresh job while preserving the original disclosed selector and
machine binding. Offline/replaced roots require recovery/reselection; a new
filesystem at the old path cannot silently impersonate the original source.

Only complete successful discovery reconciles missing members in the selected
recursive prefixes. Exact selections never reconcile siblings. Partial, denied,
symlink-skipped, depth-limited, or cross-filesystem walks preserve previous
memberships. Nested mounts are skipped conservatively; select a mounted folder
directly when intended. Filesystem identity uses volume UUID where available on
macOS, filesystem ID/device fallback on Unix, and volume serial/file ID on
Windows, plus creation time where available. Runtime-device fallbacks can require
reselection after a remount; all filesystems/reboot scenarios are not qualified.

## Bounds and UI contract

- 256 input paths, 1 MiB combined selection JSON, four simultaneous preview
  preparations, eight retained previews, five-minute grant/preview lifetime.
- 10K source headers / 16 MiB logical source context before preview traversal;
  30-second discovery checkpoints, depth 64, 100K discovered files, eight open
  traversal directories. Filesystem system calls may stall on a broken mount;
  deadlines bound work between calls, not kernel response time.
- 32 active jobs, 1000 retained jobs, 100K retained item rows, 128 MiB logical job
  metadata. This reserves 1024 bytes plus path bytes per item and 40960 plus
  selector/stamp JSON bytes per job; it is not an exact SQLite/RSS bound.
- Old terminal history is evicted in bounded chunks. `history_truncated` discloses
  partial history; asset records and user annotations are never job-history data.
- Images retain SS-042's 64 MiB input / 16,777,216 pixels / 512 MiB worker requested
  heap / 30-second child limit. Mixed-lane audio hashes at most 512 MiB, streaming
  64 KiB buffers; a 120-second quantum watchdog cancels audio analysis. Legacy
  audio limits remain unchanged. Native worker termination cannot fix an OS-stuck
  parent filesystem call.
- IPC pages contain at most 50 records / 2 MiB. The job's `current` is a 4 KiB
  display hint; full relative names remain in paged item records.
- Source generation/sequence counters are decimal strings. Events signal an
  authoritative DB refresh; stale job/generation/sequence results are fenced.
  Events throttle at 100 ms; terminal events bypass throttling; one-second polling
  recovers dropped events. Database restore resets UI state and invalidates
  pending previews rather than reusing a stale sequence from a different DB.

Full SQLite backups retain every new durable field; validation precedes rollback
or live mutation. Restore requires imports idle/cancelled, cancels restored active
jobs and resets running items. Existing audio sidecar/portable formats remain
guarded for any image-bound source, even after cancellation/history eviction.
Job history, machine grants and local retry state are deliberately not portable
JSON. SS-048 owns the mixed metadata formats; do not bypass these guards.

## Verification and qualification limits

See [SS-043 review](reviews/SS-043.md). Real helper/FFmpeg fixtures establish decode
and publication behavior; renderer fixtures establish IPC/UI/accessibility only.
Loopback playback is not physical-device qualification. Native OS picker/drop
adapters compile in platform CI and share scoped core validation; real installed
picker/drop, removable-drive ACLs, mount/reboot identity and signed installed
packages still require platform qualification. No AI quality claim is made.
