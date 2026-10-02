# Playback pause and tab loading regression fixes

Base: merged main 830739f (PR #12). Branch: fix/playback-pause-and-tab-loading.

The transport read state via a React state updater and immediately branched on a local variable before that updater ran. It could issue playback_play and reset position on a Pause click. The hook now reads a current state ref, serializes pending play/pause/resume commands, and rejects status responses captured before a control command. Row controls share this hook and surface IPC errors.

Search results are bound to the query/page that produced them. A changed query immediately hides the previous list, including during the 180ms debounce, and displays a custom waveform animation with an accessible loading label and reduced-motion support. Returning to a tab issues a fresh query; late responses cannot overwrite it. Navigation clears selection/focus; pagination is disabled during loading. Search failure ends loading with an error and empty results.

Validation:
- npm test: 117 passed, including eight added regression cases (transport pause/resume preserving position, row pause/resume, immediate loader during a deferred favorites request, rejecting a late tab response, preventing stale inspector reopening, and command serialization/status reconciliation/failure recovery).
- npm run build: passed.
- SOUNDSHELF_FFMPEG=/opt/homebrew/bin/ffmpeg SOUNDSHELF_FFPROBE=/opt/homebrew/bin/ffprobe node scripts/cargo.mjs test --workspace --locked -- --include-ignored: passed, including real FFmpeg fixtures and two new stereo-frame alignment unit tests.
- git diff --check: passed.

Review found that clearing shared search error on every refresh erased selected-file import failures; removed that reset and verified the import regression suite. Actual audible device behavior and installed Windows/Linux UI were not exercised by these mocked UI checks. Native audio callbacks now defer consuming an incomplete stereo frame until all channel samples are available. This preserves channel alignment and exact frame counts across underruns. Deterministic callback/loopback tests reproduce the partial-frame condition; real-media end detection also passes.

CI follow-up: the first PR revision exposed stale passive-effect refs in keyboard playback and a row test whose mock kept reporting playing after pause. Refs now update with the render and the mock follows actual pause/resume state. Full-media qualification also exposed a race where callbacks consumed half a frame while the decoder published channels individually; the added alignment tests cover that fix. Search selection lookups now cancel on navigation, preventing stale details reopening. CI is rechecked after this revision is pushed.
