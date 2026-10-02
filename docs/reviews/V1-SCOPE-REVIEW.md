# CreativeShelf audio v1 scope review

Date: 2026-10-02
Baseline: 8acd3a6, feat/SS-019-mcp-catalog-and-clip-tools
Review branch: chore/creative-shelf-v1-scope-review
Outcome: plan/backlog updated; visible branding changed; not release-qualified.

## Evidence and gaps

- Referenced originals already exist: `crates/core/src/library.rs::scan_paths`
  reads/hashes registered source paths and publishes analysis to SQLite. It does
  not copy originals into application storage. `src-tauri/tauri.release.conf.json`
  lists only FFmpeg/ffprobe as resources. This is configuration evidence, not an
  inspection of a signed installer. SS-024/031 require artifact-content and size
  checks. Exports intentionally create new media and are separate from import.
- Repeat analysis caching already exists: `scan_paths` checks cached profiles by
  content hash, but still traverses folders and hashes their media. No automatic
  per-source `.creativeshelf/catalog.json` read/write path exists. SS-029 fills
  this gap; no sidecar feature was implemented during this planning review.
- SS-021 provides explicit portable export/import. Its importer can reuse stored
  analysis without media reads, but imports append and reject identity collisions;
  it is not idempotent folder reimport or automatic missing-member pruning. Do
  not label it completion of the new requirement.
- `sources.rs::reconcile_paths` already protects incomplete scans, rejects stale
  source generations and marks confirmed unseen DB members missing. Existing
  availability tests cover missing/offline distinction. SS-029 extends that
  behavior to sidecar pruning and conflict/crash recovery.
- `media.rs` publishes measured profiles and generic measured tags. That does
  not establish sound identity. SS-016/017 now explicitly require sound-event
  recognition and readable/searchable labels, multi-event/unknown handling,
  confidence/coverage/model provenance, user corrections and evaluation. MCP
  accesses the shared catalog; it is not an audio recognition model.
- SS-019 remains in-review; this audit does not merge or reclassify its PR.

## Changes

CreativeShelf is the generic working name. Sidebar, document/window title,
bundle product name, desktop/MCP bridge visible errors, current guidance and
frontend expectations were updated. Persisted app identifier, data/log paths,
Rust package names, bridge executable/environment variables, catalog schema and
export suffix remain compatibility identifiers. SS-030 is partial until native
package/upgrade/configuration evidence establishes safe renaming or documented
aliases. Historical review documents retain their original naming context.

Both implementation-plan copies were synchronized and their obsolete "next step
is Phase 0" statement corrected. Five tickets were added: SS-029 source catalog,
SS-030 branding/compatibility, SS-031 tester release qualification, SS-032 upcoming
images/elements and SS-033 upcoming Remotion presets. Images/presets are not
implemented or required for audio v1.

SS-020 narrows baseline delivery to generic export/Remotion audio handoff/agent
instructions; direct Resolve automation is deferred. SS-022 no longer depends
on optional provider setup. SS-023/024 include source catalog qualification,
MCP bridge packaging, zero source recordings and measured package size. Existing
SS-016/017 cover the new AI tag requirement; no duplicate AI ticket was created.

## Remaining release work

Canonical order: SS-019 review, SS-029, SS-030, SS-020, SS-022, SS-023, SS-024,
SS-031. Seven unfinished tickets plus one review remain for baseline audio v1.
The count can increase when testing finds defects; it is not a delivery-date
estimate. Total backlog: 33, with 18 complete, 1 in-review, 1 partial and 13
not-started. Five optional enhancements and two upcoming asset-type tickets sit
outside the baseline path. If AI recognition ships in the tester build, its
SS-016/017 evidence becomes mandatory for that build.

The first tester candidate must be signed/installable on an explicitly tested
platform matrix, work offline without development tools, pass real native
listening/device/export and scoped installed-MCP-client workflows, preserve
backup/upgrade/source metadata and satisfy security/accessibility/resource gates.
SS-031 records artifact identity and feedback/recovery guidance. Passing source
or mock tests cannot certify an installer or prove there are no bugs.

## Validation actually run

- `npm test`: 10 files, 126 tests passed after branding changes.
- `npm run build`: TypeScript check and Vite production build passed. Frontend
  assets are roughly 339 kB uncompressed; this is not total installer size.
- `SOUNDSHELF_FFMPEG=/opt/homebrew/bin/ffmpeg SOUNDSHELF_FFPROBE=/opt/homebrew/bin/ffprobe node scripts/cargo.mjs test --workspace`:
  138 passed, 18 explicitly ignored fixture tests; no failures. Explicit media
  tool paths prevent discovery-dependent playback tests from silently skipping.
- Same command with `-- --ignored`: all 18 fixture tests passed, including
  relocated-source analysis reuse/deletion hiding, selected-file scope, worker
  restart, exact lossless clip boundaries, cancellation/disk-full/collision
  recovery, and idempotent MCP export preserving original audio.
- The initial sandboxed default Rust run failed on localhost bind in the desktop
  MCP lifecycle test. A run with local loopback permission passed; this was an
  environment limitation, not a demonstrated product defect.
- Rust emits existing vendor/tao warnings; they are not new compilation errors.
- Native playback fixtures verify decoded PCM with LoopbackSink; they do not
  establish physical-speaker output or installed-platform qualification.
- Ticket status totals and matching implementation plans checked programmatically.
- `git diff --check`: passed.

Not performed: signed package inspection/size measurement, clean-machine or
cross-platform installation, physical output-device listening, deployed client
compatibility, sidecar implementation tests, or AI recognition quality tests.
Those are explicitly tracked gates, not claimed successes.
