# sdk/

Language clients for Maidan. The **server** crate is unpublished;
these packages are the public clients.

| Dir | Registry package | Status |
|-----|------------------|--------|
| `python/` | `maidan` (PyPI) | 0.0.1 name hold |
| `typescript/` | `maidan` (JS registry) | 0.0.1 name hold |
| `rust/` | `maidan` (crates.io) | 0.0.1 name hold |
| `go/` | module in this repo | stub only |

**Live here.** Independent SemVer from the server. A `v280.0.0`
server tag does not publish these. Publish only on an explicit
`sdk-*` tag. Details in [docs/Clients.md](../docs/Clients.md) §2.

Implement from:

- [docs/Clients.md](../docs/Clients.md) — doors, work order, repo
- [docs/Client Contract.md](../docs/Client%20Contract.md) — method map
- [docs/Client Testing.md](../docs/Client%20Testing.md) — scenarios

The SDK is REST + WebSocket. MCP is the LangChain / AutoGen /
Cursor door (`client.mcp_url` is a string, not a dependency).
A2A is a recipe, not a fourth library. Do not generate the full
OpenAPI. Rust must not depend on `maidan-server`.

0.1.0 is the first usable release. 0.0.1 stays the reservation.
Do not implement without a go.
