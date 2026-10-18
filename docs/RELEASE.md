# CreativeShelf release preparation

There is **no qualified tester installer yet**. The requested tester platforms are
macOS, Windows and Linux. Native candidate tooling supports macOS arm64/x86_64,
Windows x64 and Linux x64; each architecture needs its own signed artifact and
installed qualification. Linux also depends on SS-034 GTK remediation. CI passing
is implementation evidence, not clean-machine release qualification.

## Verified minimal media

Run `npm ci` and `npm run media:build` in an isolated clean checkout. This verifies
the pinned official FFmpeg 9.0.2 archive size, SHA-256 and detached signature
against the published signing fingerprint before extracting or compiling it.
The recipe enables audio container decoders, lossless encoders, required filters
and file/pipe protocols. It disables network, shared libraries and external library
autodetection; GPL/nonfree components are not enabled. OpenPGP is a build dependency
and is not shipped in the native application.

The generated ignored `.release-input/media-lock.json` records exact target binary
hashes, configuration, source hash and the full LGPL notice. `release/media-lock.json`
is the alternative third-party approval file and intentionally has no approvals.
Review compiler/runtime provenance and the complete redistribution notice before
release. Publish the exact corresponding source archive and build recipe alongside
downloads; those do not belong inside the installed application. Do not substitute
Homebrew/Chocolatey binaries, which can depend on developer libraries.

`npm run release:check` verifies bounded regular inputs, checksums, native
architecture, version, configuration and required output muxers. macOS permits
only system dynamic dependencies. Windows PE inspection rejects non-system DLL
imports and delayed imports. Linux media executables must have neither an ELF
interpreter nor shared-library dependencies. These media checks do not make the
GUI static: Linux WebKitGTK/audio dependencies require clean offline qualification.
The minimal-media CI workflow builds and exercises the exact tools on all three
native hosts, preserving binaries, notices, locks and corresponding source.

## Direct-download releases

`.github/workflows/unsigned-release.yml` builds, inspects and publishes installers
to GitHub Releases from a pushed tag. No paid certificates are used.

| Tag | Release | Notes file | Linux GPG signature |
|---|---|---|---|
| `vX.Y.Z-alpha.N` | prerelease, "unsigned tester preview" | `docs/UNSIGNED_TESTER_RELEASE.md` | when provisioned |
| `vX.Y.Z-beta.N`, `vX.Y.Z-rc.N` | Latest, "public beta"/"release candidate" | `docs/RELEASE_NOTES.md` | required |
| `vX.Y.Z` | Latest | `docs/RELEASE_NOTES.md` | required |

macOS is ad-hoc signed (no Developer ID or notarization, so Gatekeeper asks once).
Windows is unsigned until a free SignPath Foundation certificate is approved.
Linux AppImage, deb and checksum list get detached `.asc` signatures. Public
releases fail closed unless every one verifies against `release/linux-release-key.asc`.

### One-time Linux signing key

Install GnuPG first (macOS: `brew install gnupg`). Create a dedicated signing-only
key, separate from personal keys and without subkeys, so its fingerprint matches
the signature check:

```
gpg --quick-gen-key "CreativeShelf Releases <YOUR-EMAIL>" ed25519 sign 3y
gpg --armor --export "CreativeShelf Releases" > release/linux-release-key.asc
gpg --armor --export-secret-keys "CreativeShelf Releases" | pbcopy
```

Leave the passphrase empty when prompted: the key is protected by GitHub's encrypted secret store
instead. Save the copied private key as the repository secret `LINUX_GPG_PRIVATE_KEY`
(Settings → Secrets and variables → Actions). Commit only the public
`release/linux-release-key.asc`. Back up the private key offline. Rotating it means
publishing a new public key and announcing the change in the next release.

### Publishing

1. Merge the release changes to main and confirm CI passes.
2. Tag the exact commit, for example `git tag v0.9.0-beta.1 && git push origin v0.9.0-beta.1`.
3. The workflow publishes DMG, setup.exe, AppImage, deb, Linux signatures and key,
   checksums, inspection reports, provenance and the FFmpeg source.

### Free Windows signing (SignPath Foundation)

Apply at https://signpath.org with the public repository. Once approved, add their
signing action to the Windows job before inspection, then update the notes files
and `SECURITY.md` to drop the SmartScreen step.

## Signed candidates

1. Use a clean checkout of the candidate commit with checked-in dependency locks.
   Build and review media as above. Provision signing identities privately in the
   native keychain/certificate store/GPG setup. Never commit keys or passwords.
2. Configure signing settings and run `npm run desktop:bundle`. macOS requires
   APPLE_SIGNING_IDENTITY, APPLE_TEAM_ID, APPLE_ID and APPLE_PASSWORD. Windows
   requires WINDOWS_CERT_THUMBPRINT and HTTPS WINDOWS_TIMESTAMP_URL plus signtool.
   Linux requires LINUX_SIGNING_FINGERPRINT and its privately provisioned key.
   The manual candidate workflow fails closed if provisioning is absent; it does
   not create or import signing identities automatically.
3. The command builds the MCP bridge with `--locked --target`, stages exactly five
   explicit resources (two media tools, bridge, license and provenance), and signs
   resource executables on macOS/Windows before native packaging. A preexisting
   staging directory stops the build: use a fresh isolated checkout. macOS creates
   app/DMG, Windows NSIS/MSI with offline WebView2 and downgrades disabled, and
   Linux AppImage/deb with detached GPG signatures. No automatic upload/publication.
4. Extract the actual installer and run
   `npm run release:inspect -- /absolute/extracted-app-directory /absolute/installer`.
   Keep Linux's detached `.asc` beside its original artifact. Inspection verifies
   expected publisher signatures (including Gatekeeper/stapling on macOS), exact
   resource membership/checksums, architecture and absence of source recordings,
   SQLite/development data. Internal links are recorded only when their canonical
   targets stay inside the extracted tree; media/bridge resources must be regular.
   Modified executable bytes require independent expected-publisher verification;
   notices cannot change. Windows main executable is also verified.
5. Preserve `release-reports/stage.json` and `package.json`: commit and dependency
   lock hashes, exact installer hash/compressed size, extracted size, media/resource
   overhead, zero model packs and signature results. Complete SS-031 against this
   exact artifact before human release approval, including offline clean install,
   physical listening, installed MCP, recovery, upgrade, uninstall and minimum OS.

Release startup requires bundled media and does not use developer Homebrew tools
or environment overrides. Settings → Agent access → Show installed bridge exposes
its absolute installed path and discovery argument independently of PATH/workspace.
Pair clients separately and keep credentials private. The stable compatibility
executable name remains `soundshelf-mcp`.

## Isolated local checks

`npm run desktop:local` creates a macOS-only diagnostic bundle named CreativeShelf
Local with the distinct identifier `app.creativeshelf.qualification.local`. Its
catalog is separate from the existing installed application. This permits local
workspace testing without signing credentials; it does **not** authorize distribution.
Zip that local bundle, then use `npm run release:inspect-local -- /absolute/app /absolute/zip`.
The report explicitly records an unsigned local purpose, signature=false and dirty
workspace provenance where applicable. Signed and local inspection modes cannot
be interchanged. Do not rebuild or replace a bundle during an active import.

## Updates and rollback

Automatic updates are disabled: there is no update endpoint, updater plugin or
update signing key. Use a newly signed full installer after backing up the catalog.
Verify offline upgrade/uninstall retention of user data and originals on every
qualified platform. Schema rollback uses a pre-migration backup with its matching
application version; do not downgrade a migrated live database. The signed updater
acceptance criterion remains deferred, not implemented by the full-installer policy.

Packaging follows [Tauri resources](https://v2.tauri.app/develop/resources/),
[macOS signing](https://v2.tauri.app/distribute/sign/macos/) and
[Windows signing](https://v2.tauri.app/distribute/sign/windows/).
