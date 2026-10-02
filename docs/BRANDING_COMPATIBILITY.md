# CreativeShelf identity and compatibility

The product, window, desktop bundle, catalog Markdown and advertised MCP server
use CreativeShelf. Generated client configuration uses the `creativeshelf` label.
Client labels are arbitrary: existing `soundshelf` entries keep working.

The installed application identifier remains `app.soundshelf.desktop`. This is
its stable upgrade identity, deliberately retained to keep existing catalogs and
OS grants in the same location. No second data directory is created and no data
migration or destructive rollback runs for this display-name change. Existing
fallback data and startup-log directories retain `SoundShelf` for recovery.

Rust package and binary names (`soundshelf`, `soundshelf-mcp`), the
`SOUNDSHELF_MCP_TOKEN` and media-tool environment variables, SQLite/catalog
schemas, cache versions and exported manifest keys are compatibility aliases.
Use the existing executable path when configuring the stdio bridge. Its server
metadata now advertises CreativeShelf. Existing JSON backups and manifests
remain readable. Source media is referenced and never bundled by this change.

An installed, populated upgrade and native package identity checks are release
qualification gates in SS-024/031. A source build alone does not verify OS
installer replacement, signing or paired client reconnect behavior.
