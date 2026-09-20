# SS-009 bug review follow-up

Independent review findings and resolutions:

- Resource packaging: addressed by keeping development Tauri config free of absent resources and adding a release-only config that requires audited `media/ffmpeg` and `media/ffprobe` files. Packaging is still a release gate; no binaries are committed.
- Stale file after a discovery race: addressed by adding a path to reconciliation's `seen` set only after containment and hashing succeed. A file removed before verification can now become missing after a complete scan.
- Queue progress ordering: addressed on `fix/SS-009-job-progress-race` by recording `queued` before sending work to the bounded queue.
- Individual file drops: intentionally rejected with an explicit folder-only message in this increment. SS-026 owns the source-scope/data-model work required to import selected files without silently importing siblings.
- FFprobe output bounds: the bounded reader returns an error as soon as the 1 MiB limit is exceeded and `run_stream` kills/reaps the child on reader failure. The existing timeout/kill fixture covers a stalled child; a future fuzz fixture should exercise malicious metadata output directly.

Verification after fixes: `npm test` 2 passed; `npm run build` passed; `node scripts/cargo.mjs test --workspace --locked` passed 29 non-ignored tests, with 2 explicit FFmpeg integration tests separately passing when run with local Homebrew FFmpeg. Cross-platform packaging remains unverified on this macOS host.
