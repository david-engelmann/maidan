# INIT-11 — CLI trust model (reframed fourth pass)

**Findings:** F-42 (P1, reframed)
**Research:** `research/R-08-cli-evidence.md`
**Repo state (2026-09-17):** the fourth pass read `crates/maidan-cli/src/main.rs` end to end (310 lines). The audit's framing was wrong in the way that matters, and this brief retracts its recommendation loudly rather than quietly.

## Retraction

The audit said: "`maidan-cli` opens the database directly instead of speaking the public HTTP API — rework it as an HTTP client." That recommendation assumed the CLI is an operator console that *could* speak HTTP to a server. It isn't. The CLI **is a server host**: it builds the store from `DATABASE_URL`, wires the search indexer and event bus, and serves MCP (`McpServer::new(store, ...)`, stdio transport). There is no remote server for it to speak to — in stdio mode it *is* the server. "Rewrite as an HTTP client" is incoherent for this binary; the corpus withdraws it. (F-42's factual substrate — `DATABASE_URL`, `maidan-store` dependency, no HTTP client — was true; the inference drawn from it was not.)

## Problem statement, reframed

The CLI's actual auth story, from `main.rs:218-224`:

```rust
let auth = if let Ok(token) = std::env::var("MAIDAN_MCP_TOKEN") {
    resolve_bearer(store.as_ref(), &token).await.context("resolve MAIDAN_MCP_TOKEN")?
} else {
    AuthContext::bypass()
};
```

1. **Silent full bypass by default (the sharp edge).** No `MAIDAN_MCP_TOKEN` → `AuthContext::bypass()` — every capability check in every MCP tool it serves becomes vacuous. No warning is logged, no flag is required, nothing distinguishes "operator chose full authority" from "operator forgot the env var." The MCP tools *do* enforce per-tool capabilities (`auth.require_capability(...)` in `crates/maidan-mcp/src/tools/`) — but only against the ambient context the CLI hands them, and the default ambient context is omnipotent.
2. **Ambient authority is undocumented as a trust boundary.** The CLI also mints a bootstrap token with `capability::all()` (main.rs:153-161, the `init` path). The model — one bearer for the whole local server, or unbounded bypass — is never stated as the intended trust model, so a reader can't tell which parts are deliberate and which are missing checks.
3. **`bypass()` in release-adjacent code.** INIT-07 asks whether `bypass()` should exist outside tests; the CLI is the concrete case. It is load-bearing here (local single-user operation), so the question is form, not existence.

## Why it matters

The CLI is the binary a new operator runs first (`maidan init`), and the stdio MCP server is how local agents connect. A silent-bypass default means the most common local setup — no token configured — runs every tool with full authority, and nothing in the logs says so. Anyone who later adds a token expecting least-privilege gets it; anyone who doesn't gets omnipotence without notice. This is the same "looks deliberate, isn't" shape as the old quickstart pin (INIT-01).

## Advisory recommendation (revised)

- **Make bypass explicit, not default.** Require an affirmative opt-in for full authority: e.g. `--bypass-auth` / `MAIDAN_BYPASS_AUTH=1`, and log a clear warning whenever bypass is active ("no capability checks are being enforced"). The `init` bootstrap path (which legitimately needs `capability::all()` before any token exists) should be the *only* silent exception, and it should say so.
- **Document the trust model** in the CLI's help/README: ambient bearer or explicit bypass; MCP tools enforce per-tool capabilities against that ambient context; this binary is a local-operator tool, not a multi-tenant server — do not expose its transports to a network.
- **Do not** pursue the withdrawn HTTP-client rework. If a *remote* operator CLI is ever wanted, that's a new binary, not a rework of this one.
- Alternatives considered: removing `bypass()` outright breaks legitimate local single-user use; keeping it silent preserves the current hazard. Explicit opt-in is the middle path the project's own idiom (fail-closed, loud decisions) points to.

## Open questions for the building agent

- Is `MAIDAN_MCP_TOKEN` documented anywhere in the CLI's `--help` or README? (Fourth pass didn't find it — verify.)
- Should the stdio transport (local agent, same machine) and any TCP transport have different auth defaults?
- Does anything else in the workspace construct `AuthContext::bypass()` outside tests? (INIT-07's auditability question.)

## Signals of resolution

- Running the CLI without a token requires an explicit bypass flag and logs the bypass loudly; or a token is required.
- The trust model is stated in the CLI help text and README.
- No silent `AuthContext::bypass()` remains on any user-facing path.
