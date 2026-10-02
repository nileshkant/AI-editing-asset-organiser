# Playback pause and tab loading regression fixes

Base: merged main 830739f (PR #12). Branch: fix/playback-pause-and-tab-loading.

The transport read state via a React state updater and immediately branched on a local variable before that updater ran. It could issue playback_play and reset position on a Pause click. The hook now reads a current state ref, serializes pending play/pause/resume commands, and rejects status responses captured before a control command. Row controls share this hook and surface IPC errors.

Search results are bound to the query/page that produced them. A changed query immediately hides the previous list, including during the 180ms debounce, and displays a custom waveform animation with an accessible loading label and reduced-motion support. Returning to a tab issues a fresh query; late responses cannot overwrite it. Navigation clears selection/focus; pagination is disabled during loading. Search failure ends loading with an error and empty results.

Validation:
- npm test: 113 passed, including four added regression cases (transport pause/resume preserving position, row pause/resume, immediate loader during a deferred favorites request, and rejecting a late tab response).
- npm run build: passed.
- node scripts/cargo.mjs test --workspace --locked: passed; media-tool fixtures remain ignored under this command.
- git diff --check: passed.

Review found that clearing shared search error on every refresh erased selected-file import failures; removed that reset and verified the import regression suite. Actual audible device behavior and installed Windows/Linux UI were not exercised by these mocked UI checks. No native playback/media changes.
