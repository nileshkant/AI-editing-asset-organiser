# Local MCP access

SS-018 adds an opt-in MCP service using the official Rust SDK `rmcp` 3.5.0. Open CreativeShelf → Settings → Agent access → Start MCP. Port 0 picks an available port; a fixed occupied port produces an actionable error. Pair each client separately. Copy its displayed HTTP configuration into a header-capable MCP client and keep the credential private. Hiding it or leaving Settings clears the displayed copy.

SS-019 adds catalog and clip tools with explicit source and capability grants; see [MCP tool contract](MCP_TOOLS.md). Status-only pairings still grant no catalog access. Shell/SQL/arbitrary filesystem tools remain unavailable.

Pairings are temporary: Stop MCP, full Quit or a process restart invalidates every token. The service does not start automatically. Revoke immediately blocks subsequent requests from that client; an already accepted status request may finish. Another paired client remains authorized. There is no tray mode or background daemon.

HTTP binds only IPv4 loopback, accepts the exact `127.0.0.1:<port>` Host, and permits absent Origin (native clients) or the exact endpoint origin. DNS names, hostile origins, duplicate security headers, URL query credentials and invalid/revoked bearer credentials fail before protocol processing. Requests are limited to 64 KiB, 16 concurrent requests and five seconds to read a body; client count is capped at 32. These bounds are not a complete performance qualification or a full MCP OAuth implementation.

## Compiled stdio clients

Build the bridge with `npm run mcp:bridge`. The resulting executable is `target/release/soundshelf-mcp` (`.exe` on Windows). Packaging the bridge into installers is SS-024 work. Configure a command-based client with this absolute executable path, arguments `--discovery` and the absolute `<CreativeShelf data directory>/runtime/mcp.json` path, and environment variable `SOUNDSHELF_MCP_TOKEN` containing that client's paired credential. A direct `http://127.0.0.1:<port>/mcp` argument is also supported.

The discovery file contains only the endpoint and is removed on normal Stop/Quit. A stale discovery file after a crash cannot start a service. The bridge restricts endpoint URLs to numeric loopback HTTP, ignores proxies, rejects redirects, checks access before reading stdin and transports MCP messages with the official SDK. It never opens SQLite or launches CreativeShelf. Credentials go in the client environment or HTTP Authorization header, never endpoint URLs or command arguments. Bridge failures use a fixed stderr message and keep stdout for protocol traffic.

## Protocol references

Protocol initialization, negotiation, tool discovery and transport handling are delegated to the [official Rust SDK](https://github.com/modelcontextprotocol/rust-sdk). The [MCP transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) describes loopback and Origin protections. Tests use a real SDK client over HTTP and the compiled bridge, including negotiated 2025-11-25 initialization. This is not a claim of compatibility with every agent application's configuration format.
