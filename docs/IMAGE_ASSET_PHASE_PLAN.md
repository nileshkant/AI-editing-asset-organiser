# CreativeShelf — Image library and reusable elements proposal

Date: 16 October 2026
Status: proposed product scope; no image feature implementation or release claim
Parent ticket: [SS-032](../tickets/SS-032.md)

## 16 October 2026 implementation backlog

This proposal is now expanded into **39 repository tickets, SS-040–078**.
Use the [strict execution queue](../tickets/IMAGE_QUEUE.md) for implementation
order and [IMAGE_CONTRACT.md](../tickets/IMAGE_CONTRACT.md) for mandatory technical
and PR rules. The queue maps all IMG-01–26 features to their owners.

One ticket is implemented, tested, reviewed, and merged before the next starts.
Do not skip a blocked ticket or use the Core/Expansion labels below to jump
ahead. Those labels retain product-priority context; the queue is the concrete
sequence requested after this proposal. SS-032 remains a tracking umbrella.

## Product direction

Turn folders of generated and collected images into a searchable production library that people and authorized AI agents can reuse in videos, thumbnails, presentations, and other assets. The workflow is: select files/folders → index technical metadata → optionally analyze the actual images → review and add tags → find assets → preview → export/use in a project → finally, extract selected objects into reusable transparent PNGs.

Support all supported images regardless of whether they were AI-generated, photographed, purchased, or drawn. Generation history is optional metadata, not a condition for import. Do not infer an image's origin from its appearance.

The core product value is **reuse with production context**: the right subject, style, dimensions, composition, recorded usage conditions, and a working file that a creator or agent can actually use.

## Existing-ticket review and reuse

Reviewed the local SS-001 through SS-039 ticket definitions/acceptance lists, backlog, product decisions, MCP contract, current audio types, and local merge history at `5084baa`. This is a planning review, not a fresh implementation or installed-release audit.

- SS-002–013 and SS-026 provide the desktop, sources, catalog identity, jobs, user annotations, search, playback, clips, and export foundation.
- SS-018–022 and SS-029 provide agent access, editor handoff, portability, backup, and source-folder catalogs. Extend these shared services for images.
- SS-030 already established the broader CreativeShelf branding.
- SS-032 is the existing image-assets umbrella, currently not started. This proposal expands it into 26 planning workstreams; it does not create 26 published issues or mark existing tickets done.
- SS-014–017 remain planned provider/AI capabilities. Reuse their configuration and model-management concepts, but qualify image input separately. Image tagging does not need to wait for audio event recognition itself.
- SS-039 is planned approved agent import and audio tagging. Reuse the source-grant pattern for images; current MCP tools do not already provide image import or analysis.
- SS-033 Remotion presets remain separate. Ordinary image handoff to a Remotion project should not depend on implementing executable presets.
- The backlog says 23 complete, 2 in review, 6 partial, 8 not started. That snapshot is stale in places: local history includes SS-037/038 merged through PRs #35/#36 and alpha.6 through #37. Merged code alone does not resolve their outstanding installed checks.
- SS-023/024/031/034 still record release/platform acceptance gaps; SS-035/036 retain platform checks. This proposal proceeds with image planning without representing audio as fully release-qualified. SS-032 currently lists SS-031 as a prerequisite; revise that relationship explicitly if implementation is reprioritized.
- The existing baseline derives audio measurements and supports manual tags; specific AI sound-event recognition remains planned. Image analysis must be delivered as a real new capability.

The initial proposal changed no implementation or user media. Its subsequent
ticket expansion establishes the image execution queue and clarifies shared
legacy-ticket ownership; existing implementation statuses are not marked done.

## Intended creators

- **Short-form and video creators:** find backgrounds and elements appropriate for the script, frame shape, and title placement.
- **YouTubers and thumbnail designers:** compare expressions, framing, color, and clean cutouts.
- **Motion designers:** reuse transparent elements and keep a coherent illustration style across scenes.
- **Freelancers and small studios:** organize project approvals, source attribution, campaign variants, and editor handoffs.
- **Educators and presentation makers:** retrieve diagrams, illustrations, and reference material without remembering filenames.
- **AI-assisted creators:** let an authorized agent search an existing library, inspect suitable previews, and obtain verified files.

These are proposed target use cases, not claims of validated customer demand. Before expanding beyond the first usable release, observe representative creators performing the end-to-end workflow with their own or consented sample libraries.

## Delivery scope and priorities

There are **26 proposed workstreams: 13 Core, 9 Expansion, and 4 Final**. These
IMG planning IDs map to 39 SS implementation/qualification tickets in IMAGE_QUEUE.md;
they are not time estimates.

**First useful image release:** IMG-01–09, IMG-11, IMG-13, IMG-19–20. A creator can select a folder, browse it, obtain optional automatic descriptions/tags, correct them, search naturally, and have an authorized agent export a usable image. This includes one qualified image-analysis path; it need not implement every cloud provider and local pack simultaneously. Manual operation remains useful without AI.

**Creator expansion:** IMG-10, IMG-12, IMG-14–18, IMG-21–22. These improve provenance, visual discovery, composition fit, organization, and project reuse.

**Final feature stage, as requested:** IMG-23–26, implemented after the image discovery/handoff workflow is dependable. Prototype segmentation feasibility early if useful, but deliver object extraction last.

Stage numbers describe product progression. The Core subset crosses stages 1–4.
The subsequently requested strict queue delivers all accepted Expansion work
in its assigned position; changing that sequence requires an explicit amendment.

## Feature proposals

### Stage 1: Build a reliable image library

Outcome: Import a folder, browse it, tag it manually, move it, and recover it without losing audio or metadata.

#### IMG-01 — Shared image and element catalog (Core)

**Feature:** Extend the existing catalog with image and derived-element types, stable asset IDs, content versions, and type-specific metadata.

**Why it is needed:** The audio library and existing agent integrations must survive the addition of images. A cutout also needs its own identity without losing its relationship to the original.

**Creator use case:** A creator moves a mixed audio/image project to an external SSD and keeps all tags, clips, and image references.

**Acceptance boundary:** Upgrade a populated audio catalog without changing sound IDs or clip recipes; round-trip mixed catalogs and restore a pre-upgrade backup.

**Dependencies / reuse:** Existing SS-003, SS-021, SS-029; SS-032 umbrella.

#### IMG-02 — Folder and individual-image import (Core)

**Feature:** Select folders recursively or exact files; show an import preview, progress, cancellation, retry, and optional watching of approved folders. Start with JPEG, PNG, and WebP.

**Why it is needed:** AI generations accumulate across downloads and project folders. Import must handle batches without requiring manual file-by-file work or moving originals.

**Creator use case:** Select an AI exports folder once, then index new generations when Refresh is pressed or an explicitly enabled watcher detects them.

**Acceptance boundary:** Mixed folders, interrupted jobs, unsupported formats, corrupt images, and offline drives have clear per-file states; exact-file import never includes siblings; exported assets cannot trigger infinite import loops.

**Dependencies / reuse:** IMG-01 and existing source/job services.

#### IMG-03 — Technical metadata and thumbnails (Core)

**Feature:** Measure dimensions, aspect ratio, file type/size, orientation, alpha presence, and actual transparent pixels; create bounded thumbnail caches and color-managed previews.

**Why it is needed:** An attractive image can still be unusable at the required resolution or aspect ratio. A PNG extension alone does not prove transparent content.

**Creator use case:** A thumbnail designer filters for images at least 1920 pixels wide; a motion designer filters for actual transparent elements.

**Acceptance boundary:** Rotated and color-profile fixtures preview correctly; huge decoded images hit explicit resource limits; transparency fields distinguish channel support from transparent pixels.

**Dependencies / reuse:** IMG-02.

#### IMG-04 — Visual browsing and inspection (Core)

**Feature:** Provide a thumbnail grid, adjustable sizes, full-image zoom, side-by-side comparison, keyboard navigation, and an inspector with provenance and analysis status.

**Why it is needed:** Creators make visual judgments faster by seeing several candidates. Tags alone cannot tell them whether the exact expression or framing works.

**Creator use case:** Compare four generated characters before choosing the one with the best pose.

**Acceptance boundary:** Large-library scrolling remains responsive during imports; keyboard and screen-reader workflows work; previews distinguish loading, failure, and offline originals.

**Dependencies / reuse:** IMG-03; reuse existing workspace/accessibility patterns.

#### IMG-05 — Personal tags and bulk organization (Core)

**Feature:** Add, remove, rename, and batch-apply user tags, notes, favorites, ratings, and simple tag aliases. Keep user annotations separate from machine suggestions.

**Why it is needed:** Only the creator knows labels such as client approved, episode 12, hero character, or do not use. Bulk edits make a large library manageable.

**Creator use case:** Select 80 images and label them Brand A / Summer campaign without losing their individual AI tags.

**Acceptance boundary:** User edits survive reanalysis, restart, and reimport; rejected tags do not return silently for the same analysis; batch changes support undo or a recoverable history.

**Dependencies / reuse:** IMG-01, IMG-04; extend SS-007 semantics.

#### IMG-06 — Portable metadata, backup, and relinking (Core)

**Feature:** Extend source-folder catalogs and backups for image metadata; reconcile moved, missing, read-only, and offline sources while preserving annotations.

**Why it is needed:** Creator libraries commonly live on removable drives. Rebuilding all labels after a move wastes time and can incur repeated AI charges.

**Creator use case:** Move a 10,000-image library to another drive, relink it, and retain descriptions and selections.

**Acceptance boundary:** Unchanged files reuse compatible analysis; partial scans do not prune assets; older/newer catalog versions report actionable compatibility states; backup/restore preserves images and audio metadata.

**Dependencies / reuse:** IMG-01–05; existing SS-006, SS-021, SS-022, SS-029.

### Stage 2: Understand the images

Outcome: Run optional visual analysis with understandable provenance, durable corrections, and controlled data transfer.

#### IMG-07 — AI configuration, privacy, and job budgets (Core)

**Feature:** Offer image-capable providers or optional qualified local packs; show what is sent, scope consent to selected sources, estimate batch usage when possible, and enforce budget/concurrency limits.

**Why it is needed:** Some creators prioritize private client work; others want cloud quality or speed. Thousands of images must not produce surprise uploads or an uncontrolled queue.

**Creator use case:** Analyze a public reference folder in the cloud while keeping an unreleased client campaign local.

**Acceptance boundary:** No cloud upload before authorization; cancellation/retry and quota failures preserve baseline browsing; model capabilities are verified independently; secrets remain outside catalogs and logs.

**Dependencies / reuse:** Reuse/extend SS-014 and SS-017 rather than duplicate provider infrastructure; image-specific evaluation required.

#### IMG-08 — Structured visual analysis and auto-tagging (Core)

**Feature:** Generate an editable caption and structured subject/object, scene, action, style, mood, lighting, palette, and composition suggestions. Analyze actual pixels and label filename-derived hints separately.

**Why it is needed:** Generation filenames rarely describe their contents. Structured fields let people and agents ask for useful combinations instead of browsing every image.

**Creator use case:** A watercolor fox in a rainy forest is discoverable by fox, forest, watercolor, rain, muted palette, and left-positioned subject.

**Acceptance boundary:** Use a labeled creator-image corpus; measure tag precision/recall and caption errors; support uncertain/unknown states; record model, content version, and analysis schema. Do not invent numerical confidence when the provider supplies no calibrated score.

**Dependencies / reuse:** IMG-03, IMG-07.

#### IMG-09 — Review and correction workflow (Core)

**Feature:** Show suggested, accepted, edited, and rejected tags; expose analysis origin and date; queue uncertain results for review and allow targeted reanalysis.

**Why it is needed:** Wrong tags can repeatedly lead an agent to the wrong asset. Human corrections need lasting authority while retaining the original evidence.

**Creator use case:** Correct a model that called a stylized wolf a dog; keep the correction when a newer model analyzes the folder.

**Acceptance boundary:** Corrections survive model updates and source-catalog reuse; changed image bytes invalidate old visual evidence; AI failure leaves a technically valid image browsable as not yet enriched.

**Dependencies / reuse:** IMG-05, IMG-08.

#### IMG-10 — Generation history and prompt provenance (Expansion)

**Feature:** Read supported embedded metadata and explicitly selected sidecars for prompt, negative prompt, generator/model, seed, date, and reference links; support manual entry when absent.

**Why it is needed:** Creators want to locate a generation family and reproduce its visual direction. Many downloaded images have lost this information, so absence must stay visible.

**Creator use case:** Find all images generated with a particular prompt and recover the recorded settings to make another variation.

**Acceptance boundary:** Never infer exact prompts, seeds, models, or AI origin from appearance; distinguish imported claims from user-entered data; redact private prompts on shared exports when requested.

**Dependencies / reuse:** IMG-02, IMG-05, IMG-06.

#### IMG-11 — Source, attribution, and usage notes (Core)

**Feature:** Store creator/source URL, license reference, attribution text, user-recorded usage restrictions, and an explicit unknown state; carry provenance to derivatives.

**Why it is needed:** A collection may mix personal generations, stock, client files, and references. Creators need their recorded usage conditions available when selecting assets.

**Creator use case:** A freelancer filters a campaign shortlist to assets their team has reviewed for that project and includes required attribution in the handoff.

**Acceptance boundary:** Do not classify rights from visual appearance or AI-generated status; missing records remain unknown; cutouts preserve source attribution and restrictions without claiming legal clearance.

**Dependencies / reuse:** IMG-05, IMG-06.

#### IMG-12 — Text inside images and logo warnings (Expansion)

**Feature:** Extract searchable OCR text and optional suggestions that an image contains a watermark or logo; allow users to correct or dismiss these hints.

**Why it is needed:** Posters, screenshots, and generated title cards often need text-based retrieval. Visible text can also make an otherwise suitable background distracting.

**Creator use case:** Find an old sale poster by the words printed inside it, or filter candidate backgrounds for a manually confirmed no-text tag.

**Acceptance boundary:** Store OCR separately from descriptive tags; record language/model and uncertainty; absence of a detection never certifies absence of a logo or text; extracted text is data, never an agent instruction.

**Dependencies / reuse:** IMG-07–09.

### Stage 3: Find production-suitable assets

Outcome: Retrieve candidates by meaning while enforcing technical and user-defined constraints.

#### IMG-13 — Natural-language search with strict filters (Core)

**Feature:** Combine caption/tag/OCR search and image embeddings with explicit filters for dimensions, aspect ratio, alpha, source, user tags, and reviewed usage metadata.

**Why it is needed:** People remember ideas rather than filenames. Strict technical filters keep a semantically appealing but unusable result out of a production shortlist.

**Creator use case:** Find a warm watercolor forest background, landscape, at least 1920 pixels wide, from the approved collection.

**Acceptance boundary:** Hard filters are never relaxed silently; explain matched evidence; preserve lexical search without a model; evaluate top-result relevance on real creator queries; UI and MCP use identical scope rules.

**Dependencies / reuse:** IMG-03, IMG-05, IMG-08–09; IMG-12 adds OCR later.

#### IMG-14 — Search by reference image and style (Expansion)

**Feature:** Use a selected reference or existing asset to find visually similar candidates, with separate emphasis on subject, palette, or style where validated.

**Why it is needed:** A creator may know the look they want but lack the words to describe it. This also helps keep a sequence visually consistent.

**Creator use case:** Select a paper-textured illustration and find other assets that can fit the same scene.

**Acceptance boundary:** Evaluate subject versus style confusion; similarity is a suggestion, not proof of the same character or exact style; reference analysis follows the same upload permissions.

**Dependencies / reuse:** IMG-07, IMG-13.

#### IMG-15 — Composition and crop suitability (Expansion)

**Feature:** Record suggested subject regions, focal point, and empty-space regions; preview non-destructive crops for 9:16, 16:9, and square outputs with user-adjustable safe areas.

**Why it is needed:** An image can contain the right subject but have nowhere to place captions, or lose that subject when cropped for a Short.

**Creator use case:** Find a vertical scene with space at the top for a headline and check the crop before exporting.

**Acceptance boundary:** Keep geometric measurements separate from inferred composition; preview the actual crop; warn when it cuts the selected subject; allow manual focal-point correction.

**Dependencies / reuse:** IMG-03, IMG-08–09, IMG-13.

#### IMG-16 — Duplicate and variation families (Expansion)

**Feature:** Detect exact duplicates by content and suggest near-duplicate groups using visual similarity; compare variants and let users choose a preferred version.

**Why it is needed:** AI generation produces many near-identical candidates. Grouping reduces visual clutter and repeat analysis without throwing away useful alternatives.

**Creator use case:** Stack 40 variations of a character and favorite the three expressions needed for a video.

**Acceptance boundary:** Never auto-delete or merge independent annotations; distinguish exact duplicates from visual similarity and intentional derivatives; retain each file identity and grouping override.

**Dependencies / reuse:** IMG-01, IMG-03; IMG-14 for visual grouping.

#### IMG-17 — Collections, smart collections, and brand kits (Expansion)

**Feature:** Create project boards containing audio and images; save filter-based collections; store user-defined character, style, palette, and campaign references.

**Why it is needed:** Production is organized around projects and recurring visual identities rather than file formats. A mixed collection keeps the materials for one video together.

**Creator use case:** A series creator opens a kit containing their approved character art, backgrounds, transitions, and music.

**Acceptance boundary:** Collections reference assets without copying originals; smart collections update predictably; agent access still checks every underlying source; character membership is user-confirmed.

**Dependencies / reuse:** IMG-01, IMG-05, IMG-13.

#### IMG-18 — Output-specific quality review (Expansion)

**Feature:** Show deterministic resolution/crop checks and optional review hints for blur, compression, text artifacts, or other visual defects; preserve human approval/rejection notes.

**Why it is needed:** AI images may look acceptable as thumbnails but fail when enlarged or animated. Suitability depends on the intended output.

**Creator use case:** Flag a background that cannot fill a 4K frame without enlargement and let a creator inspect a questionable face at full resolution.

**Acceptance boundary:** Separate measured limits from subjective hints; test on illustrations and photos; no universal quality score or guarantee that AI has detected all defects.

**Dependencies / reuse:** IMG-03, IMG-04; IMG-08 for optional hints.

### Stage 4: Use the library in creator workflows

Outcome: Export verified files and let authorized agents find and hand them to a real project.

#### IMG-19 — Image export and editor handoff (Core)

**Feature:** Export the original or a new resized/cropped PNG/JPEG/WebP, with optional portable manifest; support drag-out or a reliable file handoff on qualified platforms.

**Why it is needed:** Discovery is only useful if the selected asset reaches the editing project with the right size, alpha, and provenance.

**Creator use case:** Export a chosen background into a Remotion public folder and use the verified path in a composition.

**Acceptance boundary:** Never alter originals or overwrite existing files silently; verify output dimensions/alpha and source version; cancellation/disk-full leaves existing files intact; exported files work after the app closes.

**Dependencies / reuse:** IMG-03, IMG-06, IMG-11; IMG-15 enables saved crop recipes; reuse SS-013/020 principles.

#### IMG-20 — Scoped image tools for AI agents (Core)

**Feature:** Add image search/get/preview/resolve/annotation/export tools through the existing MCP service; return asset ID, version, dimensions, provenance, and job states. Add agent-triggered import only with an exact native source grant.

**Why it is needed:** This completes your central requirement: future AI workflows can find and use the library instead of asking you to upload the same images repeatedly.

**Creator use case:** An authorized video agent searches approved images, inspects a bounded preview, exports the chosen asset, and records its identity in the project.

**Acceptance boundary:** UI/tool search agrees; read, preview-byte disclosure, annotation, source-path, import, and export access are explicit; unauthorized IDs/facets/previews leak no assets; verify real client handoff and missing/stale-file recovery.

**Dependencies / reuse:** IMG-13, IMG-19; reuse SS-018/019/037 and the approval pattern proposed in SS-039.

#### IMG-21 — Brief-to-asset shortlist (Expansion)

**Feature:** Turn a script or scene brief into editable candidate lists with reasons, technical fit, and visible gaps; allow users or agents to confirm the selection.

**Why it is needed:** Creators usually need a coherent group of images for several scenes, not a single search result. A shortlist translates story requirements into reusable assets.

**Creator use case:** For a five-scene history Short, suggest one background and two possible supporting images per scene, and report a scene with no suitable library match.

**Acceptance boundary:** Use current permitted assets only; never fabricate a missing file or claim a preview was inspected when it was not; expose constraints and permit manual replacement.

**Dependencies / reuse:** IMG-13, IMG-19–20; IMG-17 helps consistent kits.

#### IMG-22 — Project usage history and portable asset packs (Expansion)

**Feature:** Record assets exported or explicitly reported as used in a project; package selected media and metadata into a portable handoff with attribution and version records.

**Why it is needed:** Creators need to revisit a video, avoid accidental repetition, and pass a complete set of materials to collaborators.

**Creator use case:** Reopen Episode 12 and find the exact background and character variant exported for it, then hand the approved pack to an editor.

**Acceptance boundary:** Label exported versus confirmed-used events distinctly; do not claim editor usage without integration evidence; bundles exclude secrets and out-of-scope files and retain a readable manifest.

**Dependencies / reuse:** IMG-06, IMG-11, IMG-19–20.

### Stage 5: Extract reusable objects — final feature stage

Outcome: Select one object, refine it, export a real transparent PNG, and reuse that element later.

#### IMG-23 — Select a particular object (Final)

**Feature:** Click an object or draw a box, add positive/negative points, and preview a segmentation mask. Whole-background removal can be a shortcut, but must not replace selection of a specific object.

**Why it is needed:** Your final requirement is to extract one chosen element from a multi-object image. A generic foreground remover may keep the wrong subjects or all of them.

**Creator use case:** Select only the fox from a forest illustration, leaving the trees and other animals outside the cutout.

**Acceptance boundary:** Multi-object fixtures demonstrate correct selection; masks use original-image coordinates and content versions; support correction, cancel, and bounded compute; evaluate local/provider options before choosing one.

**Dependencies / reuse:** IMG-03, IMG-04, IMG-07; begins the final feature stage.

#### IMG-24 — Mask refinement and edge inspection (Final)

**Feature:** Add erase/restore brushes, zoom, feathering, and edge cleanup; preview against light/dark/checkerboard backgrounds; qualify optional hair/fur alpha matting separately.

**Why it is needed:** A rough mask is often unusable in motion graphics. Hair, fur, thin lines, translucent edges, and color halos need inspection and correction.

**Creator use case:** Restore a missing whisker and remove a background halo around a watercolor animal before placing it on a dark slide.

**Acceptance boundary:** Test thin details, soft edges, glass, and partial occlusion; allow semi-transparent alpha where supported; undo/redo works. Do not promise recovery of hidden object parts or perfect transparency.

**Dependencies / reuse:** IMG-23.

#### IMG-25 — Verified transparent PNG export (Final)

**Feature:** Export the selected visible object using source pixels and an alpha mask, with tight bounds or preserved canvas, optional padding, and a saved extraction recipe.

**Why it is needed:** Creators need a real transparent file usable in editors, not a checkerboard baked into an image or an AI-redrawn approximation of the object.

**Creator use case:** Save the fox as a tightly cropped transparent PNG and place it over several backgrounds in a video.

**Acceptance boundary:** Inspect RGBA output and partial alpha; preserve selected visible source pixels, orientation, and intended colors; never overwrite original; stale source/mask blocks export; verify the PNG in an independent editor.

**Dependencies / reuse:** IMG-19, IMG-23–24.

#### IMG-26 — Reusable extracted-element library (Final)

**Feature:** Register each exported cutout as a derived asset with parent ID/version, mask/recipe, its own tags, preview, source attribution, and agent discovery.

**Why it is needed:** Extraction should build a reusable element collection. Otherwise every creator repeats the same selection work for the next video.

**Creator use case:** An agent later searches for transparent watercolor fox, retrieves the approved cutout, and can trace it back to the full illustration.

**Acceptance boundary:** Give derivatives independent identity; do not blindly inherit background tags; existing rendered PNGs remain usable if the source goes offline; source changes mark regeneration recipes stale; retain scoped access and provenance.

**Dependencies / reuse:** IMG-01, IMG-06, IMG-08–11, IMG-20, IMG-25.

## Example asset record

For an illustrative image of a fox in a forest, keep separate records for:

- **Measured facts:** 2048 × 3072 pixels, PNG, portrait orientation, alpha channel present but no transparent pixels.
- **Model suggestions:** fox, forest, watercolor, muted green, subject toward left, possible space for text on right.
- **User annotations:** Episode 12, approved character, do not crop ears.
- **Generation provenance:** imported or user-entered prompt/model/seed, or unknown.
- **Source/usage notes:** recorded source and attribution, reviewed status, or unknown.
- **Asset state:** stable ID, content version, source availability, technical-ready state, and separate AI-analysis state.
- **Derived elements:** a separately identified fox cutout linked to the exact source version and extraction recipe.

Measured dimensions are facts. “Watercolor” and “space for text” are interpretations. “Approved character” is a creator decision. These must not collapse into one undifferentiated tag list.

## Data and architecture decisions

1. Extend shared Rust catalog/search/export services; keep React and MCP as adapters. Avoid storing images inside the audio `Profile` or renaming/removing existing audio contracts.
2. Prefer additive, versioned asset-kind support. Preserve stable sound IDs, clip recipes, source identities, folder-catalog compatibility, and backups.
3. Keep originals by reference and immutable. Store thumbnails, embeddings, masks, and analysis in bounded caches or managed data; materialize image derivatives through explicit export.
4. Make technical import readiness independent of optional AI completion. A valid image remains browsable when AI is disabled, queued, or failed.
5. Key reusable analysis to content hash + model/version + preprocessing/schema version. Key per-file annotations to asset identity. Identical bytes can share analysis without merging user metadata.
6. Share source authorization and strict search filters across UI/MCP, including previews, counts, similarity queries, derived assets, and collection membership.
7. Keep object selection, mask refinement, and PNG encoding separate. Semantic object labels, object localization, segmentation, and soft-edge alpha matting are different capabilities.
8. A cutout contains the visible source pixels. Recovering hidden limbs, generating a new pose, removing the object and filling the original background, or inventing new pixels is a separate generative-editing scope.
9. Treat filenames, prompts, OCR, captions, and model responses as untrusted content. They cannot authorize tools, filesystem access, or external uploads.
10. Do not promise seamless remote-agent access to local files. Verify each client/runtime path: a local client may use granted paths; a remote client needs an explicitly supported media-transfer workflow.

## Proposed acceptance and evaluation plan

Create a consented fixture set spanning photos, illustration, watercolor, collage, 3D renders, screenshots, text-heavy images, existing transparent PNGs, repeated characters, and several objects per frame.

- **Import/recovery:** test corrupt files, extreme dimensions, orientation, color profiles, missing drives, changed bytes, cancellation, restart, read-only folders, and cache rebuilds.
- **Search usefulness:** label real creator queries and evaluate relevance among the top results, hard-filter accuracy, and time from query to a usable selection.
- **Tagging:** evaluate per-category precision/recall, false descriptions, uncertain results, and correction persistence. A successful provider response is not quality evidence.
- **Scale and cost:** measure cold import, warm reuse, scrolling/search latency, memory, disk-cache growth, AI latency, and per-batch usage on declared hardware and library sizes. Set release targets after a representative spike; do not invent throughput estimates.
- **Agent reuse:** one real permitted client must search, inspect, choose, export, and use an image in a small video/project; negative cases must reject unauthorized, missing, or stale assets.
- **Cutouts:** inspect mask boundaries and halos on varied backgrounds; verify soft alpha where supported, real PNG transparency, unchanged visible source content, lineage, and independent-editor use.
- **Regression/release:** preserve existing audio workflows and verify migrations/backup restoration. Qualify actual installed image workflows on each advertised platform.

Success should be judged by faster reuse and fewer selection/correction errors, not the number of tags generated. Any optional analytics for these measures should be opt-in; local test sessions are sufficient initially.

## Future ideas to park beyond this phase

- **Shared studio review and synchronization:** comments, approvals, and conflict-aware shared collections help teams; add after single-user identity and portability are dependable.
- **Additional formats and source connectors:** TIFF, HEIC, AVIF, animated formats, SVG, layered design files, and generator/cloud-drive connectors reduce conversion and manual download work; qualify each format/provider rather than claim universal support. SVG and project files need their own safe rendering rules.
- **Generative editing and enhancement:** upscaling, outpainting, background replacement, pose variation, and object removal can prepare missing material; they introduce new provenance, cost, and fidelity requirements and are outside cutout export.
- **Video assets and automated scene assembly:** extend reuse to clips and complete editing pipelines after image handoff is established; SS-033 preset work remains its own scope.
- **Public asset marketplace:** could support distribution later, but public hosting, moderation, payments, and redistribution rules are a separate product rather than prerequisites for a personal creator library.

## Research informing the proposal

Checked official sources on 16 October 2026. These establish relevant existing patterns, not a comparative benchmark or a model/vendor selection.

- [Immich search documentation](https://docs.immich.app/features/searching/) documents metadata and contextual image search, OCR, and model tradeoffs. This supports keeping exact filters alongside semantic retrieval.
- [Immich duplicate utility](https://docs.immich.app/features/duplicates-utility/) describes visually similar asset detection. For CreativeShelf, proposed duplicate review must distinguish generation variants from removable copies.
- [Eagle product overview](https://www.eagle.cool/home) describes image/meaning search, tags, duplicate discovery, and agent integration. My product inference is that generic organization alone is insufficient differentiation; CreativeShelf should emphasize mixed audio/image production reuse, output suitability, and traceable elements.
- [Meta SAM 2 repository](https://github.com/facebookresearch/sam2) and [image predictor](https://github.com/facebookresearch/sam2/blob/main/sam2/sam2_image_predictor.py) demonstrate promptable image segmentation with points/boxes. This makes interactive selection a credible implementation direction; it does not prove finished alpha quality, desktop performance, or a qualified distribution package.

## Backlog handoff

SS-032 is the image-phase umbrella. The workstreams are now split into SS-040–078
with concrete outcomes, technical requirements, edge cases, test scenarios,
acceptance criteria, and predecessor/next links. Shared provider/pack/import
ownership is recorded against SS-014, SS-017, and SS-039 to prevent duplication.

Recommended first implementation decision: the additive image-asset schema and minimal JPEG/PNG/WebP import-to-preview path, with a disposable migration fixture representing an existing audio catalog. Also evaluate one tagging path on creator images. Defer provider/model selection and delivery estimates until that evidence is available.

## 17 October 2026 dependent-PR workflow

The user authorized continuing to the next ticket after all checks pass on the
predecessor PR, without waiting for merge. This supersedes merge-before-start
wording above. Keep one ticket per PR, target unmerged dependent PRs to the
predecessor branch, and merge in ticket order. Retarget and recheck dependents
after predecessor merges. IMAGE_QUEUE.md is the canonical execution order.
