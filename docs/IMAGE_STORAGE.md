# Image storage foundation — SS-041

SQLite schema v10 adds image persistence to the v9 audio catalog. The image
capability is disabled (`IMAGE_CAPABILITY_ENABLED = false`): there is no new
Tauri command, MCP tool, picker, import worker, decoder, or gallery. These are
internal shared-core operations for later tickets, not permission grants.

## Identity and tables

`asset_registry` is the cross-kind identity/membership authority. An identity
and a `(source_id, relative_path)` pair are unique across audio, images and
reserved elements. Existing audio IDs, paths, tables and wire payloads remain
unchanged; backfill and audio insert/update/delete triggers maintain registry
membership. New images use random canonical UUIDs, never content-derived IDs.
Deleting a source cascades its metadata; no original media is deleted.

`image_assets` stores current content/profile selection, independent technical,
availability and enrichment states, optimistic annotation revision, annotations,
evidence, recipes and nullable reserved lineage. `image_asset_versions` retains
seen content digests per identity. `image_contents` stores known BLAKE3 digests;
`image_analyses` is immutable and keyed by digest plus analyzer version. Identical
files share technical facts while retaining separate annotations and identities.
Foreign keys and indexes cover source/kind lookup, identity and content selection.
Profile facts are validated with the SS-040 contract before publication.

Lineage fields are reserved for later element workflows. An absent parent is
allowed by the contract, so it deliberately has no parent foreign key. Known
audio parents fail typed reads; restore also rejects cycles and excessive depth.
No public element-creation or lineage-edit operation is enabled here.

## Core operations and conflict semantics

- `register_image` accepts a current available `Source`, normalized relative path
  and optional verified digest. A file-scoped source must contain that exact
  member. Source generation and membership are checked in an immediate write
  transaction. Registration performs no file reads or hashing: future workers
  must establish containment, permission and verified bytes before calling it.
- Re-registering the same path and digest preserves the record. Replacing its
  digest preserves identity, annotations and their revision, retains history,
  clears the selected technical profile, and marks content-bound evidence and
  recipes stale. A known digest cannot be erased by passing null.
- `publish_image` and `publish_cached_image` require the expected current digest,
  source generation and available membership. Conflicting facts under an existing
  digest/analyzer pair fail atomically; change the analyzer version when facts
  change. Hashing/decoding must run outside the catalog lock.
- `set_image_annotations` requires both expected content version and current
  annotation revision. Here null means **expect unknown digest**, not wildcard.
  Success increments revision once; stale versions/revisions and revision
  exhaustion fail without edits. Offline local annotation edits remain allowed.
- `delete_image` requires current source generation, digest and annotation
  revision. It deletes metadata/history only, retaining shared analysis caches.
- `image` is a trusted internal full-record read; an optional digest checks the
  current version when supplied. `image_in_sources` checks authorized source
  IDs before exposing existence/kind details. Adapters must use scoped reads and
  still enforce their own persisted image capability/read/edit grants.
- `list_images` accepts at most 128 source IDs and 1–100 rows per page. It uses
  stable ascending UUID keyset cursors, returns header/dimensions/revision only,
  and caps serialized pages at 2 MiB without skipping the first deferred row.
  Empty scopes return no records. It is not a ranked or filtered search API.

Full records retain the SS-040 limits and receive a 2 MiB serialized check.
The catalog currently admits at most 100,000 image records; restore applies the
same bound. Database writes and reads are metadata-only and bounded by contract
limits. There is no decoder, model, export job or cancellation state machine in
this ticket. Those workers need their own resource/cancellation tests later.

## Paths, movement and older writers

Membership remains source ID plus exact normalized relative path. Different
paths with identical bytes are distinct; a rename is a new identity until an
explicit reconciliation workflow is implemented. Existing source APIs reject
overlapping roots; selecting multiple files does not grant sibling access.
The legacy audio-only root relink and file-to-folder conversion paths reject
image-bearing sources until SS-047 can verify image movement correctly.

Until SS-048, audio-only portable export of a mixed library fails visibly.
Source-folder save/rebuild/reimport rejects image-bearing sources and preserves
existing snapshots. Audio-only sources in the same database still save through
a source-scoped export. Final save and rebuild gates hold an immediate database
write transaction, covering registration by another connection between staging
and publication. Image mutations keep the source dirty with a visible mixed
catalog support warning. No image is silently omitted from a replacement JSON.
Existing audio MCP exports remain source-scoped audio exports under existing
permissions. They do not advertise image support or export an entire database.

## Upgrade and recovery

Persistent older catalogs get a consistent, separately synced `VACUUM INTO`
backup before schema changes. v5–v9 use
`<database-name>.pre-v10-<uuid>.sqlite`; pre-v5 retains the pre-v5 naming rule.
Backups use create-new ownership (0600 on Unix), never replace another file,
and backup failure cancels the upgrade. Failed owned backup files are removed.

Schema creation/backfill, foreign-key checking and `user_version` publication
share one immediate transaction. Concurrent upgrades recheck the version under
the writer lock; a database changed during backup is rejected for retry rather
than upgraded against a stale backup. Busy waits are bounded. Failed DDL or
SQLite-full errors roll back the schema and version; the old catalog and the
pre-upgrade backup remain readable with a compatible binary. Newer schema
versions fail rather than downgrade.

Full v10 SQLite backup/restore includes images, history and shared analyses.
Restore checks exact schema layout, SQLite integrity/foreign keys, registry
coverage, typed image records, selected-file membership, current version history
and lineage before creating a rollback snapshot or touching live data. Restore
requires the existing explicit UI confirmation and worker stop protocol.

To return to a v9 binary, close all app instances, retain the v10 database as a
separate recovery copy, and restore the pre-upgrade v9 snapshot with its matching
binary/data location. Never edit `user_version` or copy a live database without
its WAL. The pre-upgrade snapshot does not include later image edits. An older
binary cannot read a v10 backup; future mixed portable transfer is SS-048.

## SS-048 update

Schema v15 removes the guard errors above: mixed libraries export portable v2
and save folder v2 snapshots, while audio-only libraries still write v1. The
dirty triggers remain. Legacy audio-only root relink and file-to-folder
conversion still reject image sources, which use the SS-047 image actions. A
v14 binary cannot open v15; restore the `*.pre-v15-<uuid>.sqlite` backup to go
back. See [Mixed catalogs](MIXED_CATALOGS.md).
