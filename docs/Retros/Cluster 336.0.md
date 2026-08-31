# Cluster 336.0 retro — agent cold-start: whoami + initialize instructions (audit P1.3)

> Tag **`v336.0.0`**. Phase XXIV (post-gate hardening). **Cluster 5 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The cheapest adoption unlock the audit found: an agent handed only a base URL + token **could
not run the hero loop**, because every hero-loop tool needs the caller's own `member_id` yet
no self-discovery existed, and MCP `initialize` omitted the spec `instructions` field.

- **MCP `whoami` tool** (`tools/whoami.rs`) — returns `{member_id, workspace_id, capabilities,
  is_bearer, bypass}` straight from the request's `AuthContext` (no store access).
  `workspace:read`.
- **`initialize.instructions`** — the MCP `initialize` response now carries the spec
  `instructions` field: a one-paragraph cold-start guide ("call `whoami` first, then the
  six-tool hero loop: `claim_next_thread` → `get_thread_context` → work → `set_thread_result`
  → `wait_for_ready`/`wait_for_result`"), so an MCP client sees how to drive Maidan at connect.
- **`AuthContext::capabilities()`** accessor added (the field was private).

## Surprises / decisions

- **`whoami` reflects auth, not the store.** It reveals only the token's *own* identity, so it
  needs no store read and leaks nothing cross-tenant. Gated `workspace:read` (the base read
  cap every hero-loop agent already holds) rather than special-casing the capability gate.
- **Bearer vs session surfaced explicitly.** `is_bearer` tells an agent whether its token is an
  acts-as-any orchestrator bearer or a pinned session — the distinction that governs whether it
  may act as other members. `bypass` flags dev auth-disabled mode.
- **REST `GET /me` split to Cluster 337.** The MCP side is the agent transport the audit
  emphasized; the REST twin (with its new-route preflight) follows next, keeping this cluster
  MCP-focused.

## Test evidence

`whoami_returns_identity_and_initialize_carries_instructions` (a bearer `AuthContext`: `whoami`
returns the member/workspace/caps + `is_bearer=true`; `initialize` result carries an
`instructions` string mentioning `whoami` + `claim_next_thread`); full `maidan-mcp` lib suite
(60) + MCP contract-sync (85 tools) + `mcp_capability_matrix_e2e` green. fmt + strict clippy +
`--all-targets` + bootstrap-strip clean.

## Forward look

**337:** the REST `GET /me` twin. Then P1.4 post-path round-trips → P1.5 egress wire tests +
LSN replica CI → P2 docs/polish (gRPC doc contradiction, tool-count `78→85` drift,
Integration.md flagship-surface gaps, etc.).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
