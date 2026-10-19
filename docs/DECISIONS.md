# Implementation decisions

## 2026-09-20: Desktop and portable identity

The product is a Tauri 2 desktop application. It does not depend on the existing HTML catalog, Python scripts or this audio collection. Installed application data belongs in the OS app-data directory, not the source folder.

Sources have stable UUIDs and editable absolute root paths. Sounds have independent stable UUIDs and normalized relative locations. Content versions own reusable analysis keyed by content digest and analyzer version. Moving a folder changes its root and source generation, not sound IDs. Verification may read/hash content but must not decode/recompute unchanged content. Conflicting or partially matching roots require reconciliation, never silent reassignment.

## 2026-09-20: User metadata and evidence

User tags and comments are independent of generated analysis. Automatic reanalysis cannot overwrite user edits. Display labels contain spaces, not underscore separators. Provenance distinguishes measured properties, filename inference, user annotations and model suggestions. A text model cannot truthfully claim to have heard an audio file when sent only metadata.

## 2026-09-20: Optional intelligence

Offline natural-language search includes synonym expansion, typo correction and validated numeric intent extraction. MCP exposes search; MCP itself does not provide semantic intelligence. Optional model-backed query interpretation and audio understanding enhance the same search service. All AI features start disabled, with explicit network and audio-upload consent.

OpenAI uses its Responses contract for text; Anthropic uses Messages; Gemini uses generateContent; LM Studio uses OpenAI-compatible local endpoints. Credentials are provider-specific: a key must have access to the selected model. Local servers may require no key. Audio input is a separate capability, never inferred from text support. No key is stored in renderer localStorage, ordinary SQLite, logs or exports.

Verified contracts:
- https://developers.openai.com/api/docs/guides/text
- https://platform.claude.com/docs/en/api/messages/create
- https://ai.google.dev/api/generate-content
- https://lmstudio.ai/docs/developer/openai-compat

## Release gates

Passing local unit tests is not production qualification. Native playback, device changes, long-file memory, clean-machine installation, OS permissions, security, model capability checks, accessibility and signed updates must pass on each advertised platform. Windows/Linux cannot be marked tested from a macOS run.

## 2026-10-17: Still-image decoder and color isolation (SS-042)

Use pinned `image = 0.25.10` with only jpeg/png/webp features, `moxcms = 0.9.1`
with default features disabled for explicit scalar RGB/gray ICC transforms,
`flate2 = 1.1.10` with the Rust backend for bounded metadata expansion, and
`crc32fast = 1.5.2` for PNG structure validation. Cargo.lock pins the transitive
PNG 0.18.1, image-webp 0.2.4 and zune-jpeg 0.5.15 decoders. Image itself also
requires moxcms 0.8.1; this is a separate transitive instance, not our explicit
ICC adapter. Dependency license texts are retained under docs/licenses/image-analysis.
No native codec/color library or Python runtime ships for this feature.

The upstream [ImageDecoder interface](https://docs.rs/image/0.25.10/image/trait.ImageDecoder.html)
provides full decode and metadata operations. Its [resource limits](https://docs.rs/image/0.25.10/image/struct.Limits.html)
do not promise a strict allocation cap for every backend. Therefore callers
must not decode inside the desktop process. Our one-shot worker uses a global
Rust allocation budget, named size/pixel/metadata/output bounds, and a parent
watchdog that kills/reaps on deadline or cancellation. It reads only the already
opened file through stdin and returns bounded JSON, never paths or pixel arrays.
A crash/abort leaves the app's catalog and original bytes intact. These are
resource/fault isolation measures; the process is not an OS security sandbox.

The [moxcms implementation](https://github.com/awxkee/moxcms) supplies pure Rust
ICC parsing/transforms. We admit RGB/gray matrix/TRC profiles supported by this
feature set, convert display pixels to sRGB, preserve straight alpha, and reject
unsupported profiles rather than produce misleading colors. CMYK/lossless JPEG,
LUT-only ICC, unsupported PNG transfer/primary metadata, animation and other
formats fail explicitly. Untagged images use a visible assumed-sRGB warning;
16-bit originals retain measured depth/alpha while display quantizes to RGBA8
with a warning. Raw bytes never change. See IMAGE_ANALYSIS.md for exact policy.

Measured on local macOS arm64 debug builds: admitted 4096×4096 RGBA8 and rotated
RGBA16 fixtures completed full decode with allocator peaks 134,357,671 and
201,597,641 bytes respectively. Separate OS measurements recorded maximum RSS
142,491,648 and 276,987,904 bytes. These are individual fixture measurements,
not universal memory/latency claims. The enforced Rust heap limit is 512 MiB;
stack, mappings and allocator overhead are additional. Platform support requires
the real worker fixtures on each advertised target. The existing CI matrix runs
them on macOS, Windows and Linux; installed packaging remains a later gate.

## SS-044 preview delivery and cache (18 October 2026)

Previews are delivered as a binary IPC response (`CSPV` envelope). The renderer
makes blob URLs it owns and revokes. We considered a custom URI scheme serving
cache files. It was rejected because a URL handler is a second authorization
surface that the CSP and every future caller would need to scope correctly, and
the ticket forbids filesystem protocol expansion. Commands keep authorization,
cancellation and stale fencing in one core service. The CSP adds only `blob:`.

Thumbnails, inspect previews and tiles come from the SS-042 worker in a new
`preview` mode, not from in-process decoding. The worker's allocation budget and
watchdog are the reason decoding was isolated in the first place. Arguments are
fixed tier names and small integers, never paths. The three thumbnail tiers share
one decode. Transparent sources resample premultiplied (16-bit) to avoid hidden-RGB
edge bleed; a real-worker regression fails under straight-alpha resampling.

The cache is a content-addressed directory of self-verifying records rather than
catalog tables. It is disposable derived data: putting it in SQLite would add
migration, backup and portable-format obligations and grow user backups for no
recovery value. A namespace hash of preview version, analyzer and color policy
invalidates old preprocessing wholesale. The quota lives in its own
`app_settings` key because `Preferences` rejects unknown fields and would break
older binaries. The SS-040 ceilings (2,048 px, 4 MiB) are enforced per output;
inspect steps down to a smaller size rather than raise them.

## SS-045 gallery windows and schema v12 (19 October 2026)

The gallery needs random access over up to 100,000 images in a user-visible
order, so it reads offset windows rather than the SS-041 ID keyset pages. The
contract already describes metadata lists as live observations. Selection is
held by asset ID, so shifting offsets during an import cannot change what the
user selected. Name order is ASCII case-folded path, then exact path, then
ascending ID, which makes it deterministic across platforms without ICU.

Without an index every window sorted the full catalog while holding the catalog
lock (252 ms at 100K images in release). We added schema v12 with one expression
index rather than an in-memory order cache, because there is no reliable cheap
change signal to invalidate such a cache. The cost is the established restore
rule: v11 database backups need the v11 binary. No data fields changed.

A virtualized grid with roving focus and a 120 ms preview settle delay keeps DOM
nodes, preview requests and blob URLs proportional to the viewport. In a 100K
synthetic Chromium run, a 40-step scroll to the end issued zero preview requests;
21 followed for the settled view.

## SS-046 image annotations, history and schema v13 (19 October 2026)

Creator annotations stay in the SS-040 `annotations` object, the only
user-owned slot. Technical facts live in analyses and future AI suggestions in
evidence, so reanalysis and content replacement never rewrite them. Edits are
explicit operations (add/remove tags, set notes/favorite/rating) rather than
array replacement, because full-array writes silently drop another editor's
tags. A single edit must name the revision it read; a mismatch is `CONFLICT`
and the editor reloads. Bulk edits take revision tokens from a preview: items
changed since are reported and left untouched, never merged blindly.

A bulk edit is one SQLite transaction with its history record. Restart or a
storage failure leaves either all of it or none; per-item outcomes cover
validation, scope and conflicts. Undo stores compact inverses (tags added,
tags removed with positions, previous scalar values). It restores an item only
while the item is still at the revision the action produced, so a later edit
always wins. History is bounded (100 actions, ≤ 1,000 items per bulk edit,
≤ 10,000 per rename) and local: it is undo state, not asset metadata, so it is
excluded from portable formats. The consequence: undo does not follow a catalog
to another computer.

Tags use audio's display rules plus Unicode NFC, with case-insensitive identity.
That needed `unicode-normalization` 0.1.26 (pinned, license retained); audio
code is unchanged. Rename and aliases apply to image tags only. Audio
annotations have no revisions, so a combined rename could not be conflict-safe
or reversible; the preview reports the audio count that stays unchanged.
Aliases are library-wide, so they can only be created by a rename across all
sources. Targets must be canonical, so aliases never chain.

Schema v13 adds `image_tag_aliases` and `image_annotation_actions`. Aliases are
durable user data that SS-048 must carry; until then they block audio-only
portable export, like image rows. As with v12, v12 backups need the v12 binary.
