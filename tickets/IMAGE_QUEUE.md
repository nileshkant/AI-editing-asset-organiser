# Image phase — strict execution queue

Date: 16 October 2026
Status: 39 tickets; SS-040–046 merged, SS-047 in review; SS-048–078 not-started
Current: the user resumed the queue on 18 October 2026 after the SS-079 release candidate. SS-046 merged (PR #48); SS-047 is in review.
Parent: [SS-032](SS-032.md)
Specification: [Image phase plan](../docs/IMAGE_ASSET_PHASE_PLAN.md)
Mandatory rules: [IMAGE_CONTRACT.md](IMAGE_CONTRACT.md)

## Canonical order

**SS-040 → SS-041 → … → SS-078. Start with SS-040.**

Each ticket is one PR. After its current-head checks pass and known blockers
are cleared, start the next ticket without waiting for merge. Branch from the
verified predecessor head and target the dependent PR to its predecessor branch.
Merge in numerical ticket order; retarget the next PR to main after the parent
merges and verify checks on its final head. No skipped tickets or automatic merges.

This 17 October 2026 user instruction supersedes the earlier merge-before-start
rule. A ticket is still marked complete only after review and merge.

This user-requested sequence is authoritative for image work. The phase plan's
Core/Expansion labels express product value, not permission to jump around this
queue. SS-032 is a tracking umbrella and does not consume an extra implementation
PR. Its children do not depend on the umbrella being complete.

If a blocker needs a new separately scoped prerequisite, stop and explicitly
amend the queue before implementing it. Preserve chronological dependency order;
do not silently append a fix after the feature that needs it.

## Foundation and inherited release gates

Reuse the merged audio catalog/source/agent services. Verify actual main history,
current schema, and predecessor evidence at execution time; existing metadata
can lag merges. Do not wait for future audio-event recognition to implement image
recognition, and do not duplicate shared provider/model infrastructure.

Existing audio installed/platform/signing qualification remains tracked by its
original tickets. The 16 October image planning instruction supersedes the old
SS-032 requirement to wait for all audio qualification before image development;
it does not waive applicable release gates or mark old tickets complete. The
image workflow is qualified in SS-072 and the complete element workflow in SS-078.

Image-phase code can merge while development capabilities are gated. Do not
advertise a complete image feature before its end-to-end owner passes acceptance.

## Ordered tickets

### 1. Reliable local image library

1. [SS-040: Image contracts and compatibility fixtures](SS-040.md) — complete; [PR #39](https://github.com/nileshkant/AI-editing-asset-organiser/pull/39); predecessor: verified merged foundation.
2. [SS-041: Additive asset identity and image schema migration](SS-041.md) — complete; [PR #40](https://github.com/nileshkant/AI-editing-asset-organiser/pull/40), merged; predecessor: SS-040.
3. [SS-042: Bounded image decoding and technical analysis](SS-042.md) — complete; [PR #41](https://github.com/nileshkant/AI-editing-asset-organiser/pull/41), merged; predecessor: SS-041.
4. [SS-043: Durable image import jobs and scoped selection](SS-043.md) — complete; [PR #43](https://github.com/nileshkant/AI-editing-asset-organiser/pull/43), merged after roadmap [PR #42](https://github.com/nileshkant/AI-editing-asset-organiser/pull/42); predecessor: SS-042.
5. [SS-044: Versioned thumbnails and secure preview cache](SS-044.md) — complete; [PR #46](https://github.com/nileshkant/AI-editing-asset-organiser/pull/46) merged as 6f02e4b, see [review](../docs/reviews/SS-044.md); predecessor: SS-043 (merged).
6. [SS-045: Image gallery and inspection workspace](SS-045.md) — complete; [PR #47](https://github.com/nileshkant/AI-editing-asset-organiser/pull/47) merged as e6b24eb, see [review](../docs/reviews/SS-045.md); predecessor: SS-044 (merged).
7. [SS-046: Image annotations and reversible bulk tagging](SS-046.md) — complete; [PR #48](https://github.com/nileshkant/AI-editing-asset-organiser/pull/48) merged as 09eb726, see [review](../docs/reviews/SS-046.md); predecessor: SS-045 (merged).
8. [SS-047: Image relinking refresh and opt-in folder watching](SS-047.md) — in-review; [PR #50](https://github.com/nileshkant/AI-editing-asset-organiser/pull/50), see [review](../docs/reviews/SS-047.md); predecessor: SS-046 (merged).
9. [SS-048: Mixed image catalogs backup and restore](SS-048.md) — not-started; predecessor: SS-047.

### 2. AI configuration, analysis, and provenance

10. [SS-049: Shared AI provider configuration and secret handling](SS-049.md) — not-started; predecessor: SS-048.
11. [SS-050: Image AI evaluation corpus and qualified adapters](SS-050.md) — not-started; predecessor: SS-049.
12. [SS-051: Optional local model pack lifecycle](SS-051.md) — not-started; predecessor: SS-050.
13. [SS-052: Consented image enrichment queue and usage budgets](SS-052.md) — not-started; predecessor: SS-051.
14. [SS-053: Structured image descriptions and AI tag persistence](SS-053.md) — not-started; predecessor: SS-052.
15. [SS-054: AI tag review corrections and selective reanalysis](SS-054.md) — not-started; predecessor: SS-053.
16. [SS-055: Generation metadata and prompt provenance](SS-055.md) — not-started; predecessor: SS-054.
17. [SS-056: Image source attribution and recorded usage conditions](SS-056.md) — not-started; predecessor: SS-055.
18. [SS-057: OCR search data and text or logo review hints](SS-057.md) — not-started; predecessor: SS-056.

### 3. Search and creator organization

19. [SS-058: Exact image search filters and deterministic ranking](SS-058.md) — not-started; predecessor: SS-057.
20. [SS-059: Versioned image embeddings and semantic retrieval](SS-059.md) — not-started; predecessor: SS-058.
21. [SS-060: Reference-image subject palette and style search](SS-060.md) — not-started; predecessor: SS-059.
22. [SS-061: Composition analysis and non-destructive crop recipes](SS-061.md) — not-started; predecessor: SS-060.
23. [SS-062: Exact duplicates and reviewed variation families](SS-062.md) — not-started; predecessor: SS-061.
24. [SS-063: Mixed collections smart collections and brand kits](SS-063.md) — not-started; predecessor: SS-062.
25. [SS-064: Output suitability and quality review](SS-064.md) — not-started; predecessor: SS-063.

### 4. Export, agents, and project reuse

26. [SS-065: Verified image export engine and atomic publication](SS-065.md) — not-started; predecessor: SS-064.
27. [SS-066: Image export UI and editor handoff](SS-066.md) — not-started; predecessor: SS-065.
28. [SS-067: Scoped image MCP search metadata and previews](SS-067.md) — not-started; predecessor: SS-066.
29. [SS-068: Scoped image MCP mutations import and export jobs](SS-068.md) — not-started; predecessor: SS-067.
30. [SS-069: Brief-to-asset shortlists](SS-069.md) — not-started; predecessor: SS-068.
31. [SS-070: Project asset usage and export history](SS-070.md) — not-started; predecessor: SS-069.
32. [SS-071: Portable project asset packs](SS-071.md) — not-started; predecessor: SS-070.

### 5. Installed discovery-to-handoff gate

33. [SS-072: Image discovery and handoff installed qualification](SS-072.md) — not-started; predecessor: SS-071.

### 6. Final object extraction and qualification

34. [SS-073: Segmentation and matting runtime qualification](SS-073.md) — not-started; predecessor: SS-072.
35. [SS-074: Interactive object selection and mask proposals](SS-074.md) — not-started; predecessor: SS-073.
36. [SS-075: Mask refinement alpha edges and undo](SS-075.md) — not-started; predecessor: SS-074.
37. [SS-076: Transparent PNG cutout rendering and verification](SS-076.md) — not-started; predecessor: SS-075.
38. [SS-077: Derived element catalog lineage and agent reuse](SS-077.md) — not-started; predecessor: SS-076.
39. [SS-078: Image and reusable element final qualification](SS-078.md) — not-started; predecessor: SS-077.

## Roadmap coverage

Every IMG workstream is included. SS-072 and SS-078 provide end-to-end
qualification in addition to feature-level tests. Qualification-only references
are omitted from the following ownership list so delivery ownership is clear.

- **IMG-01 — Shared image and element catalog:** [SS-040](SS-040.md), [SS-041](SS-041.md).
- **IMG-02 — Folder and individual-image import:** [SS-043](SS-043.md), [SS-047](SS-047.md).
- **IMG-03 — Technical metadata and thumbnails:** [SS-042](SS-042.md), [SS-044](SS-044.md).
- **IMG-04 — Visual browsing and inspection:** [SS-045](SS-045.md).
- **IMG-05 — Personal tags and bulk organization:** [SS-046](SS-046.md).
- **IMG-06 — Portable metadata, backup, and relinking:** [SS-047](SS-047.md), [SS-048](SS-048.md).
- **IMG-07 — AI configuration, privacy, and job budgets:** [SS-049](SS-049.md), [SS-050](SS-050.md), [SS-051](SS-051.md), [SS-052](SS-052.md).
- **IMG-08 — Structured visual analysis and auto-tagging:** [SS-050](SS-050.md), [SS-053](SS-053.md).
- **IMG-09 — Review and correction workflow:** [SS-054](SS-054.md).
- **IMG-10 — Generation history and prompt provenance:** [SS-055](SS-055.md).
- **IMG-11 — Source, attribution, and usage notes:** [SS-056](SS-056.md).
- **IMG-12 — Text inside images and logo warnings:** [SS-057](SS-057.md).
- **IMG-13 — Natural-language search with strict filters:** [SS-050](SS-050.md), [SS-058](SS-058.md), [SS-059](SS-059.md).
- **IMG-14 — Search by reference image and style:** [SS-060](SS-060.md).
- **IMG-15 — Composition and crop suitability:** [SS-061](SS-061.md).
- **IMG-16 — Duplicate and variation families:** [SS-062](SS-062.md).
- **IMG-17 — Collections, smart collections, and brand kits:** [SS-063](SS-063.md).
- **IMG-18 — Output-specific quality review:** [SS-064](SS-064.md).
- **IMG-19 — Image export and editor handoff:** [SS-065](SS-065.md), [SS-066](SS-066.md).
- **IMG-20 — Scoped image tools for AI agents:** [SS-067](SS-067.md), [SS-068](SS-068.md).
- **IMG-21 — Brief-to-asset shortlist:** [SS-069](SS-069.md).
- **IMG-22 — Project usage history and portable asset packs:** [SS-070](SS-070.md), [SS-071](SS-071.md).
- **IMG-23 — Select a particular object:** [SS-073](SS-073.md), [SS-074](SS-074.md).
- **IMG-24 — Mask refinement and edge inspection:** [SS-073](SS-073.md), [SS-075](SS-075.md).
- **IMG-25 — Verified transparent PNG export:** [SS-076](SS-076.md).
- **IMG-26 — Reusable extracted-element library:** [SS-077](SS-077.md).

## Intentional splitting and handoffs

- SS-040 freezes contracts; SS-041 migrates storage. Decoder, import jobs,
  preview cache, gallery, annotations, source lifecycle, and portable persistence
  each follow in their own PR. Gated image metadata is protected from old snapshot
  writers until SS-048.
- SS-049 configures credentials; SS-050 qualifies adapters/models; SS-051 installs
  local packs; SS-052 controls actual inference scheduling/consent. SS-053/054
  implement suggestions and durable human review.
- SS-058 implements deterministic exact filters before SS-059 semantic ranking.
  Similarity and creator organization consume those APIs.
- SS-065 verifies image rendering/export before SS-066 editor UI/handoff.
  SS-067 read tools precede SS-068 mutations and approved agent import.
- SS-069–071 build shortlists, usage history, and portable media packs on those
  verified services. SS-072 proves this workflow before extraction begins.
- SS-073 qualifies segmentation/matting; SS-074 selects an object; SS-075 refines
  its mask; SS-076 exports PNG; SS-077 registers/reuses elements; SS-078 qualifies
  the full phase. This preserves the requested final-feature order.

## Shared legacy-ticket ownership

- SS-014: provider configuration primitives are reused/extracted by SS-049.
  Other provider coverage and legacy acceptance remain open until proven.
- SS-015: optional AI query interpretation is not a hard prerequisite. Exact
  image filters and embedding retrieval are SS-058/059; optional brief parsing
  in SS-069 must qualify its own text capability.
- SS-016: audio recognition remains independent; do not add it as a hidden
  prerequisite of image analysis.
- SS-017: pack lifecycle primitives are reused/extracted by SS-051; image model
  evaluation is SS-050 and segmentation evaluation is SS-073. Audio models and
  recognition quality are still owned by SS-016/017.
- SS-039: exact native selection-grant infrastructure is reusable in SS-068;
  its requested audio import/recognition behavior is not falsely completed.
- SS-033: executable Remotion presets remain separate. Image project handoff
  needs SS-066, not SS-033.

## Agent starting instruction

Read tickets/README.md, tickets/IMAGE_CONTRACT.md, and the first incomplete
ticket in this queue. Verify its immediate predecessor PR has all checks passing on its current head and no known blocking findings.
Implement only that ticket, satisfy every R/A criterion with recorded evidence,
open its PR when authorized, and start the next numbered ticket only after all current PR checks pass.
Do not wait for merge; preserve predecessor-first merge order. If blocked, record the blocker and stop the queue.

## Deferred scope

No new tickets are added here for collaboration/cloud sync, additional formats,
generator connectors, generative editing, video indexing, public marketplace,
or executable presets. They remain the phase plan's explicitly later ideas.
