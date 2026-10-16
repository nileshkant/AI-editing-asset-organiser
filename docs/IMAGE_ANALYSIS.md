# Bounded still-image analysis — SS-042

This adds a core analysis service and a dedicated `creativeshelf-image-worker`
executable. Image capability remains disabled. No picker, import job, gallery,
thumbnail, AI tagger or image MCP endpoint is enabled. SS-043 owns import jobs,
worker resource packaging/lifecycle integration and scoped selection in the app.
SS-044 owns cached previews. Originals are read-only throughout this service.

## Call sequence and ownership

1. Capture `ImageSelection` from a catalog source and exact relative member under
   the catalog lock. Capture performs metadata validation only. Release the lock;
   `hash` resolves containment and streams BLAKE3 in 64 KiB chunks. Never broaden
   an exact-file selection to siblings. Register the verified digest using the
   same captured source generation through SS-041's metadata API.
2. Capture `ImageAnalysisRequest` for that registered image/version. Release the
   catalog lock. The worker receives an opened file as stdin; no image path or
   metadata text becomes a command argument, shell command or network request.
3. `analyze` requires a caller-supplied absolute, trusted worker executable; it
   does not search PATH or discover arbitrary tools. One worker may run per app
   process. `worker_busy` is advisory; atomic acquisition returns BUSY to extra
   callers. SS-043 must bound/fairly schedule its durable queue separately.
4. The child hashes exactly the bytes it decodes, verifies structure/metadata,
   completes full decode, converts display color/orientation and measures alpha.
   Parent compares the result with the expected digest and rehashes the current
   contained path. A changed digest never becomes technical-ready.
5. The returned `AnalyzedImage` cannot publish. Call `reverify` outside the catalog
   lock immediately before publication; it rehashes current bytes, including
   same-size/same-timestamp replacements. Only the resulting consumed
   `VerifiedImageAnalysis` can publish through the catalog. Publication rechecks
   registered identity/path/content and source generation/availability/membership
   transactionally. It does not overwrite user annotations or their revisions.

Filesystem edits cannot be made atomic with a SQLite commit against arbitrary
external writers. The final byte fence precedes publication; edits after it need
refresh. Do not delay publication after reverify or treat stat equality as proof
of unchanged content. Local filesystem resolution/open/read calls can stall at
the OS layer on a broken mount; checkpoints bound work between such calls, not
the kernel's filesystem response time. Decoder computation and blocked child
reads are independently killable by the watchdog.

## Admission and resource limits

- Encoded input: 1 byte through 64 MiB, checked at file open and again during child read/hash.
- Raw width/height: 1–32,768; at most 16,777,216 pixels (4096²). Checked multiplication
  rejects overflow, zero dimensions, excessive pixel count and buffer sizes before
  codec output allocation. These are stricter operational limits than SS-040's
  contract ceilings of 512 MiB/100 million pixels.
- Decoded source buffer: at most 128 MiB, admitting 16-bit RGBA at the pixel cap.
- Raw and expanded auxiliary metadata: combined 1 MiB. PNG zlib metadata expansion
  stops after the bound; JPEG APP/COM and WebP auxiliary chunks share the bound.
  PNG chunks are capped at 65,536, JPEG scans at 256, EXIF entries and ICC tags at
  4096. We parse only bounded orientation/ICC data, not embedded instructions.
- Frame count: one still frame. APNG control/frame chunks, WebP animation flags or
  frame chunks and JPEG MPF reject before decode. No first-frame-only success.
- All decoder/color Rust heap allocations: 512 MiB total live requested bytes.
  The dedicated binary's allocator counts moving-reallocation peaks too and
  exits with code 78 if the budget or a system allocation fails. Only that child
  terminates. This is a heap limit, not a cross-platform RSS reservation; mappings,
  stack, system libraries and allocator overhead are additional.
- Decoder deadline: 30 seconds maximum (callers can shorten it), including parent
  hash checkpoints. Watchdog polls at 10 ms, kills and reaps the process on
  cancellation/deadline. The worker slot and watchdog are released on completion.
- Worker stdout: 16 KiB maximum. Stderr is discarded; no unbounded diagnostic or
  image text reaches logs. Replies contain metadata, pixel hash and four corners,
  not a full pixel array. The worker performs no media writes.

Size checks and decoder Limits are defense in depth; neither substitutes for the
separate-process allocation budget. This process does not implement an OS sandbox.
There is no nested decoder process or native codec dependency. Catchable panics
become worker errors; aborts/stack exhaustion become isolated failed exit status.

## Technical and color policy

Content signatures choose JPEG, PNG or WebP independently of extension. A mismatch
adds `extension_mismatch`; unsupported content returns `UNSUPPORTED_FORMAT`.
Structure validation covers PNG lengths/CRC/end marker, JPEG segments/scans/EOI,
WebP RIFF/chunk bounds, EXIF orientation structure and ICC tag offsets. A header
alone or a cached thumbnail cannot establish technical readiness.

Profile facts are raw/oriented dimensions, EXIF 1–8, encoded format/size, encoded
sample depth, ICC identity, decoded alpha-channel presence and actual transparency.
Aspect ratio is an exact reduced integer pair derived from oriented dimensions.
Transient results include source color-space family, transparent/partial counts,
canonical pixel hash, corner samples and measured worker allocator peak. These
are internal results, not new durable fields or public IPC contracts.

Transparency is measured before display quantization: no alpha and opaque alpha
are distinct, binary transparency has only zero/full alpha, partial transparency
has at least one intermediate sample. A 16-bit alpha of 1/65535 remains measured
as partial even if it rounds to zero for display. Palette tRNS is expanded to a
real alpha channel. Lossless WebP always decodes its ARGB plane, even when an
alpha-use hint is unset; an opaque plane and actual transparency remain distinct.
Alpha never becomes premultiplied in this pipeline.

For WebP, a temporary extended RIFF container supplies verified dimensions and
alpha-plane presence to the selected decoder. Encoded image payloads and metadata
are copied unchanged; input identity remains the original bytes. This avoids the
wrapper's simple-VP8L alpha-hint and 16,384-wide dimension edge cases. Google's
[lossless format specification](https://developers.google.com/speed/webp/docs/webp_lossless_bitstream_specification)
defines that alpha-use bit as a hint, not permission to discard pixel alpha.

Canonical display pixels are oriented sRGB RGBA8 with straight alpha. EXIF is
applied exactly once; 5–8 swap dimensions. RGB/gray supported ICC matrix/TRC
profiles convert through moxcms to sRGB, preserving alpha verbatim. Persisted
`source_color_profile` records `icc-blake3:<digest>`, not an invented profile name.
ICC bytes are not copied into the database. Untagged files assume sRGB with an
`assumed_srgb` warning. An explicit PNG sRGB chunk declares sRGB. Unsupported
PNG gAMA/cHRM/cICP without a usable ICC/sRGB declaration fails visibly. Conflicting
ICC/sRGB declarations and malformed ICC data fail. Unsupported color spaces,
CMYK JPEG and LUT-only ICC return a color-profile error. No silent color bypass.

High-bit-depth images retain original bytes and measured depth; display converts
to 8-bit with `high_bit_depth_display8`. JPEG supports ordinary 8-bit baseline/
extended sequential/progressive still images, PNG supports palette/grayscale/RGB/
alpha through 16 bits, and WebP supports still lossy VP8/lossless VP8L. Other
JPEG variants, TIFF/HEIC/AVIF/SVG/PSD and all animations are unsupported.

Errors are typed internal codes for cancellation, timeout, limits, unsupported
format/animation/color, invalid data, source/version change, unavailable source,
permission, missing/failed worker and overload. Failed work leaves the record
pending; SS-043 owns durable job error/state transitions and retry policy.

## Dependencies, platforms and delivery

Direct decoder/color dependencies are exact-pinned and locked. Only JPEG/PNG/WebP
image features are enabled; there is no default GIF/TIFF/AVIF/rayon expansion.
Explicit moxcms uses scalar matrix/TRC functionality; image's own transitive
moxcms 0.8.1 is separately locked. License texts for all new stack components
and PNG are in [licenses/image-analysis](licenses/image-analysis). Flate2 and
crc32fast were already in the application graph; this service makes them direct.
See [the implementation decision](DECISIONS.md) for rationale and upstream sources.

The pure Rust stack avoids a new native ABI/tool install on macOS arm64/x86-64,
Windows x86-64 and Linux x86-64. Local tests establish macOS arm64 only; hosted
matrix results must establish their actual runner targets. No claim is made for
32-bit, mobile, other architectures or installed image workflows. SS-043 must
build/package the exact worker and these notices with image runtime resources;
never execute a helper found in an imported source folder.

This ticket adds no schema: the SS-041 profile fields and validators already
represent its durable facts. Full SQLite backup preserves them; mixed JSON
transfer stays blocked until SS-048. No transient samples/counts become durable
by accident. There are no new UI controls or external AI/model calls to qualify.
