# Implementation decisions

## 2026-09-20: Desktop and portable identity

The product is a Tauri 2 desktop application. It does not depend on the existing HTML catalog, Python scripts or this audio collection. Installed application data belongs in the OS app-data directory, not the source folder.

Sources have stable UUIDs and editable absolute root paths. Sounds have independent stable UUIDs and normalized relative locations. Content versions own reusable analysis keyed by content digest and analyzer version. Moving a folder changes its root and source generation, not sound IDs. Verification may read/hash content but must not decode/recompute unchanged content. Conflicting or partially matching roots require reconciliation, never silent reassignment.

## 2026-09-20: User metadata and evidence

User tags and comments are independent of generated analysis. Automatic reanalysis cannot overwrite user edits. Display labels contain spaces, not underscore separators. Provenance distinguishes measured properties, filename inference, user annotations and model suggestions. A text model cannot truthfully claim to have heard an audio file when sent only metadata.

## 2026-09-20: Optional intelligence

Offline natural-language search includes synonym expansion, typo correction and validated numeric intent extraction. MCP exposes search; MCP itself does not provide semantic intelligence. Optional model-backed query interpretation and audio understanding enhance the same search service. All AI features start disabled, with explicit network and audio-upload consent.

OpenAI uses its Responses contract for text; Anthropic uses Messages; Gemini uses generateContent; LM Studio uses OpenAI-compatible local endpoints. Credentials are provider-specific: a key must have access to the selected model. Local servers may require no key. Audio input is a separate capability, never inferred from text support. No key is stored in renderer localStorage, ordinary SQLite, logs or exports.

Verified contracts:
- https://developers.openai.com/api/docs/guides/text
- https://platform.claude.com/docs/en/api/messages/create
- https://ai.google.dev/api/generate-content
- https://lmstudio.ai/docs/developer/openai-compat

## Release gates

Passing local unit tests is not production qualification. Native playback, device changes, long-file memory, clean-machine installation, OS permissions, security, model capability checks, accessibility and signed updates must pass on each advertised platform. Windows/Linux cannot be marked tested from a macOS run.
