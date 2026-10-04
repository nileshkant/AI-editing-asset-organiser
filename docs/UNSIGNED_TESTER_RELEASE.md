## Changes in v0.1.0-alpha.6

- SS-035/036: retain the merged Windows media console fix and reorganized Settings.
- SS-037: MCP credentials and scoped permissions survive restarts. Enabling MCP remembers its endpoint and starts it with the app; Stop disables automatic startup, while Quit preserves it. Rotate explicitly replaces one token; Revoke permanently removes that client. Occupied ports and unreadable settings report failures without resetting credentials.
- SS-038: removed the remaining Development build header and added Settings → MCP guide, including offline setup, transport/bridge, all current tools, permissions, export, troubleshooting and reviewed AI tagging. Windows CRLF reference parsing is covered by a regression.

MCP changes are reviewed in [PR #35](https://github.com/nileshkant/AI-editing-asset-organiser/pull/35) and [PR #36](https://github.com/nileshkant/AI-editing-asset-organiser/pull/36). This preview combines their branches without automatically merging them into main. Local evidence: 166 frontend tests, production bundle and 197 native tests; the installer workflow repeats native and frontend checks on each platform before inspection and publication.

**Upgrade pairing:** alpha.5 and earlier used memory-only tokens. Pair once on this updated version and update the client configuration. Subsequent app restarts keep that credential and URL. Lost tokens are shown only when created/rotated; rotate to recover access.

**Testing team:** pair a disposable client with Read and one disposable source, copy its configuration, quit/reopen CreativeShelf and verify the same token/URL can search. Rotate and verify the old token fails; revoke and verify it remains rejected after restart. Stop MCP, reopen the app and verify it stays stopped; Start enables automatic startup again. Check Windows imports/seek for console flashes and all Settings panes at 100%/150%/200% scaling. Read both guide references and confirm no clipped or missing tool names. Record OS, installer hash and exact client configuration format; do not include tokens in reports.

**AI/import limits:** MCP currently searches and edits catalog tags but cannot import audio or recognize recordings. Import in the app first. An audio-capable client can analyze approved audio and save reviewed event tags through sounds.annotate; automatic import recognition and approved MCP import remain upcoming SS-016/017/039. Images and presets remain future work.

CreativeShelf's audio tester preview is distributed directly here, outside app stores. **These installers have no trusted publisher signature. This is an experimental prerelease, not a certified production release.** macOS uses free ad-hoc signing for executable integrity, without Developer ID or Apple notarization. Windows is unsigned. Linux has checksums without a publisher GPG signature.

Download the installer matching your operating system and the architecture shown in its filename/provenance:

- Windows: `*-setup.exe` (x64). Run the installer. It includes an offline WebView2 installer; antivirus, SmartScreen or organization policies may warn or block installation.
- macOS: `.dmg`. Open it and drag CreativeShelf to Applications. Minimum macOS 14.2. Apple Silicon and Intel builds are not interchangeable; check the filename. Gatekeeper may require explicit approval under System Settings → Privacy & Security. See Apple's guidance: https://support.apple.com/102445. Managed Macs may prohibit unverified apps.
- Linux: `.AppImage` or `.deb` (x64, Ubuntu 24.04 build). For AppImage, make the downloaded file executable and run it; FUSE/runtime requirements vary by distribution. On Debian/Ubuntu use `sudo apt install ./DOWNLOADED_FILE.deb` so required desktop libraries are resolved. Other distributions and older glibc versions are not yet qualified.

The release includes SHA-256 checksum lists, exact-commit provenance and actual extracted-package inspection reports. Checksums detect differing downloads; they do not establish trusted publisher identity. No script disables operating-system security protections.

Audio originals stay in folders you select. Source-folder `.creativeshelf/catalog.json` metadata supports warm imports; the application stores references, analysis, tags and settings. No user audio or user database is bundled. Automatic imports will write that metadata into selected source folders: test with a copied, small audio folder first. The bundled audio tools and MCP bridge are included; AI sound-event recognition, images and presets are future work.

MCP is off before first setup. Enable it in Settings → Agent access, select authorized sources and permissions, then pair your client. Once enabled it starts with the app on the remembered port. Stop disables automatic startup without deleting pairings; Quit closes the listener until next launch. Keep credentials private. The installed bridge path and complete MCP guide are available in Settings.

Automated tests and package inspection are prerequisites for publishing these files. Physical audio output, real third-party MCP clients, screen-reader use, two-hour sessions, all-platform clean installs and upgrade/uninstall behavior remain qualification work. Back up existing catalogs before installing a preview over an earlier build; schema migrations retain a pre-migration database backup.

Please report your OS/version and downloaded filename, the steps to reproduce, expected/actual behavior and relevant diagnostics in the repository's GitHub Issues. Avoid uploading private recordings, source paths, catalogs or credentials.

FFmpeg is built from the verified upstream source attached to this release under LGPL terms. Its license/provenance notices are bundled, and the version-tagged repository contains the exact build scripts and source/signature lock. No automatic updater is enabled; install later previews manually.
