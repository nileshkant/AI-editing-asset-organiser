# AI providers and keys (SS-049)

Shared provider configuration for optional AI features. This ticket configures
providers, stores keys and runs an explicit connection test. **It analyzes no
images.** Image analysis, per-source consent, budgets and model qualification
come later (SS-050 to SS-052).

Code: `crates/core/src/ai_provider.rs` (profiles, endpoint policy, secrets,
service) and `ai_provider_wire.rs` (HTTP adapters and the connection test).
Desktop adapter: `src-tauri/src/providers.rs` (OS keychain and IPC). Renderer:
Settings → **AI providers** (`src/components/views/AiProviderSettings.tsx`).

## Providers and wire contracts

| Kind | Default endpoint | Generate | Model check | Embeddings |
| --- | --- | --- | --- | --- |
| OpenAI | `https://api.openai.com/v1` | `POST chat/completions` | `GET models/{model}` | `POST embeddings` |
| Anthropic | `https://api.anthropic.com/v1` | `POST messages` (`anthropic-version: 2023-06-01`) | `GET models/{model}` | none: reported unsupported, no request |
| Google Gemini | `https://generativelanguage.googleapis.com/v1beta` | `POST models/{model}:generateContent` | `GET models/{model}` | `POST models/{model}:batchEmbedContents` |
| OpenAI-compatible server | none: user enters it | `POST chat/completions` | `GET models`, looked up by ID | `POST embeddings` |

- Keys travel only in headers (`Authorization: Bearer`, `x-api-key` or
  `x-goog-api-key`), never in URLs or bodies.
- Structured output uses `response_format: json_schema` (OpenAI and compatible),
  `responseJsonSchema` (Gemini), or a schema instruction for Anthropic. Every
  answer is parsed and checked against the requested object schema (required
  properties and property types) before it is returned. A non-conforming answer
  fails with `SCHEMA_INCOMPATIBLE`.
- The `ProviderAdapter` trait (`model`, `generate`, `embed`) is the contract later
  tickets use. It reports the served model version and token usage when the
  provider supplies them, and it never retries. SS-050 decides retry and
  billing uncertainty for real analysis.

### Bounds

| Limit | Value |
| --- | --- |
| Images per request | 8 (PNG, JPEG or WebP), each up to 5 MiB, 20 MiB total |
| Text plus instructions | 64 KiB |
| Serialized request | 32 MiB |
| Response body | 2 MiB; larger fails with `RESPONSE_TOO_LARGE` |
| Output tokens | 1 to 8,192 |
| Embedding inputs | 64 texts, 8,192 dimensions |
| Timeouts | 10 s to connect, 60 s per request (connection test uses the same) |
| Profiles | 16 |

## Capabilities are measured, not assumed

Each profile records `image_input`, `structured_output` and `embeddings` as
`untested`, `supported`, `unsupported` or `inconclusive`. Only an explicit
connection test of that exact configuration revision sets them, and any change
(endpoint, model, kind, local option or key) resets them to `untested`. Text
support never implies image support.

The connection test runs, in order:

1. **Key and model.** Authentication and model existence. On failure the test
   stops, and capabilities stay `untested`.
2. **Image input.** A generated 16×16 red PNG with "what single color fills this
   image?". The result is `supported` only if the answer says red. If the request
   is accepted but the answer is wrong, the result is `inconclusive`. A provider
   refusing image input gives `unsupported` (`IMAGE_UNSUPPORTED`), which is
   distinct from authentication, quota or transport errors.
3. **Structured output.** A fixed JSON schema request.
4. **Embeddings,** only when the user ticks it.

The test never sends user images, file names, tags or catalog data. It does cost
a few tokens on paid providers, and usage is shown when reported.

## Endpoint and network policy

- Remote endpoints must use HTTPS. Certificates are verified with the operating
  system's trust store (`rustls-platform-verifier`).
- Loopback (`localhost`, `127.0.0.0/8`, `::1`) and private endpoints (RFC 1918,
  `100.64.0.0/10`, `fc00::/7`, single-label names, `.local`, `.lan`, `.internal`,
  `.home.arpa`) require the per-provider **local endpoint** opt-in. With the
  opt-in, plain HTTP is allowed for them.
- Link-local addresses (including the `169.254.169.254` cloud metadata service),
  multicast, broadcast, unspecified and reserved addresses are never contacted,
  even with the opt-in.
- The same policy is checked again on **every address a hostname resolves to**
  when connecting. A public name pointing at a private address is refused
  without the opt-in.
- URLs with user names, passwords, queries or fragments are rejected.
- Redirects are never followed (`REDIRECTED`), so a key cannot be forwarded.
- No proxy is used for provider requests in this version. Networks that require
  an HTTP proxy cannot reach remote providers yet.
- Endpoints come only from the user's provider settings. Nothing in a request
  (prompt text, file names, image metadata) can choose where it is sent.

## Keys

- **System keychain:** macOS Keychain, Windows Credential Manager, or the Linux
  Secret Service. Service `app.creativeshelf.providers`, account = profile ID.
- **This session only:** process memory; forgotten when the app quits.
- A key is never returned to the renderer, logged, written to SQLite, included
  in backups, portable or folder catalogs, diagnostics or exports. `Debug`
  output prints `Secret(redacted)`. Provider error messages are shortened and
  have the key and key-shaped tokens removed.
- A key is bound to the endpoint origin it was entered for. Changing the
  provider kind or the endpoint origin deletes the stored key. Renaming or
  changing the model keeps it.
- Rotation replaces the key in place. Switching between keychain and session
  removes the other copy.
- If the keychain is unavailable or denies access, saving fails with
  `KEYCHAIN_UNAVAILABLE` or `KEYCHAIN_DENIED` and nothing changes. Session-only
  keys still work. Removing a provider deletes its key first, so a refused
  deletion keeps the profile instead of orphaning the secret.
- Listing providers never reads the keychain, so opening Settings causes no
  keychain prompt. The keychain is read only when a test runs.

## Persistence, recovery and concurrency

- Profiles are a machine-local `app_settings` row (`ai_providers`, version 1).
  They are classified as backup only in the SS-048 persistence inventory and are
  not part of portable or folder catalogs: endpoints and keys are machine
  decisions. No schema migration is needed.
- A newer settings version fails with an explicit message rather than being
  rewritten.
- After a restore or on a new machine, keychain profiles whose key is missing
  report `KEY_MISSING` when tested; enter the key again.
- Every mutation carries the profile revision. Stale edits fail with `CONFLICT`
  and the settings view reloads. A connection test records results only if the
  revision is unchanged when it finishes. Callers get an adapter for one exact
  revision and never fall back to another provider.
- Startup builds the service without any network or keychain access (tested).

## Consent

Configuring a provider or key is not consent to send anything. Later analysis
tickets must ask, per source, what may be sent to which provider. This screen
says so, and no network analysis starts from it.

## Error codes

`INVALID_SETTINGS`, `INSECURE_ENDPOINT`, `LOCAL_NOT_PERMITTED`,
`ADDRESS_NOT_ALLOWED`, `KEY_MISSING`, `KEYCHAIN_UNAVAILABLE`, `KEYCHAIN_DENIED`,
`CONFLICT`, `NOT_FOUND`, `AUTHENTICATION`, `MODEL_NOT_FOUND`,
`IMAGE_UNSUPPORTED`, `SCHEMA_INCOMPATIBLE`, `UNSUPPORTED`, `TIMEOUT`,
`QUOTA_EXCEEDED`, `RATE_LIMITED`, `PROVIDER_UNAVAILABLE`, `REDIRECTED`, `TLS`,
`UNREACHABLE`, `RESPONSE_TOO_LARGE`, `REQUEST_TOO_LARGE`, `MALFORMED_RESPONSE`,
`REJECTED`.
