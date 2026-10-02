# Audio v1 security and performance qualification

## Boundaries

The renderer has application IPC and native pick/save dialogs. It has no generic
shell/filesystem API. Original recordings stay in user sources; the catalog and
folder metadata contain references. Desktop local data and deliberate backups
are private and unencrypted. Pair credentials are ephemeral. MCP is off by
default, loopback-only, bearer-authenticated, origin/host validated and source
scoped, with separate edit/export/path permissions and native destination grants.
Revocation blocks later dispatch; an accepted export may finish publication.

Treat audio, folder catalogs, portable imports, annotations and MCP arguments as
untrusted. Source membership/canonical containment protects resolved media.
Metadata imports have byte/item limits and validate paths/identities before
mutation. Source snapshots use revision/OS lock checks, dirty retries and staged
atomic writes; offline/access failure never proves deletion. SQLite migrations
and restore retain backups. Restore accepts compatible integrity-checked schema;
diagnostics serialize an explicit allowlist instead of redacting arbitrary logs.

Media tools are separate qualified binaries. All source probe/decode paths now
allow only file/pipe protocols and standalone supported demuxers. HLS/concat
playlists and network protocols are denied. Arguments use process argv, never
shell text. Watchdogs, cancellation, one import worker and decoder thread limits
bound work. This does not sandbox native codec vulnerabilities; dependency and
binary qualification remains necessary. See official FFmpeg
[protocol options](https://ffmpeg.org/ffmpeg-protocols.html#Protocol-Options) and
[format options](https://ffmpeg.org/ffmpeg-formats.html#Format-Options).

Waveforms adapt base resolution to at most 65,536 buckets per channel and 32
channels. Cache reads cap at 64 MiB; dimensions/payload lengths are checked before
allocation, samples must be finite, and the cache algorithm version is checked.
Version 1 waveform caches rebuild on demand. Parser corpus tests are deterministic
regressions, not coverage-guided fuzzing or a complete security audit.

## Performance and accessibility targets

Candidate targets: ordinary search pages at 100k records within 1 second p95,
UI command feedback within 100 ms, native play/seek feedback within 150 ms after
cached startup, no steady playback underruns under one concurrent import,
and stable memory over a two-hour mixed-use session. These are release targets,
not measured achievements unless an exact artifact/platform result is recorded.
Keyboard-only search, selection, playback, clip export, settings/confirmation
and MCP pairing must remain usable; focus must be visible and trapped/restored
in dialogs. Screen reader and WCAG contrast checks require native qualification.

Search now reads a compact derived profile containing tags and measurements,
without waveform samples. Full profiles remain authoritative for playback,
inspection and source catalogs. Schema 9 builds this search cache transactionally
with a pre-migration database backup; analysis insert/update/delete changes keep
it synchronized. Restore checks agreement with authoritative profiles first.
Pagination still does not bound the metadata working set: search reads candidate
metadata and ranks it in memory. The repeatable benchmark supports up to 100k
records, 3,200 waveform buckets and distinct per-file profiles, plus the former
full-profile materialization path for controlled comparison. Runtime measurements
and fixture limitations are in `docs/reviews/SS-023.md`. These checks do not certify
a two-hour session, all query patterns, or platform installer behavior.

## Required installed stress checks

Use a signed candidate on each declared platform. Record versions, hash and
physical output route. Run long audio + concurrent imports/exports, repeated
seek/device disconnect, external-drive loss, source snapshot concurrent edits,
populated backup/restore, disk-full/interruption and recovery. A real MCP client
must test scoped discovery, failed unauthorized calls, paging, revocation,
completion/retry and Stop/Quit/reconnect. Include keyboard and screen reader use.
No mocked sink or CI unit result certifies physical audibility or installer safety.

Remaining gates include sustained fuzzing, codec binary/license audit, platform
signing, real installed clients/audio, worst-case memory and long-session results.
SS-024 and SS-031 retain these gates; this document does not declare production
readiness or the absence of bugs.
