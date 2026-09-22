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
- [SS-027: macOS 26 native startup compatibility](SS-027.md) | complete
- [SS-028: Windows application icon resource](SS-028.md) | complete

## Execution queue

This is the canonical implementation order. It prioritizes a reliable local
editing workflow before integrations and keeps optional AI off the critical
path.

1. [SS-008: Offline natural language and fuzzy search](SS-008.md) | partial | finish indexed retrieval and scale tests
2. [SS-010: Native audio playback](SS-010.md) | not-started | establish audible native transport
3. [SS-009: Library workspace and accessibility](SS-009.md) | partial | finish against real playback and large libraries
4. [SS-011: Waveform visualization](SS-011.md) | not-started | add playback-synced multiresolution waveform
5. [SS-012: Clip recipes and selection](SS-012.md) | not-started | add exact non-destructive selections
6. [SS-013: Clip export and handoff](SS-013.md) | not-started | produce verified editor-ready media
7. [SS-026: Individual-file import](SS-026.md) | not-started | extend stable source scopes
8. [SS-021: Portable catalog and migration](SS-021.md) | not-started | add portable export/import after identity work stabilizes
9. [SS-018: MCP lifecycle authentication](SS-018.md) | not-started | establish the secured local service
10. [SS-019: MCP catalog and clip tools](SS-019.md) | not-started | expose the completed common services
11. [SS-020: Editor integrations and agent skill](SS-020.md) | not-started | build on working exports and MCP
12. [SS-014: Provider configuration and credentials](SS-014.md) | not-started | add optional provider infrastructure
13. [SS-022: Settings diagnostics and backup](SS-022.md) | not-started | consolidate service, provider, backup, and diagnostics controls
14. [SS-023: Security and performance qualification](SS-023.md) | not-started | qualify the completed local and MCP workflows
15. [SS-024: Cross platform installers and releases](SS-024.md) | not-started | package only after qualification passes
16. [SS-025: Subscription-backed MCP access](SS-025.md) | not-started | add client-specific setup to the released MCP service
17. [SS-015: AI query interpretation](SS-015.md) | not-started | optional enhancement after offline search is complete
18. [SS-016: AI sound description and suggestions](SS-016.md) | not-started | optional, consented enrichment
19. [SS-017: Optional offline model packs](SS-017.md) | not-started | optional model distribution after baseline qualification

## Progress snapshot

- Complete: 9 of 28 tickets.
- In review: 0 of 28 tickets.
- Partial: 2 of 28 tickets.
- Not started: 17 of 28 tickets.
- Next ticket: SS-008.

## Release definition

All required acceptance criteria, a real audio-output test, installed desktop tests on each advertised platform, security/privacy review, signed installers, dependency/license review and backup/upgrade recovery must pass before calling the app production-ready. AI adapters are fixture-tested without spending the user's API credits; provider live tests need explicit setup.
