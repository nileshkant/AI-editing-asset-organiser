# Merged-main hardening and keyboard usability review

Date: 2026-10-16. Branch: `fix/workspace-usability-review`.
Reviewed base: `5084baad5d472e7e5cf193fb655b4534a2a4fb70` (`origin/main`, merged PR #37).

## Scope and provenance

Fetched main and traversed the merged delivery history, including the playback,
waveform, clip/export, selected-file imports, portable/source catalogs, MCP,
recovery, release qualification and Settings follow-ups. The retained
[history](evidence/WORKSPACE-HARDENING/merged-history.txt) identifies the revisions.
Review concentrated on their current shared services, desktop IPC and renderer
integration, with the full existing regression suite covering the delivered
features. Historical PR discussion threads were not fetched; the local Git
history and merged implementation were the review sources.

The initial workspace already contained edits in `crates/core/src/source_catalog.rs`,
`crates/core/tests/source_catalog.rs`, `docs/PORTABLE_CATALOG.md`, the untracked
`docs/reviews/SOURCE-SNAPSHOT-SIZE.md`, and `.mcp.json`. They were preserved.
The source-snapshot changes were included in the tested working tree and remain
separate from the fixes authored here. No live user catalog or source recordings
were used for testing. Release ticket acceptance states were not promoted. The publication follow-up
excludes those pre-existing edits and validates the isolated PR tree separately.

## Demonstrated major defects and fixes

- **P1 — waveform cache path escape.** Portable catalog version identifiers are
  opaque strings, but `cache_path` interpolated them directly into paths. Traversal
  and absolute identifiers could address `.sswf` files outside the cache. Cache
  identifiers now become a single digest filename; existing hexadecimal digest
  cache names remain usable. The traversal regression fails on the original code
  and passes after the fix, including Windows separators and absolute paths.
- **P1 — invalid clip previews and escaping clip seeks.** Desktop preview accepted
  unrelated sound/clip IDs and stale recipes without checking boundaries. Shared
  catalog validation now checks ownership, current source, rate and frame range.
  Playback keeps source start separate from clip-relative position. Seeking retains
  the clip's bounds, pause state and duration, including seeking to the end of a
  very short clip. A real FFmpeg fixture makes only the selected source region
  silent: the regression checks the decoded samples, timeline and 14,400 remaining
  frames after a seek. The initial implementation reported 0.5 seconds at the
  beginning of a clip; the test failed before the fix.
- **P1 — transient native audio errors poisoned playback.** The real macOS smoke
  test exposed a CoreAudio underrun being latched as a permanent error while the
  stream remained usable. CPAL `Xrun` now increments the underrun counter. Actual
  device failures still surface as errors. A unit regression covers both cases,
  and the rebuilt native app subsequently reached the correct full-file and clip
  endpoints without the latched error.
- **P1 — metadata overwrite from stale rows.** Favoriting used the search row's
  copies of tags/comments, and saving comments used the inspector's old favorite
  value. Independent annotation patches now read unmodified fields from current
  catalog state while holding the catalog lock. Native restart and core tests
  verify the saved comment and favorite survive both operations.
- **P1 — decimal clip entry was difficult or impossible.** Formatting every
  keystroke with `toFixed(3)` moved the caret and rewrote partially entered
  decimals. Boundary inputs retain editing text and normalize on blur/Enter.
  Typing `0.5` and `1.25` now produces `[24000,84000)` at 48 kHz. The keyboard
  regression failed with the original implementation; native WebKit entry was
  also exercised.
- **P1 — keyboard controls could trigger the wrong action.** Global Space/Enter
  handlers intercepted native buttons. Arrow navigation selected results without
  moving actual focus, and details required tabbing through many result controls.
  Native controls now own their activation keys. Result arrows/Home/End move and
  scroll actual focus, Enter opens details, details receive focus, and closing
  returns it to the selected result. Modified shortcuts and form editing remain
  with their owners; `/` can focus search from result buttons.

## Additional regressions fixed

- Relink uses a native modal dialog, restores its opener, traps Tab/Shift-Tab,
  supports Enter/Escape, prevents duplicate submission, and shows errors inside
  the dialog. Source-change confirmation uses native modal background isolation
  and restores focus after dismissal.
- Cancelling imports no longer cancels subsequent waveform decoding. Waveform
  cancellation follows desktop shutdown independently of the import flag.
- Deferred detail responses and annotation saves cannot reopen closed details or
  replace a different selection. Removed the duplicate selected-sound fetch.
- Same-query refreshes retain mounted rows, preserving focus after metadata saves.
  Changed queries still hide stale results while waiting for the new response.
- Play/Stop errors restore UI state; playback can retry after backend errors.
  Polling allows only one status request at a time. Seeks are serialized and
  coalesced to the latest requested position, and pre-seek status replies cannot
  reset the timeline. Pending transport controls expose their disabled state.
- Waveform shortcuts no longer consume typing or number-input cursor keys.
  Fit-to-selection uses the new zoom when calculating pan. Superseded tile
  requests are invalidated during cleanup. Clip playback errors are surfaced.
- Replaced interactive listbox options with list items containing explicitly
  labelled buttons. Added a skip link, shortcut help, live announcements and an
  explicit All sources action. Text and transport spacing improve small-window
  use; details reclaim list-only space in the narrow layout.
- Updated transitive `source-map-js` from 1.2.1 to 1.2.2 after the lock audit
  reported GHSA-68fv-2mgg-jv7q. The final npm audit has zero findings.

## Automated evidence

- `npm test`: **182 passed across 19 files**. This includes the existing release
  script tests; adding browser tests did not remove them from Vitest discovery.
- `npm run build`: **passed** TypeScript and Vite production build.
- `DEVELOPER_DIR=/Library/Developer/CommandLineTools SOUNDSHELF_FFMPEG=/opt/homebrew/bin/ffmpeg SOUNDSHELF_FFPROBE=/opt/homebrew/bin/ffprobe node scripts/cargo.mjs test --workspace --locked -- --include-ignored`:
  **206 passed, 0 failed, 0 ignored**, including generated real media, catalog
  migration/recovery, import scopes, MCP, export and performance scenarios.
- `npm run test:browser`: **5 passed** in Chromium. Includes axe WCAG A/AA checks
  for the library, inspector, all six Settings panes at **1280×820 and 720×560**,
  relink and source confirmation, plus native button activation, Tab/Shift-Tab,
  result arrows/Home/End, Settings tabs, focus return, decimal typing, metadata,
  filters and transport. No axe rules were disabled. The browser fixture models
  IPC explicitly and does not establish native audio correctness.
- `npm audit --json`: **0 vulnerabilities**.
- `DEVELOPER_DIR=/Library/Developer/CommandLineTools node scripts/cargo.mjs build -p soundshelf --locked`:
  **passed**; that development binary was launched for the native smoke checks.
- `git diff --check`: **passed**.

[Raw logs, before/after regressions, screenshots and native export facts](evidence/WORKSPACE-HARDENING/)
are retained alongside [checksums](evidence/WORKSPACE-HARDENING/checksums.json).
Six targeted keyboard/waveform tests were also run in a disposable copy with the
original merged implementations: **all six failed**, and all pass in the final
suite. The clip and cache regressions have separate original/fixed logs.
A browser accessibility job now retains reports/screenshots/traces in CI; its
remote execution awaits a pushed revision.

Environment: macOS **27.0.1** build **26A434**, Apple Silicon, Node **24.21.0**,
npm **11.19.0**, Cargo **1.98.1**, FFmpeg/FFprobe **9.0.2**. Default Xcode
compilation initially failed because its license was not accepted. Selecting the
already-installed Command Line Tools with `DEVELOPER_DIR` enabled the checks;
no license agreement or system preference was changed.

## Native macOS smoke observations

Launched a separately identified development app with a disposable data directory.
Imported a generated two-second, 48 kHz stereo WAV using the native file picker.
Imports reported **1/1 complete, 0 failed**. Tab reached Filters, Space opened it,
and Tab through its controls reached the sound; Enter opened details. Saved a
comment, typed decimal boundaries, and saved a half-second clip `[24000,48000)`.

Exported using the native Save dialog. The UI reported **24,000 frames** and
indexed the export. Independent FFprobe inspection reports **48,000 Hz, two
channels, 24-bit PCM and 24,000 duration frames**; the adjacent manifest agrees.
Favoriting preserved the comment. Restarting retained the comment, favorite,
clip and exported sound.

After applying the underrun fix and rebuilding, native device playback advanced
through the original recording and reported **Finished 0:02.00**. Clip preview
advanced from **0:00.09 of 0:00.50** to **Finished 0:00.50**. Output metering was
nonzero during playback. Listening confirmation, device-switching and prolonged
physical audio use remain qualification work. Native source confirmation isolated its background accessibility tree; Escape
cancelled safely and returned focus to the button when opened using the keyboard.
The final rebuilt executable also completed the half-second preview. Its hash and
observations are in `native-smoke.txt`. The disposable review app was closed afterwards.

## Remaining qualification

Installed/signed Windows and Linux packages, their OS webviews, Narrator/Orca,
VoiceOver interaction, display scaling/zoom beyond the tested viewport sizes,
long-session native audio, and external editor/client workflows require their
existing release qualification checks. Chromium axe results and a local native
macOS development run do not complete SS-023/024/031/034 or the outstanding
installed accessibility acceptance. No new feature tickets were started.

## Publication verification

Before opening the PR, code commit `6f4b852bf1ac0bfaea8fcf94f61de29455ff86dd` was tested in a clean,
detached Git worktree. This excludes the pre-existing source-snapshot changes
and all unrelated untracked files. Results: **182 frontend/release tests,
204 Rust/media tests, 5 browser tests, 0 failed, 0 ignored**; production
frontend build passed. The earlier 206-test Rust result and native smoke
observations above describe the initial review workspace, which contained two
additional source-snapshot tests. They remain historical evidence and are not
attributed to the isolated PR tree.

[Isolated PR logs and checksums](evidence/WORKSPACE-HARDENING/PR-VALIDATION/)
record the exact code commit and commands. GitHub Actions will also verify the
pushed PR head; installed platform qualification remains unchanged.
