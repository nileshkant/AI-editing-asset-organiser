# Scoped catalog and clip MCP tools (SS-019)

Open Settings → Agent access, start MCP, enter a client name and explicitly select allowed sources. Read is separate from edit, export and local-path permissions; write/path permissions are off by default. No selected sources means empty catalog results. Pairings remain temporary and expire on Stop/Quit/restart. Re-pair to change permissions.

Discovery advertises only permitted tools, and direct calls also enforce permission checks. The authentication gate passes an internal client identity to the SDK handler; tool arguments cannot choose their owner or grant permissions. Pending requests recheck revocation before dispatch. Already accepted operations may finish; revocation requests cancellation of owned exports before the native publication boundary.

## Tool contract

- `service_status`: app connection status.
- `library.status`: ready/available count for permitted sources.
- `sources.list`: source IDs, names, scope and availability, without machine roots.
- `sounds.search`, `sounds.facets`: canonical UI filter fields `text`, `source_ids`, `tags`, `favorites_only`, `min_duration`, `max_duration`, `offset`, `limit`. Responses include scoped counts, facets and query interpretation. Search items contain `sound` (with `content_hash` as content version) and indexed `availability`. Waveforms are omitted from ordinary read results.
- `sounds.get`, `sounds.resolve`: `{id, version}`. Resolve additionally requires path permission and validates current source membership and canonical containment.
- `sounds.annotate`: `{id, version, tags, comment, favorite}`; edit permission required. Uses the same annotation normalization as the UI.
- `clips.create`: `{sound_id, name, recipe, idempotency_key}`. Recipe uses the UI's source-frame fields: `asset_id`, `asset_version_id`, `source_sample_rate_hz`, decimal-string `start_frame`/`end_frame`, optional `channel_policy` (preserve), `gain_db`, `fade_in_ms`, `fade_out_ms`. Range is inclusive start/exclusive end. Edit permission required.
- `clips.update`: `{id, name, recipe, revision}`; expected revision and current content version are checked.
- `clips.get`: `{id}`; `clips.list`: `{id, version, offset, limit}` for a sound. Source permission applies to clips created by either the UI or agents.
- `clips.export`: `{clip_id, revision, destination_id, options, idempotency_key}` returns `{job_id}` immediately. Options match native export: format `wav`/`flac`, optional sample rate and fades. Export permission required.
- `jobs.get`: `{id}`. `jobs.list`: `{offset, limit}`. These show owned export jobs and import jobs from permitted sources. Import current filenames, lease details, selected paths and raw errors are omitted. `jobs.cancel` can cancel only the caller's export jobs; agents cannot cancel shared import jobs.
- `catalog.export`: same filters/pagination as search, returning `catalog`, `root_map`, `total`, `next_offset`. The catalog is a valid portable subset with relative media paths and corresponding clips/annotations/evidence. Global saved searches are excluded. Root-map values are null unless path permission is granted.

Pagination defaults to 50, permits 1–100 items and offsets up to 1,000,000. Search/clip/job pages provide `next_offset`; null means end. Metadata can change between requests, so offset pagination is not a frozen snapshot. Read operations query indexed metadata without filesystem scanning, decoding or hashing. Availability reflects the catalog's last observation; resolve/export check current files and export verifies content hashes.

Malformed/unknown arguments produce MCP invalid-params errors. Domain failures return `isError` plus a structured code, including `PERMISSION_DENIED`, `ASSET_CHANGED`, `ASSET_MISSING`, `SOURCE_OFFLINE`, `NOT_READY`, `INVALID_RANGE`, `EXPORT_COLLISION`, `JOB_CANCELLED`, `MODEL_UNAVAILABLE`, `DISK_FULL`, `RATE_LIMITED`. Public errors use fixed messages instead of database/filesystem/FFmpeg diagnostics.

## Export approvals and retries

For an export-enabled client, use its **Approve WAV/FLAC destination** button in Settings. The native save dialog selects one exact file. Give the returned destination ID to that client; no tool accepts an arbitrary output path. The shared native exporter has one pending destination grant, valid for ten minutes and one use. A later native destination selection supersedes the earlier grant. Another client's grant cannot be borrowed. Existing files and sidecars are never replaced.

After `clips.export`, poll `jobs.get`. A completed result includes the approved media/manifest paths and frame count. Destination path disclosure is part of explicit export approval, independent of source-path permission. Cancellation before publication cleans owned staging; completed publication wins a cancellation race. Original media remains immutable. Native staging/journal recovery protects committed output.

Idempotency keys are per client and operation. Exact retries return the existing clip/job; changed arguments with the same key conflict. Export job snapshots and retry records are session-only and cleared on Stop/Quit. Committed media/manifests and import-job catalog persistence survive restarts. Limits are 200 export job records, 1,000 retry records, two export worker threads and the native exporter's one concurrent render. At capacity, reconnect after stopping MCP; do not silently issue new retry keys.

HTTP request and transport protections remain those in [MCP access](MCP_ACCESS.md). Structured tool payloads are capped at 2 MiB and returned in structured and text forms; request a smaller page if a portable export exceeds the cap. No generic shell, SQL, filesystem reads, root changes, source deletion or agent-triggered rescan tools exist. Editor handoff and the distributable agent skill are documented in [Editor handoff](EDITOR_HANDOFF.md). Installed-client adapters, installer packaging and full performance qualification remain later tickets.
