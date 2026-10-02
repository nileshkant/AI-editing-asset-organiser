---
name: soundshelf-development
description: Implement and review SoundShelf desktop tickets with verified local-media behavior, stable catalog identity, scoped file access, and test evidence. Use for this repository's Rust core, Tauri commands, React UI, migrations, and CI fixes.
---

# SoundShelf development

Locate the repository containing `crates/core`, `src-tauri`, `src`, and `tickets`. Read `tickets/README.md` and the selected ticket before implementation; consult `docs/PRODUCT_PLAN.md` and `docs/DECISIONS.md` only for relevant design questions. The backlog's execution queue defines the order. Verify prerequisite completion against Git history and PR status because ticket metadata can lag merges.

Create one feature branch for the ticket from reviewed main or its necessary prerequisite branch. Keep unrelated tickets out of its PR. Record a demonstrated blocker rather than silently choosing an easier ticket. Respect the user's choice of ticket, branch, and scope. This skill grants no additional permission to publish, merge, send messages, or access private external systems.

## Architecture and invariants

- `crates/core` owns catalog, source containment, search, media analysis, jobs, playback, waveform and export rules. Keep UI and future MCP callers on these shared services; do not duplicate validation in an adapter.
- `src-tauri/src/service.rs` owns worker lifecycle and queues. Tauri commands in `src-tauri/src/main.rs` adapt IPC; `src/api.ts` and `src/types.ts` define the React contract. Check both serialization directions, including camelCase command arguments and string-encoded sample counters.
- Source IDs survive relocation. A sound's identity is distinct from its content hash; identical media at different paths can share analysis while retaining independent annotations. Same-path replacement preserves user metadata and invalidates version-bound recipes.
- Original recordings are immutable. Resolve media through its source scope and canonical containment, reject traversal/symlink escapes, and never broaden an explicit file selection to sibling files without confirmation.
- Publication requires verified mandatory analysis. Detect source generation/content changes before publishing. Reconcile missing files only after complete discovery; cancellation and discovery errors must not mass-delete metadata.
- Schema upgrades are transactional and covered by migration/preservation/failure tests. Keep recovery possible with a compatible rollback or pre-migration backup. Do not edit a user's live catalog to validate migrations; use disposable fixtures.
- Decode, file hashing and export validation use bounded buffers outside long-lived catalog locks. Bound queues/concurrency and report overload. Measure a relevant performance scenario before claiming an optimization; preserve existing benchmark budgets unless a separate requirement changes them.
- Clip selections use inclusive start/exclusive end in source sample frames. Validate integer arithmetic, source version/rate, and explicit clamping feedback. Exports default to lossless WAV, verify staged output, commit without replacing existing files, and clean only owned temporary artifacts.

## Evidence and completion

Map each acceptance criterion to observable code behavior and a relevant test or documented manual check. Cover negative paths affecting this ticket, not just its happy path. Use real generated FFmpeg fixtures for media claims; mocks cover IPC/UI state but cannot establish frame accuracy or actual audio decoding.

Use project scripts so the repository's local Rust toolchain is selected:

```sh
npm test
npm run build
node scripts/cargo.mjs test --workspace --locked
```

For media verification, resolve actual native FFmpeg/FFprobe executables, set `SOUNDSHELF_FFMPEG` / `SOUNDSHELF_FFPROBE`, and run Rust tests with `-- --include-ignored`. On Windows, use native paths and actual binaries rather than Git Bash paths or launcher shims; watchdog cancellation must reach the decoder. `npm run check` is a convenient baseline but does not enable ignored media fixtures by itself.

Run focused checks while iterating, then the required regression suite. Inspect the final diff. Write actual commands/results, review findings/fixes, and platform/manual limitations in `docs/reviews/<ticket>.md`. A UI control, a skipped fixture, or local macOS success does not establish cross-platform acceptance. Keep unverified criteria open. Mark a ticket complete only after it is reviewed and present in main; use the backlog's intermediate states accurately.

When publication is authorized, push the tested branch, open/update its ticket-specific PR, and verify CI on the exact pushed revision. Follow any new failures to their demonstrated cause; preserve meaningful tests rather than disabling them or widening budgets. Link the PR in ticket metadata. Leave merging to explicit user authorization. Never commit recordings, live databases, credentials, compiler caches, or media-tool binaries.
