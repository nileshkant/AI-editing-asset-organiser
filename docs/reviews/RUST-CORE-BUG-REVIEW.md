# Rust core bug review — November 2026

Review of main `eb7b81f` (SS-058 merged, schema v18) on 12 November 2026,
branch `claude/project-thread-adzcpf`. Scope: correctness of `crates/core`
(catalog, schema migrations, scanning, audio search, portable and folder
catalogs, backup/restore), `crates/agent` (MCP server) and the `src-tauri`
commands. Security-only issues and the React UI were reviewed separately. No
queue ticket was started.

## Method

The files below were read in full: `catalog.rs`, `sources.rs`, `library.rs`,
`jobs.rs`, `search.rs`, `portable.rs`, `source_catalog.rs`, `maintenance.rs`,
`agent.rs`, the v15–v18 schema files, `crates/agent/src/{lib,tools}.rs`, and
`src-tauri/src/{main,service,maintenance}.rs`. `image_search.rs` was read for
the v18 migration and index flush, and `export.rs` for publication and locking.
The remaining image modules were not read line by line.

Each suspected bug was reproduced with a test before it was fixed, except where
noted.

## Result

No data-loss or catalog-corruption bug was found. The v16–v18 migrations run in
one immediate transaction with a foreign-key check, the fresh-install and
upgrade paths execute the same schema files, scan reconciliation only marks
files it could not verify, and restore validates the backup before touching
the live catalog.

## Fixed in this change

| # | Severity | Finding | Fix | Test |
|---|---|---|---|---|
| 1 | Medium | The MCP agent kept every idempotency key and export job for the app session. After 1,000 keyed calls or 200 export jobs, `clips.create` and `clips.export` returned `RATE_LIMITED` until restart. | Keys are bounded per client (256, oldest forgotten first); a full job list forgets its oldest finished job. Running jobs are never dropped. | `crates/core/tests/agent_limits.rs`; `agent::limits` unit tests |
| 2 | Medium | An apostrophe inside a query word (`dog's`, `don't`) opened a quoted phrase, so the query found nothing unless the title had the same apostrophe. | Inner apostrophes (`'` and `’`) belong to their word. Documents index the split and the joined spelling, so either form finds either title. A leading `'` still opens a phrase. | `search::tests::apostrophes_inside_words_do_not_open_a_phrase` |
| 3 | Medium | Backup, portable catalog export, the support report and clip export publish with a hard link. exFAT/FAT volumes and some network shares have none, so saving to a USB stick failed. | `publish::publish_new` falls back to an exclusive create plus copy. It still never replaces a file or follows a symlink. | `publish::tests` (four tests) |
| 4 | Low | A tag present as both a user tag and an analysis tag counted twice in the tag facet (one sound showed "rain 2"). | Facets count each sound once per tag. | `search::tests::tag_facets_count_each_sound_once` |
| 5 | Low | A restore that was refused (imports running, file changed, version mismatch) had already stopped the MCP server, and nothing restarted it. | MCP restarts on its remembered port when the restore is refused. A successful restore still leaves it stopped, as the UI says. | `maintenance::tests` in `src-tauri` |

### Failing-then-passing evidence

Each test was run against the code before its fix, then after.

- Finding 1, before: `assertion left == right failed: call 999` (the 1,000th
  create returned `RATE_LIMITED`). After: 1,100 creates pass, a recent key
  replays its original clip, and another client's key survives.
- Finding 2, before: `Interpretation { terms: ["dog"], phrases: ["s bark"] }`
  and 0 results. After: passes.
- Finding 4, before: `left: [FacetItem { value: "rain", count: 4 }]` for two
  sounds. After: count 2.
- Finding 3 could not be reproduced on a real exFAT volume: the review
  container's kernel has no FAT driver. The fallback is exercised by injecting
  the errors the operating systems report for a volume without hard links
  (`PermissionDenied` on Linux, `Unsupported` on macOS and Windows).
- Finding 5: the helper is new, so there is no "before" run. The tests pin the
  three cases: refused restore restarts MCP, successful restore leaves it
  stopped, and MCP that was off stays off.

### Behavior changes worth knowing

- An idempotency key is remembered for the client's most recent 256 keyed
  calls, not for the whole session. A retry with a forgotten key creates again.
- `jobs.get` on an export job older than the most recent 200 returns
  `PERMISSION_DENIED`, the same as an unknown job.
- On a volume without hard links, a crash in the middle of publication can
  leave a partial destination file. With hard links publication stays atomic.

## Found, not fixed

These are low severity and have no deterministic test without a new seam, so
they are recorded here instead of being changed without evidence.

- **Upgrade backup left behind.** `Catalog::open` writes
  `*.pre-v18-<uuid>.sqlite` before upgrading. If the open then aborts because
  another process changed the catalog during the backup, the copy is not
  removed. Each retry leaves one more full copy.
- **Folder relink can report success without changing anything.**
  `Catalog::relink` updates with `WHERE generation=?` and does not check that a
  row changed. Only reachable when another connection moves the source between
  the check and the write.
- **Unreadable file marked missing.** In `scan_paths`, a file that exists but
  fails containment or hashing is not added to `seen`, so reconciliation marks
  its existing sound "missing" instead of "failed". The next successful scan
  corrects it. From reading; not reproduced because the container runs as root.
- **Portable import collisions surface as raw SQLite errors.** Importing a
  catalog whose source IDs or saved-search names already exist rolls back
  correctly, but the message is the constraint text.

## Tests run

Linux x86_64, Rust 1.97, FFmpeg from the distribution, the same command CI uses:

```
SOUNDSHELF_FFMPEG=$(command -v ffmpeg) SOUNDSHELF_FFPROBE=$(command -v ffprobe) \
  cargo test --workspace --locked --no-fail-fast -- --include-ignored
```

Result: 440 passed, 2 failed, 0 ignored. The two failures also fail on main in
the review container, because it runs as root and root ignores the read-only
directories the tests rely on:
`image_migration_tests::real_backup_write_failure_cancels_upgrade_before_schema_mutation`
and `source_catalog::read_only_snapshot_failure_is_retryable_and_member_access_denial_never_prunes`.
Both pass on this branch when their test binaries are run as an unprivileged
user (`setpriv --reuid=65534`).
