# Security policy

## Supported versions

Security fixes go into the newest release on the
[Releases page](https://github.com/nileshkant/AI-editing-asset-organiser/releases/latest).
Tester previews (`-alpha` tags) are not supported once a newer release exists.

## Reporting a vulnerability

Please report security problems privately through
[GitHub's private vulnerability reporting](https://github.com/nileshkant/AI-editing-asset-organiser/security/advisories/new),
not in a public issue. Include the CreativeShelf version, your operating system
and the steps to reproduce. Leave out private recordings, catalogs, file paths and MCP tokens.

You should get a reply within 7 days. Once a fix is released, the advisory is
published with credit to the reporter, unless you'd rather stay anonymous.

## What CreativeShelf does and doesn't do

- It makes no internet connections and has no telemetry.
- Agent access (MCP) is off by default. When enabled, it listens only on
  `127.0.0.1` and requires a per-client token with scoped permissions.
- Original audio files are never modified.

## Verifying downloads

Every release publishes SHA-256 checksums. Linux downloads also carry detached
GPG signatures made with the key in [`release/linux-release-key.asc`](release/linux-release-key.asc),
fingerprint `2FB8 3BE9 E38E 1AA7 2E2D  A3D7 08C4 C1F4 239D 6311`.
macOS builds are ad-hoc signed (not notarized) and Windows builds are not yet
publisher-signed; both rely on the published checksums.
