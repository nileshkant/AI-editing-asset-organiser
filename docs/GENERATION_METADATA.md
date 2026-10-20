# Generation metadata and prompt provenance (SS-055)

CreativeShelf reads generation settings that other tools wrote into image files
and keeps them as **claims**: each value records the format, container and key it
came from. Nothing is inferred from pixels, no embedded workflow is run, no URL is
fetched and no path in the metadata is followed. Your own entries are stored
separately and are never replaced by a new file version or sidecar.

Code: `crates/core/src/image_generation.rs` (reader),
`crates/core/src/image_provenance.rs` (catalog), `generation_schema.sql`
(schema v16). Rationale: [DECISIONS.md](DECISIONS.md) ("SS-055 generation
metadata"). Review evidence: [reviews/SS-055.md](reviews/SS-055.md).

## Supported formats (the allowlist)

| Format | Where it is read | Fields |
| --- | --- | --- |
| A1111-style `parameters` | PNG `tEXt`/`zTXt`/`iTXt`, JPEG and WebP EXIF `UserComment`, `<stem>.txt` sidecar | prompt, negative prompt, steps and other settings, seed, sampler, model, model hash, LoRA references |
| ComfyUI `prompt` graph | PNG text chunks | prompt and negative prompt by following one `KSampler`'s links to `CLIPTextEncode`, checkpoint, LoRAs, seed, sampler; `LoadImage` names as private references |
| InvokeAI `invokeai_metadata` | PNG text chunks | prompt, negative prompt, model, seed, scheduler, settings |
| NovelAI `Comment` | PNG text chunks, only with `Software` = `NovelAI` | prompt, negative prompt (`uc`), seed, sampler, settings |
| IPTC digital source type | JPEG and WebP XMP | `DigitalSourceType` (for example `trainedAlgorithmicMedia`), `xmp:CreatorTool` |
| Software tag, EXIF date | PNG text, EXIF | generator and `DateTimeOriginal` with `OffsetTimeOriginal`, only alongside another claim |

The parser version is `creativeshelf-generation-v1` and is stored with every
result. Anything else is ignored. A ComfyUI `workflow` chunk and unknown
structured chunks are kept only as a name and byte count ("kept unread and never
run").

Fallbacks:

- No supported metadata: status `none`, every field shows **Unknown**.
- Malformed JSON, bad CRC or bad compressed data: status `malformed` with an
  issue per part; other parts are still read.
- A limit hit: status `limit_exceeded`. A value over a limit is dropped, never
  truncated.
- A ComfyUI graph with no `KSampler` or more than one is **ambiguous**: its prompts
  stay unknown rather than guessed.
- A seed written as a JSON number that cannot be represented exactly is
  **inexact** and left unknown. Seeds are kept as text, so
  `18446744073709551617` survives.
- A date without a time zone is shown with "time zone unknown".
- Empty values stay empty (`(recorded as empty)`), distinct from unknown.

Metadata never affects whether an image is ready. Decode, thumbnails and analysis
use the same bytes and the same readiness rules as before.

## Limits

| Limit | Value |
| --- | --- |
| Metadata segments or chunks per file | 64 |
| One segment | 1 MiB |
| Decompressed text, per chunk and per file | 1 MiB |
| Prompt or negative prompt | 64 KiB |
| Other text fields | 1 KiB |
| Settings / references / claims / issues | 64 / 32 / 8 / 16 |
| JSON nesting / ComfyUI graph nodes | 32 / 4,096 |
| Sidecars per image / sidecar size | 1 / 64 KiB |
| Parser CPU per file | 2 s |
| Stored extraction | 512 KiB |
| Your provenance record | 256 KiB |

XMP is searched as text. No XML parser runs, so entities and DTDs are never
expanded.

## When files are read

- **Import.** New content is read once: the import already reads the image to
  hash it, and the metadata reader runs over the same buffer. The result is keyed
  by content hash and parser version.
- **Upgraded libraries.** Images imported before schema v16 have no result. The
  next Check files or refresh reads each one once, checking that the bytes still
  match the stored hash.
- **Changed files.** A new content hash gets its own result; the old version's
  result stays with the old version. Your entries stay.
- **Sidecars.** Only `<image stem>.txt` next to the image, only for folder sources
  (an exact-file grant never reads a sibling), only inside the source root, and
  only in the A1111 layout. A sidecar is re-read when its size or modified time
  changes. Its revision increments whenever its contents change or it is
  removed; a removed sidecar's values no longer apply.

## Origins shown in the inspector

Each field in the inspector's **Generation provenance** section shows one origin:

- **Your entry**: typed by you. An empty box removes your value, and any imported
  claim shows again.
- **Claimed by** *format · container "key"*: written by another tool and not
  verified. When claims disagree, the first is shown with "another claim records
  a different value".
- **Inherited from the parent image**: a derived element with no value of its
  own takes the nearest parent's value, field by field, up to 32 lineage hops.
- **Unknown**.
- **Removed when this library was exported**: the value was redacted by a
  shared export.

You can edit prompt, negative prompt, generator, model, seed, sampler, created
and AI-generated (yes or no). Edits are checked against the revision you saw;
if the record changed elsewhere, the latest is shown and nothing is overwritten.
**Mark imported values reviewed** records which content, parser and sidecar
revision you reviewed. If any of these changes, the review is shown as outdated.
Editing needs the original to be available, as for annotations.

## Sharing and privacy

Portable JSON exports and folder snapshots leave out, unless you opt in:

- **Prompts**: prompt, negative prompt and any setting whose name contains
  "prompt".
- **Private references**: ComfyUI `LoadImage` names and any reference or setting
  that looks like a local path (`/…`, `~…`, `C:\…`, `\\server`, `file:`).

Removed fields are listed in the exported record's `redacted` list, so an
import shows them as removed rather than unknown. The Catalog data settings
choose this per JSON export. Folder snapshots use the library setting. Changing
that setting marks affected snapshots for rewrite. Markdown exports carry no
generation data. Database backups are complete local copies and keep everything.

Portable JSON carries generation data as an optional `generation` field on each
v2 image record: `embedded` (this version's extraction), `sidecar` and
`provenance` (your entries and review). Older v2 documents without it import
unchanged.

## Search policy

The library setting decides which generation text image search may index:
**Do not search it**, **Everything except prompts** (default) or **Also search
prompts**. Local paths are never indexed. Image search arrives in SS-058; until
then the choice is saved and nothing is indexed.

## Not supported

C2PA manifests, generator connectors, prompt reconstruction from appearance,
automatic regeneration and folder watching of `.txt` sidecars (a Check files or
refresh picks up sidecar changes).
