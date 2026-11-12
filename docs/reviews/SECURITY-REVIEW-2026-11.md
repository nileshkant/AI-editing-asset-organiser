# Security review of merged main

Date: 2026-11-12. Branch: `claude/project-thread-b6aip1`.
Reviewed base: `eb7b81f` (`origin/main`, merged PR #57, schema v18).

## Scope

Read for security problems, not general bugs: Tauri capabilities, CSP and the
IPC command surface (`src-tauri`), path and file handling, SQL construction,
FFmpeg and image-worker process invocation, the MCP server and bridge
(`crates/agent`, `crates/core/src/agent*.rs`), AI provider networking and key
storage, backup/restore and catalog import of untrusted files, the renderer,
the GitHub workflows, and both dependency locks (`cargo audit` 0.22.2,
`npm audit`). No queue ticket was started.

## Findings, most severe first

| # | Severity | Finding | Status |
|---|----------|---------|--------|
| 1 | Medium | Bundled SQLite (libsqlite3-sys 0.36.0, RUSTSEC-2026-0459) can return uninitialized memory when a database is opened beside a crafted `-wal` file. Restore opens a user-picked `.sqlite` file. | Fixed: rusqlite 0.41.1 / SQLite 3.54.1 |
| 2 | Medium | rmcp 3.5.0 Streamable HTTP server keeps an unbounded cache keyed by requested tool name (RUSTSEC-2026-0461). A paired MCP client could exhaust the app's memory. The bearer-token guard runs first, so an unpaired caller cannot. | Fixed: rmcp 3.9.0 |
| 3 | Low | rmcp 3.5.0 client buffers response bodies without a limit (RUSTSEC-2026-0462); affects the `soundshelf-mcp` bridge, which only talks to `127.0.0.1`. RUSTSEC-2026-0457 (OAuth client) is in the same crate but that code is not used. | Fixed: rmcp 3.9.0 |
| 4 | Low | undici 7.0.0 to 7.30.0 (eight advisories, high) in the JavaScript lock. Development and test tooling only, not shipped; it fails the `advisories` CI job. | Fixed: lock updated |
| 5 | Low | tokio 1.53.1 (RUSTSEC-2026-0386) and rustix 1.1.5 (RUSTSEC-2026-0486) soundness advisories. The affected APIs are not called here. | Fixed: tokio 1.54.1, rustix 1.1.6 |
| 6 | Low | Provider address policy judged IPv6 addresses by their IPv6 form only. NAT64 (`64:ff9b::/96`), 6to4 (`2002::/16`) and IPv4-compatible spellings of private, loopback or link-local IPv4 targets (for example `64:ff9b::a9fe:a9fe` for `169.254.169.254`) counted as remote, so they skipped the local opt-in and the never-contact list. Reaching them needs a translator on the network, which is why this is low. | Fixed |
| 7 | Low | On Linux the data folder was created with the process umask, so on a shared machine with a traversable home another account could read `library.sqlite` (file paths, notes, prompts) and the preview cache. | Fixed: folder is owner-only |
| 8 | Low | CSP had no `base-uri`, `form-action`, `object-src` or `frame-ancestors`; the first two do not fall back to `default-src`. | Fixed |
| 9 | Low | `SECURITY.md` says the app makes no internet connections. Since SS-049 an optional, user-configured AI provider is contacted. | Fixed: wording corrected |
| 10 | Low | Older audio commands (`import_path`, `import_root`, `relink_source`, `relink_file`, `rebuild_import_folder`) accept a path string from the renderer, while newer image, export, backup and catalog commands only accept natively picked paths. Only matters if the renderer is compromised, which the CSP makes hard. | Open: needs a UI change (the relink dialog takes a typed path) |
| 11 | Low | Workflows that hold signing secrets use actions by movable tag (`dtolnay/rust-toolchain@stable`, `msys2/setup-msys2@v2`, `actions/*@v4`). | Open: recommend pinning to commit SHAs |
| 12 | Info | `cargo audit` still reports nine unmaintained transitive crates (`unic-*`, `proc-macro-error`, `zvariant`, `zbus_names`) through Tauri and keyring. No fix is available upstream; `cargo audit` treats them as warnings. | Open: track upstream |

## What was checked and found sound

- Capabilities grant the main window `core:default` only; no shell, fs or http
  plugin is enabled, and file dialogs run from Rust.
- All SQL with user values is parameterized. `format!` is used only for
  constant column lists and generated `?` placeholders.
- FFmpeg, FFprobe and the image worker are started with argument arrays, a
  protocol and demuxer whitelist, bundled absolute paths, time limits and
  output caps. Nothing goes through a shell.
- MCP binds `127.0.0.1` only, requires an exact `Host`, rejects foreign
  `Origin`, compares token hashes in constant time, stores only hashes, caps
  bodies at 64 KiB and concurrency at 16, scopes every tool to the client's
  sources, and returns the same failure for unknown and out-of-scope IDs.
- Export destinations, backups, restores and catalog imports take paths only
  from native dialogs, bind previews to file digests and never overwrite.
- Folder catalogs are read with `O_NOFOLLOW`, size limits and full validation;
  member paths must be relative and may not cross symlinks.
- Provider requests never follow redirects or proxies, check every resolved
  address, cap response size and scrub keys from errors. Keys live in the OS
  credential store or session memory.
- The renderer has no `dangerouslySetInnerHTML`, `eval`, external links or
  secrets in `localStorage`.

## Changes in this branch

- `Cargo.lock`, `crates/*/Cargo.toml`: rusqlite 0.38 to 0.41 (libsqlite3-sys
  0.39.1), rmcp 3.5.0 to 3.9.0, tokio 1.54.1, rustix 1.1.6. No source change
  was needed for either upgrade.
- `package-lock.json`: `npm audit fix` (undici).
- `crates/core/src/ai_provider.rs`: `embedded_v4` maps IPv4-mapped,
  IPv4-compatible, NAT64 and 6to4 addresses to the IPv4 address they carry;
  Teredo and documentation ranges are never contacted; site-local and
  local-use NAT64 are private. Public NAT64 addresses stay remote, so
  IPv6-only networks using DNS64 keep working.
- `crates/core/src/lib.rs`, `src-tauri/src/service.rs`: `private_directory`
  creates the data folder and removes group and other access on Unix,
  including on a folder made by an earlier version.
- `src-tauri/tauri.conf.json`: CSP adds `object-src 'none'; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'`.
- `SECURITY.md`: describes the optional provider connection and key storage.

## Tests actually run

Linux x86_64 container, Rust 1.97.0, system FFmpeg as the fixture tools.

- `cargo audit` (0.22.2): before, 4 vulnerabilities (RUSTSEC-2026-0457, -0459,
  -0461, -0462) and 2 unsound warnings; after, 0 vulnerabilities and only the
  nine unmaintained warnings in finding 12.
- `npm audit`: before, 1 high (undici); after, 0.
- `cargo test --workspace --locked --no-fail-fast -- --include-ignored`:
  434 tests ran, 429 passed, and 1 was skipped. None of the 6 is related to
  this change: 4 need `SOUNDSHELF_FFMPEG`/`SOUNDSHELF_FFPROBE` and pass when
  those are set (17 of 17 in `import` and `file_scopes`); the failed
  `read_only_snapshot_failure_is_retryable_and_member_access_denial_never_prunes`
  and the skipped `real_backup_write_failure_cancels_upgrade_before_schema_mutation`
  rely on read-only folders, which do not restrict the root user this
  container runs as. CI runs them as a normal user.
- `cargo test -p soundshelf --features image-development --locked image_`: 4 passed.
- `ipv6_forms_that_carry_an_ipv4_address_follow_that_address` fails on the
  reviewed base (`[64:ff9b::a9fe:a9fe]` was classified as a remote address)
  and passes with the fix. It also asserts that public NAT64, 6to4 and global
  IPv6 addresses are still accepted without the local opt-in.
- `private_directory_tests::data_folder_is_owner_only_and_an_open_one_is_narrowed`
  and `service::tests::data_directory_is_private_to_the_user`: a 0755 folder
  becomes 0700 with its contents intact; a new folder is 0700.
- `service::tests::content_security_policy_keeps_the_renderer_closed` passes.
- `npm test`: 30 files, 332 tests passed. `npm run build` succeeded.
- `node scripts/verify-glib-patch.mjs`: 121 upstream GLib files verified.

## Known limitations

- The CSP change is asserted from the configuration file. It was not exercised
  in a packaged WebView on macOS or Windows from this environment.
- Findings 10 to 12 are not changed here.
- Data folder permissions are a Unix measure; Windows relies on the per-user
  profile ACL.
