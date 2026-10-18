# Versioned image previews — SS-044

Bounded thumbnails, inspection previews and 1:1 zoom tiles for technically ready
images. Production image capability remains disabled: previews are generated only
in the `image-development` desktop build with the trusted packaged worker. There is
no gallery (SS-045), AI upload, export, image MCP tool, or general media endpoint.

## Request contract and authorization

A caller names `asset_id` (canonical lowercase UUID), `version` (the 64-hex BLAKE3
content digest it was shown), `tier` and, for tiles only, `tile: [col, row]`. No path,
URL or filename crosses IPC. The Tauri commands are `image_preview`,
`cancel_image_preview`, `image_preview_cache_status`, `set_image_preview_cache_quota`
and `clear_image_preview_cache(confirmed)`, all under the existing main-window
capability. There is no custom URI scheme and no filesystem protocol expansion; the
CSP adds only `blob:` to `img-src` so the renderer can display bytes it received.

For every request, core:

1. Validates the request shape (IDs, digest, tier/tile pairing, tile ≤ 64).
2. Under the catalog lock, loads the asset through its current source membership.
   Unknown, removed-source or out-of-scope IDs return `NOT_FOUND`; a version that
   is not the current content digest returns `STALE_VERSION`; anything other than
   `technical_state = ready` with a profile returns `NOT_READY`. Tile positions are
   checked against oriented dimensions.
3. Serves a cache hit, or, only if the original is available, opens the exact member
   read-only. Canonical containment and symlink resolution happen at that open,
   after the lock; a swapped-in escaping symlink yields `ORIGINAL_UNAVAILABLE`.
4. Re-checks identity, digest, readiness and source registration immediately before
   serving. A change in between returns `STALE_VERSION`/`NOT_FOUND`, never bytes.

Identical bytes in different files share cached pixels by digest; each response
still carries the requesting asset's own ID. Preview bytes are a separate permission
from original paths and export; this ticket only serves the local desktop owner.
SS-067 must add its own scoped agent preview permission before any MCP exposure.

## Tiers, pixels and color

All tiers start from SS-042's verified pipeline: full decode, EXIF orientation applied
once, ICC → sRGB, straight alpha. The worker hashes the bytes it decoded; a digest that
differs from the authorized version is `SOURCE_CHANGED` and nothing is cached.

| Tier | Size (oriented pixels) | Decode |
| --- | --- | --- |
| `small` / `medium` / `large` | long edge ≤ 128 / 256 / 512 | all three from one decode |
| `inspect` | long edge ≤ 2,048, stepping down 1,536 → 1,024 → 768 → 512 if needed | own decode |
| `tile` | 512×512 grid cell at 1:1 (edge cells smaller) | own decode |

Previews never upscale. Short edges round to nearest with a minimum of 1. Every encoded
output obeys the SS-040 ceilings `MAX_PREVIEW_DIMENSION = 2048` and
`MAX_PREVIEW_BYTES = 4 MiB`; when an inspect encoding would exceed 4 MiB, the next
smaller step is used and its actual size is reported. Thumbnails and tiles are at most
~1 MiB raw, so they always fit.

Downscaling uses Catmull-Rom. Transparent sources resample in 16-bit premultiplied
space, so hidden RGB under zero alpha cannot bleed into edges; fully transparent output
pixels are `[0,0,0,0]`. Output is JPEG (quality 85 thumbnails, 92 inspect/tile) only
when every output pixel is opaque, else lossless RGBA PNG. A JPEG larger than raw RGBA
falls back to PNG. No matte or checkerboard is ever baked in. The checkerboard is a CSS
background in the renderer. The color policy is `srgb` (SS-040 `ColorPolicy`).

## Cache layout and integrity

`<app data>/cache/image-previews/` is app-owned derived data:

- `v1-<16 hex>/<dd>/<digest>/<tier>.cspv` or `tile-<c>-<r>.cspv`. The namespace hash
  covers `creativeshelf-preview-v1`, the analyzer version and the color policy. A
  preprocessing, decoder or color change therefore lands in a new namespace, and older
  `v1-<16 hex>` namespaces are removed at startup.
- Each record carries `CSPREV1\n`, a JSON header (versions, policy, digest, tier, tile,
  sizes, MIME, transparency, payload length and BLAKE3) and the payload. Reads verify
  every field, the hash and PNG IHDR/JPEG SOF dimensions. A corrupt, truncated or
  mismatched record is deleted and regenerated (`repaired`), never served.
- Writes stage to `tmp/<uuid>.part` (create-new), fsync, then rename. A failed write,
  including disk full, removes its temporary, counts `write_failures` and still
  serves the fresh decode with `cache: "uncached"`. Startup removes only
  `tmp/<uuid>.part` names and our superseded namespaces; foreign files are left alone.

Cache repair, rebuild, eviction and clear never touch originals, `image_analyses`,
technical facts or annotations. The cache is excluded from SQLite backups, source
snapshots and portable catalogs. Losing it only costs regeneration time.

## Resource bounds

- Quota: default 512 MiB, settable 64 MiB–16 GiB. LRU by last access (mtime order
  after restart). Records being read are pinned and never evicted. Clearing defers
  pinned records until their read ends.
- Decode concurrency: 2 preview workers per app, separate from the single SS-042/043
  analysis worker. Each child keeps the 512 MiB heap budget, so the worst-case image
  child heap is 1.5 GiB (not an RSS reservation). Waiting for a slot gives up after
  60 s with `BUSY`.
- Requests: at most 256 outstanding, of which at most 8 are inspect requests
  (≤ 4 MiB each). Excess requests return `BUSY`.
- Worker deadline 20 s. Stdout is capped at 8 bytes + 16 KiB header + 4 MiB per output.
  The JSON header is ≤ 16 KiB.
- Coalescing: concurrent requests for the same digest/job share one decode. A new leader
  first re-checks the cache, so a request racing a just-finished decode cannot cause a
  second decode.
- Cancellation: `cancel_image_preview(request_id)` detaches that requester. The shared
  decode is killed and reaped only when every coalesced requester has cancelled. The
  renderer cancels when a preview unmounts or its asset/version/tier changes, and
  revokes its blob URL.

## Offline, stale and error states

`original_available` reflects the catalog's source/asset availability at request time.
When it is false, a cached preview for the current version is served with
`offline_reference: true`, and the UI shows "Offline reference · original unavailable,
export disabled". Without a cached preview the result is `ORIGINAL_UNAVAILABLE`.
A cached preview never establishes that an original exists or can be exported. A
cache hit for an available asset does not probe the file, so the label is the catalog
state.

Errors are typed: `INVALID_ARGUMENT`, `NOT_FOUND`, `STALE_VERSION`, `NOT_READY`,
`ORIGINAL_UNAVAILABLE`, `SOURCE_CHANGED`, `CANCELLED`, `BUSY`, `TIMED_OUT`,
`LIMIT_EXCEEDED`, `INVALID_DATA`, `UNSUPPORTED`, `PERMISSION_DENIED`,
`WORKER_UNAVAILABLE`, `WORKER_FAILED`, `INTERNAL`. The renderer validates the binary
envelope (`CSPV`, u32 LE metadata length ≤ 16 KiB, metadata JSON, payload). It rejects
replies for any other asset, version, tier or tile, and ignores late replies after a
selection change.

Replaced content: re-registering a path with a new digest makes the old version
`STALE_VERSION` at once; the new version is `NOT_READY` until analysis publishes it.
Records for digests no image references are pruned at startup and after a database
restore. Until then they are unreachable because every request is authorized against
the current digest.

## Settings and diagnostics

Settings → Audio & recovery shows an "Image previews" card when previews are enabled
or records exist. It offers cache size, refresh, and a confirmed clear. Counts:
entries, bytes, quota, generating, hits, generated, evicted, repaired, write failures
and offline references; the status also carries misses, coalesced, cancelled and
stale rejections. No paths, asset IDs or bytes are shown.

The quota is stored under its own `app_settings` key (`image_preview_cache`), not in
`Preferences`. `Preferences` uses `deny_unknown_fields`, so a new field there would
stop an older binary from starting on this catalog. Older binaries ignore the new key.
An invalid stored value falls back to the default. Full SQLite backup/restore carries
the key, and restore reapplies it. No schema version change is needed.
