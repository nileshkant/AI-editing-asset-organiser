# Exact duplicate images (SS-062, first half)

Status: exact copies are implemented. Visual near-duplicates and variation
families need SS-060 visual retrieval and are not offered yet; the ticket keeps
those requirements open.

## What counts as an exact copy

Two image records are copies when their verified content digests (BLAKE3 of the
original bytes) are equal. A record takes part when:

- it is an image, never a derived element (SS-074 lineage stays separate);
- its bytes were hashed and are not awaiting re-verification
  (`technical_state` is `ready` or `failed`, never `pending`);
- it is not confirmed missing by a complete scan;
- it is inside its source's grant (folder, or the exact selected file).

Offline and unreadable copies stay in their stack and show that state. Images
that look the same but differ in bytes (re-encoded, cropped, recolored,
re-exported) are never exact copies. Similarity never proves identity.

Membership is computed on read from `image_assets.content_hash`, so stacks
follow imports, refreshes, relinks, restores and file changes with no
recomputation job and no stale cache.

## Decisions

A stack is a view; the only stored data are the creator's decisions, keyed by the
content digest.

| Decision | Effect | Undo |
| --- | --- | --- |
| Prefer a copy | Marks the representative copy | Clear preference, or prefer another |
| Keep separate | Takes one copy out of the stack | Return to stack |
| Dismiss | Hides the stack while its copies are unchanged | Show again |

Nothing is deleted, moved, renamed or merged. Each copy keeps its own asset ID,
path, tags, notes, rating, revisions and export behaviour.

- A copy that arrives after dismissal resurfaces the stack and is labeled
  "New since dismissed". Returning a separated copy to a dismissed stack does not.
- A stack with fewer than two copies left in it (after separations) is hidden as
  resolved. Hidden stacks are listed on request so every decision can be undone.
- If the preferred copy changes, goes missing, goes offline or becomes unreadable,
  the preference is kept and its state is shown (`changed`, `missing`,
  `offline`, `inaccessible`). A changed copy leaves the stack; if its bytes
  return, it rejoins with the decision intact. Deleting the record clears the
  preference.
- Every decision is fenced by the stack's revision. A stale editor gets
  `CONFLICT`, and the dialog reloads the stack instead of retrying.

## Authorization

The core API takes a source scope list (`None` is the desktop owner). Copies
outside the scope never contribute to stack counts, members, the preferred copy
or the representative. A stack with one copy inside the scope does not exist for
that caller, so identical bytes elsewhere are never revealed. A scoped caller
cannot override a preference it cannot see (`PERMISSION_DENIED`). The desktop
uses the gallery's source filter as the scope. No MCP tool exposes stacks yet
(image MCP tools are SS-067/068).

## Interfaces

| Command | Purpose |
| --- | --- |
| `image_duplicate_groups(sourceId, includeHidden, offset, limit)` | Page of stacks (largest first, hidden last) with exact totals |
| `image_duplicate_group(sourceId, contentHash)` | One stack, re-read after a conflict |
| `update_image_duplicate_group(sourceId, contentHash, expectedRevision, action)` | One decision: `prefer`, `clear_preferred`, `separate`, `rejoin`, `dismiss`, `restore` |

Errors: `INVALID_ARGUMENT`, `NOT_FOUND`, `PERMISSION_DENIED`, `CONFLICT`,
`INTERNAL`.

## Persistence

SQLite schema v16 (`image_duplicate_schema.sql`) adds two tables:
`image_duplicate_groups` (digest, revision, preferred copy, dismissed) and
`image_duplicate_marks` (digest, copy, `separated` or `reviewed`). Triggers keep
both to image records and mark every folder holding a copy dirty for its next
snapshot. The upgrade is transactional with the usual pre-upgrade backup; an
upgraded schema equals a fresh one.

| Data | Portable JSON | Folder snapshot | DB backup |
| --- | --- | --- | --- |
| Preferred copy, dismissal, separated and reviewed copies | yes (`soundshelf-catalog/v3`) | yes, this folder's copies only (`creativeshelf-source/v3`) | yes |
| Decision revision and time | no: local edit fence | no | yes |

- Writers still choose the lowest version: v3 only when decisions exist.
- Every reference in `image_duplicate_decisions` must be an image record of the
  same document holding exactly that digest. A folder snapshot therefore never
  names a file outside its folder; a preference for another folder's copy is
  left out of that folder's snapshot.
- Importing into a library that already decided a stack keeps the library's
  preference and dismissal and adds the imported copies' marks.

## Limits

| Limit | Value |
| --- | --- |
| Stacks per page | 25 |
| Copies listed per stack | 64 (counts stay exact) |
| Page bytes | 2 MiB (short page, never truncated silently) |
| Offset | 100,000 |
| Scopes per call | 128 |
