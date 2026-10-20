# Image annotations and reversible bulk tagging — SS-046

Creators can tag, note, favorite and rate images one at a time or in bulk,
rename tags, keep aliases, and undo recent changes. These are user facts. They
are stored apart from technical analysis and from future AI suggestions, and
neither reanalysis nor content replacement rewrites them. Production image
capability stays disabled; there is no AI tag review (SS-054), collections or
media deletion.

## Fields and rules

The annotation object is the SS-040 contract (`creativeshelf-image-contract/v1`),
unchanged: `{ revision, tags, comment, favorite, rating, caption }`.

- **Tags.** Display spelling follows the audio rules: underscores become spaces,
  whitespace collapses, at most 64 characters, no control characters. SS-046 adds
  Unicode NFC, so `Café` and `Café` are one tag. The contract adds ≤ 256 bytes.
  Identity is case-insensitive. The first spelling on an image is kept, and adding
  `FOX` to an image with `Fox` changes nothing. Up to 64 tags per image.
- **Notes** (`comment`). Plain text up to 40,000 bytes, no NUL. Stored verbatim and
  rendered only as text, never as HTML or Markdown.
- **Favorite.** Boolean.
- **Rating.** 1–5 stars, or `null` for unrated. Zero is rejected, so "no rating" is
  never confused with a low rating.
- **Caption.** Left unchanged here. The field is reserved for later caption work.
- **Revision.** Starts at 1 and increments on every successful change, including
  undo. Undo creates a new revision; it never rewinds one.

Edits require an image in an authorized scope (a folder source or a granted file),
with its source online and original available, matching audio. Offline images are
read-only. Same-content copies have separate identities and separate annotations.

## Operations (Tauri commands)

| Command | Purpose |
| --- | --- |
| `edit_image_annotations(id, expectedRevision, patch)` | One image. `CONFLICT` if the revision changed. |
| `preview_image_bulk_annotations(ids, patch)` | Counts, per-tag effects, skipped items and revision tokens; writes nothing. |
| `apply_image_bulk_annotations(targets, patch)` | Applies with the tokens; per-item results. |
| `image_annotation_history(limit)` / `undo_image_annotation_action(actionId)` | Recent actions and item-wise undo. |
| `image_tags(sourceId?)` | Tags with counts in scope, plus aliases. |
| `preview_image_tag_rename(from, to, sourceId?)` / `rename_image_tag(…, keepAlias)` | Rename or respell. |
| `remove_image_tag_alias(alias)` | Removes an alias; tags on images are unchanged. |

`patch` is `{ add_tags?, remove_tags?, comment?, favorite?, rating? }`. Absent fields
are kept and `rating: null` clears the rating. There is no field that replaces the
whole tag array. Bulk patches cannot set notes. Errors are `INVALID_ARGUMENT`,
`NOT_FOUND`, `PERMISSION_DENIED`, `CONFLICT`, `SOURCE_UNAVAILABLE`,
`LIMIT_EXCEEDED` or `INTERNAL`. Item outcomes are `applied`, `unchanged`,
`conflict`, `not_found`, `permission_denied`, `source_unavailable` or
`limit_exceeded`.

### Concurrency, transactions and undo

- **Single edits** carry the revision the editor read. If another window or process
  changed the image, the result is `CONFLICT`. The inspector reloads and shows the
  newer state; nothing is merged silently.
- **Bulk edits** are limited to 1,000 images. Preview returns `(id, revision)`
  tokens. Apply runs as one immediate SQLite transaction, together with its history
  record. An image changed after preview becomes `conflict` and is left untouched.
  Offline or out-of-scope images are skipped. If storage fails mid-batch, every item
  and the history row roll back, and a restart shows the state from before (tested
  with an injected failure on the fourth of five items).
- **Undo** is item-wise. An item is restored exactly, including tag order, only if
  it is still at the revision that action produced. Otherwise it reports `conflict`
  and keeps the newer state. Each action can be undone once; there is no redo. The
  last 100 actions are retained across restarts and pruned oldest first.

### Rename and aliases (scope)

- A rename covers the gallery's scope: all sources, or one source. Images with the
  old tag are renamed in place, keeping the tag's position. An image that already
  has the new tag keeps one copy. Respelling (`brand a` → `Brand A`) is a rename to
  the same identity. One rename can change up to 10,000 images.
- **Image tags only.** Audio annotations have no revisions, so a combined rename
  could not be conflict-checked or undone. The preview shows how many audio sounds
  carry the tag and states that they are unchanged.
- **Aliases** are library-wide. *Keep as alias* is available only for a rename
  across all sources. Afterwards, adding the old name adds the new one. Existing
  aliases of the old name are re-pointed to the new name. Undoing the rename
  removes the alias and restores the re-pointed ones. A target cannot itself be an
  alias, so aliases never chain. At most 1,000 aliases.
- Out-of-scope images are neither counted nor changed. Offline images keep the old
  tag and are counted as unavailable.

## Persistence and recovery (schema v13)

- `image_tag_aliases(alias_key, alias, canonical, created_at)` and
  `image_annotation_actions(seq, id, kind, label, state, created_at, items, entries,
  alias)` are added in one transactional upgrade. A consistent `VACUUM INTO`
  `*.pre-v13-<uuid>.sqlite` backup is taken first; a name collision rolls back and
  the v12 catalog stays usable (tested). As with every bump, a v12 database backup
  restores only with a v12 build.
- Full database backup and restore carry both tables. Restore validation rejects
  aliases that are not normalized, chain or exceed the bound, and history that is
  malformed or over the bound.
- **Source snapshots.** Edits update `image_assets`, whose shared trigger marks the
  source snapshot dirty (asserted). Image snapshots stay guarded until SS-048.
- **Portable catalogs.** Aliases are durable user data. Until SS-048, their presence
  blocks the audio-only portable export, the same way image rows do, so they are
  never silently dropped. History is deliberately excluded from portable formats. It
  is local undo state, so undo does not follow a catalog to another computer.
- **Invalidation.** After a change, the gallery refetches its visible windows and
  the inspector reloads details. No image search index exists yet. Previews are
  unaffected, because annotations do not change pixels.

### Payload for SS-048

Per image: the contract's `annotations` object, including `revision`. Library-wide:
the alias list `[{ alias, canonical }]`, with `alias_key = lower(NFC(alias))`.
Import should keep revisions monotonic and must not overwrite a newer local revision
silently. History is not part of the payload.

## UI

- **Inspector.** "Your annotations" holds a favorite toggle, a 1–5/None rating
  radio group, and tag chips with remove buttons. Tags are added as a
  comma-separated list with Enter. Notes have a byte counter and an explicit Save.
  The last change can be undone in place. Escape in a notes field with an unsaved
  draft leaves the field instead of closing the inspector.
- **Edit selected…** opens a bulk dialog for 1–1,000 selected images: add/remove
  tags, favorite, rating, preview counts, apply, per-outcome summary and undo.
- **Tags…** opens the tag list with counts, rename with preview (including the
  unchanged audio count), the alias option and alias removal.
- **History…** lists recent actions with undo for applied ones.
- All dialogs are modal with focus trapped, Escape to close and focus returned to
  the opener. They are sized against the viewport so they fit and scroll at 200%
  zoom.

## SS-048 update

The payload above shipped. Annotations (with `revision`) travel in portable v2
and folder v2 snapshots, and aliases travel in whole-library portable v2. An
identical alias on import is accepted and a conflicting one fails the import.
Undo history stays local. See [Mixed catalogs](MIXED_CATALOGS.md).
