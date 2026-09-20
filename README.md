# SoundShelf

A local-first desktop audio library. Tauri 2 hosts a React interface and a Rust service layer. User recordings are never part of this repository.

## Delivery status

Under active development. No production release has been qualified yet. See `tickets/README.md` for the incremental delivery backlog and `docs/reviews/` for test and review evidence. A checked-in feature is not necessarily a release-qualified feature.

## Principles

- One installed desktop app, not a browser tab.
- Local indexing, typo-tolerant search, playback, tags, comments and clipping need no AI key.
- Optional AI is disabled initially. Provider credentials and model capabilities are checked separately.
- Sound IDs and analysis are independent of a folder's current root path.
- Original files remain untouched. Deleted media is hidden; offline sources retain their metadata.
- UI and MCP use the same services and validation.

## Contribution workflow

Each ticket gets a branch, tests, review notes and a pull request. Dependent branches may be stacked until their parents are merged. Never squash unrelated features into a single PR. Never commit source recordings, local databases, credentials, compiler caches or bundled vendor binaries without the release/license review.

## Development

Development requires Node.js, Rust, platform build tools and FFmpeg. Release packages must bundle qualified media tools so end users do not install these themselves. Platform signing, release builds and bundled media licensing are release gates, not implied by local development success.
