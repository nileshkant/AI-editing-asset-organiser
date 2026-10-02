# Delivery tickets

This backlog is the implementation contract. A ticket's acceptance checkboxes
indicate verified behavior, not aspirational completion. The full product plan
is in `docs/PRODUCT_PLAN.md`.

## Workflow

1. Start a feature branch from its prerequisite branch or merged main.
2. Implement only the ticket's scope. Add deterministic tests and explicit failure states.
3. Run the relevant tests and inspect the diff. Record findings and fixes in docs/reviews.
4. Raise one PR with the matching ticket, test output and known limitations. Dependent PRs target their parent branch until merged.
5. Mark criteria only after evidence exists; do not merge automatically unless authorized.

Always take the first unblocked ticket in the execution queue. Do not start a
lower ticket merely because it is easier. If a ticket is blocked, record the
blocker and move to the next unblocked ticket without changing the queue.

## Status definitions

- **complete**: implemented, verified, reviewed, and present in main history.
- **in-review**: implementation is pushed but review, PR, or CI is outstanding.
- **partial**: useful implementation exists, but ticket acceptance is incomplete.
- **not-started**: the ticket's primary deliverable has not been implemented.

## Completed foundation

- [SS-001: Delivery backlog](SS-001.md) | complete
- [SS-002: Desktop foundation](SS-002.md) | complete
- [SS-003: Catalog identity and persistence](SS-003.md) | complete
- [SS-004: Background import and analysis](SS-004.md) | complete
- [SS-005: Measured profiles and descriptions](SS-005.md) | complete
- [SS-006: Folder relink and availability](SS-006.md) | complete
- [SS-007: User tags comments and favorites](SS-007.md) | complete
- [SS-008: Offline natural language and fuzzy search](SS-008.md) | complete
- [SS-009: Library workspace and accessibility](SS-009.md) | complete
- [SS-010: Native audio playback](SS-010.md) | complete
- [SS-011: Waveform visualization](SS-011.md) | complete
- [SS-012: Clip recipes and selection](SS-012.md) | complete
- [SS-027: macOS 26 native startup compatibility](SS-027.md) | complete
- [SS-028: Windows application icon resource](SS-028.md) | complete

- [SS-013: Clip export and handoff](SS-013.md) | complete
- [SS-026: Individual-file import](SS-026.md) | complete
- [SS-021: Portable catalog and migration](SS-021.md) | complete
- [SS-018: MCP lifecycle authentication](SS-018.md) | complete

## Audio v1 execution queue

This is the canonical implementation order. The 2026-10-02 scope review is in
`docs/reviews/V1-SCOPE-REVIEW.md`. Resolve the current MCP review first, then the
new source-catalog requirement before advancing to editor integrations.

1. [SS-019: MCP catalog and clip tools](SS-019.md) | complete
2. [SS-029: Source-folder metadata catalog](SS-029.md) | in-review
3. [SS-030: CreativeShelf branding and compatibility](SS-030.md) | not-started
4. [SS-020: Editor integrations and agent skill](SS-020.md) | not-started
5. [SS-022: Settings diagnostics and backup](SS-022.md) | not-started
6. [SS-023: Security and performance qualification](SS-023.md) | not-started
7. [SS-024: Cross platform installers and releases](SS-024.md) | not-started
8. [SS-031: Audio v1 tester release qualification](SS-031.md) | not-started

Seven unfinished implementation/qualification tickets remain on the audio v1 path. This is a work count, not a time estimate;
new defects discovered during qualification may require follow-ups. No ticket
completion alone proves release readiness.

## Optional enhancements after the baseline tester release

- [SS-014: Provider configuration and credentials](SS-014.md) | not-started
- [SS-015: AI query interpretation](SS-015.md) | not-started
- [SS-016: AI sound-event tags and descriptions](SS-016.md) | not-started
- [SS-017: Optional offline model packs](SS-017.md) | not-started
- [SS-025: Subscription-backed MCP access](SS-025.md) | not-started

SS-016/017 must deliver specific event labels such as rain, thunder, hiss,
footsteps and engine, searchable by people and MCP clients. These are explicitly
planned, not current recognition capabilities. MCP access itself is not AI
recognition. If the tester build includes automatic sound recognition, its
SS-016/017 acceptance evidence becomes a release gate for that build.

## Upcoming asset types — outside audio v1

- [SS-032: Image assets and reusable elements](SS-032.md) | not-started | upcoming
- [SS-033: Remotion Studio presets](SS-033.md) | not-started | upcoming

## Progress snapshot

- Complete: 19 of 33 tickets.
- In review: 1 of 33 tickets.
- Partial: 0 of 33 tickets.
- Not started: 13 of 33 tickets.
- Current implementation: SS-029.
- Next unstarted implementation: SS-030.
- New tickets from this review: 5 (3 audio v1, 2 upcoming).

## Release definition

An audio v1 tester release needs recorded end-to-end native audio and MCP
checks, source-catalog reuse/pruning, backup and upgrade recovery, security and
performance qualification, and a signed installable build on each advertised
platform. SS-031 records the exact artifact, versions, qualified platform
matrix, known limitations and feedback workflow. A deliberately smaller tested
platform matrix is acceptable; untested platforms are not advertised. Optional
AI, client subscription setup, direct Resolve automation, images and presets do
not block the baseline release. Full production readiness requires all
applicable release gates in the product plan; passing unit tests does not
certify a production package.
