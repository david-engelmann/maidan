# sdk/

Language clients for Maidan. The **server** crate is unpublished
(`publish = false`); these packages are the public clients, all
**live at 0.1.0** on their registries.

| Dir | Registry package | Status |
|-----|------------------|--------|
| `python/` | `maidan` (PyPI) | **0.1.0 (published)** |
| `typescript/` | `maidan` (JS registry) | **0.1.0 (published)** |
| `rust/` | `maidan` (crates.io) | **0.1.0 (published)** |
| `go/` | module in this repo | **`sdk/go/v0.1.0` tag** |

`pip install maidan` / `npm i maidan` / `cargo add maidan` /
`go get github.com/david-engelmann/maidan/sdk/go@sdk/go/v0.1.0`.

**Live here.** Independent SemVer from the server. A `vX.0.0`
server tag does not publish these — publish only on an explicit
`sdk-*` tag (`.github/workflows/sdk-release.yml`). Details in
[docs/Clients.md](../docs/archive/Clients.md) §2.

Implement from:

- [docs/Clients.md](../docs/archive/Clients.md) — doors, work order, repo
- [docs/Client Contract.md](../docs/Client%20Contract.md) — method map
- [docs/Client Testing.md](../docs/archive/Client%20Testing.md) — scenarios

The SDK is REST + WebSocket. MCP is the LangChain / AutoGen /
Cursor door (`client.mcp_url` is a string, not a dependency).
A2A is a recipe, not a fourth library. Do not generate the full
OpenAPI. Rust must not depend on `maidan-server`.

0.1.0 is the first usable release (shipped, clusters 294–299). 0.2.0 (in the tree, not yet tagged) adds retries with `Idempotency-Key` on every write, and auto-paging; see each README and `docs/Client Contract.md` §2.
Clients capture `Maidan-Room-LSN` as `last_room_lsn` (Cluster 390). Since
Cluster 398.8 that value is **the caller's workspace head**, not the instance's,
so it is comparable to a `log_id` the client has actually seen.
Typed responses and an error type per RFC 9457 problem `type` are in the tree
too (not yet tagged): each SDK's models follow the server's OpenAPI schemas,
and its black-box suite (`scripts/sdk-test.sh <lang>`) fails when a live
response carries a member its model does not declare. The packages still say
0.2.0; the version these ship under is the maintainer's call.
Do not bump the package version for the header capture.
