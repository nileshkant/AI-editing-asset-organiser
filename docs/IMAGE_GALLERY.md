# Image gallery and inspection workspace — SS-045

The **Images** view lets you browse, inspect and compare image candidates while
the audio library, its selection and playback stay as they were. It appears in
the sidebar only when the image import flow is enabled (the `image-development`
build) or image metadata already exists. Production image capability remains
disabled; this ticket adds no tag editing, AI, search, export or file removal.

## Data contract

- `image_gallery_page(sourceId | null, sort, offset, limit)` returns a window
  `{ items, offset, total }`. `null` means every registered source for the local
  desktop owner. `sort` is `name_asc` or `name_desc`: relative path ASCII
  case-folded, then exact path, then **ascending asset ID** as tie-breaker.
  Limits are 1–100 items per window and offset ≤ 100,000 (the catalog image cap).
  A window may be shorter when the 2 MiB serialized bound is reached; the
  renderer resumes at `offset + items.length`.
- Windows are a live observation, not a frozen snapshot: an import between two
  windows can shift offsets. The renderer refetches visible windows when import
  jobs change or a database restore occurs. Selection uses asset IDs, so it is
  never shifted by such changes.
- File-scoped sources list only their exact granted members; audio and element
  rows never appear. Summaries carry the header, oriented dimensions and the
  annotation revision; no evidence, recipes or absolute paths.
- `image_details(id)` returns the header, profile, source name/availability,
  tags and caption for the inspector.

### Schema v12

A single expression index, `asset_registry_gallery(kind, lower(relative_path),
relative_path, id)`, serves the window order directly. Without it each window
sorted the whole catalog under the catalog lock. At 100,000 images (release,
macOS arm64) the worst window was 252 ms without the index and 15 ms ascending /
43 ms descending with it. The plan is asserted to use the index without a
temporary sort.

The upgrade follows the existing path: a consistent `VACUUM INTO` pre-upgrade backup
(`*.pre-v12-<uuid>.sqlite`), then the index plus `user_version` in one immediate
transaction. A name collision rolls back and leaves the v11 catalog usable (tested).
Like every prior schema bump, full database restore accepts only backups of the
same schema version, so **v11 backups cannot be restored into a v12 build**. Keep
the v11 binary to restore them, or restore before upgrading. No data fields
changed; portable catalogs and source snapshots are unaffected.

## Renderer behavior

- **Grid.** ARIA `grid` with `aria-rowcount`/`aria-colcount`, `aria-multiselectable`
  and roving focus. Only visible rows plus one row of overscan render; cells
  are absolutely positioned. At most 12 windows of 100 summaries stay in memory,
  with the farthest evicted first. Thumbnails request a preview only after staying
  mounted for 120 ms, so fast scrolling requests nothing and scrolled-out
  thumbnails cancel and revoke their blob URLs.
- **Keyboard.** Arrows move; Home/End go to the row start/end; Ctrl/⌘+Home/End go to
  the first/last image; PageUp/PageDown move by the visible rows. Space toggles
  selection, Enter opens details and Escape closes them, returning focus to the
  cell. Ctrl/⌘-click adds to the selection. If a column change re-creates the
  focused cell, focus moves to its replacement.
- **Thumbnail size.** Small, medium or large cells (96/144/208 CSS px). The preview
  tier is the smallest covering the cell × device pixel ratio.
- **Inspector.** Technical facts (oriented and stored size, EXIF orientation,
  format, size, depth, color handling), transparency stated accurately (no alpha
  channel / alpha but opaque / binary / partial), file, source, availability,
  technical and AI state ("Not enabled"), and tags read-only.
- **Viewer.** Fit uses the inspect preview. 25–400% zoom levels and a 100% button
  add 1:1 tiles for the visible viewport only (≤ 48). Pan by dragging or with the
  arrow keys; `+`, `-`, `0` (fit) and `1` (100%) work while the viewer is focused.
  Above 100% pixels render unsmoothed. When the original is offline, the viewer
  enlarges the cached preview and says so instead of requesting uncached tiles.
- **Backgrounds.** Checkerboard, light or dark is a CSS background on the viewer and
  in comparison, never applied to image data.
- **Compare.** 2–4 selected images side by side in a modal dialog (focus trapped,
  Escape restores focus), each with dimensions, format and transparency.
- **States.** No sources, no supported images, import in progress, partial import
  failures (with a link to Imports), loading, listing errors, pending/failed
  analysis per cell, offline originals per cell and in the inspector, and stale
  previews (which refresh the visible windows).
- **Audio preservation.** While Images is shown, the audio search keeps its last
  library/favorites query and offset, and the audio selection and inspector
  return unchanged. Playback continues. Space and M keep their transport
  bindings outside the grid; in the grid, Space selects. The audio list remounts
  on return, so its scroll offset within the current 75-item page is not restored.
- **Layout.** The gallery uses a container query (≤ 700 px) to stack the inspector
  over the grid. Unlike a viewport media query, it also responds under text or
  page zoom. Verified at 1280×820, 720×560 and 200% zoom.
