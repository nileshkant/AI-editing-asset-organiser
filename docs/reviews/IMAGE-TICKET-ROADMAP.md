# Image ticket roadmap review

Date: 16 October 2026
Scope: documentation/backlog creation only
Result: structural and dependency review passed; no image implementation claimed

## Deliverables

- [Strict queue](../../tickets/IMAGE_QUEUE.md): SS-040 through SS-078, one PR at a
  time, with a mandatory green-predecessor gate and ordered merges and no skip-on-blocker behavior.
- [Shared implementation contract](../../tickets/IMAGE_CONTRACT.md): identity,
  migrations, persistence, permissions, media correctness, AI consent,
  resource limits, UI/accessibility, tests, and review/merge evidence.
- 39 detailed ticket files with integration points, numbered requirements,
  test scenarios, acceptance checkboxes, exclusions, and predecessor/next links.
- [Phase plan](../IMAGE_ASSET_PHASE_PLAN.md), product plan, ticket index, SS-032
  umbrella, and SS-014/017/039 ownership notes reconciled to the new sequence.

At the initial planning review all tickets were not-started with unchecked
acceptance. Publication is now updated for merged SS-040–042 and implemented
SS-043; the remaining 35 tickets remain not-started. This PR publishes only the
backlog and plan, not the current feature implementation.

## Checks performed

An inline Python structural audit checked:

- Exactly 39 contiguous tickets numbered SS-040–078 and identical ordered queue.
- Each ticket after SS-040 depends on exactly its immediate predecessor.
- Previous/next links and sequence indices agree; no dependency cycles.
- Required sections and consecutive R/T/A numbering exist in every ticket.
- Completion/checkmarks agree with merged evidence; planned work stays unchecked.
- Current integration-point paths exist in the repository.
- 374 local Markdown links across the audited backlog/plan files resolve.
- Every IMG-01–26 workstream has at least one implementation owner, excluding
  qualification-only mappings.
- No merge-conflict markers in the audited documents.

Final audit totals: 327 ticket-specific R requirements, 195 T scenario groups
(including shared categories), and 273 A acceptance checkboxes. These are counts
of planned requirements, not implemented features or tests run.

The initial link audit caught missing zero-padding in the coverage-map links;
those links were corrected and the complete audit passed afterward.

`git diff --check` passed. The tracked planning diff was inspected. Application
build/unit/native suites were not run because this change is documentation-only;
each implementation ticket specifies the meaningful checks required when it is
developed. Existing unrelated source-catalog working-tree changes were not part
of this work.

## Sequence review and corrections

- Split contracts/migration, decoding/import/previews/UI/persistence into ordered
  owners rather than require a single large SS-032 PR.
- Defined temporary persistence guards for SS-041–047 and explicit completion
  in SS-048, avoiding an early dependency on a later serialization ticket.
- Basic catalog reads precede the gallery; full search comes later at SS-058.
  Earlier evidence/tag/OCR tickets expose typed projections without depending
  on a future search UI.
- Evaluation harness consent is explicit before the production job queue exists.
  Shared provider and pack work has one owner instead of duplicated legacy work.
- Composition analysis does not depend on final-stage segmentation.
- Export precedes MCP mutations, and installed handoff qualification precedes
  object extraction.
- Binary editable masks/drafts are durable user work with artifact/backup
  semantics, not silently evictable preview caches.
- The final extraction sequence is backend qualification, selection, refinement,
  PNG export, derived registration/reuse, then installed qualification.

## Limits carried into implementation

Specific decoder/model/runtime choices and numerical quality/resource budgets
are intentionally owned by SS-040/042/050/073, where actual code and measurements
can establish them. This planning review does not certify AI accuracy, native
platform support, installer signing, or the absence of future bugs. Required
evaluation failures block the queue or require an explicit scope amendment.

The SS-032 legacy audio-release prerequisite was clarified as a release gate;
image development proceeds in the requested sequence while inherited installed
qualification remains open. Existing implementation and model/provider
availability must be rechecked against main when each ticket starts.


## Publication update — 17 October 2026

The roadmap was accidentally left local while SS-040–042 were delivered. The
user requested a separate roadmap PR first in the remaining merge queue, after
finishing the current SS-043 implementation. This documentation PR is based on
merged main through #41, with no implementation changes or unrelated snapshot
edits. Its queue explicitly stops after the separate SS-043 PR; no SS-044 work
has begun. Re-audit links and dependency order on publication. Earlier tests in
this document describe the initial planning audit, not new implementation proof.
