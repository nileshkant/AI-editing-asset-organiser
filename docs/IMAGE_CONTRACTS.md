# Image contract v1 — SS-040

Status: executable data contract; no image service is enabled
Baseline: merged main `f4fa563f656785fb5171d4eea459782ff13991a2`
Implementations: [Rust](../crates/core/src/image_contracts.rs),
[TypeScript](../src/contracts/imageV1.ts)
Fixtures: [image bundle](../fixtures/contracts/image-v1.json),
[shared conformance cases](../fixtures/contracts/image-v1-cases.json),
[legacy audio snapshots](../fixtures/contracts/legacy-audio-v1.json)

## 1. Existing interfaces remain unchanged

The [baseline inventory](../fixtures/contracts/baseline-inventory.json) records
all 63 registered Tauri handlers and all 17 MCP tool names at the reviewed commit.

- SQLite schema: 9; no schema bump, new table, migration, or user database access
  is introduced by SS-040.
- Audio profile analyzer: `soundshelf-pcm-v1`.
- Portable catalog: `soundshelf-catalog/v1`; source snapshot:
  `creativeshelf-source/v1`; legacy import:
  `portable-sound-effects-catalog/v3`.
- At this merged baseline, portable JSON and folder snapshots both use the
  existing 64 MiB bound. The other worktree's uncommitted 256 MiB folder-snapshot
  change is not a merged contract and is not included in this branch.
- Existing `Sound`, `Profile`, `Clip`, source UUIDs, audio annotation shape,
  and string frame counters are untouched. Search/get omit waveform payloads;
  portable profiles retain their waveform.
- Existing read tools include service/library/source status, sounds.search,
  sounds.facets, sounds.get, sounds.resolve, clips.get/list, jobs.get/list,
  and catalog.export. Existing mutations are sounds.annotate, clips.create/
  update/export, and jobs.cancel, with their established scoped grants.
- Relevant current IPC includes choose_folder/files/file, import_path/root,
  scan_source, sources, search_sounds, get_sound, annotate, create/get/update
  clip, export_clip, catalog transfer, and MCP lifecycle handlers. The inventory
  is authoritative for exact handler names, including module prefixes.
- Current access fields remain source_ids/read/edit/export/paths. A new image
  capability must not silently widen a persisted audio client's media access.

No `images.*`, `elements.*`, or image IPC handler is registered in SS-040.
Declaring serializers is not advertising a usable image feature.

## 2. Scope and version boundary

`creativeshelf-image-contract/v1` is a **conformance-bundle schema**, not a
replacement portable catalog, snapshot, database schema, or active endpoint.
It contains sources, reserved legacy audio IDs, and complete synthetic image
records so both languages can validate identities and references together.
The separate audio fixture uses the actual legacy portable/agent schemas.

The bundle fields are schema, sources, audio_ids, and images. All fields,
including nullable ones, are required. Unknown fields, unknown enum values,
wrong JSON types, missing required fields, and unsupported schema versions
fail. JSON object member order is insignificant. Use canonical serializers;
duplicate textual JSON keys are not a supported way to express revisions.
Rust and TypeScript parse JSON into the same last-key-value object semantics.

The contract parser checks envelope size before deserializing and validates
the resulting records. Direct Rust Deserialize or a TypeScript type assertion
does **not** establish semantic validity. Validate before using or serializing
manually constructed data; serializers can otherwise turn a nonfinite number
into null. Invalid Unicode surrogate strings are rejected.

SS-041 owns additive database storage. SS-048 owns future versioned mixed
portable/source envelopes. They must use new explicit schema versions rather
than insert images into legacy sounds arrays, rename old schema identifiers,
or let an older reader silently drop new fields. Old catalogs continue to
work through the old parser; unsupported future catalogs fail with an
actionable upgrade/compatible-backup message. No silent lossy downgrade.

Future durable fields require a versioned contract extension and matching
Rust/TypeScript fixtures before use. Keep old fixtures readable by their
version's parser; do not reinterpret a v1 field's meaning. This ticket does not
assign the next SQLite migration number before SS-041 inspects main.

## 3. Asset identity and source membership

The header fields are id, kind, source_id, relative_path, content_hash,
technical_state, availability, and enrichment_state.

- New image/element asset IDs are canonical lowercase, non-nil UUID strings.
  Existing sound IDs are preserved exactly, including legacy non-UUID IDs.
  A shared identity allocation/check in SS-041 must prevent image/audio
  collisions; the bundle verifies against audio_ids.
- Image content_hash is lowercase 64-character BLAKE3 hex, or explicit null
  until bytes have been established. Content is not asset identity. Two file
  records may share a digest and analysis while keeping independent annotations.
- Membership is keyed by source ID plus exact normalized relative path, across
  all media kinds in the future persistent registry. The bundle checks image
  membership and reserved audio identities; the legacy audio fixture separately
  checks the old audio membership contract. Structural validation does not
  replace a database uniqueness constraint or live source authorization.
- Paths use slash separators, preserve Unicode/code-point spelling, and reject
  absolute/drive paths, backslashes, colon, empty components, dot/dot-dot,
  NUL/C0/DEL controls, and excessive UTF-8 bytes. Do not silently Unicode-normalize,
  case-fold, or rewrite a filename. OS case collisions require reconciliation.
- Source IDs remain existing bounded strings. Folder sources have an empty files
  array and a separately granted root; file-scoped sources enumerate exact
  relative members. An image outside those members is rejected.
- Moving a root preserves IDs; replacing bytes at the same path changes the
  version, invalidates content-bound evidence/recipes, and preserves user
  annotations. Rename/duplicate-content ambiguity never silently merges IDs.
- Containment, source generation, permissions, symlink/drive replacement,
  and current-file verification belong to service operations. The validator
  does no filesystem reads and cannot authorize an arbitrary local path.

## 4. Independent state axes

TechnicalState is pending, ready, or failed. Availability is available,
offline, missing, or inaccessible. EnrichmentState is disabled, queued,
running, complete, failed, cancelled, or stale.

Ready requires a known content hash and an image profile. A profile always
belongs to a known content version. A source may be offline while a record
retains ready technical facts and cached previews; export still requires a
fresh source check. Failed AI does not change technical readiness or erase tags.

A complete enrichment state requires at least one current, non-stale model
evidence record and technical readiness. A model can report unknown/no labels
with empty tags and a null caption; retaining that evidence records successful
coverage without inventing content. Disabled state may retain older evidence.
Queued/running transitions and lease/cancellation rules are owned by SS-052.

Null means unknown/unrecorded. It is not zero dimensions, rating zero, opaque
alpha, confidence zero, or an empty string. Omitting a required nullable field
is a malformed payload, not a different spelling of null.

## 5. Technical profile

ImageProfile contains raw_width/raw_height, EXIF orientation, oriented
width/height, format, encoded_bytes, bit_depth, source_color_profile,
alpha_channel, transparency, and analyzer_version.

- Format names are jpeg, png, and webp. This reserves still-image records;
  SS-042 must qualify actual decoding and reject unsupported animation.
- Raw dimensions are before EXIF orientation. Orientations 1–4 preserve width/
  height; 5–8 swap them. Both dimension pairs and their products are bounded.
- Transparency is none, binary, or partial. Alpha-channel presence alone
  does not imply any transparent pixels. Without an alpha channel transparency
  must be none; JPEG cannot claim alpha. Partial means some pixel alpha lies
  strictly between zero and fully opaque.
- source_color_profile is a bounded descriptive identifier or null. It is not
  an embedded unbounded ICC blob. bit_depth describes source encoding with
  supported contract values 1, 2, 4, 8, 10, 12, or 16; actual decoder support
  is separately qualified.
- analyzer_version identifies preprocessing/technical measurement semantics.
  Measured facts come from verified complete decoding in SS-042, never from
  the extension, filename, thumbnail, or a model caption alone.
- Color-managed display/export uses sRGB in the initial recipe contract.
  Preserve original bytes and profile; conversion and unsupported-profile
  warnings are implemented and tested by SS-042/044/065.

## 6. Coordinates, recipes, and alpha

All crop/mask/region coordinates refer to the displayed image after EXIF
orientation, before crop/resize. Origin is top-left; x points right, y down.
Pixel (i,j) has center (i+0.5,j+0.5). Image edges span [0,width] × [0,height].

For raw edge coordinates (x,y), width W and height H, EXIF transforms are:
1: (x,y); 2: (W-x,y); 3: (W-x,H-y); 4: (x,H-y);
5: (y,x); 6: (H-y,x); 7: (H-y,W-x); 8: (y,W-x).
Pointer/display transformations must be inverted before persisting a coordinate.

PixelRect uses integer left/top inclusive and right/bottom exclusive.
Require 0 ≤ left < right ≤ source_width and 0 ≤ top < bottom ≤ source_height.
Coordinates are not silently clamped by the core. UI constraints may visibly
limit dragging, but persisted invalid input is an error.

For a normalized detector rectangle in [0,1], use floor for left/top and ceil
for right/bottom after multiplying by oriented dimensions. Reject nonfinite,
out-of-range, inverted, or zero-area input rather than quietly expand it.
Normalize pixel-center prompts using their oriented x/width and y/height;
a point at an exclusive right/bottom edge is not an inside pixel center.
Preview scaling and high-DPI device coordinates are not stored in recipes.

ImageRecipe contains its own UUID, owner asset ID, source version, positive
revision, source geometry, crop, explicit output dimensions, allow_upscale,
color_policy=srgb, and stale flag. Source dimensions are retained so an old
recipe remains interpretable when the file is replaced. Current recipes must
match the current profile; old-version recipes require stale=true.

Output dimensions are explicit: this initial contract permits intentional
nonuniform resizing, but performs no hidden stretch/fit decision. SS-061 UI
defaults to preserving aspect ratio and must preview exactly the submitted
dimensions. No axis may be enlarged beyond the crop unless allow_upscale=true.
A future richer fit-mode recipe requires explicit versioned semantics.

Core color/mask convention is unassociated RGBA at API/artifact boundaries.
Filtering/resampling must premultiply RGB by alpha, then unpremultiply safely;
zero alpha cannot divide by zero. Output alpha is original alpha multiplied
by selection alpha, without turning existing transparent pixels opaque.
sRGB transfer handling, bit depth and conversion tolerances require pixel
fixtures in rendering tickets. Checkerboard is display chrome, never media.

SS-040 reserves only geometry/identity: it does not rasterize crops, transform
pixels, segment objects, or store mask binaries.

## 7. Human annotations and generated evidence

Annotations contain positive revision, tags, comment, favorite, nullable
rating (1–5), and nullable user caption. All changes later use expected revision;
do not overwrite concurrent edits. Same-content files retain separate annotations.

Tags preserve display spelling. Contract validation rejects exact duplicates,
surrounding Unicode White_Space, controls, and size/count overflow. Alias/case-fold
normalization belongs to SS-046; deserialization never silently changes user text.
Comments and captions permit line breaks but reject NUL and size overflow.

Evidence has UUID, content_hash, origin, coverage, tags, nullable caption/model/
confidence, confidence_kind, review_state, stale flag, and created_at_ms.
Origin is model, filename, imported, or user; coverage is full_image, preview,
or element; review_state is suggested, accepted, edited, or rejected.

Model origin requires provider/model/revision/preprocessing/output_schema.
Other origins must not carry a fabricated model. Missing confidence is null
with confidence_kind=unavailable; finite scores in [0,1] are explicitly reported
or calibrated. Only model evidence may carry a score. The validator checks the
representation, not whether the provider's confidence was actually calibrated;
SS-050 must supply evaluation evidence before labeling it calibrated.

Evidence from different content must be stale. Old evidence may coexist with
current evidence, but cannot be presented as a current analysis. User correction,
rejection, and model updates never automatically overwrite human annotations.
SS-053/054 extend category/review workflow schemas with their own versioned
fixtures; this contract does not claim those workflows already exist.

## 8. Derived lineage reservation

kind=element requires lineage; kind=image must have lineage=null. Lineage records
parent_id, parent_version, recipe_id/revision, and mask_hash/revision. Self-parent,
known cycles, and a known audio parent fail. An absent parent is valid for an
authorized subset, relocated pack, or disconnected source; it grants no access
to that parent and is not evidence that the original can be regenerated.

Recipe/mask references identify exact historical extraction inputs; they do not
imply the parent's current version or crop is still usable. The v1 reservation
does not encode extraction operations or require a hidden parent's recipe body.
SS-073–077 extend extraction records and implement atomic artifact references.

Rendered child PNGs remain valid independently of a missing parent. Changed
parents make regeneration stale rather than destroying children. Child tags
must describe the child; background/other-subject tags are not blindly inherited.

Binary masks/drafts are durable user work, stored as bounded owned artifacts with
hashes/revisions and backup policy; not evictable thumbnail caches. Metadata-only
transfer must disclose absent artifact bytes. Extracting visible pixels does
not reconstruct hidden body parts or generate a new pose.

## 9. Limits and numeric encoding

Named constants are exported by Rust and TypeScript. These are conservative
contract bounds, not measured throughput or model-quality promises:

- Bundle/job-control JSON: 2 MiB UTF-8; at most 1,000 sources, image records, or
  reserved audio IDs per conformance bundle and 1,000 selected paths per source.
  A future paged service must keep smaller transport limits when necessary.
- IDs/provider/model/version labels: 256 UTF-8 bytes; relative paths: 32,768.
- Captions/comments: 40,000 UTF-8 bytes; tags: 64 × 256 UTF-8 bytes.
- Evidence and recipes: at most 128 each per asset in this bounded contract.
  Production history will page/archive rather than silently truncate.
- Dimension: 32,768 per axis; total decoded pixels: 100,000,000; encoded image:
  512 MiB. Multiply using checked widened integers, never a wrapping u32 product.
  SS-042 additionally bounds decoded bytes/strides/metadata and worker concurrency.
- Preview ceilings: 2,048 pixels per edge and 4 MiB encoded; a transport must use
  a smaller tier when encoding/resource overhead would exceed its payload cap.
- Future image list page size: default 50, maximum 100. Stable ordering is
  explicit selected sort plus ascending asset ID tie-breaker. Metadata lists
  reflect a live observation, not a frozen snapshot; opaque query/version-bound
  cursors in SS-058 must reject changed query parameters rather than leak scope.
- Revision and image geometry fields are unsigned 32-bit integers. File sizes
  and Unix-millisecond timestamps are exact nonnegative JS-safe integers;
  values above 9,007,199,254,740,991 are rejected. Existing audio sample/frame
  counters remain decimal strings and are not converted to numbers.

Future service settings may lower these limits per machine/provider. They cannot
silently raise the wire contract ceilings or accept client-supplied overrides.
Any raised format/resource ceiling needs versioned tests and measured justification.
These constants do not instantiate a queue or allocate buffers in SS-040.

## 10. IPC, errors, and authorization

New record payloads use snake_case, matching existing core data conventions.
Invocation argument envelopes use camelCase. The executable ImageEditArgs fixture
has assetId, expectedContentHash, expectedRevision, with no snake_case aliases.
It demonstrates the optimistic-write boundary; no edit handler is registered.
Mutation handlers later authenticate/scope-check before exposing asset existence,
then compare source version and annotation/recipe revision before committing.

Contract errors serialize as INVALID_ARGUMENT, UNSUPPORTED_SCHEMA, LIMIT_EXCEEDED,
INVALID_PATH, WRONG_ASSET_KIND, IDENTITY_CONFLICT, INVALID_REFERENCE,
STALE_VERSION, and INVALID_RANGE. Unknown enum/type/field failures use
INVALID_ARGUMENT; a missing/unsupported bundle schema uses UNSUPPORTED_SCHEMA.
Bounds may be rejected during primitive parsing before a more specific semantic
error is available; both languages must reject, and named shared error cases
assert identical codes.

These are validation errors, not permission results. Future service adapters map
private diagnostics to fixed public messages and existing PERMISSION_DENIED,
ASSET_CHANGED, ASSET_MISSING, SOURCE_OFFLINE, NOT_READY, MODEL_UNAVAILABLE,
RATE_LIMITED, JOB_CANCELLED, EXPORT_COLLISION, and DISK_FULL semantics.
Authorization failures precede revealing wrong-kind or identity details.
The existing audio error messages are not changed in this ticket.

## 11. Persistence and next owners

- SS-041: additive registry and migration, globally unique asset identity and
  source membership, backup before upgrade. Source generation and live-path
  checks remain shared service responsibilities.
- SS-042–047: actual decode/import/cache/gallery/annotations/source lifecycle.
  Guard mixed persistence until SS-048; do not let old writers discard images.
- SS-048: mixed metadata envelopes, source snapshots, transactional import,
  read-only/retry behavior, backup restoration. Metadata exports exclude keys,
  machine grants, private absolute roots, original media, model weights, and
  regenerable thumbnails/vector indexes.
- Database backup preserves durable local annotations/evidence/recipes plus
  declared artifact dependencies. A metadata-only catalog is not a complete
  media or editable-mask backup. Media packs explicitly include selected media
  and manifests; no automatic ancestor/sibling collection.
- SS-049–057: provider/model/consent/evidence/correction/provenance/OCR services.
- SS-058–064: canonical search and creator workflows; SS-065/066: rendering and
  editor handoff; SS-067/068: explicitly scoped image tools.
- SS-073–077: actual selection/matting/PNG/lineage implementation.
  SS-072/078 qualify exact installed workflows.

Never downgrade by editing SQLite user_version. Restore the compatible
pre-upgrade backup with the app closed. A failed migration/export must preserve
the previous durable state.

## 12. Executable evidence and limits

The same conformance cases run through Rust and TypeScript, covering nullable
fields, enum/type/schema rejection, Unicode paths, file scopes, identity
collisions, oriented geometry, alpha, revisions, uncertainty, stale evidence/
recipes, lineage, and range limits. Separate direct tests cover nonfinite values,
byte/count ceilings, invalid Unicode, and exact camelCase arguments.

Legacy fixtures round-trip the existing portable and source schemas and exercise
actual scoped core sounds.get/search and clips.get/list responses against fixed
expected JSON. Audio-tool discovery is frozen against the 17-method inventory.
These are contract tests on synthetic metadata, not a claim of actual image
decoding, AI quality, native audio listening, or installed-platform qualification.
