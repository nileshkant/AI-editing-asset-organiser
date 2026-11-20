# Image analysis (SS-052)

Optional, consented description of images by an AI provider the user configured
in SS-049 and qualified in SS-050. Code: `crates/core/src/image_enrichment.rs`,
`enrichment_schema.sql` (schema v19), `src-tauri/src/image_enrichment.rs`,
`src/components/views/ImageAnalysisSettings.tsx`. This ticket schedules and
accounts for requests and stores the validated result. It does not turn results
into tags or search fields; that is SS-053.

## What the user agrees to

Consent is per source. It names one provider at one exact revision and one model,
the fixed purpose ("describe images so they can be searched and tagged") and the
fixed payload ("a re-encoded preview copy"). The disclosure text comes from the
core (`disclosure()`), and the settings page shows it before the Allow button is
enabled. It says what is sent, what is never sent, and that data already sent
cannot be recalled.

Sent per image: one PNG with its longest side at most 512 pixels, produced from
the oriented preview of the exact content version, plus fixed instructions. The
PNG is re-encoded from decoded pixels, so EXIF, location, camera, software and
color-profile chunks are not carried over. File names, paths, tags, notes,
ratings, prompts and the original file are never sent.

Changing the provider (any revision change), model, or removing the provider
makes the stored consent stale. Nothing is sent until the user allows it again.
Removing a source deletes its consent, batches and per-asset records.

## Batches

A batch has a kind (`new_imports`, `selected`, `backfill`), a snapshot of its
items, the source generation it was made for, the provider revision, and two
ceilings: most requests and most units. Estimates are `to_send × 3024` units,
labelled as an estimate. Prices are not known to the app, so cost is never
invented. Up to 8 batches can be active and 20,000 images fit in one batch.
Finished batches are pruned to the latest 100.

"Analyze new imports" is a separate opt-in on the consent. After an import
finishes, the images it made ready are queued, at most 200 per import, with
ceilings equal to that count. It queues nothing when the provider is not ready.

## Accounting and what is not promised

Before each dispatch the runner reserves one request and `EST_UNITS` inside an
IMMEDIATE transaction that re-checks both ceilings and every fence. Two workers
cannot overspend. After the answer the reservation is settled with the provider's
reported usage, or the estimate (marked as estimated) when none was reported.

The app cannot know whether a provider billed a request whose connection broke.
Outcomes are therefore classified, never assumed:

| Outcome | Treatment |
| --- | --- |
| answer received but invalid | billed, item fails, no retry |
| timeout, 5xx, oversized or malformed body | **unknown outcome**: units stay counted as possibly billed, never retried automatically |
| 429 | retried with backoff up to 3 times, then paused |
| quota exhausted, bad key, model missing, provider not ready or changed | the batch pauses; images stay queued |
| TLS, redirect, refused connection, request rejected | not billed, item fails |

An unknown-outcome item keeps its operation id. The user can retry it only after
accepting a possible second charge. No exactly-once billing guarantee is made.

## Fences

Checked inside the same transaction as each state change: consent still present
and equal to the batch, provider present, at the batch's revision and still
tested, source present and at the batch's generation, source online, content hash
still the queued one. Cancel, revoke and source removal stop pending items at
once. A request already in flight finishes, but its answer is discarded and
counted, never published. Content that changed in flight is discarded the same
way. Restoring a backup clears consent and cancels queued work, because a restore
must never carry permission across devices; stored results are kept.

## Reuse

Results are keyed by content hash, model key, preprocessing id and schema id. An
identical image in another asset or source reuses the stored result without a
request, but only when that asset's own source has consent for the same model.
Each asset gets its own output row. Annotations and source permissions are never
merged.

## Priority and resources

One worker thread runs at most one request at a time. It yields whenever an image
import is queued or running or native work is active, and re-checks every
second. Leases last 180 seconds; at startup every lease is recovered (a request
that was in flight becomes an unknown outcome) and expired leases are recovered
every minute.

## Known limits

- No real provider or model has been exercised; tests use a scripted dispatcher
  and one real-adapter loopback test.
- Interactive priority covers imports and native tasks. Playback and
  audio-import jobs are not yet part of the busy signal.
- Not tested on macOS or Windows; the cloud container is Linux only.
