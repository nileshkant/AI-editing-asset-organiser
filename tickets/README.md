# Delivery tickets

This backlog is the implementation contract. A ticket's acceptance checkboxes indicate verified behavior, not aspirational completion. The existing full product plan is in docs/PRODUCT_PLAN.md.

## Workflow

1. Start a feature branch from its prerequisite branch or merged main.
2. Implement only the ticket's scope. Add deterministic tests and explicit failure states.
3. Run the relevant tests and inspect the diff. Record findings and fixes in docs/reviews.
4. Raise one PR with the matching ticket, test output and known limitations. Dependent PRs target their parent branch until merged.
5. Mark criteria only after evidence exists; do not merge automatically unless authorized.

The repository currently has no remote. Local commits/branches are not substitutes for published PRs; publication is pending the user's repository destination.

## Ticket index

- [SS-001: Delivery backlog](SS-001.md) | done-local | prerequisites: none
- [SS-002: Desktop foundation](SS-002.md) | todo | prerequisites: 001
- [SS-003: Catalog identity and persistence](SS-003.md) | todo | prerequisites: 002
- [SS-004: Background import and analysis](SS-004.md) | todo | prerequisites: 003
- [SS-005: Measured profiles and descriptions](SS-005.md) | todo | prerequisites: 004
- [SS-006: Folder relink and availability](SS-006.md) | todo | prerequisites: 003,004
- [SS-007: User tags comments and favorites](SS-007.md) | todo | prerequisites: 003
- [SS-008: Offline natural language and fuzzy search](SS-008.md) | todo | prerequisites: 003,005,007
- [SS-009: Library workspace and accessibility](SS-009.md) | todo | prerequisites: 002,008
- [SS-010: Native audio playback](SS-010.md) | todo | prerequisites: 004
- [SS-011: Waveform visualization](SS-011.md) | todo | prerequisites: 005,010
- [SS-012: Clip recipes and selection](SS-012.md) | todo | prerequisites: 003,011
- [SS-013: Clip export and handoff](SS-013.md) | todo | prerequisites: 012
- [SS-014: Provider configuration and credentials](SS-014.md) | todo | prerequisites: 003
- [SS-015: AI query interpretation](SS-015.md) | todo | prerequisites: 008,014
- [SS-016: AI sound description and suggestions](SS-016.md) | todo | prerequisites: 005,014
- [SS-017: Optional offline model packs](SS-017.md) | todo | prerequisites: 005,008,014
- [SS-018: MCP lifecycle authentication](SS-018.md) | todo | prerequisites: 003
- [SS-019: MCP catalog and clip tools](SS-019.md) | todo | prerequisites: 008,012,013,018
- [SS-020: Editor integrations and agent skill](SS-020.md) | todo | prerequisites: 013,019
- [SS-021: Portable catalog and migration](SS-021.md) | todo | prerequisites: 003,005,006
- [SS-022: Settings diagnostics and backup](SS-022.md) | todo | prerequisites: 003,014,018
- [SS-023: Security and performance qualification](SS-023.md) | todo | prerequisites: 004,009,010,013,018
- [SS-024: Cross platform installers and releases](SS-024.md) | todo | prerequisites: 002,023
- [SS-025: Subscription-backed MCP access](SS-025.md) | todo | prerequisites: 018,019,020,024
- [SS-026: Individual-file import](SS-026.md) | todo | prerequisites: 003,004,006,009
- [SS-027: macOS 26 native startup compatibility](SS-027.md) | in-progress | prerequisites: 002

## Release definition

All required acceptance criteria, a real audio-output test, installed desktop tests on each advertised platform, security/privacy review, signed installers, dependency/license review and backup/upgrade recovery must pass before calling the app production-ready. AI adapters are fixture-tested without spending the user's API credits; provider live tests need explicit setup.
