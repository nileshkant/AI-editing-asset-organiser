# App UI bug review — November 2026

Review and fixes: 12 November 2026, branch `claude/project-thread-itc6hf` from
main `eb7b81f` (SS-058 merged). Scope: the React/TypeScript renderer only
(`src/`): state handling, IPC calls, audio and image views, search filters,
settings and the API-key-gated AI features. Security and the Rust core are
reviewed separately. No queue ticket is started here and no IPC contract, schema
or Rust code changes.

Method: every non-test file under `src/` was read in full. Each finding below
was reproduced with a test that fails on main and passes on this branch.

## Findings and fixes

| # | Severity | Finding | Fix | Test (fails on main) |
|---|----------|---------|-----|----------------------|
| 1 | High | "Not favorites" and "Unrated" did nothing alone. `activeFilters` treated `false` as "no filter", so the gallery used the unfiltered window and never called `search_images`; "Clear all" and "Save search" stayed disabled. | `false` is empty only for plain switches (`ready_only`); `favorite`, `rated` and `alpha_channel` are yes/no/any. `src/hooks/useImageGallery.ts` | `ImageSearch.test.tsx` *counts "Not favorites" and "Unrated" as filters…*; `e2e/image-gallery.spec.ts` *"Not favorites" is a real filter…* |
| 2 | Medium | Leaving a tag, min-width or min-height field (or pressing Enter in the search box) without a change restarted the search: windows cleared, "Loading images…" flashed, scroll position lost. | The filter update returns the same object when nothing changed. `ImageSearchBar.tsx` | `ImageSearch.test.tsx` *leaves the search alone when a filter field is left unchanged…*; same e2e test (no new request while tabbing) |
| 3 | Medium | A failed first gallery window left "Loading images…" forever, with no retry. | Error state replaces the loading state and offers **Try again** (`refresh`). `ImageGallery.tsx` | `ImageGallery.test.tsx` *explains no sources, empty views…*; `ImageSearch.test.tsx` *recovers from a failed first window…* (guard; the hook already recovered) |
| 4 | Medium | The app re-rendered about 8 times a second while idle: the 120 ms playback poll and the 1.5 s jobs/sources poll stored a new object every time. One visible effect: Settings requested `folder_catalog_states` on every sources poll. | Polled values keep the previous object when their content is equal. `usePlayback.ts`, `useAudioSearch.ts` | `usePlayback.test.tsx` *does not re-render while the polled status is unchanged…*; `useAudioSearch.test.tsx` |
| 5 | Medium | Two quick annotation edits on one image could show a false "Changed elsewhere": the second edit reused the revision on screen until details reloaded. | The editor continues from the revision its own write returned; a real conflict drops it and reloads. `AnnotationEditor.tsx` | `ImageAnnotations.test.tsx` *starts a second quick edit from its own first write…* |
| 6 | Low | Arrow-key nudges at the file start or end shrank the audio selection (down to one frame). | A nudge moves the whole selection and stops at the edge. `Waveform.tsx` | `Waveform.test.tsx` *stops a nudged selection at the file edges…* |
| 7 | Low | Submitting Save Clip twice before the first reply saved two clips. | One save in flight at a time. `Waveform.tsx` | `Waveform.test.tsx` *saves one clip when the form is submitted twice…* |
| 8 | Low | With only "Duration under" set and no results, the list said "No sounds yet". | The duration filter counts as a filter for the empty state. `App.tsx` | `App.test.tsx` *says no sounds match when only the duration filter…* |
| 9 | Low | "Did you mean" replaced the first matching substring, which could be inside another word. | Whole-word, case-insensitive replacement with a literal fallback. `ImageSearchBar.tsx` (`replaceTerm`) | `ImageSearch.test.tsx` *replaces a suggested word as a whole word only* |
| 10 | Low | The AI providers pane unmounted on a settings tab change, discarding a typed key or draft, against the pane rule stated in `SettingsView.tsx`. | Mounted on first open (still no call before that), then kept. `SettingsView.tsx` | `SettingsView.test.tsx` *opens AI providers on demand and keeps a typed key…* |
| 11 | Low | Unmute did nothing after the volume slider was dragged to 0; the waveform hint said a plain click creates a selection (it seeks). | Unmute returns to the last audible level; hint corrected. `usePlayback.ts`, `Waveform.tsx` | `usePlayback.test.tsx` *unmutes to the last audible level…* |

## Checked, no change needed

- **AI key gating.** Settings → AI providers lists every image AI feature with
  what it needs ("Add an AI provider and API key to enable", "Add an API key
  for …", "Run Test connection …") and marks it `aria-disabled` until ready.
  Test connection is disabled without a key for hosted providers. Nothing in
  the renderer sends library content to a provider.
- **Stale replies.** Audio search, gallery windows, previews, details,
  import previews and playback status all fence late replies by request id,
  generation or revision; preview blob URLs are revoked.
- **Revision-fenced writes.** Clips, annotations, duplicates, generation
  provenance and provider settings send the revision last read and surface
  `CONFLICT` instead of merging.

## Left as is (noted, not fixed here)

- `McpSettings` uses `window.confirm` for token rotation; native webview
  support differs by platform and needs a check in the packaged app.
- Clip "Select" and a waveform click start playback when the sound is not the
  one playing (they seek through play). Behaviour kept; it may be intended.
- A tag typed but not added is dropped by "Save changes" in audio details.
- The import preview's Confirm button checks expiry only when it renders.

## Verification

Run on Linux, Node 22, 12 November 2026:

- `npm test`: 31 files, 344 tests pass (main: 30 files, 332 tests).
- Each new test was run against main's source and failed there (12 tests; the
  hook-level retry test is a guard and passes on both).
- `npm run build` (`tsc --noEmit` and `vite build`): pass.
- `npm run test:browser` (Chromium): 25 pass, including the new filter test,
  which fails on main at "239 images match".
- Settings open for 6 s on the renderer fixture: `folder_catalog_states`
  requests went from 4 to 0.
- Not run locally: `cargo test` (no Rust change; CI runs it on three systems).
