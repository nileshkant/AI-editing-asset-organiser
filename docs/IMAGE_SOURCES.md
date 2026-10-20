# Image source lifecycle: refresh, watching and relink (SS-047)

How CreativeShelf keeps image and mixed sources current without losing asset
identity or creator annotations. Code: `crates/core/src/image_sources.rs`,
`image_relink.rs`, `image_watch.rs`, the import engine in `image_import.rs`, and
the desktop adapters `src-tauri/src/image_sources.rs` and `image_watch.rs`.
Production image capability stays gated (`image-development`).

## Four separate states

| Question | Where it lives | Values |
| --- | --- | --- |
| Can the folder be reached, and is it the same folder? | `image_source_monitor.root_state` + `sources.available` | `online`, `offline`, `inaccessible`, `changed`, `unknown` |
| Is this member present? | `image_assets.availability` | `available`, `missing`, `inaccessible` (`offline` is derived from the source) |
| Could its bytes be read as an image? | `image_assets.technical_state` | `ready`, `pending`, `failed` |
| Did the last scan see everything? | `image_import_jobs.discovery_complete` / `DISCOVERY_INCOMPLETE` | — |

A missing or access-denied member keeps its last verified technical facts,
evidence and recipes; only a decode failure marks it `failed` and stales them.
`changed` means a different directory now sits at the root path (the stored
volume/inode or volume-serial/file-index identity differs). It is never
treated as online: it must be relinked.

## Refresh versus listed checks

Every check is an ordinary job in the SS-043 image lane: leased, fenced by
source generation, resumable and one per source at a time.

- **Refresh** (`scan: discover`): walk the selected folder. New files are
  imported; members not seen are marked `missing` **only** when the walk was
  complete and the root identity was unchanged before and after. Folder sources
  only.
- **Check files** (`scan: listed`): enumerate cataloged members from the
  database, 32 per transaction, and check each one. It never walks, never
  admits siblings and never infers absence from a partial scan; a member is
  marked `missing` only when it is individually not found under a verified root.
- **Verify file bytes** (`verify: true`): rehash every member. Imports and
  retries always verify; Refresh, Check files and watcher jobs use the stat
  fast path.

### Stat fast path

`asset_file_stats` records, per asset and content version: volume and file ID,
size, modification time, change time (Unix `ctime`, NTFS `ChangeTime`) and when
it was recorded. A quick check skips hashing and decoding only when all of these
match **and** the file was last written at least 2 s before it was recorded
(FAT/exFAT have 2 s timestamp resolution; this is the "racy timestamp" problem).
The stat is captured before hashing and recorded only if an identical stat is
read after hashing. A mismatch only means "hash it"; stats are never identity.

A tool that rewrites bytes while preserving size, mtime **and** change time (some
network/FUSE filesystems do not report change time) can defeat a quick check.
That is why **Verify file bytes** exists and imports always hash.

## Moves, renames and case changes

Identity never merges on content hash alone.

- **Automatic** only when a new path is provably the *same OS file* as exactly
  one record in the same source: same volume and file ID as that record's last
  stat, same verified content hash, and the record's old path no longer exists
  as an exact on-disk entry. This covers renames, moves within the volume and
  case-only renames on case-insensitive filesystems (`a.png` → `A.png` keeps its
  inode). The old-path check reads the parent directory and compares exact names,
  so a case-insensitive lookup cannot mistake the renamed file for the old one.
  Hard links (two live paths, one file) and copies (different file) never adopt.
- **Explicit** otherwise: `image_move_candidates` lists missing records whose
  bytes equal an available record in the same source. **Reattach** keeps the
  missing record's ID and annotations at the present path and removes the
  duplicate record. It requires expected revisions for both records, no active
  job for the source, and a duplicate with no tags, notes, favorite, rating or
  caption; otherwise it returns `CONFLICT`. Nothing on disk changes.

No cross-source guessing: candidates and adoption are limited to one source.

## Relink

Image and mixed sources relink through the native folder picker only (no typed
path crosses IPC). Audio-only sources keep the legacy relink.

1. **Preview** (no catalog lock held for file work): canonical target, rejected
   if it overlaps another source; stats every cataloged member (≤ 100,000, 30 s
   deadline); hashes an evenly spaced sample of ≤ 32 images / 256 MiB. Reports
   members, present, absent, matched, changed, unreadable and whether it
   completed. No present member → `NO_MATCHING_FILES`.
2. **Consent**: any absent, changed, unreadable or unchecked member requires an
   explicit acknowledgement.
3. **Confirm**: re-reads the target's root identity (a folder swapped in since
   the preview is `SOURCE_CHANGED`), re-checks generation and overlap, then in one
   transaction moves the source to the new root and generation, cancels its
   older-generation jobs, records the new root identity and queues an exact
   listed verification. Asset and sound IDs are unchanged; unchanged content
   reuses its analysis; changed content is reanalyzed with annotations kept;
   absent members become `missing`. Preview plans expire after 5 minutes; at
   most 8 are held.

## Opt-in watching

Watching is off until enabled per source and is never implied by an import.

- **Backend**: `notify` 8.2.0 (FSEvents, inotify, ReadDirectoryChangesW), in the
  desktop adapter only, without following symlinks. Folder sources watch
  recursively; exact-file sources watch their folder shallowly and drop every
  event that is not a listed member, before it is queued.
- **Filtering**: app-owned and transient names never become work, in watchers or
  in Refresh walks: `.creativeshelf*`, `.soundshelf*` (including export staging),
  AppleDouble `._*`, Office `~$*`, `*.tmp`, `*.part`, `*.crdownload`, `*~` and
  similar. An exact file the user explicitly selects is still imported.
- **Settling** (`WatchPlanner`, pure and clock-injected): a path is acted on after
  750 ms without events and two stats 1 s apart agree, so rapid saves and growing
  files produce one check. Settled paths are coalesced into one quick job:
  changed files by path, new folders by prefix, deleted members by path.
- **Safe fallbacks** to one complete quick refresh: more than 4,096 pending
  paths, more than 256 settled paths or folders, a full 4,096-event hand-off
  queue (callbacks never block), a backend rescan flag or error, a deleted folder
  that held members, and every watcher start (app launch, opt-in, reconnect,
  relink), because changes made while nothing was watching were not observed.
- **Lifecycle**: a 200 ms loop reconciles watchers with preferences and source
  generations every 2 s and probes watched roots every 5 s (writing only on a
  state change). Offline, access-denied or changed roots pause the watcher; a
  return to online restarts it. A batch observed under an older generation is
  rejected (`SOURCE_CHANGED`). While a job runs for the source, changes stay
  pending. Shutdown stops the loop before the import lane.

## Persistence and recovery (schema v14)

Additive and transactional with the standard pre-upgrade backup:
`image_import_jobs.scan_kind/verify/origin` (defaults preserve exact import
semantics), `asset_file_stats`, `image_source_monitor` (backfilled from existing
jobs and image records). All are in database backups and restore validation
(which rejects tampered values even if CHECK constraints were bypassed). Watch
preference and stats are machine-local: SS-048 must exclude them from portable
catalogs (stats are meaningless on another machine; watching must be re-enabled
deliberately) and carry no new fields into source snapshots, which remain gated.

## Limits and platform notes

- macOS: native watcher, availability, rename, case-only rename and relink
  evidence on APFS (case-insensitive). Windows and Linux run the same core and
  native watcher tests in CI; installed-app behavior is not yet verified.
- Linux inotify watch limits can prevent watching a large tree; the source then
  shows "Unavailable" with the OS error and Refresh remains available.
- A listed check cannot distinguish an unmounted nested mount point from a
  deleted subfolder; discovery walks never cross volumes.
- Network drive discovery is out of scope; paths are whatever the user selected.
