# Mixed image catalogs, backup and restore (SS-048)

How image libraries survive export, reimport, relocation, folder snapshots,
database backups and older-app boundaries. Code: `crates/core/src/mixed_catalog.rs`,
`portable.rs`, `source_catalog.rs`, `maintenance.rs` and the schema script
`mixed_catalog_schema.sql`. Desktop adapters: `src-tauri/src/catalog_transfer.rs`,
`maintenance.rs` and `service.rs`. Production image capability stays gated
(`image-development`); this ticket removes the SS-041 persistence guards, not
the feature gate.

## Formats and versions

| Format | Audio-only library | Library with images or tag aliases |
| --- | --- | --- |
| Portable JSON | `soundshelf-catalog/v1`, unchanged | `soundshelf-catalog/v2` |
| Folder snapshot `.creativeshelf/catalog.json` | `creativeshelf-source/v1`, unchanged | `creativeshelf-source/v2` |
| SQLite | schema v16 | schema v16 |

- v2 is v1 plus two arrays: `images` and (portable only) `image_tag_aliases`.
  Every audio field keeps its v1 meaning and shape.
- v3 (SS-062) is v2 plus `image_duplicate_decisions`, written only when a library
  has exact-duplicate decisions. See [IMAGE_DUPLICATES.md](IMAGE_DUPLICATES.md).
- Writers choose the lowest version that can hold the data. An audio-only
  library keeps writing v1, so an older app keeps reading it.
- An older app reading v2 fails on the schema string with its existing
  "unsupported schema" error. It never parses v2 as v1 and drops images.
- A v1 document that contains `images` or aliases is rejected.
- A newer version of either family (`…/v3` and later) fails with "written by a
  newer CreativeShelf; update the app". The legacy-import fallback is never tried
  for a current-family document, so its own validation error is the one shown.
- Every struct uses `deny_unknown_fields`. An unknown required field or enum
  value anywhere, including inside an image record, fails the whole document.

### Image record

```json
{
  "asset": { "header": {…}, "profile": {…} | null, "annotations": {…},
             "evidence": [], "recipes": [], "lineage": null },
  "technical_warnings": ["assumed_srgb"],
  "versions": [{ "content_hash": "…64 hex…", "first_seen": 1792476930 }]
}
```

- `asset` is exactly the SS-040 `ImageAsset` contract and is validated by it.
  This covers the UUID, kind, path rules, state axes, the profile and its
  geometry, the 2 MiB record limit, evidence, recipes and lineage.
- `availability` is the stored per-member state (`available`, `missing`,
  `inaccessible`). `offline` is derived from the source when the record is read,
  and is never exported.
- `technical_warnings` are the content-bound warnings cached with the analysis.
  They let a warm import reuse the analysis without decoding. The per-file
  extension warning is recomputed, never stored.
- `versions` is the previous-version recovery history from
  `image_asset_versions`. It must contain the current content hash.
- Annotations carry `revision`, so imports keep revisions monotonic. Tags must
  already be in normalized display form (SS-046 rules).

### Durable versus local or rebuildable

Every SQLite column is classified in `mixed_catalog::persistence_inventory`. A
test fails when a column has no classification (see "Hooks for later tickets").

| Data | Portable JSON | Folder snapshot | DB backup |
| --- | --- | --- | --- |
| Asset IDs, kind, source-relative path | yes | yes | yes |
| Content hash and version history | yes | yes | yes |
| Technical profile and cached warnings | yes | yes | yes |
| Annotations (tags, notes, favorite, rating, caption, revision) | yes | yes | yes |
| Evidence and recipes (stale flags included); lineage | yes | yes | yes |
| Tag aliases | yes | no: library-wide, like saved searches | yes |
| Exact-duplicate decisions (SS-062) | yes (v3) | yes, this folder's copies only (v3) | yes |
| Annotation undo history (SS-046) | no: local undo | no | yes |
| File stats (SS-047), root identity, watch preference, root state | no: machine-local | no | yes |
| Import/check job history | no | no | yes |
| Thumbnails, preview cache, search projections, budgets | no: rebuilt | no | thumbnails no; projections yes |
| Absolute roots, grants, credentials, media bytes, model files | never | never | roots only (local file) |

On a new machine, stats are absent, so the first **Check files** rehashes each
member once. Watching starts off and must be turned on deliberately.

## Import

Portable import is a single transaction. Sources, sounds, clips, saved searches,
images, versions, analyses and aliases are all inserted, then the same typed
validation as a database restore runs over the combined library
(`validate_image_database`: registry coverage, lineage chains, version history,
alias and annotation tables). Any failure rolls everything back. Nothing existing
is overwritten:

- An identity collision (image ID equal to an existing image or audio ID, or a
  path already taken) fails.
- A cached analysis or warning set that differs for the same content and
  analyzer fails.
- An alias whose key exists with a different target fails. An identical alias
  is accepted.

Mapped sources come online; unmapped sources import offline under the app data
folder and are relinked later. Import reads no media: no walk, hash or decode.
Run **Check files** when mapped files may have changed, as with audio.

## Folder snapshots

`source_catalog::save` writes v2 when the folder has image records. Members are
images with a known hash that are not confirmed missing; the rest stay local
until verified, matching how missing audio is pruned.

- Each member has a size and modification-time fingerprint, as for audio.
- If a member's stat differs from the previous snapshot of the same version, the
  save is refused unless that change was verified. Audio proves verification
  through its scan. Images prove it through the SS-047 `asset_file_stats` row for
  that exact content hash, size and mtime, written only after a hash.
- A save never runs while an image import or check is queued or running for the
  folder. The background saver skips the folder, and an explicit save reports a
  retry. A mid-import snapshot would publish half-registered members.
- Revision locking, the OS file lock, staged atomic rename, dirty serials and
  read-only handling are unchanged. An image registered by another connection
  after staging keeps the folder dirty (tested).

### Warm reimport (Add folder on a folder with a v2 snapshot)

`import_folder` stats each listed member. It does not walk the folder, hash
files or decode images:

- **Stat unchanged:** the member keeps its verified facts (technical state,
  profile) and is available.
- **Stat changed:** the member becomes `pending` and keeps its cached analysis
  linked. The image lane then checks only those members
  (`image_sources::enqueue_snapshot_checks`): a path-list job for up to 256
  members, or one listed check above that. If the bytes are unchanged, the cached
  analysis is reused without decoding. If they changed, evidence and recipes
  go stale and annotations are kept (SS-041 rules).
- **Not found:** `missing`, keeping its facts. It is never deleted.

A same-size, same-mtime replacement cannot be seen by stats. As with audio,
**Verify file bytes** (SS-047) handles it explicitly; a test pins this
behavior. Reimporting into the library that wrote the snapshot keeps local
annotations ("local wins"). Image identities are checked for conflicts exactly
like audio. On the desktop, a mixed folder goes to the image lane, which also
analyzes pending audio. Without the image lane, image records stay as restored
metadata and audio uses the legacy lane.

A *new* image import of a folder that already has a snapshot is still refused
with an explicit message (`EXISTING_CATALOG`): add the folder first so its
identities are restored, then import images into that source. It never adopts a
snapshot as if it were new.

## Database backup and restore

- Backups are unchanged: `VACUUM INTO` captures committed WAL state, including
  writes from other connections (tested), and publication is staged and atomic.
- **Restore preview** (new): choosing a backup validates it completely without
  changing anything, including version, integrity, foreign keys, image tables,
  job tables, monitor tables and search projections. It then shows counts of
  sources, sounds, images, clips and tag aliases. The preview is bound to a
  BLAKE3 digest of the file, so a backup edited after preview is refused and no
  rollback file is written.
- A backup from a newer app reports that it needs the newer version. An older
  schema is refused with the existing guidance.
- After restore, every folder catalog is marked dirty with the review warning,
  and saved revisions are kept. A snapshot saved after the backup is newer than
  the restored revision, so the next save reports a conflict instead of
  overwriting it (tested with image annotations).
- Images whose files are not on disk after a metadata-only restore or portable
  import stay `missing` or offline with their metadata. Nothing is deleted.
  Relink the source or run Check files when the media is back.

## Schema v15

`mixed_catalog_schema.sql` replaces three SS-041 triggers. Image writes and image
import jobs still mark the folder snapshot dirty, but no longer record the
"requires mixed catalog support" error. Existing rows with that exact error are
cleared. The upgrade is transactional with the usual `*.pre-v15-<uuid>.sqlite`
backup. A failure leaves v14 intact (tested). An upgraded schema is identical to
a fresh one, as restore requires (tested). A v14 binary cannot open a v15
database; restore the pre-upgrade backup to go back.

## Limits

| Limit | Value | Behavior when exceeded |
| --- | --- | --- |
| Portable JSON and folder snapshot size | 64 MiB each (existing) | Export or save fails; nothing truncated |
| Images per document | 100,000 (`mixed_catalog::MAX_IMAGES`, matches the DB cap) | Fails |
| Sounds / clips / sources | 100,000 / 100,000 / 10,000 (existing) | Fails |
| Versions per image | 1,000 | Export or import fails |
| Tag aliases | 1,000 (SS-046 cap) | Fails |
| Tags per image | 64 (SS-046) | Fails |
| One image record | 2 MiB (contract) | Fails |
| Backup file | 512 MiB (existing) | Preview and restore fail |

Measured sizes and times are in the [SS-048 review](reviews/SS-048.md). A
10,000-image, 1,000-sound library is about 13.7 MiB (14.4 MB) of JSON.

## Hooks for later tickets (R8)

- `persistence_inventory::every_durable_column_has_a_persistence_decision` opens
  a fresh catalog and fails for any column not classified as portable, backup
  only (with a reason), or rebuildable. A ticket that adds a durable field must
  classify it and, if it is portable, extend `PortableImage` (or a new
  versioned field), import and export, snapshot validation and restore
  validation in the same PR.
- `portable_image_payload_names_every_portable_image_field` pins the contract
  fields the payload carries.
- New fields go into a new schema version (`…/v3`). Never change the meaning of
  a v1 or v2 field.
