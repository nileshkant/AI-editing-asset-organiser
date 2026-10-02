# CreativeShelf release preparation

There is **no qualified tester installer yet**. Automated CI compile/test platforms
are not the supported release matrix. The candidate tooling currently supports
native macOS arm64/x86_64 preflight; neither architecture is qualified. Windows
packaging remains blocked until static dependency closure and certificate checks
exist. Linux release qualification also depends on SS-034's GTK audit remediation.

`npm run desktop:binary` is a development/native compilation check. It does not
produce a tester package. `npm run desktop:bundle` is the guarded candidate path.
Release builds require bundled media and never discover tools through developer
Homebrew locations or SOUNDSHELF environment overrides.

## Media approval

`release/media-lock.json` intentionally has no approved entries. A release owner
must obtain/build target-specific static LGPL media tools, review their origin,
configuration, redistribution license, source availability, and runtime dependencies,
then commit a reviewed lock entry. Do not copy Homebrew/Chocolatey tools wholesale:
these may depend on machine libraries or enable GPL/nonfree components.

Each target entry records `version`, exact `configuration`, `license` (currently
LGPL-2.1-or-later), HTTPS `sourceUrl`, reproducible `buildInstructions`, and
`ffmpeg`, `ffprobe`, `notices` entries containing relative `path`, `bytes`, and
SHA-256 `sha256`. The notice must contain the actual license and source/build
instructions needed for redistribution. A field alone is not legal/license evidence.
Review the complete notice in SS-024 before approving a lock. Place approved files
under ignored `.release-input/`. `npm run release:check` verifies bounded regular
files, hashes, native architecture, exact version/configuration, and macOS system-only
dynamic dependencies. It rejects GPL/nonfree/shared configurations. It neither
fetches remote binaries nor executes build instructions from metadata.

## Candidate steps

1. Use a clean checkout of the candidate commit with the checked-in dependency locks.
   Provision audited `.release-input` and the Apple signing identity in a private
   keychain. No signing certificate/password belongs in this repo or reports.
2. Set private Apple signing/notarization environment settings (APPLE_SIGNING_IDENTITY,
   APPLE_TEAM_ID, APPLE_ID, APPLE_PASSWORD), then run `npm run desktop:bundle`.
   The target is the native host architecture. The compiled bridge is built with
   `--locked --target`; five explicit resources are staged in a new directory:
   two media executables, the MCP bridge, license notice and build provenance.
   Executables are signed before Tauri signs/notarizes the app. A preexisting
   `src-tauri/release-assets` stops the build; prepare a fresh isolated checkout.
3. Inspect the extracted signed `.app` and its DMG:
   `npm run release:inspect -- /absolute/CreativeShelf.app /absolute/CreativeShelf.dmg`.
   APPLE_TEAM_ID must match the expected publisher. This verifies app/resource
   signatures, Gatekeeper assessment, exact resource membership/checksums, and
   rejects linked members, source recordings and SQLite/development data.
   Code signing may alter executable hashes; a changed executable requires the
   independently verified expected publisher signature. Notices cannot change.
4. Preserve `release-reports/stage.json` and `package.json`. Reports identify
   commit, lock hashes, installer SHA-256/compressed bytes, extracted inventory
   and uncompressed bytes, media runtime overhead separately, and zero model packs.
   Source originals stay in user folders; caches/exports are runtime user data.
5. Complete SS-031 against this **exact signed artifact**, including offline clean
   install, listening, MCP, recovery, upgrade and uninstall. No automatic publication
   occurs. The manual candidate workflow stops on missing approval inputs.

Tauri resource maps explicitly place resources at `media/` and `mcp/` independent
of input directory names. Settings → Agent access → Show installed bridge returns
its absolute installed path and the local discovery argument. It never searches
PATH or relies on a workspace. Pair a client separately; keep its credential private.
The bridge uses the stable compatibility name `soundshelf-mcp`.

## Updates and rollback

Automatic updates are disabled for this first version: there is no update endpoint,
updater plugin or signing key. Use a newly signed/notarized full installer, after
backing up the catalog in Settings. Uninstall/reinstall must retain user library data
and source files; verify that behavior on a clean test machine. Never replace an
installed app from an unsigned download. Schema rollback uses the pre-migration
backup with the matching app version; do not downgrade a live migrated database.
The ticket's signed updater acceptance criterion remains deferred, rather than
claiming a nonexistent update mechanism.

Resource mapping and platform signing follow [Tauri resource documentation](https://v2.tauri.app/develop/resources/),
[macOS signing](https://v2.tauri.app/distribute/sign/macos/) and
[Windows signing](https://v2.tauri.app/distribute/sign/windows/).
