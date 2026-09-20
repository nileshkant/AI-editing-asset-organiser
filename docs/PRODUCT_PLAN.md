# SoundShelf: Product and Engineering Plan

Status: proposed implementation specification, not an implemented application.
Prepared: 2026-09-20. SoundShelf is a working name, subject to naming review.
Scope: an installable, local desktop audio library for macOS, Windows, and Linux, with background analysis, reliable playback, precise clip export, and an MCP server that runs with the app.

## 1. Product Decisions

The application starts with an empty library. Users install it, drop files or folders, and search and audition the completed results. It has no dependency on this repository, its paths, this sound pack, or a development server.

The recommended stack is Tauri 2, React, TypeScript, a Rust application core, SQLite, bundled FFmpeg/ffprobe executables, and native audio output. One local service layer supports both the interface and MCP. The core product needs no account, subscription, API key, Python, Node, Homebrew, Docker, or separately installed FFmpeg.

Core decisions:

- Local files remain local. Cloud AI is separately enabled, scoped, and optional.
- Read source media by reference by default. Never edit originals during analysis, playback, or clipping.
- Default import is recursive for folders; individually dropped files do not silently import their siblings.
- Publish each successfully analyzed file individually. Do not wait for an entire folder to finish before publishing its completed files.
- Pending and failed files appear in Imports, not ordinary Library search or default MCP results.
- Save clip definitions without copying audio; materialize a new media file when the user exports.
- Compute real waveform data. The waveform, spectrum, and player clock have different meanings and separate rendering paths.
- Treat measured properties, filename claims, model predictions, and user corrections as different evidence.
- Use a local database for search. JSON and Markdown remain portable exports, not the live database.
- Support agents while the application is running, including an explicit tray mode. Fully quitting stops MCP.
- Production completion means passing platform, audio, recovery, accessibility, security, and installation checks, not merely compiling an executable.

### Supported release targets

Proposed launch targets: macOS 14.2+ on Apple Silicon and Intel; Windows 11 on x64 and ARM64; Ubuntu 24.04 LTS and a pinned supported Fedora release on x64, with Linux ARM64 builds included only after equivalent hardware tests. Windows 10 and earlier macOS versions are compatibility candidates, not initial support claims. The exact CPU/OS matrix is locked after the first packaging and audio spike.

The proposed macOS floor accounts for the currently documented CPAL CoreAudio requirement. Each released dependency version must be pinned; the floor must be rechecked against that version. [CPAL platform support](https://github.com/RustAudio/cpal)

"No separate application dependencies" is achievable. "Runs on every operating system with no system runtime" is not an honest promise: Tauri uses system webviews and audio services. Windows packages can provision WebView2; Linux packages must handle their supported WebKitGTK and audio runtime requirements. We will provide a fully offline installer variant where necessary. [Tauri Windows installers](https://v2.tauri.app/distribute/windows-installer/), [Tauri Linux AppImage packaging](https://v2.tauri.app/distribute/appimage/)

## 2. Existing Playback Failure and Migration Risks

The reported silence is not yet reproduced with an audible output-device test. Code inspection identifies these concrete risks in `tools/build_sound_catalog.py`:

- `startVisualizer()` creates an AudioContext and routes playback through it without calling and awaiting `resume()`.
- The static page serves audio through file URLs. Routing a CORS-cross-origin media element through Web Audio is required to output silence. This is a possible explanation, not proof of the user's exact failure. [Web Audio security requirement](https://www.w3.org/TR/webaudio/#MediaElementAudioSourceNode-security)
- The early return for an existing analyser prevents restarting the drawing loop after pause/resume.
- List rendering replaces audio elements, so changing a filter or favorite can destroy the active player.
- Every playback error is treated as file deletion. An unsupported codec or output problem must not remove a valid sound.
- Idle visualizer bars are generated from an ID; they are not a waveform of the file.

First implementation checkpoint: reproduce playback using a known audible WAV and an affected MP3; inspect muted state, context state, source URL, decoded samples, system output device, and output meter. For a transitional repair, separate playback from optional visualizer attachment, resume the context in the user gesture, preserve one player, and distinguish playback errors from missing files. The new application uses native output and will not rely on browser Web Audio routing for sound.

Other legacy issues to address during migration:

- Only the first 20 seconds are measured, after conversion to 12 kHz mono. These are sampled measurements, not full-file loudness or high-frequency spectral analysis.
- Zero-crossing rate is not a reliable pitch estimator or proof of a sound being tonal.
- Stereo downmix can cancel opposite-phase channels. Measure channels separately before computing any combined energy.
- Present descriptions mostly restate titles and coarse metrics. They are not evidence that a model listened to the recording.
- Current high/medium labels are heuristic metadata matches, not calibrated recognition probabilities.
- The cache is updated before old codec metadata is reused; changed files can retain stale duration or format metadata, and new files can lack it.
- A changed pathname changes an ID. The new data model separates stable identity, file versions, and location.
- Root selection, overlap handling, and cache keys currently use inconsistent rules.
- There is no durable job queue, migration system, signed installer, or robust local authorization layer.

Migration is explicit: Settings > Data > Import legacy catalog. Original sounds are registered as sources; old descriptions are imported with `legacy_filename_inference` provenance. Existing measurements are labeled with their limited coverage and remeasured in the background. No source media is copied into the app package or shipped to other users.

## 3. Technology Choices and External Modules

### Application shell and interface

- Tauri 2: native windowing, lifecycle, drag/drop, dialogs, system paths, single-instance coordination, packaging, and signed update support. Bundle platform-specific audio binaries as sidecars. [Tauri sidecars](https://v2.tauri.app/develop/sidecar/)
- React + TypeScript + Vite: compile a local UI with typed components and commands. No frontend runtime server or remote assets in the installed product.
- Radix primitives: accessible dialogs, menus, popovers, tooltips, segmented controls, and focus management.
- TanStack Virtual: render visible library rows, not 100,000 DOM nodes.
- TanStack Query: paginated server state and event-driven invalidation. Zustand: transient selection, layout, and transport display state only.
- Lucide: consistent icons with names, tooltips, and accessible labels.
- CSS design tokens and small CSS/WAAPI transitions: no large animation engine unless a specific interaction demonstrably needs it.
- Canvas 2D waveform renderer: tiled min/max and RMS envelope drawing from cached binary peaks. HTML overlays provide accessible selection handles and numeric alternatives. WebGL is reserved for a later dense spectrogram if profiling justifies it.

### Rust core

- Tokio for orchestration, bounded channels, timers, and subprocess management. CPU-heavy analysis uses a bounded worker process, not the async executor or renderer thread.
- `rusqlite` with bundled SQLite and FTS5: a serialized writer plus short-lived/read-pool queries. Use typed repositories and migrations; no generic ORM or separate database service. [Rusqlite](https://docs.rs/rusqlite/latest/rusqlite/)
- `notify` for native filesystem events and its polling fallback for sources with unreliable watches. Event monitoring is supplemented by reconciliation. [Notify limitations](https://docs.rs/notify/latest/notify/)
- CPAL for system audio output. Start with CoreAudio on macOS, shared-mode WASAPI on Windows, and the qualified Linux backend. No ASIO driver installation or exclusive audio mode at launch.
- `ringbuf` for a bounded single-producer/single-consumer PCM buffer; `rubato` for tested resampling to the selected output device's rate. [Ringbuf](https://docs.rs/ringbuf/latest/ringbuf/), [Rubato](https://docs.rs/rubato/latest/rubato/)
- `rustfft` for spectra; `ebur128` for standards-based loudness measurements. Maintain explicit units and channel layouts. [RustFFT](https://docs.rs/rustfft/latest/rustfft/), [Ebur128](https://docs.rs/ebur128/latest/ebur128/)
- `serde`, `uuid`, `blake3`, `tracing`, a bounded log writer, and `thiserror`: serialization, stable IDs, content integrity, diagnostics, and typed errors.
- `keyring` for optional API keys and client secrets; persistent storage only after verifying the selected OS backend is durable. If the OS store is unavailable, allow session-only credentials, never plaintext fallback. [Keyring](https://docs.rs/keyring/latest/keyring/)
- Official Rust MCP SDK `rmcp`, with its supported HTTP integration, for protocol handling and stdio bridge support. [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)

### Media tools

- Bundle FFmpeg and ffprobe built for each target architecture. They supply broad decoding, metadata, stream selection, sample-based trimming, fades, and export encoders. Use fixed argument arrays, never shell command interpolation. [FFmpeg filters](https://ffmpeg.org/ffmpeg-filters.html)
- Build a minimal audio-oriented distribution. Disable GPL/nonfree options for the intended LGPL build; audit every included library, encoder, and build flag; ship matching source/build instructions and notices. Release blocks on license compliance. [FFmpeg license guidance](https://www.ffmpeg.org/legal.html)
- A compiled `soundshelf-worker` executable orchestrates analysis and is restartable independently of the UI. The playback decoder and export decoder have separate process lifetimes and priorities.
- The installed app locates its own binaries, never a random executable on PATH or inside another app.

### Optional model packs

- ONNX Runtime, or a qualified equivalent native runtime, is packaged only with the optional local model feature. CPU execution is the baseline; GPU acceleration is optional and must not require users to install drivers for basic operation. [ONNX Runtime C interface](https://onnxruntime.ai/docs/get-started/with-c.html)
- Evaluate an audio-event classifier and a CLAP-family audio/text embedding model. These are AI even when entirely offline and keyless. PANNs and CLAP are research candidates, not approved binaries or promised accuracy. Model-weight redistribution, conversion correctness, inference speed, and event coverage each need qualification. [PANNs](https://github.com/qiuqiangkong/audioset_tagging_cnn), [CLAP research implementation](https://github.com/microsoft/CLAP)
- Do not bundle Python or a full training environment merely to run inference. Conversion and training tools are build-time tooling only.
- Do not assume a repository's code license grants redistribution of every linked checkpoint or training asset.
- The referenced Microsoft CLAP repository is archived at the time of this review. Use it as research evidence; it is not the default maintained runtime dependency. Candidate selection must include maintenance and model-weight review.

### Alternatives not selected

- Electron: a credible fallback if system-webview compatibility blocks release; its bundled browser/runtime works against the small-package target. Do not switch just to avoid testing native audio.
- A browser-only/PWA app: insufficiently consistent background folder access, filesystem watching, local MCP lifecycle, and native editor handoff across the requested platforms.
- A packaged Python server: unnecessary runtime, process, and installer overhead for this product; also perpetuates the present folder-bound design.
- Three separate native UIs: substantially more duplicated work to deliver the same library/selection experience.
- FFmpeg compiled to WebAssembly for the core pipeline: increases renderer memory and complexity for work that can run natively.
- Browser `decodeAudioData()` as the primary playback path: whole-buffer decoding is unsuitable for long recordings and reintroduces webview-specific media behavior.
- A separate vector database server: unnecessary for a local desktop library. Optional vector search begins as a versioned local index.
- Automatic cloud AI: unnecessary for playback, indexing, trimming, filtering, and agent access.
- WaveSurfer as the audio engine: introduces a browser media clock competing with the native transport. The waveform renderer consumes the native clock and cached peaks instead.

Dependency versions are locked after the compatibility spike, with lockfiles, an SBOM, license checks, and a review/update policy. There will be no production dependency on an unpinned `latest` release.

## 4. Application Architecture and Module Boundaries

```text
Desktop UI (React/TypeScript)
    | typed commands, bounded event subscriptions
    v
Rust application service
    |-- Library/source service
    |-- Search/query service -------- SQLite + FTS5
    |-- Durable job scheduler ------- SQLite jobs + leases
    |      |-- analysis worker ------ bundled FFmpeg/ffprobe
    |      |-- optional model worker
    |      `-- export worker -------- atomic output files
    |-- Native playback engine ----- PCM ring buffer -> CPAL -> speakers
    |-- Waveform cache service ------ binary peak tiles
    |-- Agent service --------------- local MCP + compiled stdio bridge
    |-- Editor handoff service ------ exported files + manifests/adapters
    `-- Settings/secrets/diagnostics/update services
```

Boundary rules:

- The UI cannot run arbitrary commands, execute SQL, or traverse arbitrary paths. Commands operate on registered IDs and explicit user selections.
- UI commands and MCP tools call the same validation and domain services; there is one definition of filtering, selection, export, and availability.
- Audio callback code does no file I/O, network I/O, logging, database operations, heap allocation, or blocking lock acquisition.
- Decoder crashes fail a job or preview; they must not crash the library window.
- DB transactions are short. No transaction remains open while a file is decoding or an AI request is in flight.
- The service owns file identity and source authorization. Workers receive only a specific job's resources.
- Every event carries an entity ID, revision, and job generation. Stale events cannot overwrite newer state.
- UI progress is coalesced to approximately 5-10 updates per second. Meters may use a separate bounded stream at 20-30 Hz; drawing interpolates at display refresh rate.
- Long-running commands return a job ID immediately. UI windows and MCP connections can reconnect and obtain a durable job snapshot.

Proposed repository layout:

```text
apps/desktop/                 React interface, tokens, accessibility, UI tests
apps/desktop/src-tauri/       Tauri host, capabilities, platform packaging
crates/domain/               identities, units, selection/filter contracts
crates/storage/              schema, migrations, repositories, backups
crates/library/              sources, discovery, watches, relinking
crates/analysis/             DSP aggregation, descriptions, provenance
crates/playback/             transport, buffering, CPAL, meters
crates/waveform/             peak pyramid format and queries
crates/jobs/                 scheduling, leases, retry, cancellation
crates/export/               recipes, rendering, verification, handoff
crates/agent/                MCP tools/resources, auth, result limits
crates/models/               optional model adapters and manifests
crates/platform/             secrets, paths, file drag, device integration
bin/worker/                  isolated media worker
bin/mcp-bridge/              agent stdio connection to the running app
packages/contracts/          generated TypeScript and JSON schemas
fixtures/                    generated/redistributable audio and expected data
packaging/                   sidecar builds, signing, installers, notices
docs/                        public workflows, integration, maintenance
```

## 5. User Journeys and Publication States

### First launch

Open directly into an empty Library with two actions: Add folders and Add files. A drop target occupies the empty work area. No account, API key, or model download is required. Initial defaults are local-only processing, referenced media, normal CPU usage, and system audio output.

A successful drop opens a compact import confirmation with detected roots, import mode, recursive setting, and exclusions. Clicking Import registers the authorized sources and returns immediately. Imports then shows progress; the Library remains usable.

### Import pipeline

`discovered -> waiting_for_stable_file -> probing -> queued -> decoding -> measuring -> indexing -> ready`

Alternate states: `paused`, `cancelled`, `failed`, `unsupported`, `needs_permission`, `source_offline`, and `retry_wait`.

"Ready" means the mandatory non-AI profile has succeeded: real duration and codec data, validated decoding, waveform peaks, required measurements, and an evidence-labeled description. Optional semantic inference can improve a ready asset later without holding it hostage to a model download or cloud outage.

Publication rules:

- Only ready, available, current-version entries appear in ordinary Library and MCP search.
- Optional enhancement status is a separate field: off, queued, running, complete, or failed.
- A batch can partially succeed. Failed files remain visible in Imports with a specific reason and retry action.
- File content changing during processing cancels that generation and schedules a new one after it stabilizes.
- Refreshing an unchanged file does not duplicate analysis; a failed or incomplete stage is retryable and is never marked successful just because the signature matches.
- For a changed file, the old profile is stale and excluded from default selection until a new version is ready. Existing exported clips remain independent media.
- Unknown event meaning does not mean a corrupt file. It can be ready with `event_status=unknown` and `review_recommended=true`.

### Import by reference or managed copy

Default Reference mode indexes only the selected paths. Original deletion removes the referenced instance from usable results. Folder roots remain editable.

Optional Copy into library mode creates an app-managed original with an integrity check before indexing. This is explicit because it consumes disk and changes the meaning of deleting the external source: the managed copy remains available. Copying uses a temporary destination and atomic completion; interruption never creates a ready half-file.

Individually dropped files use a file-list source. Dropping one file does not authorize recursively scanning its containing directory.

## 6. Screen Layout and Element-Level UI

### Visual language

An audio workbench with a bold type hierarchy, neutral charcoal/off-white surfaces, green selection accents, and restrained amber/error-red status colors. Dark, Light, and System themes share semantic color tokens. No oversized marketing hero or nested decorative cards.

Use bundled fonts or native system fallbacks; no remote font fetch. Default body 14 px, secondary 12 px, section titles 18-22 px, fixed letter spacing 0. Controls 36-40 px high, icon targets at least 36 px, surfaces at most 8 px radius. Long labels wrap or truncate with accessible full text; numeric columns use tabular figures.

Motion is functional: 120-160 ms hover/selection changes, 180-220 ms inspector transitions, subtle progress interpolation. No moving wallpaper. Reduced Motion removes sliding and spring motion. All operations remain understandable without animation or color.

Primary desktop arrangement:

```text
Window/titlebar: SoundShelf                         Imports  |  Agent status
Navigation       Search + filter chips + view/sort controls
                 Library results / waveforms       Detail inspector
                 ...                                / clip controls
Persistent player: play | time | seek/waveform | volume | expand
```

Default window 1440 x 900; minimum target 900 x 600. Below 1100 px, inspector becomes an overlay and navigation can collapse. At 200% text scale, controls reflow into fewer columns. The app is desktop first; a mobile web version is outside this release.

### A. Navigation rail

Elements: Library, Favorites, Collections, Saved clips, Imports with active-count badge, and Settings. Source folders appear under Library with expandable subfolder navigation and availability indicators. The bottom contains Agent connection status and a compact storage indicator.

Selecting a source changes filters, never moves files. Context menu: Rename display name, Open in file manager, Rescan, Pause watching, Relink, Remove from library. Remove unregisters the source and its exclusive memberships; it does not delete disk files.

### B. Library toolbar and results

Search box: free text, clear icon, recent queries, optional field suggestions. Typing is debounced about 150 ms; stale queries cancel. Cmd/Ctrl+K focuses search, Escape closes suggestions before clearing anything.

Quick filter controls: source multi-select, category multi-select, duration range, character presets, and Favorites toggle. More filters opens a structured popover. Each active filter is a removable human-readable chip; Clear all restores defaults. Saved search action sits next to the filter summary.

Right-side controls: list/grid segmented icons, sort menu, column chooser in list view, Add files/folders menu. Default is dense list view, with grid available for visual browsing.

List row elements: selection checkbox, play/pause icon, title, real waveform thumbnail, exact duration, category, key tags, source name, favorite star, availability/review icon, and overflow menu. Avoid dozens of player instances. Clicking a title opens its inspector; clicking play changes only the persistent transport.

Grid item elements: title and favorite, real waveform, concise description, duration, two or three priority tags, play button, overflow. Secondary tags stay in the inspector. All labels replace internal underscores with spaces; stored slugs remain stable.

States: initial empty library, import in progress, no search matches, source offline, failed query with retry, partial results updating. Pending imports are not fake ready rows. Results include match count, optional match explanation, stable selection across new arrivals, and cursor-based pagination.

Selection behavior: single click, Shift range, Cmd/Ctrl multi-select, select visible results, and explicit select all matching. Bulk actions: tag, favorite, collection, export, reanalyze, or remove catalog entries. Physical file deletion is not a launch feature.

### C. Persistent minimal player

Always mounted independently of the result list. Elements: current title, Play/Pause, compact seek waveform, elapsed/total time, Mute, volume slider, Expand, and overflow. Space toggles playback when no text field or dialog owns focus.

Behavior: only one preview plays at a time. Selecting another sound replaces the transport source; filtering and favoriting never interrupt it. Play loading state is distinct from playing. Surface missing device, unsupported file, and missing source as different errors.

The spectrum is small and optional. It represents measured frequency-band energy, not pitch and not a randomly animated equalizer. Waveform is persistent file amplitude data; spectrum is live playback data.

### D. Detail and editing inspector

Resizable side panel with tabs: Overview, Clip, Metadata, and History.

Overview: editable display title, evidence-labeled description, source breadcrumb, category/tag editor, suggested editorial uses, duration, channels/rate, loudness, and analysis coverage. An evidence disclosure explains whether each semantic claim came from a filename, user review, local model, or cloud model.

Clip: enlarged waveform, overview navigator, zoom in/out/fit-selection/fit-file icons, playhead, selection shading, draggable start/end handles, and exact time controls. All drag controls have numeric alternatives.

Clip controls: Start, End, Duration fields; Start + End / Start + Duration segmented mode; optional Lock duration; Set start at playhead; Set end at playhead; Clear range; Loop selection; Preview selection; Save clip; Export.

Expanded transport: output device menu, volume, mono/stereo audition, L/R channel solo, loop, playback speed, and Preview loudness match. Playback speed is explicitly tape-style at launch and changes pitch; pitch-preserving stretch is an optional later capability, not an implicit promise.

Metadata: original filename, source membership, codec/container, sample rate, channel layout, bit depth where meaningful, size, measured duration, tags, metrics/units, hash/version, decoder build, and editable rights/license notes. A filename such as "royalty free" is not automatically treated as a license grant.

History: user edits, version changes, clip derivations, exports, and provenance. Undo applies to editable catalog fields and clip recipes. Regenerating analysis preserves user corrections.

### E. Imports and background work

Top strip: total discovered, queued, analyzing, ready, failed, skipped, and bytes processed. Progress is indeterminate during discovery; show stage counts rather than a fabricated ETA. Time remaining appears only once throughput is stable.

Job row: file/root, stage, progress, elapsed time, retry reason, Pause/Resume, Cancel, and Open details. Batch actions: pause all, resume all, retry failed, cancel selected. Error details include permission, unsupported format, malformed media, offline root, disk exhaustion, or worker timeout.

A failed optional AI task does not revert a successfully analyzed file to failed. UI remains usable while jobs run. Closing the window follows the configured tray/quit policy; quitting checkpoints jobs.

### F. Collections, saved searches, and favorites

Collections are references to assets/clips, not duplicated media. Manual ordering is available inside collections. Saved searches store the shared filter schema and reevaluate current availability. Favorites survive rescans, rename, and relinking. Smart collections are saved searches with a display name.

### G. Saved clips and export drawer

Clip rows: name, source title, selected duration, waveform segment, version badge, export status, play, edit, and reveal-export action. Editing creates a recipe revision. Missing originals affect virtual clips; materialized exports continue to work independently.

Export drawer: filename, preset, format, sample rate, bit depth, channels, optional gain/fades, destination, estimated size, and background progress. Existing path conflicts offer Keep both or Choose another destination. Overwrite is never the default.

After completion: Show in folder, Copy path, Drag exported file, or Send through a configured editor adapter. OS drag-out uses a real completed file and native file drag support, not an HTML pseudo-file.

### H. Settings pages

Settings > Folders:
List columns: display name, root path, import mode, inclusion rule, ready count, pending count, disk usage, watch status, last successful scan. Root path is editable via text or native picker. Save validates the directory and previews relink matches/conflicts before switching. Source IDs do not change when names or paths change.
Controls: Add folders/files, recursive toggle, exclude patterns, hidden-file toggle, symlink policy, auto-watch, polling interval for unreliable sources, Rescan, Relink, Remove. Failed/unavailable roots remain visible with an actionable state.

Settings > Playback:
Output device, follow-system-default toggle, test tone button, volume, buffer preset, channel policy, autoplay-next default off, remember position default on, loudness matching default off. Device test does not require microphone access.

Settings > Analysis:
CPU mode Eco/Balanced/Fast, worker limit with conservative bounds, battery pause, automatic reanalysis policy, full-file measurement status, optional extra features, and cache limits. Derived metrics can be recomputed independently when their algorithm version changes.

Settings > Agent access:
Enable MCP, current endpoint, service status, Start/Stop, start with app toggle, client pairing, allowed sources/collections, read/export/edit capabilities, approved export destinations, activity log, revoke client, and copy tested client configuration. No API key is required for local MCP.

Settings > Optional intelligence:
Three clear states: Standard offline, Local models, Cloud provider. Local packs show disk/RAM estimates, supported tasks, download/import-pack, checksum status, and remove-pack. Cloud fields: provider adapter, endpoint if supported, model, API key, test connection, allowed sources, maximum clip duration, budget, and request history. Default is Standard offline.

Settings > Export:
Default destination, format preset, filename pattern, sample rate/bit depth, channel mapping, overwrite policy fixed to ask/keep-both, metadata policy, and optional small fades. Source originals are never export targets.

Settings > Appearance and accessibility:
Theme, UI density, font-size setting, reduce motion, waveform colors/palette, channel layout, waveform scale, and shortcuts. Every unfamiliar icon has a tooltip and an accessible name.

Settings > Data and storage:
Database location display, managed media path, cache budget, size by cache type, clear regenerable caches, backup, restore, legacy import, portable catalog export, relink report, and optional integrity verification. Destructive data reset requires a targeted confirmation.

Settings > About and diagnostics:
App/component versions, update channel/check, licenses, dependency notices, audio device status, diagnostic report export with path/secret redaction, and recent errors. Telemetry is off; diagnostics are exported deliberately.

## 7. Native Audio and Real Waveform Design

### Playback signal path

Bundled decoder -> bounded PCM chunks -> channel mapping/resampling -> preview gain and short de-click ramps -> ring buffer -> CPAL output.

The decoder worker performs file reads and seeks. The audio callback only consumes prepared samples and updates counters. The output frame counter is the authoritative playback clock. UI playhead position is derived from that clock and the source/output-rate mapping, not a CSS timer or independent HTML audio element.

On seek: increment transport generation, cancel/flush old decode work, clear stale queued frames, decode at the new position with codec preroll, and publish the new position when playable frames exist. Old generations cannot leak samples into the new position.

Short files and short selected loops may use a bounded in-memory PCM cache. Long files stream in bounded chunks. Playback receives priority over analysis. Output underflow emits silence and a diagnostic counter; it does not block the callback or spin at full CPU.

State machine: idle, loading, ready, playing, paused, seeking, ended, device-unavailable, source-unavailable, error. Device disconnection pauses, reopens or follows the configured default, then offers resume. Sleep/wake and Bluetooth-rate changes recreate the stream as needed. Output gain is ramped to prevent clicks.

A live post-gain meter helps distinguish silent source data from playback/device failure. A test tone verifies the output path separately. A moving playhead alone never counts as proof that audio works.

### Waveform storage

Decode the whole recording once for the mandatory pass. For each channel, compute min, max, and RMS buckets, then build a multiresolution pyramid. Keep the source channel structure; anti-phase stereo must not disappear from the visualizer.

Each binary cache entry records format version, content/version ID, sample rate, channels, frame count, algorithm version, and checksum. Query only the tile and resolution required for the viewport. Do not load hours of PCM or transmit millions of peaks to React.

Rendering layers: background and amplitude guides, channel waveforms, selection shading, transient markers if enabled, playhead, and pointer/touch handles. At sample zoom, fetch a bounded raw-sample window; do not pretend coarse peaks provide sample-level accuracy.

Waveform scale choices: linear amplitude default; dB display optional; visual normalization clearly marked and independent of audible normalization. Colors represent channels/selection unless a separately labeled spectral view is active.

Spectrum: log-frequency bars from a Hann-windowed FFT on the actual playback signal. Smooth attack/release visually; stop work when hidden or paused. No estimated musical pitch is shown for broadband noise.

Optional spectrogram: computed in the background and tiled, with real time/frequency axes and a legend. Cache eviction affects only visualization, never the underlying source or saved clip recipe.

## 8. Selection, Clip Recipes, and Export Precision

### Interaction rules

- Single waveform click seeks; drag creates a selection in Select mode. Pan mode is a segmented alternative so drag intent is unambiguous.
- Start + End: edit either endpoint; duration is derived.
- Start + Duration: click Set start at playhead, enter 15 seconds, and end becomes start + 15 seconds.
- Lock duration: moving start moves end by the same amount within valid bounds.
- Keyboard I/O sets boundaries; arrow keys nudge using the active time scale; Ctrl/Cmd+Z undoes region edits when the waveform editor owns focus.
- Selection labels use seconds/milliseconds by default, with optional sample-frame display. Video frame-rate presets are a display/conversion mode, not the internal time base.
- Zero-crossing snap is optional. Show any boundary shift and allow exact boundaries without snapping.
- Crossing handles keeps a valid ordered range or clamps at one frame with visible feedback; do not silently invert start/end.
- Exceeding the file end shows remaining duration and offers Fit to remaining audio. Never silently claim a 15-second clip was produced when only 7 seconds exist.
- Empty, negative, non-finite, and out-of-range selections cannot be saved or exported.

### Canonical representation

Use source sample-frame indices: inclusive `start_frame`, exclusive `end_frame`, and the source sample rate. A sample frame includes all channels at a single time instant. JSON uses decimal strings for frame counters so it remains safe across consumers with different integer limits. The backend parses checked integers.

Example: at 48,000 Hz, a start at 12.500 seconds is frame 600000. A 15-second selection ends at frame 1320000, exclusive. Its duration is exactly 720000 / 48000 seconds.

```json
{
  "asset_id": "stable-uuid",
  "asset_version_id": "version-uuid",
  "source_sample_rate_hz": 48000,
  "start_frame": "600000",
  "end_frame": "1320000",
  "channel_policy": "preserve",
  "gain_db": 0,
  "fade_in_ms": 0,
  "fade_out_ms": 0
}
```

Clip recipes bind to a content version. If the source changes, show Source changed and require an explicit rebind for virtual clip export. A materialized clip keeps its own hash and provenance regardless of its original source's later availability.

### Export pipeline

Validate availability, permission, current version, sample boundaries, output format, and destination. Return a job ID. Decode sufficient preroll, trim decoded PCM on sample boundaries, apply the selected channel/gain/fade recipe, resample only if requested, and encode to a temporary file in the destination filesystem.

Verify the rendered result with decoder/probe checks: readable stream, expected channels/rate, correct frame count for lossless formats, nontruncation, output hash, and final recipe metadata. Atomically rename only after success. Cancellation removes only its own temporary output.

Default editor export: WAV PCM 24-bit, 48 kHz, original channel count where supported; offer Preserve source sample rate and 32-bit float. Stereo and multichannel mappings must be explicit. FLAC is another lossless choice. AAC/MP3 are optional supported presets after codec/license qualification.

Lossy formats can introduce encoder delay and padding; they are not promised bit-identical or exact-duration interchange. Sample-exact interchange is tested on decoded PCM/WAV and FLAC. Fast compressed stream-copy cuts are not the precision mode.

Fades are off by default; a short-click-prevention preset can be enabled and is recorded in the recipe. Dither applies only when appropriate for reducing integer bit depth. Preview loudness matching is not baked into exports unless explicitly selected.

Exports live outside watched source folders by default. If the user exports inside a watched folder, match the known output hash/provenance to avoid duplicate import loops. Never overwrite source media.

## 9. What Analysis Can and Cannot Know

### Standard offline, no AI and no key

Reliable objectives: codec/container, duration, channels/layout, rate, bit depth where meaningful, actual peaks/waveforms, per-channel energy, sample peak, clipping indicators, silence regions, onset envelope, spectral centroid/rolloff/flatness, full-file loudness where applicable, file integrity, and exact-byte duplicate detection.

Engineering interpretation: measurements can describe a short noisy burst, slowly rising level, repeated transients, spectral brightness, or long steady texture. They do not identify the physical source with certainty. Titles, folder names, and embedded tags supply separate textual evidence.

Descriptions use auditable clauses: "Filename identifies this as a paper tear. The recording is 0.8 seconds long, with a quick onset and a short noisy tail." Without source evidence: "A 0.8-second broadband burst with a quick onset; source unconfirmed." Do not promote an inferred sound source to a measured fact.

Pitch uses a validated periodicity estimator only on suitable material, with voiced/tonal coverage and confidence. Noise, mixtures, and silence return unavailable rather than invented Hertz. Musical key and tempo are optional extended analyses with confidence; a short whoosh does not receive a forced BPM or key.

Loudness caveats: very short effects may not support useful integrated loudness. Return not-applicable/insufficient-duration rather than zero. RMS and LUFS have distinct units. Full-file statistics and interval statistics remain separate.

### Local models, no API key

Potential capabilities: audio-event suggestions, text-to-audio retrieval, similar-sound retrieval, speech/non-speech detection, and optional transcription with an appropriate separate model pack. These are probabilistic outputs with model/version and interval coverage.

Semantic embedding search is useful even without generated prose. Class labels such as mechanical scrape or applause can support concise template descriptions. A full general-purpose local captioning model is a separate size/RAM tradeoff, not required for the base installer.

Qualification procedure: compare candidate models on a labeled, licensed sound-effect fixture set covering short effects, ambience, noise, music, silence, and ambiguous mixtures. Measure top-k retrieval, false certainty, CPU latency, memory, and quantization drift. An unknown/abstain result is supported. No pack ships merely because an example notebook runs.

### Optional cloud API

Potential capabilities: richer perceptual descriptions, more nuanced event combinations, explanations of similarities, and natural-language query interpretation, using an audio-capable provider. A text-only model cannot listen to an audio file simply because we supply its filename.

Keys are optional and added only in Settings. Before a batch, display the chosen provider/model, permitted sources, excerpts or full clips being sent, and estimated usage where available. Enforce request/day limits, cancellation, bounded retries, and no automatic provider fallback. Mask keys and exclude them from catalog exports, backups, logs, and MCP output.

Cloud processing cannot be described as fully local. The standard product and local-model modes remain offline; choosing a cloud provider explicitly permits the stated audio transfer. Provider retention terms are linked when that adapter is implemented.

No AI mode guarantees exact identification of an arbitrary unnamed effect, license ownership, the best artistic choice for an unseen video, or source separation without artifacts. User corrections remain authoritative and are never silently replaced by reanalysis.

## 10. Search and Filter Contract

Search supports phrase/token matching across title, confirmed tags, descriptions, embedded metadata, and source-relative path. Normalize display/search text, but preserve original filesystem spelling and bytes. Whole-word taxonomy matching prevents train/rain-style errors.

Filter groups exposed identically through UI and MCP:

- Organization: sources, subfolders, collections, favorites, original/saved clip/export, date added/modified.
- Meaning: category hierarchy, event labels, source, action, environment, perspective, texture, user tags, suggested uses, review status, and evidence type.
- Time: min/max duration, active duration, silence at start/end, and clip suitability for a requested length.
- Level: RMS dBFS, sample peak dBFS, integrated LUFS when valid, crest factor, clipping flags, and preview normalization compatibility.
- Character: measured brightness, noise/periodicity, transient count/density, attack estimate, dynamic variation; units and coverage available in advanced details.
- Technical: format, codec, sample rate, channels/layout, bit depth where meaningful, size, decode health, and analysis version.
- Optional model/music: event confidence, text similarity, audio similarity, speech likelihood, BPM/key/pitch only when available.
- Availability: ready/current/online defaults; explicit maintenance views for offline, missing, stale, unsupported, and failed.

Within one multi-select facet values are OR; across facets they are AND. Exclusions use an explicit NOT group. Numeric ranges have documented inclusive/exclusive bounds. Missing measurements do not masquerade as zero. Filter counts reflect the other active facets, and expensive counts may refresh separately.

Ranking: exact user tags/title phrases first, text relevance next, then declared event evidence, duration suitability, and requested character. Optional embedding score is one separately labeled factor. No model probability is mixed with a filename confidence score as if they were calibrated equivalents.

Default responses contain 25 compact results; maximum 100 per page. Cursor includes deterministic tie-breaking and a catalog revision. New results do not move a currently selected row while the user is editing. A stale cursor restarts or reports a revision mismatch predictably.

Optional local similarity uses versioned embeddings in a rebuildable local index. Start with exact search over a bounded embedding set; add a qualified approximate index only when measured performance requires it. Never compare vectors from different model versions as if they occupied one space.

## 11. Database, Identity, and Storage

Use SQLite in the OS application-data directory, never inside this repository or a watched media folder. WAL allows concurrent readers; one service serializes writes with busy timeouts and transaction batching. Keep the live database on local disk, not a cloud-sync/NFS location. [SQLite WAL](https://sqlite.org/wal.html)

FTS5 indexes denormalized searchable text; structured facets use conventional indexed columns/junction tables. Rebuild search indexes without changing asset identity. [SQLite FTS5](https://sqlite.org/fts5.html)

Logical schema:

- `libraries`: id, display_name, created_at, settings_revision.
- `sources`: id, library_id, name, kind(folder/file_list/managed), native_root, display_root, recursion, include/exclude rules, symlink/watch policy, availability, root_generation, last_successful_scan.
- `source_memberships`: source_id, file_instance_id, relative_path, last_seen_scan_id; one file can belong to overlapping sources.
- `file_instances`: id, native_path representation, native filesystem identity if available, stat_signature, current_version_id, availability, first_seen, last_verified.
- `assets`: stable logical id, display_title, user_description, review_state, created_at. Favorites/collections refer here, not to a pathname hash.
- `asset_versions`: id, asset_id, content_hash, hash_algorithm, byte_length, source_signature, container, codec, duration_frames, sample_rate, channels/layout, created_at.
- `instance_versions`: file_instance_id, version_id, active flag; identical content can share expensive analysis while keeping different locations.
- `analysis_runs`: id, version_id, stage, engine_version, config_hash, coverage, started/finished, status, typed_error; successful results are immutable.
- `audio_metrics`: run_id/version_id, measurement name, value, unit, validity, interval/channel scope; common sortable metrics also have indexed columns in a query projection.
- `event_segments`: version_id, frame_start/end, event/taxonomy id, evidence type, model/version, score or null, user override.
- `semantic_claims`: version_id, field, value, evidence, rule/model version, confidence type, source span/interval, accepted/rejected state.
- `tags` and `asset_tags`: normalized tag key, human label, namespace, asset_id, provenance, user override; support multiple languages without altering path identity.
- `collections` and `collection_items`: IDs, names, manual order, asset/clip references.
- `favorites`: asset_id or clip_id, created_at.
- `saved_searches`: id, name, versioned filter JSON, sort, display configuration.
- `clips` and `clip_revisions`: clip id, name, asset/version id, sample boundaries, gain/fades/channel recipe, created_at, revision.
- `exports`: id, clip_revision or asset_version, destination reference, path, preset, status, content hash, bytes, validation result, created_at.
- `jobs`: id, type, subject, stage, state, priority, generation, retry_count, lease_owner/expiry, checkpoint, progress, cancellation flag, timestamps.
- `waveform_caches`: version_id, algorithm/format version, cache path, levels, frames/channels, size, last_accessed, checksum.
- `model_packs` and `embeddings`: manifest/version, task, content digest, runtime compatibility, vector dimension, cache location, active status.
- `agent_clients`: id, name, secret reference/hash as appropriate, scopes, approved sources/destinations, expiry, revoked_at, last_seen.
- `activity_events`: time, actor(client/user/system), action, entity id, result, redacted details; retention bounded.
- `settings` and `schema_migrations`: versioned preferences and migration history, without plaintext credentials.

Database invariants:

- Foreign keys enabled; unique memberships, job deduplication keys, and content-version analysis keys.
- Clip constraints require `0 <= start < end <= source_frame_count` and matching source version/rate.
- Source generation must match before a job may publish.
- `ready` requires a committed successful mandatory analysis run, not just a nonempty cache file.
- Full content hashes verify identity; size/mtime/file ID are optimization hints, never proof of equality after a detected mutation.
- Rename preserves file/asset identity when native file ID or content evidence supports it. Same-path replacement creates a new version.
- No automatic deduplication deletes physical media. Exact copies share analysis but preserve file locations and user-visible memberships.
- Non-UTF-8 Unix paths and Windows-native path encoding remain losslessly addressable by the backend; UI/MCP use IDs and sanitized display paths.

Storage layout:

```text
Application data/
  catalog.sqlite (+ WAL/SHM while running)
  settings.json                 small bootstrap preferences only
  media/                        only explicit managed copies
  exports/                      default materialized user clips
  cache/peaks/                  regenerable multiresolution waveform files
  cache/pcm/                    capped preview chunks
  cache/models/                 optional verified packs
  cache/embeddings/             versioned, regenerable local search index
  jobs/                         app-owned temporary/checkpoint files
  backups/                      bounded consistent snapshots
  logs/                         redacted rolling diagnostics
  runtime/                      per-user MCP discovery/socket info
```

Use the SQLite backup API for live snapshots, not a blind copy of the database while WAL is active. Migrations run before workers start and create a recovery snapshot. Downgrading across an incompatible schema is blocked with a restore path.

Backup choices: catalog/settings only, or portable bundle including selected managed media/exports. Referenced external sounds are not silently included. Secrets and machine-native file grants are not portable.

## 12. Folder Watching, Relinking, and Deletion

Register each source with a native path and persistent UUID. Root placeholders in exports map to that UUID. Moving a root changes one mapping; relative paths and logical IDs stay unchanged when files match.

Watcher events are hints: debounce edits, coalesce bursts, wait for file size/mtime stability, and reconcile after overflow. Network/removable sources use fallback polling and a foreground refresh check. Defaults target a confirmed local deletion becoming hidden within two seconds; watcher limitations can extend that until reconciliation.

Deletion protocol:

1. On a file-delete event, check the source root and parent availability.
2. If the volume/source is unavailable, mark the source offline; do not infer thousands of deleted files.
3. If the source is online and the individual file is confirmed absent, mark that instance missing and remove it from ordinary search/MCP immediately.
4. Keep a catalog tombstone and user metadata for recovery. Another available duplicate instance can still resolve the asset.
5. An incomplete scan cannot mark unseen files deleted. Deletion reconciliation runs only against a completed successful scan generation.

Relink protocol: user selects a replacement root; app samples and counts relative-path matches, then verifies content as needed. Show matched, modified, ambiguous, and missing counts. Commit a new root generation, cancel obsolete jobs, restore matching IDs, and schedule only new/changed files. Never attach two different recordings just because filenames match.

Overlapping roots: identify canonical file instances and attach memberships to both sources; analyze once. Removing one source cannot remove a file that remains included by another authorized source. Symlink following is off by default; if enabled, enforce root policy, detect cycles, and deduplicate canonical identities.

A folder path edit changes catalog configuration. Moving actual media is a separate operation and is not performed implicitly.

## 13. Background Job Reliability and Resource Budgets

Persistent queue and leases make jobs restartable. After a crash, expired in-progress leases become queued/recoverable. Each completed stage commits atomically with its artifact checksum. Failed output is never reused as a successful cache.

Default scheduling: one playback decoder when needed; one foreground export slot; up to two analysis workers, bounded by CPU, memory, and disk policy; optional model work uses a separate low-priority budget. Additional workers are benchmark-driven, not a fixed six for every machine.

Pause: stop scheduling and either checkpoint or restart the current stage safely. Cancel: signal the worker, terminate it after a bounded grace period if necessary, and remove only job-owned temporary files. A file-level decode may restart from the beginning after an abrupt crash unless a validated block checkpoint exists.

Retries: transient I/O/offline issues retry with bounded backoff; invalid media does not retry forever; permission failures wait for a user action. Per-file timeouts scale with duration and stage, with an upper bound and visible reason. A long valid recording is not rejected by a universal 90-second timeout.

Backpressure: bounded queues, chunked PCM pipes, stderr drain limits, bounded event streams, and explicit disk budget. Do not capture hours of raw PCM into one process stdout buffer. Sparse/huge files, archive bombs, recursive roots, and unsupported URL inputs are refused or bounded before expensive work.

Lifecycle: app starts scheduler and optional MCP; close-to-tray is an explicit preference; Quit checkpoints work and closes endpoints. No permanent daemon/service installation at launch. Updates wait for exports or offer pause/restart; they do not interrupt an atomic export commit.

## 14. MCP Contract and Agent Workflow

The app embeds an actual MCP implementation, not merely a REST endpoint named MCP. Primary transport is authenticated Streamable HTTP on loopback. Offer a compiled stdio bridge for clients that require command-based MCP. The bridge connects to the running app; it does not open a second database or silently start an invisible analysis daemon.

The transport follows negotiated MCP versions, initialization, tool/resource discovery, and cancellation rules. Local HTTP binds to loopback and validates Origin, Host, and credentials. [MCP transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)

Endpoint example: `http://127.0.0.1:<configured-port>/mcp`. If the port is occupied, show an actionable state; an automatic alternate port is reflected in discovery and copied configuration. The stdio bridge uses the per-user runtime discovery file so it can survive endpoint changes.

Each paired client gets explicit scopes and source restrictions. For local HTTP header-capable clients, provision a per-client credential; clients needing a different interoperable authorization flow must use a tested adapter or the stdio bridge. Do not claim a homemade API-key scheme implements the entire MCP OAuth authorization specification.

Default capabilities: read catalog and search. Export is a separately enabled capability with approved destination roots. Library management and cloud analysis are off for agents unless specifically granted. Tokens are not passed through URLs. No public tunnel, LAN binding, or cloud relay is part of the local product.

### Proposed tools

- `library.status`: app version, index revision, ready/importing/offline counts, supported optional capabilities.
- `sources.list`: authorized sources and availability, with paths only if the client has path access.
- `sounds.search`: text, shared structured filter object, sort, limit, cursor; returns compact candidates with IDs, descriptions/evidence, duration, availability, and match reasons.
- `sounds.get`: asset/version details and current permitted file instances; version-aware.
- `sounds.facets`: facet counts for the same filter object used by search.
- `sounds.similar`: source asset or text query plus exclusions; returns MODEL_UNAVAILABLE when an optional semantic model is absent, or explicitly labeled acoustic-only similarity if requested.
- `sounds.waveform`: bounded peak tiles for a visible time interval and resolution, not arbitrary PCM dumps.
- `sounds.resolve`: validate current availability and return a local path or short-lived scoped media handle, according to client privileges.
- `clips.create`: validated start/end or start/duration, source version, recipe, display name, idempotency key; creates a virtual clip.
- `clips.update`: revision-checked recipe edit; stale changes produce a conflict rather than losing user work.
- `clips.list`: filtered saved clips and materialization states.
- `clips.export`: clip revision, format preset, approved destination ID, idempotency key; returns export job ID promptly.
- `jobs.get`, `jobs.list`, `jobs.cancel`: persistent job state and bounded errors; enforce ownership/scopes.
- `catalog.export`: portable JSON/JSONL/Markdown of an authorized subset, without credentials or machine paths by default.
- `sources.scan`: authorized existing source only; returns job ID and never blocks a tool call for a full scan.
- `editor.prepare_handoff`: completed export and target/preset, returning media plus an integration manifest.

Do not expose generic shell, raw SQL, arbitrary filesystem reads/writes, secret retrieval, unrestricted root changes, or source deletion tools.

### Example request and response shape

```json
{
  "query": "short metallic scrape",
  "filters": {
    "duration_sec": {"min": 0.1, "max": 2},
    "availability": ["available"],
    "readiness": ["ready"],
    "review_recommended": false
  },
  "limit": 5,
  "sort": "relevance"
}
```

```json
{
  "catalog_revision": "revision-id",
  "items": [{
    "asset_id": "asset-id",
    "version_id": "version-id",
    "title": "Metal scrape",
    "description": "Filename identifies a metal scrape; short noisy attack and tail.",
    "duration_sec": 1.24,
    "evidence": ["filename", "full_file_measurement"],
    "review_recommended": false,
    "match_reasons": ["title phrase", "duration within range"]
  }],
  "next_cursor": null
}
```

Errors are typed: APP_NOT_RUNNING, SOURCE_OFFLINE, ASSET_MISSING, ASSET_CHANGED, NOT_READY, INVALID_RANGE, PERMISSION_DENIED, MODEL_UNAVAILABLE, EXPORT_COLLISION, DISK_FULL, JOB_CANCELLED, and RATE_LIMITED. Protocol errors and domain errors remain distinguishable.

A typical agent searches, inspects two or three candidates, chooses a version and range, creates a clip, exports it, waits for that job, and passes the completed file to its editing workflow. Repeated requests with the same idempotency key cannot create duplicate clips/exports accidentally.

The optional agent skill is documentation of this workflow and the tool schema. It is not required for MCP, does not replace the running service, and cannot make a cloud agent reach a private local computer. Remote agents need an explicitly separate transfer/connectivity solution; none is enabled by default.

## 15. Editor Integration

Universal baseline: export WAV/FLAC or a selected supported format, reveal it, copy its path, and native-drag the completed file into an editor. This requires no editor-specific plugin and is the first compatibility target.

Remotion: export media into an explicitly approved project media directory and return a manifest with project-relative path, clip timing, source provenance, and optional placement data. The agent uses the media through the project's supported audio component. A local absolute filesystem path is not automatically a browser-fetchable URL. Validate each adapter against the actual project version. [Remotion Audio](https://www.remotion.dev/docs/media/audio)

DaVinci Resolve: manual import is baseline. A later installed-version adapter may import into the media pool or place clips using its local scripting API when the user's edition/settings support it. The adapter must discover/test availability and target project/timeline before reporting success. Obtain and validate the SDK supplied with the installed Resolve version; do not promise identical scripting support on every version/edition.

MCP itself does not control every editor automatically. It supplies search, selections, media, and export status; direct timeline operations require the editor's API or an agent with separate editor access. Export success and editor-import success are separate states.

Other editors: manual file import first; add adapters only with documented APIs, known version support, and reversible import/placement behavior. Avoid brittle UI-click automation as a production integration.

## 16. Local Access, Privacy, and File Safety

- Tauri capabilities grant only the required native operations. Disable arbitrary renderer-side shell/file access; use a restrictive CSP and no remote script loading.
- Library content, ID3 tags, filenames, and AI captions are untrusted text. Render as text; do not interpolate them into HTML or shell commands.
- MCP source scopes filter results before matching, facets, counts, path resolution, previews, and similarity so hidden libraries cannot leak through aggregate results.
- Export destinations are registered explicitly; reject traversal, symlink escape, source overwrite, device files, and mismatched ownership/permissions. Revalidate at open/commit time.
- Decode known local regular files; prohibit network input protocols and dangerous nested playlists in the bundled media invocation. Large artwork/metadata is size-bounded.
- Limit request sizes, tool rates, file preview ranges, concurrent exports, and token lifetime. Session IDs are not authentication credentials.
- API keys stay in the OS credential store or memory-only mode. DB records store credential references; deleting a provider configuration removes its secret when requested.
- Cache cleanup touches only manifest-tracked app-owned files. No recursive deletion of arbitrary user-selected source paths.
- Diagnostic bundles redact keys and can redact paths and filenames. No telemetry or automatic crash upload without explicit opt-in.

## 17. Packaging, Installation, Updates, and Size

Deliverables are platform-specific signed packages, not one universal binary. Each includes the app, its appropriate media/worker/bridge binaries, compiled UI, SQLite support, icons, notices, and a self-check manifest. All media analysis functions work on a clean machine without this project.

- macOS: separate arm64/x64 DMGs or a tested universal app, Developer ID signing, nested sidecar signing, and notarization. First launch from a quarantined download is a test case.
- Windows: per-user NSIS installer, signed app/sidecars, bundled or installer-provisioned WebView2. Provide an offline variant with the runtime when absent. No administrator requirement for ordinary use.
- Linux: AppImage plus Debian package, with an RPM build if Fedora qualification succeeds. Test the declared glibc/WebKitGTK/audio baseline; AppImage is not a guarantee of compatibility with every distro. Flatpak is a possible later package because filesystem and editor handoff portals add distinct behavior.
- Updates: signed manifests/artifacts, explicit channel, resumable download, no replacement while exporting, migration backup before upgrade, recovery from interrupted installation. Offline users can install a new signed package manually.
- Uninstall: remove application binaries; preserve user media and export files. Offer targeted removal of app data separately.

Size goal, not a promised measurement: aim for a base per-architecture compressed package below approximately 100 MB without local AI packs or an embedded Windows webview runtime. Measure before committing a public size claim. A fully offline Windows installer can be materially larger. Large caption/embedding model packs are separate downloads or offline importable packages.

Build infrastructure needs Rust, a Node package manager, platform SDKs, C/C++ build tools, and signing credentials. End users do not. Build/test on each OS and architecture rather than shipping an untested cross-compiled artifact. Signing identities and release hosting are distribution prerequisites; they do not block local development and unsigned development builds.

## 18. Performance Targets and Validation Hardware

Targets must be benchmarked; none is asserted as achieved today. Reference workloads: a 3,766-file library like this one, a generated 100,000-entry catalog, many micro-effects, stereo music, 2-8 hour recordings, multichannel files, external drives, and slow network-mounted sources.

Reference hardware: a four-core/8 GB machine with SSD, a current Apple Silicon laptop, and representative Windows/Linux physical audio hardware.

- Warm startup to interactive Library: under 2 seconds on the reference local SSD workload.
- Search: p95 under 150 ms at 100,000 indexed entries; first results rendered under 250 ms, excluding cold disk spin-up.
- Drop feedback: under 100 ms; job ID returned under 250 ms; directory traversal continues asynchronously.
- Local ready sound: playback begins within 250 ms warm / 750 ms cold for standard fixtures; codec/storage exceptions are measured separately.
- Seek: under 150 ms warm / 500 ms cold on common fixtures, with a visible loading state if exceeded.
- Waveform pan and handle drag: target 60 fps at normal display sizes; no long renderer tasks above 50 ms during ordinary interaction.
- Baseline idle memory: target under 250 MB; active standard analysis bounded below 750 MB excluding optional models, subject to profiling.
- No duration-proportional PCM memory growth for long files.
- Ordinary idle CPU below 1% on the reference system; suspended animations when hidden.
- Confirmed local deletion hidden within 2 seconds with normal watcher delivery; reconciliation documents longer fallback latency.
- Library browsing and audio remain responsive while imports and exports run; no audible underruns in the normal reference stress test.

Indexing throughput is reported in audio-minutes per wall-minute and by codec/hardware, not a universal files-per-second claim. Cache budgets and worker count must be adjustable.

## 19. Test Cases, Scenarios, and Release Gates

### Audio and waveform acceptance

1. WAV, MP3 CBR/VBR, FLAC, AIFF, AAC/M4A, Ogg Vorbis, and Opus fixtures decode, analyze, audition, and export on each supported OS.
2. Known tone fixture: nonzero decoded PCM and correct measured frequency/level within algorithm tolerance; audible output verified on physical devices.
3. Play, pause, resume, seek, loop, mute, volume, and next-source replacement have no unexpected silence or stale frames.
4. Favorite/filter/sort/inspector changes do not stop the active transport.
5. Output device removal, Bluetooth reconnection, sleep/wake, and device-rate changes recover or report an actionable state.
6. Anti-phase stereo remains visible and measurable; mono audition reports its intentional downmix behavior.
7. Silent and near-silent files do not produce fake pitch, random spectral bars, invalid logarithms, or division by zero.
8. A high-frequency tone above 6 kHz is measured correctly; the old 12 kHz-downsample shortcut cannot pass this test.
9. An event after minute five appears in full-file waveform/analysis; analyzing just the first 20 seconds cannot pass.
10. Stereo, mono, 5.1, float PCM, 24-bit PCM, unusual rates, and clipped fixtures produce correct channel/rate/validity information.
11. Waveform tiles match a known impulse at every zoom level and high-DPI scale; they are not ID-derived decorative data.
12. Malformed headers, truncated audio, huge tags/artwork, extension/content mismatches, and unsupported streams fail one item without freezing the queue.

### Import, files, and recovery

13. Drag a folder containing 10,000 files while previewing an existing sound; UI stays interactive and unpublished files stay out of normal search.
14. A folder with valid, corrupt, unsupported, and inaccessible files publishes successes and retains specific error rows for the rest.
15. Dropping the same folder twice or overlapping parent/child roots does not duplicate analysis or lose source membership.
16. Dropping one file imports only that file, not its siblings.
17. A file being copied settles before analysis; a mid-analysis mutation invalidates the result.
18. Rename a file: favorites, clips, and user metadata retain identity when matching evidence is available.
19. Replace content at the same path: new version and metrics; old virtual clip requires explicit rebind.
20. Delete one original: default UI/MCP exclude it; do not classify playback/codec errors as deletion.
21. Unplug an external drive: root becomes offline, not an empty/deleted library; reconnect restores instances.
22. Deny permission halfway through a scan: no mass tombstoning of unseen entries.
23. Relink a moved folder: matching files keep IDs, mismatches are shown, stale jobs cannot publish.
24. Restart during decode/index/export: durable queue resumes/retries safely with no half-ready asset or partial final output.
25. Disk full: pause/fail only affected jobs, preserve DB/originals, expose retry after space is freed.
26. Unicode, very long paths, spaces, quotes, newlines, case-only renames, Windows reserved names, UNC paths, and supported non-UTF-8 paths are handled or explicitly rejected without mangling identities.
27. Symlink loops, junction escapes, network-watch failures, and watcher overflow have bounded recovery paths.
28. Remove a source: originals remain untouched and overlapping authorized memberships still work.

### Clip/export precision

29. Start 12.500 seconds plus duration 15 seconds at 48 kHz produces exactly 720000 source frames in same-rate lossless export.
30. A one-frame selection, end-of-file selection, reverse handle drag, invalid numbers, and duration beyond EOF have deterministic outcomes.
31. Lossless output boundaries match a generated impulse/chirp fixture; seek preroll never leaks into the export.
32. MP3/AAC padding is handled and documented; exact timing is validated against decoded output and gapless metadata where supported.
33. Preview/recipe/export agree on channel mapping, gain, fade, and rate conversion; preview-only matching does not change export unexpectedly.
34. Cancel export, close app, lose destination drive, and collide with an existing filename without corrupting an existing file.
35. Exporting into a watched folder does not create an infinite import/export loop.
36. Export persists and can be imported manually into a separate editor after the app exits.
37. Source changes after saving a clip are detected before export; existing rendered copies remain valid.

### Search, provenance, and MCP

38. Train/rain and similar substring fixtures classify and search without false token matches.
39. UI and MCP with identical filters return the same authorized IDs and documented ranking, excluding pending/missing items.
40. Unknown measurements do not match numeric zero; no-key mode contains no placeholder AI claims.
41. User corrections survive reanalysis and override conflicting machine suggestions with provenance retained.
42. Scope-restricted client cannot infer another source through counts, facets, similar search, exports, or direct IDs.
43. Missing/revoked token, hostile Origin/Host, path traversal, malformed JSON, enormous request, and unsupported protocol negotiation produce bounded failures.
44. MCP initialize/discover/search/get/create/export/job-status workflow passes through both HTTP and stdio bridge clients.
45. Retrying with the same idempotency key creates one clip/export, including after a dropped connection.
46. App stopped or MCP disabled returns a clear connection state; bridge does not create a hidden second catalog service.
47. Restart/port conflict/discovery changes reconnect correctly without exposing secrets in config snippets or logs.
48. All semantic-search fields behave predictably when no local model or cloud key is installed.

### UI, packaging, and optional AI

49. Keyboard-only navigation, focus traps/restore, tooltips, screen readers, reduced motion, and 200% text scale work across all screens.
50. Layout tests at 900x600, 1280x800, 1440x900, and 4K/high-DPI cover long filenames and translated labels; no inaccessible or overlapping controls.
51. Drag/drop feedback, queue pause/cancel, source editing, range handles, favorites, and saved searches persist correctly.
52. Local-only mode with network disabled performs install variant verification, import, search, playback, clipping, export, and MCP.
53. Fresh-machine installation with no development tools runs the bundled decoder and bridge; no dependency on workspace paths or PATH.
54. Signed/quarantined macOS install, Windows runtime absent/present, and declared Linux package baselines are tested separately.
55. Upgrade with a populated library, failed migration, interrupted update, backup restore, and uninstall preserve documented data boundaries.
56. Optional local pack download corruption, wrong architecture, insufficient RAM, canceled installation, and checksum mismatch leave the base app working.
57. Optional provider invalid key, timeout, quota, rate limit, model unavailable, budget exceeded, and cancellation fail enhancement only.
58. Packet/endpoint instrumentation verifies zero audio upload until a cloud job is explicitly authorized.
59. Diagnostics, backups, MCP responses, and catalog export contain no API keys or pairing secrets.
60. Long-running ingestion/playback stress test demonstrates bounded memory, bounded logs, scheduler fairness, and recoverable worker failures.

Testing tools: Rust unit/integration and property tests for sample ranges, path authorization, and queue transitions; generated waveform/PCM fixtures for numerical checks; Vitest/Testing Library for interface state; browser layout tests; installed-app WebdriverIO/Tauri tests; MCP protocol tests; physical-device listening/loopback checks for actual sound output. Renderer tests alone cannot certify audio or native drag/drop.

The current Tauri testing documentation describes WebdriverIO with an embedded driver path for macOS as well as Windows/Linux. Use test-build-only instrumentation and verify production packages do not expose the driver. [Tauri WebDriver testing](https://v2.tauri.app/develop/tests/webdriver/)

## 20. Implementation Sequence and Completion Criteria

### Phase 0: Correctness and compatibility spike

Reproduce current silent playback; validate native decoding/output/seek on target platforms; render real peaks; export an exact known region; build a minimal installer with bundled binaries. Validate native folder drop and OS drag-out of a completed WAV. Audit runtime size/license constraints.

Exit: audible playback, matching visual data, exact export fixture, and a clean-machine installer proof. Resolve architecture risks here before polishing the entire interface.

The release-blocking spike questions are concrete: native seek accuracy for VBR/compressed files; OS file drag-out; reliable webview/native event delivery; sound output after device reconnect; every sidecar architecture; clean-machine webview requirements; and realistic installer size. Record measured outcomes and the selected versions in architecture decision records. An unsupported optional feature is disabled visibly; a failure of core playback, clipping, or installation blocks release.

### Phase 1: Local library foundation

Build stable IDs, source registry, migrations, durable jobs, full-file analysis, waveform caches, relinking/watch reconciliation, and search APIs. Add fixture-driven provenance and filename inference. Import this library as a user-selected source for migration testing.

Exit: files enter search only after required analysis, missing/offline distinctions work, restart/cancel is safe, and user edits survive reindexing.

### Phase 2: Complete daily-use interface

Implement the navigation, library layouts, filters, persistent player, inspector, imports view, source settings, collections, and accessibility/motion system. Use representative real peaks and long filenames throughout design validation.

Exit: primary workflows are usable with pointer and keyboard and remain responsive during import.

### Phase 3: Clip editor and exports

Add exact selections, Start + Duration, recipe revisions, loop audition, background export, native drag-out, and manual editor workflow. Include cache and export storage controls.

Exit: full clip precision and crash/collision tests pass; exported media works in a separate editor after the app closes.

### Phase 4: Agent integration

Embed MCP, pairing/scopes, transport/bridge, bounded search, clip/export jobs, portable catalog export, and optional agent workflow documentation. Add the Remotion handoff adapter; gate any direct Resolve adapter on installed-version validation.

Exit: an authorized local agent finds and exports a precise clip without scanning original folders, and unauthorized queries cannot access other sources.

### Phase 5: Production packaging and hardening

Complete clean-machine testing, platform audio/device QA, backup/migration recovery, security checks, performance benchmarks, signed installers, notices, and update recovery. Documentation covers support matrix and runtime limitations.

Exit: required phases 0-5 satisfy all applicable launch acceptance tests, including no-model/no-key behavior. Optional capability tests become mandatory when that capability is shipped. A visual demo or single-OS development build is not labeled production complete.

### Optional parallel phase: Intelligence packs

Benchmark local event/embedding models first. Add model pack management and semantic retrieval only when useful quality and resource targets are achieved. Add a cloud adapter only for a clearly validated audio-capable provider and an explicit user configuration.

Exit: base functionality remains unchanged without a model/key, enhanced claims retain evidence, and costs/data transfers stay within configured permissions. General captioning quality is evaluated separately from successful HTTP requests.

## 21. Explicit Limits and Decisions Already Made

The plan deliberately includes an operational offline product without AI. It also preserves an upgrade path for better semantic recognition without making that recognition a prerequisite for playback or clipping.

Already chosen defaults: referenced media, per-file asynchronous publication, full-file mandatory waveform/measurements, one native preview player, virtual clips plus materialized exports, local SQLite, stable root IDs, no automatic source deletion, optional local models, cloud off, and authenticated local MCP tied to application lifecycle.

Not launch guarantees: a tiny installer including every runtime and every AI model; support for every historical OS; perfect recognition of unnamed audio; universal automatic editor control; processing encrypted/DRM media; archive extraction; background operation after full Quit; remote/cloud agents directly reaching localhost; audio generation, source separation, multitrack editing, or VST hosting.

Release prerequisites that will be concrete later: final app name/identifier, signing identities, distribution location, tested dependency lock, and qualified optional model licenses. Development and reversible local testing can proceed before distribution credentials exist.

The next implementation step, after this planning review, is Phase 0. No new application or installer is claimed to exist as part of this document.
