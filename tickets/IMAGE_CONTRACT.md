# Image phase — mandatory implementation contract

Date: 16 October 2026
Updated workflow: 17 October 2026 (user-authorized dependent PRs)
Applies to: SS-040 through SS-078
Parent: [SS-032](SS-032.md)
Execution order: [IMAGE_QUEUE.md](IMAGE_QUEUE.md)

This is part of every image ticket's acceptance contract, not optional guidance.
Read it together with the selected ticket and its predecessor review. Ticket-local
requirements narrow this contract; they cannot silently waive it. All requirements
are prospective; this document records no completed implementation or tests.

## 1. Strict one-ticket / one-PR execution

1. Start with SS-040, then proceed numerically through SS-078. Read the actual
   predecessor PR, checks, and review evidence; old status labels may lag Git history.
2. Start the next ticket after its immediate predecessor has a PR with all
   checks passing on its current head and no known blocking findings. Do not
   wait for merge. Branch from that verified head and target the dependent PR
   to the predecessor branch; use reviewed main if the predecessor is merged.
   Develop in ticket order, with one implementation PR per ticket.
   Retarget to main after its predecessor merges, refresh the branch as needed,
   and verify all checks again on the final proposed merge revision.
   Merge in ticket order; never merge a dependent PR before its predecessor.
3. One ticket owns one PR. Scope a PR to that ticket, its necessary tests,
   documentation, schema changes, and review evidence. Do not combine several
   numbered tickets or implement future capabilities for convenience.
4. If blocked, record the exact failing condition, attempted resolution, evidence,
   and next required action in the ticket/review. Stop this queue. Do not skip a
   ticket, silently reduce its acceptance, or proceed because another is easier.
5. If review reveals a genuinely separate prerequisite/fix, explicitly amend this
   queue with its position and dependencies before implementing it. Never append
   an out-of-order hidden dependency or absorb unrelated work into a large PR.
6. A future ticket mentioned under handoff or exclusions is not a dependency.
   Reserve only the contract hooks needed now; do not build that future feature.
7. Publication/merge follows the user's actual authorization in that session.
   Planning these tickets does not itself publish issues, open PRs, or authorize
   merges/releases. Once explicitly authorized, do not ask repeatedly.
8. Existing audio release qualification remains open where evidence is absent.
   Image development can use the merged foundation; production release still
   requires all applicable inherited gates. The image queue does not claim that
   SS-023/024/031/034 or optional audio AI work is complete.

## 2. Repository and contract ownership

- Rust core owns identity, containment, validation, search, jobs, media facts,
  recipes, and export rules. Keep Tauri commands and MCP adapters thin.
- Worker lifecycle belongs in src-tauri/src/service.rs; IPC contracts go through
  src-tauri/src/main.rs, src/api.ts, and src/types.ts. Agent transport/tool adapters
  are in crates/agent and src-tauri/src/agent.rs.
- Existing source modules are sources.rs, library.rs, jobs.rs, portable.rs,
  source_catalog.rs, maintenance.rs, and catalog.rs. Paths listed in tickets are
  current integration points, not a demand to put all image code in large files.
  Add small image-specific modules with tests as needed.
- Freeze the initial contract in SS-040, then update its versioned examples when
  later tickets add fields. Validate both serialization directions, camelCase
  invocation arguments, null/absent semantics, safe integers, unknown enum values,
  lengths, array bounds, and structured public error codes.
- Preserve sounds.*, clips.*, audio Profile, sample counters, application data
  paths, pairing compatibility, and existing UI behavior unless a separately
  explicit migration proves compatibility. Do not rename the whole project.
- Capability negotiation must distinguish implemented, configured, permitted,
  and currently available. A button or tool definition is not implementation.
- Reuse shared SS-014 provider and SS-017 pack infrastructure when present.
  SS-049/051 own the shared image-phase implementation subsets if absent;
  update legacy tickets with evidence links, not false completion. Audio event
  recognition and SS-039's audio-specific requirements remain separate.

## 3. Identity, state, and persistence

- Asset UUID is independent from content hash and current root path. Identical
  bytes may reuse versioned analysis but keep per-file user annotations/identity.
- Source generations fence jobs and open media. A same-path replacement creates
  new content evidence and stale recipes, while preserving user-owned metadata.
  Ambiguous renames and overlapping roots require explicit reconciliation.
- Technical readiness, AI enrichment, availability, and human review are separate
  state axes. Optional AI absence/failure cannot hide a technically valid image.
  A cached preview does not prove an original is available for export.
- Every new durable field must have migration, backup/restore, source-snapshot,
  portable import/export, search invalidation, and deletion/retention behavior.
  A ticket may explicitly exclude a field from a portable format only with a
  documented reason, user-visible consequence, and tests.
- The deliberate SS-041–047 staging exception keeps mixed image persistence
  gated until SS-048: earlier PRs must reject/guard incomplete portable or
  sidecar paths, preserve the previous metadata, and prove they cannot silently
  drop images. They do not implement SS-048 early. After SS-048, every later
  durable-field addition must extend persistence in its own PR.
- Binary editable masks/drafts are durable user work, not disposable thumbnails.
  Define app-owned artifact references, hashes, revision transactions, and
  backup inclusion; metadata-only exports must report missing artifact bytes.
  Never embed unbounded masks/model arrays in JSON or pretend a mask reference
  alone restores an editable extraction. Media packs may include selected
  owned mask artifacts only under their explicit manifest and sharing scope.
- Schema upgrades are transactional with consistent pre-upgrade backup; newer
  schemas fail clearly. Test disposable fixtures, never the user's live catalog.
  Never simulate a downgrade by changing user_version.
- Preserve source snapshot locks, expected revisions, atomic publication, dirty
  retry states, and read-only warnings. Do not truncate metadata to fit size limits.
  Snapshot limits and portable-transfer limits remain distinct.
- User edits, AI suggestions, corrections/rejections, measured facts, filename
  hints, OCR, and imported provenance have separate origins. No future inference
  can silently erase accepted user choices.
- Define optimistic concurrency for all edited records. On conflict, refresh and
  resolve; do not overwrite unseen user changes or retry mutations blindly.

## 4. Filesystem and data access

- Original image/audio bytes remain immutable. Explicit exports produce new files;
  only app-owned source metadata sidecars are the established source write exception.
- Folder grants can be recursive only as shown in the native selection. Exact-file
  grants never expand to siblings. A selected output file does not grant its parent
  directory or the original image to an agent.
- Resolve current source membership and canonical containment, including symlink
  changes, drive/file replacement, OS path case, Unicode, and unavailable mounts.
  Recheck at read/publication boundaries to reduce time-of-check/time-of-use races.
- Missing file, offline root, inaccessible path, malformed decoder output, and
  cancelled/partial discovery are distinct. Only complete discovery can establish
  missing membership; partial scans must not mass-delete records.
- Preview bytes, original paths, metadata, edits, import, export, and cloud upload
  have distinct permissions. Apply authorization before counts, facets, similarity,
  collections, lineage, histories, or error details can reveal restricted assets.
- Filenames, comments, captions, OCR, prompts, and model output are untrusted data,
  not tool instructions. No shell interpolation, raw HTML, SQL concatenation,
  executable presets, or automatic URL following.

## 5. Media correctness and resource bounds

- Supported initial still formats are JPEG, PNG, and WebP. Reject unsupported
  animation or formats explicitly. Future formats need separate qualified work.
- Distinguish raw versus EXIF-oriented dimensions, alpha channel versus actual
  transparency, partial alpha, color profile, and unknown technical facts.
- Use the same canonical oriented-pixel coordinate contract for previews, crops,
  detector regions, masks, brushes, and export; test odd sizes and high DPI.
- Full required decode verifies technical readiness. Large headers, compression
  bombs, malformed metadata, excessive dimensions, and allocation overflow must
  fail before unbounded work.
- Bound workers, queues, buffers, IPC/results, cache/storage, logs, retries, and
  undo histories. Media/inference work runs outside long-lived DB locks and UI
  thread. Maintain fairness for audio playback and user interaction.
- Set named measurable limits in the owning implementation ticket. Do not invent
  latency/quality claims or change existing benchmark thresholds to hide failures.
- Preview and export must agree on crop rounding, orientation, resampling, color,
  original alpha, matte selection, and recipe revision.
- Stage, verify, and publish without overwriting existing output. Handle disk-full,
  cancellation, crash, external drive removal, and failed indexing separately.
  Committed media remains usable even if registration later fails.
- Extraction copies visible source content through a mask. Generating unseen
  parts, inpainting the background, pose changes, and redrawing are outside scope.

## 6. AI model and consent rules

- All AI is optional and disabled until explicitly configured/enabled. Keys or a
  paired agent are not consent to upload images, prompts, or local metadata.
- Verify each model's actual modality/capability, revision, license, resource
  needs, preprocessing, schema, and languages at implementation time using primary
  documentation and real inference. Do not hard-code an assumed vendor/version.
- Give the user a clear selected-source/provider/data-transfer scope; strip
  unrelated embedded metadata before upload by default. Local execution still
  counts as AI and retains model provenance.
- Measure quality on labeled consented fixtures and held-out queries; freeze
  thresholds before final measurement. Mocks prove error paths, not model quality.
- Record unavailable confidence honestly. Self-reported model scores are not
  calibrated probabilities; similarity is not identity or legal clearance.
- Bound spend/work with estimates labeled as estimates. Unknown remote billing
  outcomes need explicit retry handling; do not promise exactly-once external billing.
- Pin vector-space identity. Never mix different model/dimension/preprocessor
  embeddings or allow semantic retrieval to relax authorization/hard filters.
- Qualify every advertised required capability. A failed evaluation is a blocker
  or an explicitly reviewed scope amendment, not a reason to tick acceptance and
  silently disable a requested feature. Optional methods may stay unavailable
  only where the ticket explicitly defines a supported alternative.

## 7. UI and human workflows

- Provide useful empty, pending, running, partial, failed, cancelled, offline,
  stale, unknown, and unsupported states. No fabricated progress/quality scores.
- Preserve independent selection, active audio transport, and job state across
  tabs/filters. Fence late asynchronous responses.
- Keyboard navigation, focus trap/restore, screen-reader labeling, reduced motion,
  high-DPI coordinates, long names, 200% text, and minimum window layouts apply
  to every new control. Native OS interaction needs installed verification.
- Bulk edits disclose affected scope/counts, conflict and partial-failure policy,
  and recoverable undo/history where promised. Destructive choices require their
  established confirmation; routine viewing/analysis does not add new approval loops.

## 8. Mandatory testing and PR evidence

Each ticket defines R1…Rn requirements, T1…Tn focused scenarios, and A1…An outcomes.
Every R requirement is mandatory; the shorter A list does not replace it.

1. Before coding, map requirements to changes and tests/manual evidence. Identify
   missing external prerequisites immediately without skipping work.
2. Use generated/licensed disposable media with known pixels, masks, dimensions,
   color, alpha, and metadata. Store fixture generation/source provenance rather
   than private libraries, live DBs, secrets, or large model weights in Git.
3. Add meaningful success, boundary, race, cancellation, restart, and negative
   tests for changed behavior. Do not add tests that merely mirror implementation.
4. Use real decoder/export/AI/native/client evidence for those claims; UI mocks
   cannot establish media fidelity, model quality, or installed platform support.
5. Run focused tests during implementation, then relevant regression checks:
   npm test; npm run build; node scripts/cargo.mjs test --workspace --locked.
   For audio/media regressions use qualified native FFmpeg/FFprobe fixtures and
   --include-ignored where applicable. Use npm run test:browser for changed UI
   where available. Contract-only docs need only appropriate fixture/link checks.
6. Record commands, actual results, candidate commit, OS/architecture, dependency/
   model versions, evidence paths, each R/A coverage mapping, review findings/fixes,
   and unverified manual/platform checks in docs/reviews/SS-NNN.md.
7. PR title/body must describe this ticket's final behavior and validation.
   Include migration/recovery impact, limitations, and any external dependency.
   Check CI on the exact pushed commit when publication is authorized.
8. Mark complete only after implemented, verified, reviewed, and merged. Required
   unverified criteria remain open; optional exclusions need explicit rationale.
   At handoff, identify the next numbered ticket, known contracts it consumes,
   and no claimed completion of its future behavior. A green current-head PR
   permits the next ticket to start; complete status still requires review/merge.

## 9. Scope discipline and acceptance limits

This plan is detailed implementation guidance, not a proof that every possible
bug has been anticipated. Agents must inspect the then-current code and report
new risks found by testing/review. The response to an uncovered requirement is
a documented correction and acceptance evidence, never silent scope drift.

Deferred beyond this queue: team/cloud synchronization, public marketplace,
additional image formats/animations/layered files, generator/cloud connectors,
video indexing, generative editing, and SS-033 executable Remotion presets.
These are not hidden prerequisites of the image phase.

## 17 October 2026 workflow amendment

The user explicitly removed the wait-for-merge development gate. This supersedes
older ticket/review text requiring merge before the next ticket starts. Keep
one ticket per PR, all current-head checks passing, explicit dependency links,
and predecessor-first merge order. If a predecessor changes, refresh dependent
branches and recheck affected PRs rather than relying on stale green checks.

## Current execution hold — 17 October 2026

The user subsequently requested finishing SS-043, publishing the previously
uncommitted roadmap/tickets in a separate PR ahead of it, and then stopping.
This current hold overrides automatic continuation after green checks. Keep
SS-044–078 not-started until a new user instruction resumes work. Earlier PRs
#39–41 are merged; the two remaining PRs must base on that merged state.
