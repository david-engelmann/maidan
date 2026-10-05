# sdk/

Language clients for Maidan. The **server** crate is unpublished
(`publish = false`); these packages are the public clients. The registries
carry **0.1.0**. The tree is **0.3.0**, which publishes with the next server
release.

| Dir | Registry package | Status |
|-----|------------------|--------|
| `python/` | `maidan` (PyPI) | **0.1.0 (published)** |
| `typescript/` | `maidan` (JS registry) | **0.1.0 (published)** |
| `rust/` | `maidan` (crates.io) | **0.1.0 (published)** |
| `go/` | module in this repo | **`sdk/go/v0.1.0` tag** |

`pip install maidan` / `npm i maidan` / `cargo add maidan` /
`go get github.com/david-engelmann/maidan/sdk/go@sdk/go/v0.1.0` install 0.1.0
until 0.3.0 is tagged.

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

0.1.0 is the first usable release (shipped, clusters 294–299). 0.3.0 is
next, and 0.2.0 is skipped: it was never tagged, and nobody depends on it.
0.3.0 adds retries with `Idempotency-Key` on every write, auto-paging, typed
responses that follow the server's OpenAPI schemas, and an error type per RFC
9457 problem `type` (#1129). Each SDK's black-box suite
(`scripts/sdk-test.sh <lang>`) fails when a live response carries a member its
model does not declare.

Clients capture `Maidan-Room-LSN` as `last_room_lsn` (Cluster 390). Since
Cluster 398.8 that value is **the caller's workspace head**, not the
instance's, so it is comparable to a `log_id` the client has actually seen.

**Publishing 0.3.0.** Push `sdk-ts-v0.3.0`, `sdk-py-v0.3.0`, `sdk-rs-v0.3.0`
and `sdk-go-v0.3.0` at the commit the server release is cut from. Each job
refuses a tag that differs from its package's version.

`cache-fixtures/cases.json` holds the inputs and outputs of the cache helpers
(boot prefix hash, breakpoints, cache keys, gateway sessions). All four SDK
suites read it, so the helpers give the same request parts in every language.
