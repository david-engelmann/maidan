# R-08 — CLI evidence

## What was checked (fresh clone, ca2ddd3, 2026-09-17)

`crates/maidan-cli/src/main.rs` end to end (310 lines) plus `Cargo.toml`. This supersedes the 399.3 pass: the facts were right, the interpretation was wrong.

## Findings

- **It is a server host, not a console.** `main` builds the store from `DATABASE_URL`, wires `LocalFsStore` artifacts, the search indexer (`Indexer::new(...).spawn()`), an in-memory bus, and serves MCP (`McpServer::new(store, artifacts, search, embedding_provider).with_event_bus(bus)`, stdio transport). There is no remote server for it to speak to — "rewrite as an HTTP client" (the audit's original recommendation) is incoherent for this binary. Retracted in INIT-11.
- **Direct store dependency** (true as reported): `maidan-store`, `maidan-auth`, `maidan-search`, `maidan-bus`, `maidan-mcp`, `maidan-artifacts`, `sqlx`. **No HTTP client** (true as reported): no `reqwest`/`ureq`.
- **Ambient auth with silent bypass default** (the actual finding): main.rs:218-224 —
  `MAIDAN_MCP_TOKEN` → `resolve_bearer(store, token)` → `AuthContext`; else `AuthContext::bypass()`. No warning logged, no flag required. The MCP tools it serves *do* enforce per-tool capabilities (`auth.require_capability(...)` throughout `crates/maidan-mcp/src/tools/`) — but only against the ambient context, and the default ambient context is omnipotent.
- **Bootstrap mint with `capability::all()`** (main.rs:153-161, the `init` path): legitimate — no token can exist before the first one — but unstated as the trust model.

## Interpretation (revised)

The architectural position is defensible: a local-operator binary that *is* the server for stdio MCP. What's not defensible is the *silent* part — nothing distinguishes "operator chose full authority" from "operator forgot the env var," and the trust model (one ambient bearer, or unbounded bypass; local tool, not multi-tenant server) is written nowhere. The fix is explicitness (opt-in bypass flag + loud logging + documented trust model), not a protocol rework. See INIT-11.

## What was not checked

- Whether `MAIDAN_MCP_TOKEN` appears in `--help` or any CLI README (not found in the fourth pass — verify before citing as absent).
- Whether anything else in the workspace constructs `AuthContext::bypass()` outside tests (INIT-07's question).
