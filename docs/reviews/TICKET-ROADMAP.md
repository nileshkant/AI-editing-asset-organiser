# Ticket roadmap audit

Date: 2026-09-20

## Scope

All 28 tickets were compared with the main history, source modules, automated
tests, and existing review records. The ticket number is an identity, not a
priority. `tickets/README.md` is now the canonical execution order.

## Status decisions

- SS-001, SS-002, SS-003, SS-004, SS-007, SS-027, and SS-028 are complete in main history.
- SS-005 is implemented, reviewed, and in PR review.
- SS-006, SS-008, and SS-009 contain working code but retain
  unchecked acceptance criteria, so they are partial rather than complete.
- SS-010 through SS-026 are not started at ticket scope. Incidental foundations
  such as a settings shell or CI matrix do not complete their primary outcomes.

## Priority decisions

The queue first closes existing partial foundations after the Windows icon resource.
Native playback, waveform selection, and export follow because they form the
smallest useful video-editing workflow. MCP is scheduled only after the shared
search and clip services exist. Packaging follows security qualification.
Subscription-backed clients depend on the released MCP workflow. Optional AI
features remain last because offline cataloging, search, playback, and export
must work without credentials.

## Dependency correction

SS-009 now depends on SS-010. Its acceptance criterion requiring the active
playback state to remain independent from list filtering cannot be verified
before native playback exists.

## Validation

- Every ticket has one recognized status.
- Every ticket appears in either the completed foundation or execution queue.
- The snapshot totals 28 tickets.
- `git diff --check` passes.
