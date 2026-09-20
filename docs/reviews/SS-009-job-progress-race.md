# SS-009 job progress race review

Finding: `enqueue` sent a source to the worker before recording its queued progress. A fast worker could publish `discovering` or `analyzing`, after which the caller overwrote that state with `queued`.

Fix: publish the queued state before sending. On a full bounded queue, remove only the queued entry that this enqueue created and return a clear error. The worker can now only advance progress after the initial state exists.

Validation: `node scripts/cargo.mjs test --workspace --locked`, `npm test`, and `npm run build` passed on macOS ARM64. This branch contains the service ordering fix plus conservative reconciliation and release-resource declarations. A dedicated concurrent enqueue test should be added when the service receives its isolated test harness.
