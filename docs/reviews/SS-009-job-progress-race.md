# SS-009 job progress race review

Finding: `enqueue` sent a source to the worker before recording its queued progress. A fast worker could publish `discovering` or `analyzing`, after which the caller overwrote that state with `queued`.

Fix: publish the queued state before sending. On a full bounded queue, remove only the queued entry that this enqueue created and return a clear error. The worker can now only advance progress after the initial state exists.

Validation: workspace Rust tests and the Tauri compile remain required in the parent SS-009 review. This branch contains a service-only ordering fix; a dedicated concurrent enqueue test should be added when the service receives its isolated test harness.
