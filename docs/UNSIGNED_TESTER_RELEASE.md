## Changes in v0.1.0-alpha.5

- SS-035: Windows media workers now use CREATE_NO_WINDOW during imports, waveform generation, playback/seek and export. Windows CI observes no console in native child processes and preserved output pipes. Installed-app flash/focus observation remains a tester check.
- SS-036: Settings has five keyboard-accessible categories, grouped controls, wrapping actions/paths and preserved form/import-preview state. Settings keyboard controls no longer trigger library shortcuts. The version now uses packaged alpha metadata instead of Cargo’s fixed version; removed the misleading Development label. Browser checks cover the minimum 720x560 window; installed OS scaling and screen-reader checks remain pending.

The fixes are reviewed separately in [PR #33](https://github.com/nileshkant/AI-editing-asset-organiser/pull/33) and [PR #34](https://github.com/nileshkant/AI-editing-asset-organiser/pull/34). This tester artifact combines the branches without automatically merging those PRs into main.

Testing team: follow [SS-035 Windows checks](https://github.com/nileshkant/AI-editing-asset-organiser/blob/v0.1.0-alpha.5/tickets/SS-035.md#tester-procedure) and [SS-036 Settings checks](https://github.com/nileshkant/AI-editing-asset-organiser/blob/v0.1.0-alpha.5/tickets/SS-036.md#testing-team-handoff). Record the installer hash, OS, scaling and screen recording with any report. Do not use alpha.4 to validate these fixes.

CreativeShelf's audio tester preview is distributed directly here, outside app stores. **These installers have no trusted publisher signature. This is an experimental prerelease, not a certified production release.** macOS uses free ad-hoc signing for executable integrity, without Developer ID or Apple notarization. Windows is unsigned. Linux has checksums without a publisher GPG signature.

Download the installer matching your operating system and the architecture shown in its filename/provenance:

- Windows: `*-setup.exe` (x64). Run the installer. It includes an offline WebView2 installer; antivirus, SmartScreen or organization policies may warn or block installation.
- macOS: `.dmg`. Open it and drag CreativeShelf to Applications. Minimum macOS 14.2. Apple Silicon and Intel builds are not interchangeable; check the filename. Gatekeeper may require explicit approval under System Settings → Privacy & Security. See Apple's guidance: https://support.apple.com/102445. Managed Macs may prohibit unverified apps.
- Linux: `.AppImage` or `.deb` (x64, Ubuntu 24.04 build). For AppImage, make the downloaded file executable and run it; FUSE/runtime requirements vary by distribution. On Debian/Ubuntu use `sudo apt install ./DOWNLOADED_FILE.deb` so required desktop libraries are resolved. Other distributions and older glibc versions are not yet qualified.

The release includes SHA-256 checksum lists, exact-commit provenance and actual extracted-package inspection reports. Checksums detect differing downloads; they do not establish trusted publisher identity. No script disables operating-system security protections.

Audio originals stay in folders you select. Source-folder `.creativeshelf/catalog.json` metadata supports warm imports; the application stores references, analysis, tags and settings. No user audio or user database is bundled. Automatic imports will write that metadata into selected source folders: test with a copied, small audio folder first. The bundled audio tools and MCP bridge are included; AI sound-event recognition, images and presets are future work.

MCP is off by default. Enable it in Settings → Agent access, select authorized sources and permissions, then pair your client. Keep pairing credentials private. The installed bridge path is shown in the app.

Automated tests and package inspection are prerequisites for publishing these files. Physical audio output, real third-party MCP clients, screen-reader use, two-hour sessions, all-platform clean installs and upgrade/uninstall behavior remain qualification work. Back up existing catalogs before installing a preview over an earlier build; schema migrations retain a pre-migration database backup.

Please report your OS/version and downloaded filename, the steps to reproduce, expected/actual behavior and relevant diagnostics in the repository's GitHub Issues. Avoid uploading private recordings, source paths, catalogs or credentials.

FFmpeg is built from the verified upstream source attached to this release under LGPL terms. Its license/provenance notices are bundled, and the version-tagged repository contains the exact build scripts and source/signature lock. No automatic updater is enabled; install later previews manually.
